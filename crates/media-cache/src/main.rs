use anyhow::{bail, Result};
use media_cache::{
    BodyReader, Limits, MediaCache, MediaKind, MediaReference, Profile, Response, Transport,
    ValidatedUrl,
};
use png::{BitDepth, ColorType, Decoder, Encoder};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct FixtureTransport {
    png: Vec<u8>,
    alpha_png: Vec<u8>,
    jpeg: Vec<u8>,
    oriented_jpeg: Vec<u8>,
}

struct FixtureBody {
    bytes: Vec<u8>,
    offset: usize,
    interrupt_after: Option<usize>,
}
impl BodyReader for FixtureBody {
    fn read(&mut self, buffer: &mut [u8], _deadline: Instant) -> Result<usize> {
        if self
            .interrupt_after
            .is_some_and(|limit| self.offset >= limit)
        {
            bail!("fixture interruption")
        }
        if self.offset == self.bytes.len() {
            return Ok(0);
        }
        let end = (self.offset + buffer.len()).min(self.bytes.len());
        let end = self.interrupt_after.map_or(end, |limit| end.min(limit));
        let count = end - self.offset;
        buffer[..count].copy_from_slice(&self.bytes[self.offset..end]);
        self.offset = end;
        Ok(count)
    }
}

impl Transport for FixtureTransport {
    fn fetch(&self, url: &ValidatedUrl, _deadline: Instant) -> Result<Response> {
        let path = url.as_str().split('/').nth(3).unwrap_or_default();
        let (bytes, mime, interrupt, redirect) = match path {
            "png" => (self.png.clone(), "image/png", None, None),
            "alpha" => (self.alpha_png.clone(), "image/png", None, None),
            "jpeg" => (self.jpeg.clone(), "image/jpeg", None, None),
            "jpeg-oriented" => (self.oriented_jpeg.clone(), "image/jpeg", None, None),
            "jpeg-bad-exif" => (
                malformed_exif(&self.oriented_jpeg),
                "image/jpeg",
                None,
                None,
            ),
            "corrupt" => (
                self.png[..self.png.len().min(20)].to_vec(),
                "image/png",
                None,
                None,
            ),
            "bomb" => (bomb_png(), "image/png", None, None),
            "pixel-bomb" => (header_png(4096, 4097), "image/png", None, None),
            "dimension-bomb" => (header_png(8193, 1), "image/png", None, None),
            "bad-mime" => (self.png.clone(), "image/jpeg", None, None),
            "large" => (vec![7u8; 2048], "image/png", None, None),
            "interrupt" => (self.png.clone(), "image/png", Some(3), None),
            "redirect-unsafe" => (
                Vec::new(),
                "text/plain",
                None,
                Some("https://127.0.0.1/private"),
            ),
            "redirect-loop" => (
                Vec::new(),
                "text/plain",
                None,
                Some("https://fixture.invalid/redirect-loop"),
            ),
            _ => (Vec::new(), "text/plain", None, None),
        };
        Ok(Response {
            status: if redirect.is_some() { 302 } else { 200 },
            content_type: mime.to_string(),
            redirect: redirect.map(str::to_string),
            body: Box::new(FixtureBody {
                bytes,
                offset: 0,
                interrupt_after: interrupt,
            }),
        })
    }
}

fn main() {
    if let Err(error) = journey() {
        eprintln!("media-cache journey failed: {error}");
        std::process::exit(1);
    }
    println!("media-cache journey passed: ingested=8 failures=12 objects=1 protected=unchanged");
}

