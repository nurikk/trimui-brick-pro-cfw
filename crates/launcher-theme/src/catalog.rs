use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Reason, ThemeError};

pub const THEMES_CATALOG_FORMAT: &str = "themes-catalog-v1";
pub const MAX_THEME_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;

pub trait CatalogTransport {
    fn fetch(&self, locator: &str, max_bytes: usize) -> Result<Vec<u8>, ThemeError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectCatalogTransport;

const MAX_REDIRECTS: usize = 5;
const MAX_RESOLVED_ADDRESSES: usize = 16;
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
const CURL_METADATA_MARKER: &str = "\nbrickpro-curl-metadata:";

#[derive(Debug)]
struct CurlResponse {
    body: Vec<u8>,
    status: u16,
    redirect: String,
    remote_ip: IpAddr,
}

impl CatalogTransport for DirectCatalogTransport {
    fn fetch(&self, locator: &str, max_bytes: usize) -> Result<Vec<u8>, ThemeError> {
        fetch_https(locator, max_bytes, resolve_host, run_curl)
    }
}

fn fetch_https<R, F>(
    locator: &str,
    max_bytes: usize,
    resolve: R,
    mut request: F,
) -> Result<Vec<u8>, ThemeError>
where
    R: Fn(&str) -> Result<Vec<IpAddr>, ThemeError>,
    F: FnMut(&str, &str, &[IpAddr], usize, u64) -> Result<CurlResponse, ThemeError>,
{
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let mut locator = locator.to_string();
    let mut transferred = 0usize;

    for redirects in 0..=MAX_REDIRECTS {
        let host = https_host(&locator).ok_or_else(unsafe_url_error)?;
        let addresses = resolve(host)?;
        if addresses.is_empty()
            || addresses.len() > MAX_RESOLVED_ADDRESSES
            || addresses.iter().any(|address| unsafe_ip(*address))
        {
            return Err(unsafe_url_error());
        }
        let remaining_bytes = max_bytes
            .checked_sub(transferred)
            .ok_or_else(download_budget_error)?;
        let remaining_time = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| ThemeError::new(Reason::Io, "catalog request timed out"))?;
        let response = request(
            &locator,
            host,
            &addresses,
            remaining_bytes,
            remaining_time.as_secs().max(1),
        )?;
        if unsafe_ip(response.remote_ip) || !addresses.contains(&response.remote_ip) {
            return Err(unsafe_url_error());
        }
        transferred = transferred
            .checked_add(response.body.len())
            .filter(|total| *total <= max_bytes)
            .ok_or_else(download_budget_error)?;

        if (200..300).contains(&response.status) {
            return Ok(response.body);
        }
        if (300..400).contains(&response.status) && !response.redirect.is_empty() {
            if redirects == MAX_REDIRECTS {
                return Err(ThemeError::new(
                    Reason::InvalidPath,
                    "catalog redirect limit exceeded",
                ));
            }
            if https_host(&response.redirect).is_none() {
                return Err(unsafe_url_error());
            }
            locator = response.redirect;
            continue;
        }
        return Err(ThemeError::new(
            Reason::Io,
            format!("catalog request returned HTTP {}", response.status),
        ));
    }
    unreachable!("redirect loop returns at its configured bound")
}

fn resolve_host(host: &str) -> Result<Vec<IpAddr>, ThemeError> {
    if let Ok(address) = host.parse() {
        return Ok(vec![address]);
    }
    let addresses = (host, 443)
        .to_socket_addrs()
        .map_err(|error| ThemeError::new(Reason::Io, format!("catalog DNS failed: {error}")))?
        .map(|address| address.ip())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(addresses)
}

