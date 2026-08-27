use anyhow::{anyhow, bail, Result};
use png::{BitDepth, ColorType, Decoder, Encoder, Transformations};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const MAX_TRANSFER_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_WIDTH: u32 = 8192;
pub const MAX_HEIGHT: u32 = 8192;
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
pub const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 5;
pub const MAX_URL_BYTES: usize = 2048;
pub const MAX_AUTHORITY_BYTES: usize = 255;
pub const MAX_PATH_BYTES: usize = 1536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaKind {
    BoxArt,
    Screenshot,
    TitleScreen,
    Logo,
}

impl MediaKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::BoxArt => "box-art",
            Self::Screenshot => "screenshot",
            Self::TitleScreen => "title-screen",
            Self::Logo => "logo",
        }
    }
}

#[derive(Clone, Debug)]
pub struct MediaReference {
    pub content_id: String,
    pub kind: MediaKind,
    pub url: String,
    pub region: Option<String>,
    pub language: Option<String>,
    pub provider: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedUrl(String);

impl ValidatedUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicErrorCategory {
    InvalidInput,
    UnsafeUrl,
    Transport,
    Image,
    Storage,
    Quota,
}

impl PublicErrorCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid-input",
            Self::UnsafeUrl => "unsafe-url",
            Self::Transport => "transport",
            Self::Image => "image",
            Self::Storage => "storage",
            Self::Quota => "quota",
        }
    }
}

#[derive(Debug)]
pub struct CacheError {
    pub category: PublicErrorCategory,
    detail: String,
}

impl CacheError {
    fn new(category: PublicErrorCategory, detail: impl Into<String>) -> Self {
        Self {
            category,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.detail)
    }
}

impl std::error::Error for CacheError {}

pub trait BodyReader: Send {
    fn read(&mut self, buffer: &mut [u8], deadline: Instant) -> Result<usize>;
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub redirect: Option<String>,
    pub body: Box<dyn BodyReader>,
}