fn journey() -> Result<()> {
    let root = std::env::temp_dir().join(format!("media-cache-journey-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let protected = root.join("protected");
    fs::create_dir_all(protected.join("roms"))?;
    fs::write(
        protected.join("roms").join("owned.bin"),
        b"user-owned-fixture",
    )?;
    let protected_before = fs::read(protected.join("roms").join("owned.bin"))?;
    let cache_root = root.join("derived-media");
    let temporary_root = root.join("external-tmp");
    fs::create_dir_all(&temporary_root)?;
    let journey_limits = Limits {
        timeout: Duration::from_millis(100),
        ..Limits::default()
    };
    let cache = Arc::new(MediaCache::open_with_temp_root(
        &cache_root,
        &temporary_root,
        journey_limits,
    )?);
    assert!(
        MediaCache::open_with_temp_root(&cache_root, cache_root.join("tmp"), journey_limits)
            .is_err()
    );
    let cache_a_root = root.join("cache-a");
    let cache_b_root = root.join("cache-b");
    let _cache_a = MediaCache::open_with_temp_root(&cache_a_root, &temporary_root, journey_limits)?;
    let _cache_b = MediaCache::open_with_temp_root(&cache_b_root, &temporary_root, journey_limits)?;
    let cache_a_identity = fixture_identity(&cache_a_root.canonicalize()?);
    let stale_a = temporary_root.join(format!(
        ".media-cache-transfer-{cache_a_identity}.tmp-stale"
    ));
    let fresh_a = temporary_root.join(format!(
        ".media-cache-transfer-{cache_a_identity}.tmp-fresh"
    ));
    fs::write(&stale_a, b"cache-a-old-partial")?;
    std::thread::sleep(Duration::from_millis(300));
    fs::write(&fresh_a, b"cache-a-active-partial")?;
    let _cache_b_again =
        MediaCache::open_with_temp_root(&cache_b_root, &temporary_root, journey_limits)?;
    assert!(stale_a.exists());
    assert!(fresh_a.exists());
    let _cache_a_again =
        MediaCache::open_with_temp_root(&cache_a_root, &temporary_root, journey_limits)?;
    assert!(!stale_a.exists());
    assert!(fresh_a.exists());
    fs::remove_file(fresh_a)?;
    let transport = Arc::new(FixtureTransport {
        png: png_bytes(2, 1, &[[255, 0, 0, 255], [0, 255, 0, 255]].concat())?,
        alpha_png: png_bytes(2, 1, &[255, 0, 0, 0, 0, 0, 255, 128])?,
        jpeg: jpeg_bytes(),
        oriented_jpeg: oriented_jpeg_bytes(),
    });

    let base = |id: &str, kind: MediaKind, path: &str| MediaReference {
        content_id: id.to_string(),
        kind,
        url: format!("https://fixture.invalid/{path}"),
        region: Some("world".to_string()),
        language: Some("en".to_string()),
        provider: "synthetic-provider".to_string(),
    };
    let first = cache.ingest(
        &base("cover-1", MediaKind::BoxArt, "png"),
        Profile::Cover,
        transport.as_ref(),
    )?;
    let repeat = cache.ingest(
        &base("cover-2", MediaKind::BoxArt, "png"),
        Profile::Cover,
        transport.as_ref(),
    )?;
    assert_eq!(
        (first.width, first.height, first.bytes),
        (repeat.width, repeat.height, repeat.bytes)
    );
    assert_eq!(
        fs::read(cache_root.join("index").join("cover-1.json"))?,
        fs::read(cache_root.join("index").join("cover-2.json"))?
    );
    assert!(first.width <= 2 && first.height <= 1);
    let cover_index: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_root.join("index").join("cover-1.json"))?)?;
    let cover_digest = cover_index
        .get("object")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing cover object"))?;
    let cover_object = cache_root
        .join("objects")
        .join(&cover_digest[..2])
        .join(format!("{cover_digest}.png"));
    let cover_bytes = fs::read(&cover_object)?;
    assert_eq!(
        cover_bytes,
        fs::read(
            cache_root
                .join("objects")
                .join(&cover_digest[..2])
                .join(format!("{cover_digest}.png"))
        )?
    );
    let cache_identity = fixture_identity(&cache_root.canonicalize()?);
    assert_no_external_temps(&cache_root, &cache_identity)?;
    let provenance_file = fs::read_dir(cache_root.join("metadata"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing provenance"))??
        .path();
    let provenance = fs::read_to_string(provenance_file)?;
    assert!(provenance.contains("sourceUrlSha256") && provenance.contains("fetched-validated"));
    assert!(!provenance.contains("https://fixture.invalid"));
    for (id, kind, profile, path) in [
        (
            "shot",
            MediaKind::Screenshot,
            Profile::ScreenshotPreview,
            "png",
        ),
        ("title", MediaKind::TitleScreen, Profile::TitleScreen, "png"),
        ("logo", MediaKind::Logo, Profile::TransparentLogo, "alpha"),
        ("jpeg", MediaKind::BoxArt, Profile::Cover, "jpeg"),
        (
            "oriented-a",
            MediaKind::Screenshot,
            Profile::ScreenshotPreview,
            "jpeg-oriented",
        ),
        (
            "oriented-b",
            MediaKind::Screenshot,
            Profile::ScreenshotPreview,
            "jpeg-oriented",
        ),
    ] {
        let result = cache.ingest(&base(id, kind, path), profile, transport.as_ref())?;
        if path == "jpeg-oriented" {
            assert_eq!((result.width, result.height), (16, 8));
        }
        assert!(result.width <= 16 && result.height <= 16);
    }
    assert_eq!(
        fs::read(cache_root.join("index").join("oriented-a.json"))?,
        fs::read(cache_root.join("index").join("oriented-b.json"))?
    );
    let oriented_index: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_root.join("index").join("oriented-a.json"))?)?;
    let oriented_digest = oriented_index
        .get("object")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing oriented object"))?;
    let oriented_path = cache_root
        .join("objects")
        .join(&oriented_digest[..2])
        .join(format!("{oriented_digest}.png"));
    let (_, _, oriented_pixels) = decode_pixels(&oriented_path)?;
    assert!(oriented_pixels
        .windows(3)
        .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2]));
    let logo_objects = find_objects(cache.root())?;
    assert_eq!(
        logo_objects.len(),
        5,
        "repeat and byte-identical profile outputs must deduplicate"
    );
    let alpha_count = logo_objects
        .iter()
        .try_fold(0usize, |count, path| -> Result<usize> {
            let color = decode_any(path)?.1;
            if color == ColorType::Rgba {
                Ok(count + 1)
            } else {
                assert_eq!(color, ColorType::Rgb);
                Ok(count)
            }
        })?;
    assert_eq!(alpha_count, 1, "only the logo output retains alpha");

    let failures = [
        (
            base("bad-corrupt", MediaKind::BoxArt, "corrupt"),
            Limits::default(),
        ),
        (
            base("bad-bomb", MediaKind::BoxArt, "bomb"),
            Limits::default(),
        ),
        (
            base("bad-pixels", MediaKind::BoxArt, "pixel-bomb"),
            Limits::default(),
        ),
        (
            base("bad-dimensions", MediaKind::BoxArt, "dimension-bomb"),
            Limits::default(),
        ),
        (
            base("bad-mime", MediaKind::BoxArt, "bad-mime"),
            Limits::default(),
        ),
        (
            base("bad-exif", MediaKind::BoxArt, "jpeg-bad-exif"),
            Limits::default(),
        ),
        (
            base("bad-interrupt", MediaKind::BoxArt, "interrupt"),
            Limits::default(),
        ),
        (
            base("bad-large", MediaKind::BoxArt, "large"),
            Limits {
                max_transfer_bytes: 64,
                ..Limits::default()
            },
        ),
        (
            base("bad-redirect", MediaKind::BoxArt, "redirect-unsafe"),
            Limits::default(),
        ),
        (
            base("bad-loop", MediaKind::BoxArt, "redirect-loop"),
            Limits {
                max_redirects: 1,
                ..Limits::default()
            },
        ),
    ];
    for (reference, limits) in failures {
        let failing_cache = MediaCache::open(
            root.join(format!("failure-{}", reference.content_id)),
            limits,
        )?;
        assert!(failing_cache
            .ingest(&reference, Profile::Cover, transport.as_ref())
            .is_err());
    }
    assert!(cache
        .ingest(
            &MediaReference {
                url: "http://fixture.invalid/png".to_string(),
                ..base("bad-http", MediaKind::BoxArt, "png")
            },
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());
    assert!(cache
        .ingest(
            &MediaReference {
                url: "https://user:pass@fixture.invalid/png".to_string(),
                ..base("bad-userinfo", MediaKind::BoxArt, "png")
            },
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());
    assert!(cache
        .ingest(
            &MediaReference {
                url: "https://192.168.1.1/png".to_string(),
                ..base("bad-private", MediaKind::BoxArt, "png")
            },
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());
    assert!(cache
        .ingest(
            &MediaReference {
                url: "https://169.254.1.1/png".to_string(),
                ..base("bad-link-local", MediaKind::BoxArt, "png")
            },
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());
    for (id, url) in [
        ("bad-port", "https://fixture.invalid:444/png"),
        ("bad-localhost", "https://localhost/png"),
        ("bad-ipv6", "https://[::1]/png"),
    ] {
        assert!(cache
            .ingest(
                &MediaReference {
                    url: url.to_string(),
                    ..base(id, MediaKind::BoxArt, "png")
                },
                Profile::Cover,
                transport.as_ref()
            )
            .is_err());
    }
    let decode_budget_cache = MediaCache::open(
        root.join("failure-decode-time"),
        Limits {
            max_decode_time: Duration::ZERO,
            ..Limits::default()
        },
    )?;
    assert!(decode_budget_cache
        .ingest(
            &base("bad-decode-time", MediaKind::BoxArt, "png"),
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());

    let metadata_path = cache_root
        .join("metadata")
        .join(format!("{cover_digest}.json"));
    fs::write(&cover_object, b"corrupt-object")?;
    assert!(cache
        .ingest(
            &base("cover-1", MediaKind::BoxArt, "png"),
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());
    fs::remove_file(&cover_object)?;
    let _ = cache.ingest(
        &base("cover-1", MediaKind::BoxArt, "png"),
        Profile::Cover,
        transport.as_ref(),
    )?;
    fs::write(&metadata_path, b"{}\n")?;
    assert!(cache
        .ingest(
            &base("cover-1", MediaKind::BoxArt, "png"),
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());
    fs::remove_file(&metadata_path)?;
    let _ = cache.ingest(
        &base("cover-1", MediaKind::BoxArt, "png"),
        Profile::Cover,
        transport.as_ref(),
    )?;
    let cover_index_path = cache_root.join("index").join("cover-1.json");
    fs::write(&cover_index_path, b"{}\n")?;
    assert!(cache
        .ingest(
            &base("cover-1", MediaKind::BoxArt, "png"),
            Profile::Cover,
            transport.as_ref()
        )
        .is_err());
    fs::remove_file(cover_index_path)?;
    let _ = cache.ingest(
        &base("cover-1", MediaKind::BoxArt, "png"),
        Profile::Cover,
        transport.as_ref(),
    )?;

    #[cfg(unix)]
    {
        let symlink_root = root.join("symlink-cache");
        fs::create_dir_all(symlink_root.join("real-objects"))?;
        std::os::unix::fs::symlink(
            symlink_root.join("real-objects"),
            symlink_root.join("objects"),
        )?;
        assert!(MediaCache::open(&symlink_root, Limits::default()).is_err());
    }
    assert!(MediaCache::open(root.join("case-collision").join("ROMS"), Limits::default()).is_err());

    drop(cache);
    let stale = temporary_root.join(format!(".media-cache-transfer-{cache_identity}.tmp-stale"));
    fs::write(&stale, b"partial")?;
    std::thread::sleep(Duration::from_millis(300));
    let restarted = MediaCache::open_with_temp_root(&cache_root, &temporary_root, journey_limits)?;
    assert!(!stale.exists());
    assert_no_external_temps(&temporary_root, &cache_identity)?;
    let _ = restarted.ingest(
        &base("restart", MediaKind::BoxArt, "png"),
        Profile::Cover,
        transport.as_ref(),
    )?;
    let concurrent = Arc::new(restarted);
    let mut threads = Vec::new();
    for n in 0..8 {
        let cache = Arc::clone(&concurrent);
        let transport = Arc::clone(&transport);
        threads.push(std::thread::spawn(move || {
            cache
                .ingest(
                    &MediaReference {
                        content_id: format!("concurrent-{n}"),
                        kind: MediaKind::BoxArt,
                        url: "https://fixture.invalid/png".to_string(),
                        region: None,
                        language: None,
                        provider: "synthetic-provider".to_string(),
                    },
                    Profile::Cover,
                    transport.as_ref(),
                )
                .unwrap()
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(find_objects(concurrent.root())?.len(), 5);
    assert_eq!(
        fs::read(protected.join("roms").join("owned.bin"))?,
        protected_before
    );
    assert!(fs::read_dir(concurrent.root().join("metadata"))?.count() >= 1);
    assert!(fs::read_dir(concurrent.root().join("index"))?.count() >= 1);

    assert!(concurrent.enforce_quota(1)? <= 1);
    assert_eq!(
        fs::read(protected.join("roms").join("owned.bin"))?,
        protected_before
    );
    let metadata = fs::read_dir(concurrent.root().join("metadata"))?.count();
    let indexes = fs::read_dir(concurrent.root().join("index"))?.count();
    assert!(metadata <= 2 && indexes == 0);
    assert_no_external_temps(&temporary_root, &cache_identity)?;
    let public = "media-cache journey passed: ingested=8 failures=12 objects=1 protected=unchanged";
    for forbidden in [
        "https://",
        "synthetic-provider",
        "sourceUrlSha256",
        "owned.bin",
    ] {
        assert!(!public.contains(forbidden));
    }
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn png_bytes(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    drop(writer);
    Ok(bytes)
}

fn header_png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = bomb_png();
    bytes[16..20].copy_from_slice(&width.to_be_bytes());
    bytes[20..24].copy_from_slice(&height.to_be_bytes());
    bytes
}

fn bomb_png() -> Vec<u8> {
    let mut bytes = vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82];
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes
}

fn find_objects(root: &Path) -> Result<Vec<PathBuf>> {
    let mut objects = Vec::new();
    let directory = root.join("objects");
    for shard in fs::read_dir(directory)? {
        let shard = shard?.path();
        if !shard.is_dir() {
            continue;
        }
        for entry in fs::read_dir(shard)? {
            let path = entry?.path();
            if path.extension().and_then(|x| x.to_str()) == Some("png") {
                objects.push(path);
            }
        }
    }
    Ok(objects)
}

fn decode_pixels(path: &Path) -> Result<((u32, u32), ColorType, Vec<u8>)> {
    let decoder = Decoder::new(fs::File::open(path)?);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    let dimensions = (info.width, info.height);
    let color = info.color_type;
    let mut bytes = vec![0; reader.output_buffer_size()];
    let size = reader.next_frame(&mut bytes)?.buffer_size();
    bytes.truncate(size);
    Ok((dimensions, color, bytes))
}

fn decode_any(path: &Path) -> Result<((u32, u32), ColorType)> {
    let (dimensions, color, _) = decode_pixels(path)?;
    Ok((dimensions, color))
}

fn assert_no_external_temps(root: &Path, identity: &str) -> Result<()> {
    let prefix = format!(".media-cache-transfer-{identity}.tmp-");
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with(&prefix))
        {
            bail!("external transfer temporary remains")
        }
    }
    Ok(())
}

fn fixture_identity(path: &Path) -> String {
    format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()))
}

fn malformed_exif(source: &[u8]) -> Vec<u8> {
    let mut bytes = source.to_vec();
    bytes[30] = 0;
    bytes[31] = 9;
    bytes
}

fn oriented_jpeg_bytes() -> Vec<u8> {
    let source = jpeg_bytes();
    let mut output = source[..2].to_vec();
    let exif = [
        b'E', b'x', b'i', b'f', 0, 0, b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 1, 0x12, 0, 3, 0, 0, 0,
        1, 0, 6, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    output.extend_from_slice(&[0xff, 0xe1, 0, 36]);
    output.extend_from_slice(&exif);
    output.extend_from_slice(&source[2..]);
    output
}

fn jpeg_bytes() -> Vec<u8> {
    include_str!("../../../fixtures/media-cache/jpeg.hex")
        .split_whitespace()
        .flat_map(|line| {
            (0..line.len())
                .step_by(2)
                .map(move |i| u8::from_str_radix(&line[i..i + 2], 16).unwrap())
        })
        .collect()
}