fn run_curl(
    locator: &str,
    host: &str,
    addresses: &[IpAddr],
    max_bytes: usize,
    max_seconds: u64,
) -> Result<CurlResponse, ThemeError> {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--tlsv1.2",
        "--connect-timeout",
        "10",
        "--max-time",
        &max_seconds.to_string(),
        "--max-filesize",
        &max_bytes.to_string(),
        "--max-redirs",
        "0",
        "--noproxy",
        "*",
        "--write-out",
        &format!("{CURL_METADATA_MARKER}%{{http_code}}\\n%{{redirect_url}}\\n%{{remote_ip}}"),
    ]);
    if host.parse::<IpAddr>().is_err() {
        let pinned = addresses
            .iter()
            .map(|address| match address {
                IpAddr::V4(address) => address.to_string(),
                IpAddr::V6(address) => format!("[{address}]"),
            })
            .collect::<Vec<_>>()
            .join(",");
        command.args(["--resolve", &format!("{host}:443:{pinned}")]);
    }
    let output = command
        .arg(locator)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| ThemeError::new(Reason::Io, format!("curl unavailable: {error}")))?;
    if !output.status.success() {
        return Err(ThemeError::new(
            Reason::Io,
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    parse_curl_response(output.stdout, max_bytes)
}

fn parse_curl_response(mut output: Vec<u8>, max_bytes: usize) -> Result<CurlResponse, ThemeError> {
    let marker = CURL_METADATA_MARKER.as_bytes();
    let marker_start = output
        .windows(marker.len())
        .rposition(|window| window == marker)
        .ok_or_else(|| ThemeError::new(Reason::Io, "curl response metadata missing"))?;
    let metadata = String::from_utf8(output.split_off(marker_start + marker.len()))
        .map_err(|_| ThemeError::new(Reason::Io, "curl response metadata is malformed"))?;
    output.truncate(marker_start);
    if output.len() > max_bytes {
        return Err(download_budget_error());
    }
    let mut lines = metadata.lines();
    let status = lines
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ThemeError::new(Reason::Io, "curl HTTP status is malformed"))?;
    let redirect = lines.next().unwrap_or_default().to_string();
    let remote_ip = lines
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ThemeError::new(Reason::Io, "curl connected address is malformed"))?;
    Ok(CurlResponse {
        body: output,
        status,
        redirect,
        remote_ip,
    })
}

fn unsafe_url_error() -> ThemeError {
    ThemeError::new(Reason::InvalidPath, "catalog URL is not safe")
}