pub trait Transport: Send + Sync {
    fn fetch(&self, url: &ValidatedUrl, deadline: Instant) -> Result<Response>;
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_transfer_bytes: u64,
    pub timeout: Duration,
    pub max_decode_time: Duration,
    pub max_redirects: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_transfer_bytes: MAX_TRANSFER_BYTES,
            timeout: Duration::from_secs(20),
            max_decode_time: Duration::from_secs(5),
            max_redirects: MAX_REDIRECTS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Cover,
    ScreenshotPreview,
    TitleScreen,
    TransparentLogo,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::ScreenshotPreview => "screenshot-preview",
            Self::TitleScreen => "title-screen",
            Self::TransparentLogo => "transparent-logo",
        }
    }

    fn allow_alpha(self) -> bool {
        self == Self::TransparentLogo
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestedMedia {
    pub content_id: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub provider: String,
    #[serde(rename = "sourceUrlSha256")]
    pub source_url_sha256: String,
    #[serde(rename = "mediaKind")]
    pub media_kind: String,
    #[serde(rename = "outputWidth")]
    pub output_width: u32,
    #[serde(rename = "outputHeight")]
    pub output_height: u32,
    #[serde(rename = "byteSize")]
    pub byte_size: u64,
    #[serde(rename = "objectSha256")]
    pub object_sha256: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexEntry {
    object: String,
    width: u32,
    height: u32,
    bytes: u64,
}

pub struct MediaCache {
    root: PathBuf,
    identity: String,
    temporary_root: PathBuf,
    limits: Limits,
}

static TRANSFER_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static OBJECT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const EXTERNAL_TEMP_STALE_MULTIPLIER: u32 = 2;

impl MediaCache {
    pub fn open(root: impl AsRef<Path>, limits: Limits) -> Result<Self> {
        Self::open_with_temp_root(root, std::env::temp_dir(), limits)
    }

    pub fn open_with_temp_root(
        root: impl AsRef<Path>,
        temporary_root: impl AsRef<Path>,
        limits: Limits,
    ) -> Result<Self> {
        let root = validate_cache_root(root.as_ref())?;
        let identity = hex_digest(root.to_string_lossy().as_bytes());
        let temporary_root = validate_external_temp_root(&root, temporary_root.as_ref())?;
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("metadata"))?;
        fs::create_dir_all(root.join("index"))?;
        reject_cache_symlinks(&root)?;
        cleanup_temporary_files(&root)?;
        cleanup_external_temporary_files(
            &temporary_root,
            &identity,
            external_temp_stale_after(limits.timeout),
        )?;
        sync_dir(&root)?;
        sync_dir(&temporary_root)?;
        Ok(Self {
            root,
            identity,
            temporary_root,
            limits,
        })
    }

    pub fn ingest(
        &self,
        reference: &MediaReference,
        profile: Profile,
        transport: &dyn Transport,
    ) -> Result<IngestedMedia> {
        validate_reference(reference)?;
        let mut url = validate_url(&reference.url).map_err(|e| anyhow!(e))?;
        let transfer = self.download(&mut url, transport)?;
        let decode_deadline = Instant::now() + self.limits.max_decode_time;
        let image = decode_image(&transfer.path, &transfer.mime, decode_deadline)
            .map_err(|e| anyhow!(e))?;
        let image = resize(image, profile).map_err(|e| anyhow!(e))?;
        let output = encode_png(&image, profile).map_err(|e| anyhow!(e))?;
        let digest = hex_digest(&output);
        let object = object_path(&self.root, &digest);
        let metadata = metadata_path(&self.root, &digest);
        let index = index_path(&self.root, &reference.content_id);
        let provenance = Provenance {
            provider: reference.provider.clone(),
            source_url_sha256: hex_digest(reference.url.as_bytes()),
            media_kind: reference.kind.as_str().to_string(),
            output_width: image.width,
            output_height: image.height,
            byte_size: output.len() as u64,
            object_sha256: digest.clone(),
            status: "fetched-validated".to_string(),
        };
        let result = (|| {
            let _guard = OBJECT_LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .map_err(|_| anyhow!("storage lock poisoned"))?;
            publish_object(&object, &output, &digest)?;
            atomic_provenance(&metadata, &provenance)?;
            atomic_json(
                &index,
                &IndexEntry {
                    object: digest,
                    width: image.width,
                    height: image.height,
                    bytes: output.len() as u64,
                },
            )?;
            Ok(IngestedMedia {
                content_id: reference.content_id.clone(),
                width: image.width,
                height: image.height,
                bytes: output.len() as u64,
            })
        })();
        let _ = fs::remove_file(&transfer.path);
        result
    }

    fn download(&self, url: &mut ValidatedUrl, transport: &dyn Transport) -> Result<Transfer> {
        let (temporary, mut file) = create_temp_file(
            &self.temporary_root,
            &format!("media-cache-transfer-{}", self.identity),
        )?;
        let start = Instant::now();
        let mut redirects = 0;
        let mut mime = String::new();
        let result = (|| loop {
            if redirects > self.limits.max_redirects {
                return Err(anyhow!(CacheError::new(
                    PublicErrorCategory::Transport,
                    "redirect limit exceeded"
                )));
            }
            *url = validate_url(url.as_str()).map_err(|e| anyhow!(e))?;
            let response = transport
                .fetch(url, start + self.limits.timeout)
                .map_err(|_| {
                    anyhow!(CacheError::new(
                        PublicErrorCategory::Transport,
                        "transport failure"
                    ))
                })?;
            if (300..400).contains(&response.status) {
                let location = response.redirect.ok_or_else(|| {
                    anyhow!(CacheError::new(
                        PublicErrorCategory::Transport,
                        "redirect missing location"
                    ))
                })?;
                *url = validate_url(&location).map_err(|_| {
                    anyhow!(CacheError::new(
                        PublicErrorCategory::UnsafeUrl,
                        "unsafe redirect"
                    ))
                })?;
                redirects += 1;
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(anyhow!(CacheError::new(
                    PublicErrorCategory::Transport,
                    "unexpected response status"
                )));
            }
            mime = response.content_type.trim().to_ascii_lowercase();
            if mime != "image/png" && mime != "image/jpeg" {
                return Err(anyhow!(CacheError::new(
                    PublicErrorCategory::Image,
                    "unsupported media type"
                )));
            }
            let mut body = response.body;
            let mut buffer = [0u8; 16 * 1024];
            let mut total = 0u64;
            loop {
                if Instant::now() >= start + self.limits.timeout {
                    return Err(anyhow!(CacheError::new(
                        PublicErrorCategory::Transport,
                        "transfer timeout"
                    )));
                }
                let count = body
                    .read(&mut buffer, start + self.limits.timeout)
                    .map_err(|_| {
                        anyhow!(CacheError::new(
                            PublicErrorCategory::Transport,
                            "body interrupted"
                        ))
                    })?;
                if count == 0 {
                    break;
                }
                if count > buffer.len() {
                    return Err(anyhow!(CacheError::new(
                        PublicErrorCategory::Transport,
                        "body reader returned invalid length"
                    )));
                }
                total = total.checked_add(count as u64).ok_or_else(|| {
                    anyhow!(CacheError::new(
                        PublicErrorCategory::Transport,
                        "transfer too large"
                    ))
                })?;
                if total > self.limits.max_transfer_bytes {
                    return Err(anyhow!(CacheError::new(
                        PublicErrorCategory::Transport,
                        "transfer too large"
                    )));
                }
                file.write_all(&buffer[..count])?;
            }
            file.sync_all()?;
            return Ok(Transfer {
                path: temporary.clone(),
                mime: mime.clone(),
            });
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn enforce_quota(&self, max_bytes: u64) -> Result<u64> {
        let _guard = OBJECT_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| anyhow!("storage lock poisoned"))?;
        let objects = self.root.join("objects");
        let mut entries = Vec::new();
        for shard in fs::read_dir(&objects)? {
            let shard = shard?.path();
            if !shard.is_dir()
                || shard
                    .file_name()
                    .and_then(|x| x.to_str())
                    .is_none_or(|x| x.len() != 2 || !x.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                continue;
            }
            for entry in fs::read_dir(shard)? {
                let path = entry?.path();
                if path.extension().and_then(|x| x.to_str()) != Some("png") {
                    continue;
                }
                let meta = fs::metadata(&path)?;
                entries.push((meta.modified().unwrap_or(UNIX_EPOCH), meta.len(), path));
            }
        }
        entries.sort_by_key(|x| x.0);
        let mut total: u64 = entries.iter().map(|x| x.1).sum();
        for (_, size, path) in entries {
            if total <= max_bytes {
                break;
            }
            let digest = path
                .file_stem()
                .and_then(|x| x.to_str())
                .ok_or_else(|| anyhow!("unsafe object name"))?
                .to_string();
            fs::remove_file(&path)?;
            let _ = fs::remove_file(metadata_path(&self.root, &digest));
            remove_indexes_for_digest(&self.root, &digest)?;
            total -= size;
        }
        sync_dir(&objects)?;
        Ok(total)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

struct Transfer {
    path: PathBuf,
    mime: String,
}

impl Drop for Transfer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn validate_reference(reference: &MediaReference) -> Result<()> {
    if !valid_opaque(&reference.content_id, 128) || !valid_opaque(&reference.provider, 128) {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::InvalidInput,
            "invalid media reference"
        )));
    }
    if reference.url.len() > MAX_URL_BYTES
        || reference
            .region
            .as_ref()
            .is_some_and(|x| !valid_opaque(x, 32))
        || reference
            .language
            .as_ref()
            .is_some_and(|x| !valid_opaque(x, 32))
    {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::InvalidInput,
            "invalid media reference"
        )));
    }
    Ok(())
}