fn download_budget_error() -> ThemeError {
    ThemeError::new(
        Reason::BudgetAsset,
        "download exceeds configured byte budget",
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemesCatalog {
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub themes: Vec<ThemesCatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemesCatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub locator: String,
    pub author: String,
    #[serde(default)]
    pub screenshot: Option<String>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(default, rename = "sizeMb")]
    pub size_mb: Option<u64>,
    #[serde(default, rename = "upstreamStatus")]
    pub upstream_status: Option<u8>,
    #[serde(default, rename = "aspectRatios")]
    pub aspect_ratios: Vec<String>,
    #[serde(default, rename = "knulliCompatible")]
    pub knulli_compatible: Option<bool>,
}

impl ThemesCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, ThemeError> {
        if bytes.len() > super::MAX_JSON_BYTES {
            return Err(ThemeError::new(
                Reason::BudgetJsonSize,
                "themes.json exceeds 131072 bytes",
            ));
        }
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| ThemeError::new(Reason::MalformedJson, error.to_string()))?;
        let catalog = if value.get("data").is_some() {
            let feed: BatoceraFeed = serde_json::from_value(value)
                .map_err(|error| ThemeError::new(Reason::UnknownField, error.to_string()))?;
            Self {
                format: THEMES_CATALOG_FORMAT.into(),
                schema_version: 1,
                themes: feed
                    .data
                    .into_iter()
                    .map(ThemesCatalogEntry::from_feed)
                    .collect::<Result<Vec<_>, _>>()?,
            }
        } else {
            serde_json::from_value(value)
                .map_err(|error| ThemeError::new(Reason::UnknownField, error.to_string()))?
        };
        if catalog.format != THEMES_CATALOG_FORMAT
            || catalog.schema_version != 1
            || catalog.themes.is_empty()
            || catalog.themes.len() > 512
        {
            return Err(ThemeError::new(
                Reason::InvalidSchema,
                "unsupported themes.json catalog",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for entry in &catalog.themes {
            validate_entry(entry)?;
            if !ids.insert(&entry.id) {
                return Err(ThemeError::new(
                    Reason::InvalidSchema,
                    "themes.json contains duplicate ids",
                ));
            }
        }
        Ok(catalog)
    }

    pub fn select(&self, id: &str, version: &str) -> Result<&ThemesCatalogEntry, ThemeError> {
        self.themes
            .iter()
            .find(|entry| entry.id == id && entry.version == version)
            .ok_or_else(|| {
                ThemeError::new(Reason::InvalidSchema, "theme selection is not catalogued")
            })
    }

    pub fn fetch<T: CatalogTransport>(
        &self,
        id: &str,
        version: &str,
        transport: &T,
    ) -> Result<Vec<u8>, ThemeError> {
        let entry = self.select(id, version)?;
        if entry.locator.starts_with("fixture:") {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "fixture locators are not downloadable",
            ));
        }
        let bytes = transport.fetch(&entry.locator, MAX_THEME_DOWNLOAD_BYTES)?;
        if bytes.len() > MAX_THEME_DOWNLOAD_BYTES {
            return Err(ThemeError::new(
                Reason::BudgetAsset,
                "theme download exceeds 32 MiB",
            ));
        }
        Ok(bytes)
    }

    pub fn load_theme<T: CatalogTransport>(
        &self,
        id: &str,
        version: &str,
        transport: &T,
    ) -> Result<super::ValidatedTheme, ThemeError> {
        let entry = self.select(id, version)?;
        if entry.locator.starts_with("fixture:") {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "fixture locators require a local fixture root",
            ));
        }
        let theme_url = package_file_url(&entry.locator, "theme.json")?;
        let mut theme = super::parse_json(&transport.fetch(&theme_url, super::MAX_JSON_BYTES)?)?;
        for spec in super::declared_assets(theme.theme()) {
            let url = package_file_url(&entry.locator, &spec.path)?;
            let bytes = transport.fetch(&url, spec.max_bytes as usize)?;
            if bytes.len() > spec.max_bytes as usize {
                return Err(ThemeError::new(
                    Reason::BudgetAsset,
                    format!("asset {} exceeds declared limit", spec.path),
                ));
            }
            theme.assets.push(super::decode_asset(&spec.path, &bytes)?);
        }
        Ok(theme)
    }
}

fn package_file_url(base: &str, path: &str) -> Result<String, ThemeError> {
    super::validate_asset_path(path)?;
    if let Some(repository) = base.strip_prefix("https://github.com/") {
        let mut parts = repository.trim_end_matches('/').split('/');
        let owner = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if !identifier(owner) || !identifier(name) || parts.next().is_some() {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "unsupported GitHub locator",
            ));
        }
        return Ok(format!(
            "https://raw.githubusercontent.com/{owner}/{name}/main/{path}"
        ));
    }
    if let Some(parent) = base.strip_suffix("theme.json") {
        return Ok(format!("{parent}{path}"));
    }
    Ok(format!("{}/{}", base.trim_end_matches('/'), path))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatoceraFeed {
    data: Vec<BatoceraEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatoceraEntry {
    theme: String,
    author: String,
    theme_url: String,
    #[serde(rename = "last_update")]
    last_update: String,
    #[serde(rename = "up_to_date")]
    up_to_date: String,
    size: String,
    screenshot: String,
}

impl ThemesCatalogEntry {
    fn from_feed(entry: BatoceraEntry) -> Result<Self, ThemeError> {
        if !calendar_date(&entry.last_update) {
            return Err(ThemeError::new(
                Reason::InvalidSchema,
                "Batocera last_update must be YYYY-MM-DD",
            ));
        }
        let size_mb = entry
            .size
            .parse::<u64>()
            .map_err(|_| ThemeError::new(Reason::InvalidSchema, "Batocera size must be numeric"))?;
        let upstream_status = entry.up_to_date.parse::<u8>().map_err(|_| {
            ThemeError::new(Reason::InvalidSchema, "Batocera up_to_date must be numeric")
        })?;
        let screenshot = batocera_screenshot_url(&entry.screenshot)?;
        Ok(Self {
            id: entry.theme.clone(),
            name: entry.theme,
            version: "1.0.0".into(),
            locator: entry.theme_url,
            author: entry.author,
            screenshot: Some(screenshot),
            updated_at: Some(entry.last_update),
            size_mb: Some(size_mb),
            upstream_status: Some(upstream_status),
            aspect_ratios: Vec::new(),
            knulli_compatible: None,
        })
    }
}

fn validate_entry(entry: &ThemesCatalogEntry) -> Result<(), ThemeError> {
    if !identifier(&entry.id)
        || entry.name.is_empty()
        || entry.name.len() > 64
        || !version(&entry.version)
        || entry.author.is_empty()
        || entry.author.len() > 64
        || !safe_locator(&entry.locator)
    {
        return Err(ThemeError::new(
            Reason::InvalidSchema,
            format!("invalid themes.json entry {}", entry.id),
        ));
    }
    if entry.screenshot.as_ref().is_some_and(|path| {
        if path.starts_with("https://") {
            !safe_locator(path)
        } else {
            !safe_catalog_path(path)
        }
    }) {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "unsafe catalog screenshot path",
        ));
    }
    Ok(())
}

fn batocera_screenshot_url(path: &str) -> Result<String, ThemeError> {
    if !safe_catalog_path(path) || !path.starts_with("themes/") {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "unsafe Batocera screenshot path",
        ));
    }
    Ok(format!("https://batocera.org/upgrades/{path}"))
}

fn calendar_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
        && value[5..7]
            .parse::<u8>()
            .is_ok_and(|month| (1..=12).contains(&month))
        && value[8..10]
            .parse::<u8>()
            .is_ok_and(|day| (1..=31).contains(&day))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn version(value: &str) -> bool {
    value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn safe_catalog_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':'])
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn safe_locator(value: &str) -> bool {
    if let Some(id) = value.strip_prefix("fixture:") {
        return identifier(id);
    }
    https_host(value).is_some()
}

fn https_host(value: &str) -> Option<&str> {
    if value.len() > 2048
        || !value.starts_with("https://")
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || value.contains(['\\', '@', '?', '#'])
    {
        return None;
    }
    let authority = value[8..].split('/').next().unwrap_or_default();
    let host = if let Some(authority) = authority.strip_prefix('[') {
        authority.strip_suffix(']')?
    } else {
        if authority.contains(':') {
            return None;
        }
        authority
    };
    if host.is_empty() || host.len() > 253 || host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        return (!unsafe_ip(address)).then_some(host);
    }
    host.split('.')
        .all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        .then_some(host)
}