fn validate_url(value: &str) -> Result<ValidatedUrl, CacheError> {
    if value.len() > MAX_URL_BYTES || !value.is_ascii() || !value.starts_with("https://") {
        return Err(CacheError::new(
            PublicErrorCategory::UnsafeUrl,
            "URL rejected",
        ));
    }
    let rest = &value[8..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let path = &rest[authority_end..];
    if authority.is_empty()
        || authority.len() > MAX_AUTHORITY_BYTES
        || path.len() > MAX_PATH_BYTES
        || authority.contains('@')
        || authority.contains(['\\', ' '])
        || path.contains('\\')
    {
        return Err(CacheError::new(
            PublicErrorCategory::UnsafeUrl,
            "URL rejected",
        ));
    }
    if authority.contains('?') || authority.contains('#') || path.contains('#') {
        return Err(CacheError::new(
            PublicErrorCategory::UnsafeUrl,
            "URL rejected",
        ));
    }
    let bracketed = authority.starts_with('[');
    let host = if bracketed {
        let end = authority
            .find(']')
            .ok_or_else(|| CacheError::new(PublicErrorCategory::UnsafeUrl, "URL rejected"))?;
        if end + 1 < authority.len() && !authority[end + 1..].starts_with(":443") {
            return Err(CacheError::new(
                PublicErrorCategory::UnsafeUrl,
                "URL rejected",
            ));
        }
        &authority[1..end]
    } else {
        let (host, port) = authority
            .rsplit_once(':')
            .map_or((authority, None), |(h, p)| (h, Some(p)));
        if port.is_some_and(|port| port != "443") {
            return Err(CacheError::new(
                PublicErrorCategory::UnsafeUrl,
                "URL rejected",
            ));
        }
        host
    };
    if host.is_empty() || host.len() > 253 || host.eq_ignore_ascii_case("localhost") {
        return Err(CacheError::new(
            PublicErrorCategory::UnsafeUrl,
            "URL rejected",
        ));
    }
    if bracketed {
        let ip = host
            .parse::<IpAddr>()
            .map_err(|_| CacheError::new(PublicErrorCategory::UnsafeUrl, "URL rejected"))?;
        if !public_ip(ip) {
            return Err(CacheError::new(
                PublicErrorCategory::UnsafeUrl,
                "URL rejected",
            ));
        }
    } else {
        if host.ends_with('.')
            || host.split('.').any(|part| {
                part.is_empty()
                    || part.len() > 63
                    || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(CacheError::new(
                PublicErrorCategory::UnsafeUrl,
                "URL rejected",
            ));
        }
        if host.parse::<IpAddr>().is_ok_and(|ip| !public_ip(ip)) {
            return Err(CacheError::new(
                PublicErrorCategory::UnsafeUrl,
                "URL rejected",
            ));
        }
    }
    Ok(ValidatedUrl(value.to_string()))
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && ip != Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && !ip.segments().starts_with(&[0, 0, 0, 0, 0, 0xff])
                && ip != Ipv6Addr::LOCALHOST
        }
    }
}

fn decode_image(path: &Path, mime: &str, deadline: Instant) -> Result<Image, CacheError> {
    check_decode_deadline(deadline)?;
    let signature = read_prefix(path, 16)
        .map_err(|_| CacheError::new(PublicErrorCategory::Image, "image unreadable"))?;
    match (mime, signature.as_slice()) {
        ("image/png", [137, 80, 78, 71, 13, 10, 26, 10, ..]) => decode_png(path, deadline),
        ("image/jpeg", [255, 216, ..]) => decode_jpeg(path, deadline),
        ("image/png", _) | ("image/jpeg", _) => Err(CacheError::new(
            PublicErrorCategory::Image,
            "declared media does not match content",
        )),
        _ => Err(CacheError::new(
            PublicErrorCategory::Image,
            "unsupported media type",
        )),
    }
}

fn check_decode_deadline(deadline: Instant) -> Result<(), CacheError> {
    if Instant::now() >= deadline {
        Err(CacheError::new(
            PublicErrorCategory::Image,
            "decode timeout",
        ))
    } else {
        Ok(())
    }
}

fn decode_png(path: &Path, deadline: Instant) -> Result<Image, CacheError> {
    let file = File::open(path)
        .map_err(|_| CacheError::new(PublicErrorCategory::Image, "image unreadable"))?;
    let mut decoder = Decoder::new_with_limits(
        file,
        png::Limits {
            bytes: MAX_DECODED_BYTES as usize,
        },
    );
    decoder.set_ignore_text_chunk(true);
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|_| CacheError::new(PublicErrorCategory::Image, "invalid PNG"))?;
    let (width, height, input_color, animated) = {
        let info = reader.info();
        (
            info.width,
            info.height,
            info.color_type,
            info.animation_control.is_some(),
        )
    };
    if animated {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "animated images are not supported",
        ));
    }
    if !matches!(
        input_color,
        ColorType::Grayscale | ColorType::Rgb | ColorType::GrayscaleAlpha | ColorType::Rgba
    ) {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "unsupported PNG color type",
        ));
    }
    check_dimensions(width, height)?;
    let (color, depth) = reader.output_color_type();
    if depth != BitDepth::Eight {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "unsupported PNG depth",
        ));
    }
    let size = checked_bytes(width, height)?;
    let mut raw = vec![0u8; reader.output_buffer_size()];
    let output = reader
        .next_frame(&mut raw)
        .map_err(|_| CacheError::new(PublicErrorCategory::Image, "invalid PNG data"))?;
    check_decode_deadline(deadline)?;
    let pixels = to_rgba(&raw[..output.buffer_size()], width, height, color)?;
    if pixels.len() > size as usize {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "decoded image too large",
        ));
    }
    Ok(Image {
        width,
        height,
        pixels,
    })
}

fn to_rgba(raw: &[u8], width: u32, height: u32, color: ColorType) -> Result<Vec<u8>, CacheError> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| CacheError::new(PublicErrorCategory::Image, "decoded image too large"))?;
    let mut output = Vec::with_capacity(pixels * 4);
    match color {
        ColorType::Rgb => {
            for chunk in raw.chunks_exact(3) {
                output.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
        }
        ColorType::Rgba => output.extend_from_slice(raw),
        ColorType::Grayscale => {
            for &value in raw {
                output.extend_from_slice(&[value, value, value, 255]);
            }
        }
        ColorType::GrayscaleAlpha => {
            for chunk in raw.chunks_exact(2) {
                output.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
        }
        ColorType::Indexed => {
            return Err(CacheError::new(
                PublicErrorCategory::Image,
                "unsupported PNG color type",
            ))
        }
    }
    if output.len() != pixels * 4 {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "invalid decoded image",
        ));
    }
    Ok(output)
}

fn checked_bytes(width: u32, height: u32) -> Result<u64, CacheError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| CacheError::new(PublicErrorCategory::Image, "decoded image too large"))?;
    if pixels == 0 || pixels > MAX_PIXELS {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "decoded image too large",
        ));
    }
    let bytes = pixels
        .checked_mul(4)
        .ok_or_else(|| CacheError::new(PublicErrorCategory::Image, "decoded image too large"))?;
    if bytes > MAX_DECODED_BYTES {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "decoded image too large",
        ));
    }
    Ok(bytes)
}

fn check_dimensions(width: u32, height: u32) -> Result<(), CacheError> {
    if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "image dimensions rejected",
        ));
    }
    checked_bytes(width, height).map(|_| ())
}

fn resize(image: Image, profile: Profile) -> Result<Image, CacheError> {
    let (max_w, max_h) = (1024u32, 768u32);
    let (mut source, crop) = if profile == Profile::Cover {
        let source_ratio = image.width as f64 / image.height as f64;
        let target_ratio = max_w as f64 / max_h as f64;
        if source_ratio > target_ratio {
            (fit_size(&image, max_h, max_h), true)
        } else {
            (fit_size(&image, max_w, max_w), true)
        }
    } else {
        (fit_size(&image, max_w, max_h), false)
    };
    if crop {
        let target_ratio = max_w as f64 / max_h as f64;
        if source.width as f64 / source.height as f64 > target_ratio {
            let height = source.height;
            source = crop_center(
                source,
                (height as f64 * target_ratio).max(1.0) as u32,
                height,
            );
        } else {
            let width = source.width;
            source = crop_center(source, width, (width as f64 / target_ratio).max(1.0) as u32);
        }
    }
    if !profile.allow_alpha() {
        for pixel in source.pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    Ok(source)
}

fn fit_size(image: &Image, max_w: u32, max_h: u32) -> Image {
    let scale = (max_w as f64 / image.width as f64)
        .min(max_h as f64 / image.height as f64)
        .min(1.0);
    let w = ((image.width as f64 * scale).round() as u32).max(1);
    let h = ((image.height as f64 * scale).round() as u32).max(1);
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h {
        for x in 0..w {
            let sx = ((u64::from(x) * u64::from(image.width)) / u64::from(w)) as usize;
            let sy = ((u64::from(y) * u64::from(image.height)) / u64::from(h)) as usize;
            let si = (sy * image.width as usize + sx) * 4;
            let di = (y as usize * w as usize + x as usize) * 4;
            pixels[di..di + 4].copy_from_slice(&image.pixels[si..si + 4]);
        }
    }
    Image {
        width: w,
        height: h,
        pixels,
    }
}

fn crop_center(image: Image, width: u32, height: u32) -> Image {
    let x0 = (image.width - width) / 2;
    let y0 = (image.height - height) / 2;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        let src = ((y + y0) * image.width + x0) as usize * 4;
        let dst = (y * width) as usize * 4;
        pixels[dst..dst + width as usize * 4]
            .copy_from_slice(&image.pixels[src..src + width as usize * 4]);
    }
    Image {
        width,
        height,
        pixels,
    }
}

fn encode_png(image: &Image, profile: Profile) -> Result<Vec<u8>, CacheError> {
    let mut output = Vec::new();
    let color = if profile.allow_alpha() {
        ColorType::Rgba
    } else {
        ColorType::Rgb
    };
    let mut encoder = Encoder::new(&mut output, image.width, image.height);
    encoder.set_color(color);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|_| CacheError::new(PublicErrorCategory::Image, "PNG encoding failed"))?;
    if profile.allow_alpha() {
        writer
            .write_image_data(&image.pixels)
            .map_err(|_| CacheError::new(PublicErrorCategory::Image, "PNG encoding failed"))?;
    } else {
        let mut rgb = Vec::with_capacity((image.width * image.height * 3) as usize);
        for pixel in image.pixels.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        writer
            .write_image_data(&rgb)
            .map_err(|_| CacheError::new(PublicErrorCategory::Image, "PNG encoding failed"))?;
    }
    drop(writer);
    Ok(output)
}

fn validate_cache_root(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute() || root.components().any(|x| matches!(x, Component::ParentDir)) {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "cache root rejected"
        )));
    }
    let protected = [
        "roms",
        "saves",
        "states",
        "themes",
        "credentials",
        "metadata",
    ];
    if root == Path::new("/")
        || root
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .any(|name| protected.contains(&name.to_ascii_lowercase().as_str()))
    {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "cache root rejected"
        )));
    }
    if root.exists() && fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "cache root rejected"
        )));
    }
    let canonical_parent = root
        .parent()
        .ok_or_else(|| anyhow!("cache root rejected"))?
        .canonicalize()?;
    let root = canonical_parent.join(
        root.file_name()
            .ok_or_else(|| anyhow!("cache root rejected"))?,
    );
    if root.exists() && !root.is_dir() {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "cache root rejected"
        )));
    }
    for protected in [
        "roms",
        "saves",
        "states",
        "themes",
        "credentials",
        "data/meta",
    ] {
        if root.join(protected).exists() {
            return Err(anyhow!(CacheError::new(
                PublicErrorCategory::Storage,
                "cache root rejected"
            )));
        }
    }
    Ok(root)
}