fn unsafe_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, third, _] = address.octets();
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_multicast()
                || address == Ipv4Addr::BROADCAST
                || first == 0
                || (first == 100 && (64..=127).contains(&second))
                || (first == 192 && second == 0 && third == 0)
                || (first == 192 && second == 0 && third == 2)
                || (first == 192 && second == 88 && third == 99)
                || (first == 198 && (second == 18 || second == 19))
                || (first == 198 && second == 51 && third == 100)
                || (first == 203 && second == 0 && third == 113)
                || first >= 224
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address == Ipv6Addr::LOCALHOST
                || segments[0] & 0xe000 != 0x2000
                || segments[..2] == [0x2001, 0x0db8]
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use std::{cell::Cell, net::IpAddr};

    use super::*;

    fn ip(value: &str) -> IpAddr {
        match value.parse() {
            Ok(address) => address,
            Err(error) => panic!("invalid test address {value}: {error}"),
        }
    }

    fn response(status: u16, redirect: &str, remote_ip: &str) -> CurlResponse {
        CurlResponse {
            body: b"theme".to_vec(),
            status,
            redirect: redirect.to_string(),
            remote_ip: ip(remote_ip),
        }
    }

    #[test]
    fn unsafe_literal_addresses_reject_before_resolution_or_connection() {
        for locator in [
            "https://127.0.0.1/theme.json",
            "https://10.0.0.1/theme.json",
            "https://169.254.169.254/theme.json",
            "https://192.168.1.1/theme.json",
            "https://[::1]/theme.json",
            "https://[fc00::1]/theme.json",
            "https://[fe80::1]/theme.json",
            "https://[::ffff:127.0.0.1]/theme.json",
        ] {
            let error = fetch_https(
                locator,
                32,
                |_| -> Result<Vec<IpAddr>, ThemeError> { panic!("must not resolve") },
                |_, _, _, _, _| -> Result<CurlResponse, ThemeError> { panic!("must not connect") },
            )
            .unwrap_err();
            assert_eq!(error.reason, Reason::InvalidPath, "{locator}");
        }
    }

    #[test]
    fn unsafe_only_and_mixed_dns_reject_before_connection() {
        for addresses in [vec![ip("127.0.0.1")], vec![ip("8.8.8.8"), ip("10.0.0.1")]] {
            let connected = Cell::new(false);
            let error = fetch_https(
                "https://themes.example/theme.json",
                32,
                |_| Ok(addresses.clone()),
                |_, _, _, _, _| {
                    connected.set(true);
                    Ok(response(200, "", "8.8.8.8"))
                },
            )
            .unwrap_err();
            assert_eq!(error.reason, Reason::InvalidPath);
            assert!(!connected.get());
        }
    }

    #[test]
    fn connected_address_must_be_public_and_pinned_by_resolution() {
        for remote_ip in ["127.0.0.1", "1.1.1.1"] {
            let error = fetch_https(
                "https://themes.example/theme.json",
                32,
                |_| Ok(vec![ip("8.8.8.8")]),
                |_, _, _, _, _| Ok(response(200, "", remote_ip)),
            )
            .unwrap_err();
            assert_eq!(error.reason, Reason::InvalidPath);
        }
    }

    #[test]
    fn every_redirect_is_revalidated_and_redirects_are_bounded() {
        let requests = Cell::new(0);
        let error = fetch_https(
            "https://themes.example/theme.json",
            64,
            |_| Ok(vec![ip("8.8.8.8")]),
            |_, _, _, _, _| {
                requests.set(requests.get() + 1);
                Ok(response(302, "https://127.0.0.1/private", "8.8.8.8"))
            },
        )
        .unwrap_err();
        assert_eq!(error.reason, Reason::InvalidPath);
        assert_eq!(requests.get(), 1);

        requests.set(0);
        let resolutions = Cell::new(0);
        let error = fetch_https(
            "https://themes.example/theme.json",
            64,
            |host| {
                resolutions.set(resolutions.get() + 1);
                Ok(vec![if host == "themes.example" {
                    ip("8.8.8.8")
                } else {
                    ip("10.0.0.1")
                }])
            },
            |_, _, _, _, _| {
                requests.set(requests.get() + 1);
                Ok(response(
                    302,
                    "https://private.example/theme.json",
                    "8.8.8.8",
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.reason, Reason::InvalidPath);
        assert_eq!(resolutions.get(), 2);
        assert_eq!(requests.get(), 1);

        requests.set(0);
        let error = fetch_https(
            "https://themes.example/0",
            64,
            |_| Ok(vec![ip("8.8.8.8")]),
            |_, _, _, _, _| {
                let next = requests.get() + 1;
                requests.set(next);
                Ok(response(
                    302,
                    &format!("https://redirect{next}.example/{next}"),
                    "8.8.8.8",
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.reason, Reason::InvalidPath);
        assert_eq!(requests.get(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn supported_github_and_batocera_https_flows_remain_valid() -> Result<(), ThemeError> {
        for locator in [
            "https://github.com/project/theme",
            "https://raw.githubusercontent.com/project/theme/main/theme.json",
            "https://batocera.org/upgrades/themes/Theme.jpg",
        ] {
            assert!(safe_locator(locator), "{locator}");
        }
        let body = fetch_https(
            "https://github.com/project/theme",
            32,
            |_| Ok(vec![ip("8.8.8.8")]),
            |locator, host, addresses, _, _| {
                assert_eq!(locator, "https://github.com/project/theme");
                assert_eq!(host, "github.com");
                assert_eq!(addresses, [ip("8.8.8.8")]);
                Ok(response(200, "", "8.8.8.8"))
            },
        )?;
        assert_eq!(body, b"theme");
        Ok(())
    }
}