fn validate_external_temp_root(cache_root: &Path, root: &Path) -> Result<PathBuf> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "temporary root rejected"
        )));
    }
    let protected = [
        "roms",
        "saves",
        "states",
        "themes",
        "credentials",
        "metadata",
    ];
    if root
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .any(|name| protected.contains(&name.to_ascii_lowercase().as_str()))
    {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "temporary root rejected"
        )));
    }
    if root.exists() && fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "temporary root rejected"
        )));
    }
    let candidate = root
        .parent()
        .ok_or_else(|| anyhow!("temporary root rejected"))?
        .canonicalize()?
        .join(
            root.file_name()
                .ok_or_else(|| anyhow!("temporary root rejected"))?,
        );
    if candidate.starts_with(cache_root) {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "temporary root rejected"
        )));
    }
    fs::create_dir_all(&candidate)?;
    if fs::symlink_metadata(&candidate)?.file_type().is_symlink() || !candidate.is_dir() {
        return Err(anyhow!(CacheError::new(
            PublicErrorCategory::Storage,
            "temporary root rejected"
        )));
    }
    Ok(candidate)
}

fn valid_opaque(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn read_prefix(path: &Path, size: usize) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = vec![0u8; size];
    let n = file.read(&mut bytes)?;
    bytes.truncate(n);
    Ok(bytes)
}
fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
fn object_path(root: &Path, digest: &str) -> PathBuf {
    root.join("objects")
        .join(&digest[..2])
        .join(format!("{digest}.png"))
}
fn metadata_path(root: &Path, digest: &str) -> PathBuf {
    root.join("metadata").join(format!("{digest}.json"))
}
fn index_path(root: &Path, id: &str) -> PathBuf {
    root.join("index").join(format!("{id}.json"))
}
fn temp_path(root: &Path, name: &str) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    let sequence = TRANSFER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    root.join(format!(
        ".{name}.tmp-{}-{:x}-{}",
        std::process::id(),
        hasher.finish(),
        sequence
    ))
}

fn create_temp_file(root: &Path, name: &str) -> Result<(PathBuf, File)> {
    for _ in 0..1024 {
        let path = temp_path(root, name);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("temporary name collision limit exceeded")
}

fn publish_object(path: &Path, bytes: &[u8], digest: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if hex_digest(&fs::read(path)?) != digest {
            bail!("completed object digest mismatch")
        }
        return Ok(());
    }
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("object path has no filename"))?
        .to_string_lossy();
    let (temporary, mut file) =
        create_temp_file(path.parent().unwrap(), &format!("object-{name}"))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match rename_noreplace(&temporary, path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary);
            if hex_digest(&fs::read(path)?) != digest {
                bail!("completed object digest mismatch")
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
    }
    sync_dir(path.parent().unwrap())
}

fn atomic_provenance(path: &Path, value: &Provenance) -> Result<()> {
    if path.exists() {
        let existing: Provenance = serde_json::from_slice(&fs::read(path)?)
            .map_err(|_| anyhow!("published provenance is invalid"))?;
        if existing.object_sha256 != value.object_sha256 {
            bail!("published provenance object mismatch")
        }
        return Ok(());
    }
    atomic_json(path, value)
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("path has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    if path.exists() {
        if fs::read(path)? != bytes {
            bail!("published JSON record mismatch")
        }
        return Ok(());
    }
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("JSON path has no filename"))?
        .to_string_lossy();
    let (temporary, mut file) = create_temp_file(parent, &format!("json-{name}"))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    match rename_noreplace(&temporary, path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary);
            if fs::read(path)? != bytes {
                bail!("published JSON record mismatch")
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
    }
    sync_dir(parent)
}
#[cfg(target_os = "linux")]
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2 as libc::c_long,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_noreplace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename unavailable",
    ))
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn reject_cache_symlinks(root: &Path) -> Result<()> {
    for directory in ["objects", "metadata", "index"] {
        let path = root.join(directory);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("cache tree contains a symlink")
        }
        for entry in fs::read_dir(path)? {
            let child = entry?.path();
            if fs::symlink_metadata(&child)?.file_type().is_symlink() {
                bail!("cache tree contains a symlink")
            }
            if child.is_dir() {
                for nested in fs::read_dir(child)? {
                    if fs::symlink_metadata(nested?.path())?
                        .file_type()
                        .is_symlink()
                    {
                        bail!("cache tree contains a symlink")
                    }
                }
            }
        }
    }
    Ok(())
}

fn remove_indexes_for_digest(root: &Path, digest: &str) -> Result<()> {
    let directory = root.join("index");
    for entry in fs::read_dir(&directory)? {
        let path = entry?.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path)?;
        let entry: IndexEntry = match serde_json::from_slice(&bytes) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if entry.object == digest {
            fs::remove_file(path)?;
        }
    }
    sync_dir(&directory)
}

fn external_temp_stale_after(timeout: Duration) -> Duration {
    timeout.saturating_mul(EXTERNAL_TEMP_STALE_MULTIPLIER)
}

fn is_stale_external_transfer(age: Option<Duration>, stale_after: Duration) -> bool {
    age.is_some_and(|age| age > stale_after)
}

fn cleanup_external_temporary_files(
    root: &Path,
    identity: &str,
    stale_after: Duration,
) -> Result<()> {
    let prefix = format!(".media-cache-transfer-{identity}.tmp-");
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !name.starts_with(&prefix) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok());
        if !is_stale_external_transfer(age, stale_after) {
            continue;
        }
        let kind = metadata.file_type();
        if kind.is_file() || kind.is_symlink() {
            fs::remove_file(path)?;
        } else {
            bail!("temporary tree contains an unexpected directory")
        }
    }
    sync_dir(root)
}

fn cleanup_temporary_files(root: &Path) -> Result<()> {
    for directory in ["objects", "metadata", "index"] {
        let path = root.join(directory);
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let child = entry.path();
            if child.is_dir() {
                for nested in fs::read_dir(&child)? {
                    let nested = nested?.path();
                    if nested
                        .file_name()
                        .and_then(|x| x.to_str())
                        .is_some_and(|x| x.starts_with('.'))
                    {
                        fs::remove_file(nested)?;
                    }
                }
            } else if child
                .file_name()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.starts_with('.'))
            {
                fs::remove_file(child)?;
            }
        }
    }
    Ok(())
}

// Baseline JPEG decoding is kept intentionally narrow: SOF0, 8-bit samples, and non-progressive
// 1x1 or uniform vertical-2-sampled grayscale or YCbCr. Other JPEG forms fail closed rather than claiming normalization.
fn decode_jpeg(path: &Path, deadline: Instant) -> Result<Image, CacheError> {
    let bytes = fs::read(path)
        .map_err(|_| CacheError::new(PublicErrorCategory::Image, "image unreadable"))?;
    if bytes.len() as u64 > MAX_TRANSFER_BYTES {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "image too large",
        ));
    }
    let mut p = Jpeg::new(&bytes, deadline);
    let (width, height, pixels, orientation) = p
        .decode()
        .map_err(|_| CacheError::new(PublicErrorCategory::Image, "invalid JPEG"))?;
    check_dimensions(width, height)?;
    orient_image(
        Image {
            width,
            height,
            pixels,
        },
        orientation,
    )
}

fn parse_exif_orientation(payload: &[u8]) -> Result<u8> {
    if !payload.starts_with(b"Exif\0\0") {
        bail!("unsupported APP1")
    }
    let tiff = &payload[6..];
    let little = match tiff.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => bail!("EXIF byte order"),
    };
    if read_tiff_u16(tiff, 2, little)? != 42 {
        bail!("EXIF magic")
    }
    let ifd = read_tiff_u32(tiff, 4, little)? as usize;
    let count = read_tiff_u16(tiff, ifd, little)? as usize;
    let entries = ifd
        .checked_add(2)
        .and_then(|value| value.checked_add(count.checked_mul(12)?))
        .ok_or_else(|| anyhow!("EXIF bounds"))?;
    if entries.checked_add(4).is_none_or(|end| end > tiff.len()) {
        bail!("EXIF bounds")
    }
    let mut orientation = 1;
    for index in 0..count {
        let entry = ifd + 2 + index * 12;
        if read_tiff_u16(tiff, entry, little)? != 0x0112 {
            continue;
        }
        if read_tiff_u16(tiff, entry + 2, little)? != 3
            || read_tiff_u32(tiff, entry + 4, little)? != 1
        {
            bail!("EXIF orientation")
        }
        orientation = read_tiff_u16(tiff, entry + 8, little)? as u8;
        if !(1..=8).contains(&orientation) {
            bail!("EXIF orientation")
        }
    }
    Ok(orientation)
}

fn read_tiff_u16(data: &[u8], offset: usize, little: bool) -> Result<u16> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| anyhow!("EXIF bounds"))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| anyhow!("EXIF bounds"))?;
    Ok(if little {
        u16::from_le_bytes([bytes[0], bytes[1]])
    } else {
        u16::from_be_bytes([bytes[0], bytes[1]])
    })
}

fn read_tiff_u32(data: &[u8], offset: usize, little: bool) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| anyhow!("EXIF bounds"))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| anyhow!("EXIF bounds"))?;
    Ok(if little {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    })
}

fn orient_image(image: Image, orientation: u8) -> Result<Image, CacheError> {
    if orientation == 1 {
        return Ok(image);
    }
    if !(1..=8).contains(&orientation) {
        return Err(CacheError::new(
            PublicErrorCategory::Image,
            "unsupported JPEG orientation",
        ));
    }
    let (width, height) = if (5..=8).contains(&orientation) {
        (image.height, image.width)
    } else {
        (image.width, image.height)
    };
    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = match orientation {
                2 => (image.width - 1 - x, y),
                3 => (image.width - 1 - x, image.height - 1 - y),
                4 => (x, image.height - 1 - y),
                5 => (y, x),
                6 => (y, image.height - 1 - x),
                7 => (image.width - 1 - y, image.height - 1 - x),
                8 => (image.width - 1 - y, x),
                _ => (x, y),
            };
            let source = ((sy * image.width + sx) * 4) as usize;
            let target = ((y * width + x) * 4) as usize;
            pixels[target..target + 4].copy_from_slice(&image.pixels[source..source + 4]);
        }
    }
    Ok(Image {
        width,
        height,
        pixels,
    })
}

struct Jpeg<'a> {
    data: &'a [u8],
    deadline: Instant,
    pos: usize,
    quant: [[u16; 64]; 4],
    huff_dc: [Option<Huffman>; 4],
    huff_ac: [Option<Huffman>; 4],
    components: Vec<JpegComponent>,
    scan: Vec<u8>,
    bit_pos: usize,
    orientation: u8,
}
#[derive(Clone)]
struct Huffman {
    codes: Vec<(u16, u8, u8)>,
}
struct JpegComponent {
    id: u8,
    q: usize,
    v: usize,
    dc: usize,
    ac: usize,
    dc_value: i32,
}

impl<'a> Jpeg<'a> {
    fn new(data: &'a [u8], deadline: Instant) -> Self {
        Self {
            data,
            deadline,
            pos: 2,
            quant: [[0; 64]; 4],
            huff_dc: [None, None, None, None],
            huff_ac: [None, None, None, None],
            components: Vec::new(),
            scan: Vec::new(),
            bit_pos: 0,
            orientation: 1,
        }
    }
    fn decode(&mut self) -> Result<(u32, u32, Vec<u8>, u8)> {
        if self.data.get(..2) != Some(&[0xff, 0xd8]) {
            bail!("not JPEG")
        }
        let mut width = 0;
        let mut height = 0;
        let mut scan_start = 0;
        while self.pos < self.data.len() {
            check_decode_deadline(self.deadline).map_err(|_| anyhow!("decode timeout"))?;
            while self.pos < self.data.len() && self.data[self.pos] != 0xff {
                self.pos += 1;
            }
            while self.pos < self.data.len() && self.data[self.pos] == 0xff {
                self.pos += 1;
            }
            let marker = *self.data.get(self.pos).ok_or_else(|| anyhow!("marker"))?;
            self.pos += 1;
            if marker == 0xd9 {
                break;
            }
            if marker == 0xe1 {
                let len = self.u16()? as usize;
                if len < 2 || self.pos + len - 2 > self.data.len() {
                    bail!("EXIF")
                }
                let end = self.pos + len - 2;
                let orientation = parse_exif_orientation(&self.data[self.pos..end])?;
                if self.orientation != 1 && self.orientation != orientation {
                    bail!("EXIF")
                }
                self.orientation = orientation;
                self.pos = end;
                continue;
            }
            if marker == 0xda {
                let len = self.u16()? as usize;
                if len < 2 || self.pos + len - 2 > self.data.len() {
                    bail!("SOS")
                }
                let end = self.pos + len - 2;
                let count = *self.data.get(self.pos).ok_or_else(|| anyhow!("SOS"))? as usize;
                self.pos += 1;
                if count != self.components.len() || count == 0 || count > 3 {
                    bail!("scan")
                }
                for _ in 0..count {
                    let id = self.u8()?;
                    let table = self.u8()?;
                    let c = self
                        .components
                        .iter_mut()
                        .find(|x| x.id == id)
                        .ok_or_else(|| anyhow!("component"))?;
                    c.dc = (table >> 4) as usize;
                    c.ac = (table & 15) as usize;
                    if c.dc > 3 || c.ac > 3 {
                        bail!("table")
                    }
                }
                if self.pos != end {
                    self.pos = end;
                }
                scan_start = self.pos;
                break;
            }
            if marker == 0xc0 {
                let len = self.u16()? as usize;
                if len < 8 || self.pos + len - 2 > self.data.len() {
                    bail!("SOF")
                }
                let precision = self.u8()?;
                if precision != 8 {
                    bail!("precision")
                }
                height = self.u16()? as u32;
                width = self.u16()? as u32;
                let n = self.u8()? as usize;
                if n == 0 || n > 3 || len != 8 + 3 * n {
                    bail!("SOF")
                }
                for _ in 0..n {
                    let id = self.u8()?;
                    let sampling = self.u8()?;
                    let v = match sampling {
                        0x11 => 1,
                        0x12 => 2,
                        _ => bail!("sampling"),
                    };
                    let q = self.u8()? as usize;
                    if q > 3 {
                        bail!("quant")
                    }
                    self.components.push(JpegComponent {
                        id,
                        q,
                        v,
                        dc: 0,
                        ac: 0,
                        dc_value: 0,
                    });
                }
                continue;
            }
            if marker == 0xdb {
                let len = self.u16()? as usize;
                if len < 67 || self.pos + len - 2 > self.data.len() {
                    bail!("quant")
                }
                let end = self.pos + len - 2;
                while self.pos < end {
                    let spec = self.u8()?;
                    if spec >> 4 != 0 {
                        bail!("16-bit quant")
                    }
                    let id = (spec & 15) as usize;
                    if id > 3 {
                        bail!("quant")
                    }
                    let mut table = [0u16; 64];
                    for value in &mut table {
                        *value = self.u8()? as u16;
                        if *value == 0 {
                            bail!("quant")
                        }
                    }
                    self.quant[id] = table;
                }
                continue;
            }
            if marker == 0xc4 {
                let len = self.u16()? as usize;
                if len < 2 || self.pos + len - 2 > self.data.len() {
                    bail!("huffman")
                }
                let end = self.pos + len - 2;
                while self.pos < end {
                    let spec = self.u8()?;
                    let class = (spec >> 4) as usize;
                    let id = (spec & 15) as usize;
                    if class > 1 || id > 3 {
                        bail!("huffman")
                    }
                    let mut counts = [0u8; 16];
                    for x in &mut counts {
                        *x = self.u8()?;
                    }
                    let total: usize = counts.iter().map(|x| *x as usize).sum();
                    let mut values = Vec::with_capacity(total);
                    for _ in 0..total {
                        values.push(self.u8()?);
                    }
                    let h = Huffman::new(counts, values)?;
                    if class == 0 {
                        self.huff_dc[id] = Some(h);
                    } else {
                        self.huff_ac[id] = Some(h);
                    }
                }
                continue;
            }
            if (0xc1..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                bail!("progressive JPEG")
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let len = self.u16()? as usize;
            if len < 2 || self.pos + len - 2 > self.data.len() {
                bail!("segment")
            }
            self.pos += len - 2;
        }
        check_dimensions(width, height)?;
        if !matches!(self.components.len(), 1 | 3) {
            bail!("component count")
        }
        if scan_start == 0 || self.components.is_empty() {
            bail!("missing scan")
        }
        self.scan = entropy_bytes(&self.data[scan_start..]);
        self.bit_pos = 0;
        let mcu_w = width.div_ceil(8);
        let max_v = self
            .components
            .iter()
            .map(|component| component.v)
            .max()
            .unwrap_or(1);
        if self.components.iter().any(|component| component.v != max_v) {
            bail!("mixed sampling")
        }
        if !self.data[scan_start..]
            .windows(2)
            .any(|pair| pair == [0xff, 0xd9])
        {
            bail!("missing EOI")
        }
        let mcu_h = height.div_ceil(8 * max_v as u32);
        let plane_bytes = u64::from(mcu_w)
            .checked_mul(u64::from(mcu_h))
            .and_then(|value| value.checked_mul(64 * max_v as u64))
            .and_then(|value| value.checked_mul(self.components.len() as u64))
            .and_then(|value| value.checked_mul(4));
        if plane_bytes.is_none_or(|bytes| bytes > MAX_DECODED_BYTES) {
            bail!("decoded image too large")
        }
        let mut planes = Vec::new();
        for i in 0..self.components.len() {
            check_decode_deadline(self.deadline).map_err(|_| anyhow!("decode timeout"))?;
            let v = self.components[i].v;
            let mut plane = vec![0f32; (mcu_w * 8 * mcu_h * 8 * v as u32) as usize];
            for by in 0..mcu_h {
                check_decode_deadline(self.deadline).map_err(|_| anyhow!("decode timeout"))?;
                for bx in 0..mcu_w {
                    for vy in 0..v {
                        let block = self.block(i)?;
                        for y in 0..8 {
                            for x in 0..8 {
                                plane[(((by * v as u32 + vy as u32) * 8 + y) * mcu_w * 8
                                    + bx * 8
                                    + x) as usize] = block[y as usize * 8 + x as usize];
                            }
                        }
                    }
                }
            }
            planes.push(plane);
        }
        let mut out = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            check_decode_deadline(self.deadline).map_err(|_| anyhow!("decode timeout"))?;
            for x in 0..width {
                let i = (y * mcu_w * 8 + x) as usize;
                let (r, g, b) = if planes.len() == 1 {
                    let v = clamp(planes[0][i] + 128.0);
                    (v, v, v)
                } else {
                    let yy = planes[0][i] + 128.0;
                    let cb = planes[1][i];
                    let cr = planes[2][i];
                    (
                        clamp(yy + 1.402 * cr),
                        clamp(yy - 0.344136 * cb - 0.714136 * cr),
                        clamp(yy + 1.772 * cb),
                    )
                };
                let o = ((y * width + x) * 4) as usize;
                out[o..o + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
        Ok((width, height, out, self.orientation))
    }
    fn block(&mut self, index: usize) -> Result<[f32; 64]> {
        let comp = &self.components[index];
        let dc_table = self.huff_dc[comp.dc]
            .clone()
            .ok_or_else(|| anyhow!("missing DC"))?;
        let ac_table = self.huff_ac[comp.ac]
            .clone()
            .ok_or_else(|| anyhow!("missing AC"))?;
        let q = self.quant[comp.q];
        if q[0] == 0 {
            bail!("missing quantization table")
        }
        let dc_size = self.decode_huffman(&dc_table)?;
        let dc = self.receive_extend(dc_size)?;
        self.components[index].dc_value += dc;
        let mut coeff = [0f32; 64];
        coeff[0] = self.components[index].dc_value as f32 * q[0] as f32;
        let mut k = 1;
        while k < 64 {
            let symbol = self.decode_huffman(&ac_table)?;
            let run = (symbol >> 4) as usize;
            let size = (symbol & 15) as usize;
            if size == 0 {
                if run == 15 {
                    k += 16;
                    continue;
                }
                break;
            }
            k += run;
            if k >= 64 {
                bail!("AC overflow")
            }
            coeff[ZIGZAG[k]] = self.receive_extend(size as u8)? as f32 * q[ZIGZAG[k]] as f32;
            k += 1;
        }
        Ok(idct(coeff))
    }
    fn u8(&mut self) -> Result<u8> {
        let x = *self
            .data
            .get(self.pos)
            .ok_or_else(|| anyhow!("short JPEG"))?;
        self.pos += 1;
        Ok(x)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }
    fn decode_huffman(&mut self, h: &Huffman) -> Result<u8> {
        let mut code = 0u16;
        for len in 1..=16 {
            code = (code << 1) | self.bit()? as u16;
            if let Some((_, _, value)) = h.codes.iter().find(|(c, l, _)| *l == len && *c == code) {
                return Ok(*value);
            }
        }
        bail!("bad huffman at bit {}", self.bit_pos)
    }
    fn bit(&mut self) -> Result<u8> {
        let byte = *self
            .scan
            .get(self.bit_pos / 8)
            .ok_or_else(|| anyhow!("short entropy"))?;
        let bit = (byte >> (7 - self.bit_pos % 8)) & 1;
        self.bit_pos += 1;
        Ok(bit)
    }
    fn receive_extend(&mut self, size: u8) -> Result<i32> {
        if size == 0 {
            return Ok(0);
        }
        let mut value = 0;
        for _ in 0..size {
            value = (value << 1) | self.bit()? as i32;
        }
        if value < (1 << (size - 1)) {
            Ok(value + 1 - (1 << size))
        } else {
            Ok(value)
        }
    }
}

impl Huffman {
    fn new(counts: [u8; 16], values: Vec<u8>) -> Result<Self> {
        let mut codes = Vec::new();
        let mut code = 0u16;
        let mut pos = 0;
        for (i, count) in counts.into_iter().enumerate() {
            for _ in 0..count {
                codes.push((code, (i + 1) as u8, values[pos]));
                pos += 1;
                code += 1;
            }
            code <<= 1;
        }
        Ok(Self { codes })
    }
}
fn entropy_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0xff {
            if bytes.get(i + 1) == Some(&0) {
                out.push(0xff);
                i += 2;
            } else {
                break;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
fn idct(input: [f32; 64]) -> [f32; 64] {
    let mut out = [0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut sum = 0f32;
            for v in 0..8 {
                for u in 0..8 {
                    let cu = if u == 0 { 0.70710677 } else { 1.0 };
                    let cv = if v == 0 { 0.70710677 } else { 1.0 };
                    sum += cu
                        * cv
                        * input[v * 8 + u]
                        * (((2 * x + 1) * u) as f32 * std::f32::consts::PI / 16.0).cos()
                        * (((2 * y + 1) * v) as f32 * std::f32::consts::PI / 16.0).cos();
                }
            }
            out[y * 8 + x] = sum / 4.0;
        }
    }
    out
}
fn clamp(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}
