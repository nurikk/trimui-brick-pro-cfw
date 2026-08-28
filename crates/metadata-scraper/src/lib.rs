use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

pub const RESULT_SCHEMA: &str = "scrape-result/v1";
const CHECKPOINT_SCHEMA: &str = "metadata-scraper-checkpoint/v1";
const CHECKPOINT_FILE: &str = "scraper-queue.json";
const MAX_JSON_BYTES: usize = 1_048_576;
const MAX_ID_BYTES: usize = 128;
const MAX_STRING_BYTES: usize = 4096;
const MAX_URL_BYTES: usize = 2048;
const MAX_JOBS: usize = 256;
const MAX_ARRAY_ITEMS: usize = 64;
const MAX_RETRY_AFTER_SECS: u64 = 3600;
const MAX_BACKOFF_SECS: u64 = 3600;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Fixture,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MatchStatus {
    Matched,
    NotFound,
    Ambiguous,
}

impl MatchStatus {
    fn as_queue_state(&self) -> QueueState {
        match self {
            Self::Matched => QueueState::Succeeded,
            Self::NotFound => QueueState::NotFound,
            Self::Ambiguous => QueueState::Ambiguous,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaKind {
    BoxArt,
    Screenshot,
    TitleScreen,
    Logo,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Region {
    Global,
    NorthAmerica,
    Europe,
    Japan,
    Other,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    English,
    Japanese,
    French,
    German,
    Spanish,
    Other,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct HttpsMediaUrl(String);

impl HttpsMediaUrl {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_https_url(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for HttpsMediaUrl {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub canonical_title: String,
    pub alternate_names: Vec<String>,
    pub description: String,
    pub release_date: Option<String>,
    pub rating: Option<f32>,
    pub players: Option<u8>,
    pub genre: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub region: Option<Region>,
}

impl Metadata {
    pub fn validate(&self) -> Result<()> {
        bounded_string(&self.canonical_title, "canonical title", true)?;
        reject_private_text(&self.canonical_title, "canonical title")?;
        bounded_vec(&self.alternate_names, "alternate names")?;
        for value in &self.alternate_names {
            bounded_string(value, "alternate name", true)?;
            reject_private_text(value, "alternate name")?;
        }
        bounded_string(&self.description, "description", false)?;
        reject_private_text(&self.description, "description")?;
        for (label, value) in [
            ("release date", self.release_date.as_deref()),
            ("genre", self.genre.as_deref()),
            ("developer", self.developer.as_deref()),
            ("publisher", self.publisher.as_deref()),
        ] {
            if let Some(value) = value {
                bounded_string(value, label, true)?;
                reject_private_text(value, label)?;
            }
        }
        if let Some(rating) = self.rating {
            if !rating.is_finite() || !(0.0..=10.0).contains(&rating) {
                bail!("rating is outside 0..=10");
            }
        }
        if let Some(date) = &self.release_date {
            let bytes = date.as_bytes();
            let valid = bytes.len() == 10
                && bytes.get(4) == Some(&b'-')
                && bytes.get(7) == Some(&b'-')
                && std::str::from_utf8(&bytes[..4])
                    .ok()
                    .and_then(|value| value.parse::<u16>().ok())
                    .is_some()
                && std::str::from_utf8(&bytes[5..7])
                    .ok()
                    .and_then(|value| value.parse::<u8>().ok())
                    .is_some()
                && std::str::from_utf8(&bytes[8..])
                    .ok()
                    .and_then(|value| value.parse::<u8>().ok())
                    .is_some();
            if !valid {
                bail!("release date must be YYYY-MM-DD");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct MediaReference {
    pub kind: MediaKind,
    pub url: HttpsMediaUrl,
    pub region: Region,
    pub language: Language,
}

impl MediaReference {
    pub fn validate(&self) -> Result<()> {
        validate_https_url(self.url.as_str())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeResult {
    pub schema: String,
    pub content_id: String,
    pub system_id: String,
    pub provider: Provider,
    pub status: MatchStatus,
    pub confidence: f32,
    pub metadata: Metadata,
    pub media: Vec<MediaReference>,
}

impl ScrapeResult {
    pub fn validate(&self) -> Result<()> {
        if self.schema != RESULT_SCHEMA {
            bail!("unsupported result schema");
        }
        validate_opaque_id(&self.content_id, "content ID")?;
        validate_opaque_id(&self.system_id, "system ID")?;
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            bail!("confidence is outside 0..=1");
        }
        self.metadata.validate()?;
        bounded_vec(&self.media, "media")?;
        for media in &self.media {
            media.validate()?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        let json = serde_json::to_string_pretty(self)? + "\n";
        if json.len() > MAX_JSON_BYTES {
            bail!("result is oversized");
        }
        Ok(json)
    }

    pub fn from_json(json: &[u8]) -> Result<Self> {
        if json.len() > MAX_JSON_BYTES {
            bail!("result is oversized");
        }
        let result: Self = serde_json::from_slice(json).context("malformed scrape result")?;
        result.validate()?;
        Ok(result)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryRecord {
    pub content_id: String,
    pub system_id: String,
}

impl DiscoveryRecord {
    pub fn new(content_id: impl Into<String>, system_id: impl Into<String>) -> Self {
        Self {
            content_id: content_id.into(),
            system_id: system_id.into(),
        }
    }

    fn validate(&self) -> Result<()> {
        validate_opaque_id(&self.content_id, "content ID")?;
        validate_opaque_id(&self.system_id, "system ID")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeRequest {
    pub content_id: String,
    pub system_id: String,
    pub rom_hash: Option<String>,
    pub filename: Option<String>,
    pub title: Option<String>,
    pub manual_query: Option<String>,
    pub hash_lookup_configured: bool,
    pub region_priority: Vec<Region>,
    pub language_priority: Vec<Language>,
}

impl ScrapeRequest {
    pub fn new(content_id: impl Into<String>, system_id: impl Into<String>) -> Self {
        Self {
            content_id: content_id.into(),
            system_id: system_id.into(),
            rom_hash: None,
            filename: None,
            title: None,
            manual_query: None,
            hash_lookup_configured: false,
            region_priority: vec![Region::Global],
            language_priority: vec![Language::English],
        }
    }

    pub fn with_hash(mut self, value: impl Into<String>) -> Self {
        self.rom_hash = Some(value.into());
        self
    }

    pub fn with_hash_lookup(mut self, configured: bool) -> Self {
        self.hash_lookup_configured = configured;
        self
    }

    pub fn with_filename(mut self, value: impl Into<String>) -> Self {
        self.filename = Some(value.into());
        self
    }

    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = Some(value.into());
        self
    }

    pub fn with_manual_query(mut self, value: impl Into<String>) -> Self {
        self.manual_query = Some(value.into());
        self
    }

    pub fn with_priorities(mut self, regions: Vec<Region>, languages: Vec<Language>) -> Self {
        self.region_priority = regions;
        self.language_priority = languages;
        self
    }

    pub fn validate(&self) -> Result<()> {
        validate_opaque_id(&self.content_id, "content ID")?;
        validate_opaque_id(&self.system_id, "system ID")?;
        for (label, value) in [
            ("ROM hash", self.rom_hash.as_deref()),
            ("filename", self.filename.as_deref()),
            ("title", self.title.as_deref()),
            ("manual query", self.manual_query.as_deref()),
        ] {
            if let Some(value) = value {
                bounded_string(value, label, true)?;
                if value.contains('\0') {
                    bail!("{label} contains NUL");
                }
                if label == "filename" && (value.contains('/') || value.contains('\\')) {
                    bail!("filename cannot contain a path");
                }
                if label == "ROM hash" && value.len() > 256 {
                    bail!("ROM hash is oversized");
                }
            }
        }
        bounded_vec(&self.region_priority, "region priority")?;
        bounded_vec(&self.language_priority, "language priority")?;
        Ok(())
    }
}

pub enum ProviderResponse {
    Result(Box<ScrapeResult>),
    Retry {
        reason: String,
        retry_after_secs: Option<u64>,
    },
    Failed {
        reason: String,
    },
}

pub trait MetadataProvider {
    fn scrape(&mut self, request: &ScrapeRequest) -> ProviderResponse;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryKind {
    Hash,
    NormalizedFilenameOrTitle,
    Manual,
}

#[derive(Default)]
pub struct FixtureProvider {
    attempts: HashMap<String, u8>,
    query_history: Vec<QueryKind>,
}

impl FixtureProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_query_kind(&self) -> Option<QueryKind> {
        self.query_history.last().cloned()
    }

    pub fn query_history(&self) -> &[QueryKind] {
        &self.query_history
    }

    pub fn scrape(&mut self, request: &ScrapeRequest) -> Result<ScrapeResult> {
        match <Self as MetadataProvider>::scrape(self, request) {
            ProviderResponse::Result(result) => Ok(*result),
            ProviderResponse::Retry { reason, .. } | ProviderResponse::Failed { reason } => {
                bail!("{reason}")
            }
        }
    }

    fn success_result(&self, request: &ScrapeRequest, title: String) -> ScrapeResult {
        let region = request
            .region_priority
            .first()
            .cloned()
            .unwrap_or(Region::Global);
        let language = request
            .language_priority
            .first()
            .cloned()
            .unwrap_or(Language::English);
        ScrapeResult {
            schema: RESULT_SCHEMA.to_string(),
            content_id: request.content_id.clone(),
            system_id: request.system_id.clone(),
            provider: Provider::Fixture,
            status: MatchStatus::Matched,
            confidence: 0.98,
            metadata: Metadata {
                canonical_title: title,
                alternate_names: vec!["Synthetic Quest DX".to_string()],
                description: "Generated fixture metadata for contract testing.".to_string(),
                release_date: Some("2024-01-02".to_string()),
                rating: Some(8.0),
                players: Some(2),
                genre: Some("adventure".to_string()),
                developer: Some("Synthetic Studio".to_string()),
                publisher: Some("Fixture Works".to_string()),
                region: Some(region.clone()),
            },
            media: vec![MediaReference {
                kind: MediaKind::BoxArt,
                url: HttpsMediaUrl::parse("https://example.invalid/generated/box-art.svg")
                    .expect("fixture URL is valid"),
                region,
                language,
            }],
        }
    }
}

impl MetadataProvider for FixtureProvider {
    fn scrape(&mut self, request: &ScrapeRequest) -> ProviderResponse {
        if let Err(error) = request.validate() {
            return ProviderResponse::Failed {
                reason: safe_reason(&error.to_string()),
            };
        }

        self.query_history.clear();
        let key = request.content_id.as_str();
        if request.hash_lookup_configured && request.rom_hash.is_some() {
            self.query_history.push(QueryKind::Hash);
        }
        if let Some(value) = request.filename.as_deref().or(request.title.as_deref()) {
            self.query_history
                .push(QueryKind::NormalizedFilenameOrTitle);
            let _normalized = normalize_query(value);
        }
        if let Some(value) = request.manual_query.as_deref() {
            self.query_history.push(QueryKind::Manual);
            let _normalized = normalize_query(value);
        }
        if self.query_history.is_empty() {
            self.query_history.push(QueryKind::Manual);
        }
        match key {
            "not-found" => ProviderResponse::Result(Box::new(ScrapeResult {
                status: MatchStatus::NotFound,
                ..self.success_result(request, "Synthetic Missing Record".to_string())
            })),
            "ambiguous" => ProviderResponse::Result(Box::new(ScrapeResult {
                status: MatchStatus::Ambiguous,
                confidence: 0.5,
                ..self.success_result(request, "Synthetic Ambiguous Record".to_string())
            })),
            "retry" | "rate-limit" => {
                let attempts = self.attempts.entry(key.to_string()).or_default();
                if *attempts == 0 {
                    *attempts = 1;
                    ProviderResponse::Retry {
                        reason: if key == "rate-limit" {
                            "provider-rate-limit".to_string()
                        } else {
                            "provider-temporary-failure".to_string()
                        },
                        retry_after_secs: Some(if key == "rate-limit" { 3 } else { 2 }),
                    }
                } else {
                    ProviderResponse::Result(Box::new(
                        self.success_result(
                            request,
                            request
                                .title
                                .clone()
                                .unwrap_or_else(|| "Synthetic Retry Record".to_string()),
                        ),
                    ))
                }
            }
            "manual" if request.manual_query.is_some() => ProviderResponse::Result(Box::new(
                self.success_result(request, "Synthetic Manual Record".to_string()),
            )),
            "success" | "hash-success" | "hash-miss" | "filename-success" | "manual-success" => {
                ProviderResponse::Result(Box::new(
                    self.success_result(
                        request,
                        request
                            .title
                            .clone()
                            .unwrap_or_else(|| "Synthetic Quest".to_string()),
                    ),
                ))
            }
            _ => ProviderResponse::Result(Box::new(ScrapeResult {
                status: MatchStatus::NotFound,
                ..self.success_result(request, "Synthetic Missing Record".to_string())
            })),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QueueState {
    Pending,
    Running,
    Retry,
    Succeeded,
    NotFound,
    Ambiguous,
    Failed,
    Cancelled,
}

impl QueueState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Retry => "retry",
            Self::Succeeded => "succeeded",
            Self::NotFound => "not-found",
            Self::Ambiguous => "ambiguous",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::NotFound | Self::Ambiguous | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub request: ScrapeRequest,
    pub state: QueueState,
    pub attempts: u32,
    pub next_attempt_at: u64,
    pub overwrite_metadata: bool,
    pub overwrite_media: bool,
    pub result: Option<ScrapeResult>,
    pub reason: Option<String>,
}

impl QueueItem {
    pub fn validate(&self) -> Result<()> {
        self.request.validate()?;
        if self.attempts > 100 {
            bail!("attempt count is too large");
        }
        if let Some(result) = &self.result {
            result.validate()?;
        }
        if let Some(reason) = &self.reason {
            validate_safe_reason(reason)?;
        }
        if self.state == QueueState::Running && self.result.is_some() {
            bail!("running item cannot have a result");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PersistedRequest {
    content_id: String,
    system_id: String,
    hash_lookup_configured: bool,
    region_priority: Vec<Region>,
    language_priority: Vec<Language>,
}

impl From<&ScrapeRequest> for PersistedRequest {
    fn from(request: &ScrapeRequest) -> Self {
        Self {
            content_id: request.content_id.clone(),
            system_id: request.system_id.clone(),
            hash_lookup_configured: request.hash_lookup_configured,
            region_priority: request.region_priority.clone(),
            language_priority: request.language_priority.clone(),
        }
    }
}

impl From<PersistedRequest> for ScrapeRequest {
    fn from(request: PersistedRequest) -> Self {
        Self {
            content_id: request.content_id,
            system_id: request.system_id,
            rom_hash: None,
            filename: None,
            title: None,
            manual_query: None,
            hash_lookup_configured: request.hash_lookup_configured,
            region_priority: request.region_priority,
            language_priority: request.language_priority,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedQueueItem {
    request: PersistedRequest,
    state: QueueState,
    attempts: u32,
    next_attempt_at: u64,
    overwrite_metadata: bool,
    overwrite_media: bool,
    result: Option<ScrapeResult>,
    reason: Option<String>,
}

impl From<PersistedQueueItem> for QueueItem {
    fn from(item: PersistedQueueItem) -> Self {
        Self {
            request: item.request.into(),
            state: item.state,
            attempts: item.attempts,
            next_attempt_at: item.next_attempt_at,
            overwrite_metadata: item.overwrite_metadata,
            overwrite_media: item.overwrite_media,
            result: item.result,
            reason: item.reason,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QueueCheckpoint {
    schema: String,
    paused: bool,
    jobs: Vec<PersistedQueueItem>,
}

#[derive(Clone, Copy, Debug)]
pub struct SchedulingPolicy {
    pub wifi_available: bool,
    pub suspended: bool,
    pub low_battery: bool,
    pub foreground_gameplay: bool,
    pub active_jobs: usize,
    pub max_concurrency: usize,
    pub storage_used_bytes: u64,
    pub storage_quota_bytes: u64,
}

impl Default for SchedulingPolicy {
    fn default() -> Self {
        Self {
            wifi_available: true,
            suspended: false,
            low_battery: false,
            foreground_gameplay: false,
            active_jobs: 0,
            max_concurrency: 2,
            storage_used_bytes: 0,
            storage_quota_bytes: u64::MAX,
        }
    }
}

impl SchedulingPolicy {
    fn validate(&self) -> Result<()> {
        if !self.wifi_available {
            bail!("wifi-unavailable");
        }
        if self.suspended {
            bail!("suspended");
        }
        if self.low_battery {
            bail!("low-battery");
        }
        if self.foreground_gameplay {
            bail!("foreground-gameplay");
        }
        if !matches!(self.max_concurrency, 1 | 2 | 4) || self.active_jobs >= self.max_concurrency {
            bail!("concurrency-limit");
        }
        if self.storage_used_bytes > self.storage_quota_bytes {
            bail!("storage-quota");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgressSummary {
    pub pending: usize,
    pub running: usize,
    pub retry: usize,
    pub succeeded: usize,
    pub not_found: usize,
    pub ambiguous: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PublicEvent {
    pub content_id: String,
    pub provider: Provider,
    pub state: QueueState,
    pub reason: Option<String>,
}

impl PublicEvent {
    fn from_item(item: &QueueItem) -> Self {
        Self {
            content_id: item.request.content_id.clone(),
            provider: Provider::Fixture,
            state: item.state.clone(),
            reason: item.reason.clone(),
        }
    }
}

pub enum DispatchOutcome {
    Dispatched(PublicEvent),
    None,
}

pub struct Queue {
    root: PathBuf,
    paused: bool,
    jobs: Vec<QueueItem>,
}

impl Queue {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = prepare_root(root.as_ref())?;
        let checkpoint = root.join(CHECKPOINT_FILE);
        let checkpoint_exists = match fs::symlink_metadata(&checkpoint) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error).context("inspect scraper checkpoint"),
        };
        let mut queue = if !checkpoint_exists {
            Self {
                root,
                paused: false,
                jobs: Vec::new(),
            }
        } else {
            refuse_symlink(&checkpoint)?;
            let bytes = fs::read(&checkpoint).context("read scraper checkpoint")?;
            if bytes.len() > MAX_JSON_BYTES {
                bail!("checkpoint is oversized");
            }
            let checkpoint: QueueCheckpoint =
                serde_json::from_slice(&bytes).context("malformed scraper checkpoint")?;
            if checkpoint.schema != CHECKPOINT_SCHEMA || checkpoint.jobs.len() > MAX_JOBS {
                bail!("unsupported scraper checkpoint");
            }
            let jobs = checkpoint
                .jobs
                .into_iter()
                .map(QueueItem::from)
                .collect::<Vec<_>>();
            for job in &jobs {
                job.validate()?;
            }
            Self {
                root,
                paused: checkpoint.paused,
                jobs,
            }
        };
        if queue
            .jobs
            .iter_mut()
            .any(|job| job.state == QueueState::Running)
        {
            for job in &mut queue.jobs {
                if job.state == QueueState::Running {
                    job.state = QueueState::Retry;
                    job.reason = Some("recovered-running".to_string());
                    job.next_attempt_at = 0;
                }
            }
            queue.persist()?;
        }
        Ok(queue)
    }

    pub fn enqueue(
        &mut self,
        request: ScrapeRequest,
        overwrite_metadata: bool,
        overwrite_media: bool,
    ) -> Result<()> {
        request.validate()?;
        if self.jobs.len() >= MAX_JOBS {
            bail!("queue is full");
        }
        if self.jobs.iter().any(|job| same_job(&job.request, &request)) {
            bail!("job already exists");
        }
        self.jobs.push(QueueItem {
            request,
            state: QueueState::Pending,
            attempts: 0,
            next_attempt_at: 0,
            overwrite_metadata,
            overwrite_media,
            result: None,
            reason: None,
        });
        self.persist()
    }

    pub fn enqueue_bulk(
        &mut self,
        requests: impl IntoIterator<Item = ScrapeRequest>,
        overwrite_metadata: bool,
        overwrite_media: bool,
    ) -> Result<usize> {
        let requests: Vec<_> = requests.into_iter().collect();
        if self.jobs.len() + requests.len() > MAX_JOBS {
            bail!("queue is full");
        }
        for (index, request) in requests.iter().enumerate() {
            request.validate()?;
            if self.jobs.iter().any(|job| same_job(&job.request, request))
                || requests[..index]
                    .iter()
                    .any(|candidate| same_job(candidate, request))
            {
                bail!("duplicate job");
            }
        }
        let count = requests.len();
        for request in requests {
            self.jobs.push(QueueItem {
                request,
                state: QueueState::Pending,
                attempts: 0,
                next_attempt_at: 0,
                overwrite_metadata,
                overwrite_media,
                result: None,
                reason: None,
            });
        }
        self.persist()?;
        Ok(count)
    }

    pub fn automatic_enqueue(
        &mut self,
        request: ScrapeRequest,
        policy: SchedulingPolicy,
        overwrite_metadata: bool,
        overwrite_media: bool,
    ) -> Result<()> {
        policy.validate()?;
        self.enqueue(request, overwrite_metadata, overwrite_media)
    }

    pub fn enqueue_discovered(
        &mut self,
        discovery: DiscoveryRecord,
        policy: SchedulingPolicy,
        overwrite_metadata: bool,
        overwrite_media: bool,
    ) -> Result<()> {
        discovery.validate()?;
        self.automatic_enqueue(
            ScrapeRequest::new(discovery.content_id, discovery.system_id),
            policy,
            overwrite_metadata,
            overwrite_media,
        )
    }

    pub fn pause(&mut self) -> Result<()> {
        self.paused = true;
        self.persist()
    }

    pub fn resume(&mut self) -> Result<()> {
        self.paused = false;
        self.persist()
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn cancel(&mut self, content_id: &str) -> Result<()> {
        let job = self.find_mut(content_id)?;
        if !job.state.terminal() {
            job.state = QueueState::Cancelled;
            job.reason = Some("cancelled-by-user".to_string());
            job.result = None;
            self.persist()?;
        }
        Ok(())
    }

    pub fn mark_running(&mut self, content_id: &str) -> Result<()> {
        let job = self.find_mut(content_id)?;
        if !matches!(job.state, QueueState::Pending | QueueState::Retry) {
            bail!("job is not dispatchable");
        }
        job.state = QueueState::Running;
        job.reason = None;
        self.persist()
    }

    pub fn dispatch_at<P: MetadataProvider>(
        &mut self,
        provider: &mut P,
        policy: SchedulingPolicy,
        now: u64,
    ) -> Result<DispatchOutcome> {
        if self.paused {
            return Ok(DispatchOutcome::None);
        }
        policy.validate()?;
        let index = match self.jobs.iter().position(|job| {
            matches!(job.state, QueueState::Pending | QueueState::Retry)
                && job.next_attempt_at <= now
        }) {
            Some(index) => index,
            None => return Ok(DispatchOutcome::None),
        };
        if self.jobs[index].attempts >= 100 {
            self.jobs[index].state = QueueState::Failed;
            self.jobs[index].reason = Some("attempt-limit".to_string());
            self.persist()?;
            return Ok(DispatchOutcome::Dispatched(PublicEvent::from_item(
                &self.jobs[index],
            )));
        }
        self.jobs[index].state = QueueState::Running;
        self.jobs[index].attempts += 1;
        self.jobs[index].reason = None;
        self.persist()?;
        let request = self.jobs[index].request.clone();
        let response = provider.scrape(&request);
        match response {
            ProviderResponse::Result(result) => {
                let result = *result;
                result.validate()?;
                if result.content_id != request.content_id || result.system_id != request.system_id
                {
                    bail!("provider result IDs do not match request");
                }
                self.jobs[index].state = result.status.as_queue_state();
                self.jobs[index].result = Some(result);
                self.jobs[index].next_attempt_at = 0;
            }
            ProviderResponse::Retry {
                reason,
                retry_after_secs,
            } => {
                validate_safe_reason(&reason)?;
                let retry_after = retry_after_secs.unwrap_or(0);
                if retry_after > MAX_RETRY_AFTER_SECS {
                    bail!("retry-after is oversized");
                }
                let exponential = 2_u64.saturating_pow(self.jobs[index].attempts.min(11));
                let delay = retry_after.max(exponential.min(MAX_BACKOFF_SECS));
                self.jobs[index].state = QueueState::Retry;
                self.jobs[index].next_attempt_at = now.saturating_add(delay);
                self.jobs[index].reason = Some(reason);
            }
            ProviderResponse::Failed { reason } => {
                validate_safe_reason(&reason)?;
                self.jobs[index].state = QueueState::Failed;
                self.jobs[index].reason = Some(reason);
            }
        }
        self.persist()?;
        Ok(DispatchOutcome::Dispatched(PublicEvent::from_item(
            &self.jobs[index],
        )))
    }

    pub fn get(&self, content_id: &str) -> Option<&QueueItem> {
        self.jobs
            .iter()
            .find(|job| job.request.content_id == content_id)
    }

    pub fn pending_requests(&self) -> Vec<ScrapeRequest> {
        self.jobs
            .iter()
            .filter(|job| matches!(job.state, QueueState::Pending | QueueState::Retry))
            .map(|job| job.request.clone())
            .collect()
    }

    pub fn finalize(
        &mut self,
        content_id: &str,
        state: QueueState,
        result: Option<ScrapeResult>,
        reason: Option<String>,
    ) -> Result<()> {
        if !state.terminal() {
            bail!("bulk finalization requires a terminal state");
        }
        if let Some(result) = &result {
            result.validate()?;
            if result.content_id != content_id {
                bail!("bulk result content ID does not match job");
            }
        }
        if let Some(reason) = &reason {
            validate_safe_reason(reason)?;
        }
        let job = self.find_mut(content_id)?;
        if job.state.terminal() {
            return Ok(());
        }
        job.state = state;
        job.result = result;
        job.reason = reason;
        job.next_attempt_at = 0;
        self.persist()
    }

    pub fn finalize_bulk(&mut self, run: &BulkRun) -> Result<()> {
        for item in &run.results {
            self.finalize(
                &item.content_id,
                item.state.clone(),
                item.result.clone(),
                item.reason.clone(),
            )?;
        }
        Ok(())
    }

    pub fn dispatch_bulk(
        &mut self,
        settings: &ScraperSettings,
        providers: Vec<Arc<dyn BulkProvider>>,
        observer: Option<&dyn BulkObserver>,
    ) -> Result<BulkRun> {
        let policy = SchedulingPolicy {
            max_concurrency: settings.parallel_jobs as usize,
            ..SchedulingPolicy::default()
        };
        self.dispatch_bulk_with_policy(settings, providers, policy, observer)
    }

    pub fn dispatch_bulk_with_media_publisher(
        &mut self,
        settings: &ScraperSettings,
        providers: Vec<Arc<dyn BulkProvider>>,
        publisher: Arc<dyn BulkMediaPublisher>,
        observer: Option<&dyn BulkObserver>,
    ) -> Result<BulkRun> {
        let policy = SchedulingPolicy {
            max_concurrency: settings.parallel_jobs as usize,
            ..SchedulingPolicy::default()
        };
        self.dispatch_bulk_with_policy_and_publisher(
            settings,
            providers,
            policy,
            Some(publisher),
            observer,
        )
    }

    pub fn dispatch_bulk_with_policy(
        &mut self,
        settings: &ScraperSettings,
        providers: Vec<Arc<dyn BulkProvider>>,
        policy: SchedulingPolicy,
        observer: Option<&dyn BulkObserver>,
    ) -> Result<BulkRun> {
        self.dispatch_bulk_with_policy_and_publisher(settings, providers, policy, None, observer)
    }

    fn dispatch_bulk_with_policy_and_publisher(
        &mut self,
        settings: &ScraperSettings,
        providers: Vec<Arc<dyn BulkProvider>>,
        policy: SchedulingPolicy,
        publisher: Option<Arc<dyn BulkMediaPublisher>>,
        observer: Option<&dyn BulkObserver>,
    ) -> Result<BulkRun> {
        policy.validate()?;
        if self.paused {
            bail!("queue-paused");
        }
        let now = unix_time_secs();
        let requests: Vec<_> = self
            .jobs
            .iter()
            .filter(|job| {
                matches!(job.state, QueueState::Pending)
                    || matches!(job.state, QueueState::Retry) && job.next_attempt_at <= now
            })
            .map(|job| job.request.clone())
            .collect();
        let jobs = requests
            .into_iter()
            .map(|request| {
                let content_id = request.content_id.clone();
                BulkJob::new(content_id.clone(), content_id, request)
            })
            .collect::<Vec<_>>();
        settings.validate()?;
        let global_slots = settings.parallel_jobs.min(policy.max_concurrency as u8) as usize;
        if policy.active_jobs >= global_slots {
            bail!("concurrency-limit");
        }
        let available_slots = (global_slots - policy.active_jobs) as u8;
        for job in &jobs {
            self.mark_running(&job.content_id)?;
        }
        let mut persist = |item: &BulkItemResult| {
            self.finalize(
                &item.content_id,
                item.state.clone(),
                item.result.clone(),
                item.reason.clone(),
            )
        };
        scrape_bulk_with_worker_limit(
            jobs,
            settings,
            providers,
            available_slots,
            publisher,
            observer,
            Some(&mut persist),
        )
    }

    pub fn get_for(&self, content_id: &str, system_id: &str) -> Option<&QueueItem> {
        self.jobs
            .iter()
            .find(|job| job.request.content_id == content_id && job.request.system_id == system_id)
    }

    pub fn dispatch<P: MetadataProvider>(
        &mut self,
        provider: &mut P,
        policy: SchedulingPolicy,
    ) -> Result<DispatchOutcome> {
        self.dispatch_at(provider, policy, unix_time_secs())
    }

    pub fn progress(&self) -> ProgressSummary {
        let mut summary = ProgressSummary::default();
        for job in &self.jobs {
            match job.state {
                QueueState::Pending => summary.pending += 1,
                QueueState::Running => summary.running += 1,
                QueueState::Retry => summary.retry += 1,
                QueueState::Succeeded => summary.succeeded += 1,
                QueueState::NotFound => summary.not_found += 1,
                QueueState::Ambiguous => summary.ambiguous += 1,
                QueueState::Failed => summary.failed += 1,
                QueueState::Cancelled => summary.cancelled += 1,
            }
        }
        summary
    }

    pub fn public_events(&self) -> Vec<PublicEvent> {
        self.jobs.iter().map(PublicEvent::from_item).collect()
    }

    fn find_mut(&mut self, content_id: &str) -> Result<&mut QueueItem> {
        self.jobs
            .iter_mut()
            .find(|job| job.request.content_id == content_id)
            .ok_or_else(|| anyhow!("job not found"))
    }

    fn persist(&self) -> Result<()> {
        let checkpoint = QueueCheckpoint {
            schema: CHECKPOINT_SCHEMA.to_string(),
            paused: self.paused,
            jobs: self
                .jobs
                .iter()
                .map(|job| PersistedQueueItem {
                    request: (&job.request).into(),
                    state: job.state.clone(),
                    attempts: job.attempts,
                    next_attempt_at: job.next_attempt_at,
                    overwrite_metadata: job.overwrite_metadata,
                    overwrite_media: job.overwrite_media,
                    result: job.result.clone(),
                    reason: job.reason.clone(),
                })
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&checkpoint)?;
        if bytes.len() > MAX_JSON_BYTES {
            bail!("checkpoint is oversized");
        }
        atomic_write(&self.root, CHECKPOINT_FILE, &bytes)
    }
}

pub fn normalize_query(value: &str) -> String {
    let mut normalized = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
        } else if !normalized.ends_with(' ') {
            normalized.push(' ');
        }
    }
    normalized.trim().to_string()
}

fn same_job(left: &ScrapeRequest, right: &ScrapeRequest) -> bool {
    left.content_id == right.content_id && left.system_id == right.system_id
}

fn prepare_root(root: &Path) -> Result<PathBuf> {
    if root.as_os_str().is_empty() {
        bail!("fixture root is empty");
    }
    if !root.exists() {
        fs::create_dir_all(root).context("create fixture root")?;
    }
    refuse_symlink(root)?;
    if !root.is_dir() {
        bail!("fixture root is not a directory");
    }
    let root = root.canonicalize().context("canonicalize fixture root")?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if root == Path::new("/data")
        || root.starts_with("/data")
        || root == Path::new("/roms")
        || root.starts_with("/roms")
        || home
            .as_deref()
            .is_some_and(|path| root == path || root.starts_with(path))
    {
        bail!("reserved filesystem root is not allowed");
    }
    Ok(root)
}

fn refuse_symlink(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("symlink path is not allowed: {}", path.display());
    }
    Ok(())
}

fn atomic_write(root: &Path, filename: &str, bytes: &[u8]) -> Result<()> {
    let destination = root.join(filename);
    refuse_symlink(root)?;
    if fs::symlink_metadata(&destination).is_ok() {
        refuse_symlink(&destination)?;
    }
    let temporary = root.join(format!("{filename}.tmp"));
    if fs::symlink_metadata(&temporary).is_ok() {
        refuse_symlink(&temporary)?;
        fs::remove_file(&temporary).context("remove stale checkpoint temporary")?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .context("create checkpoint temporary")?;
    file.write_all(bytes)
        .context("write checkpoint temporary")?;
    file.sync_all().context("sync checkpoint temporary")?;
    drop(file);
    fs::rename(&temporary, &destination).context("atomically replace checkpoint")?;
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .context("sync checkpoint directory")?;
    Ok(())
}

fn bounded_vec<T>(values: &[T], label: &str) -> Result<()> {
    if values.len() > MAX_ARRAY_ITEMS {
        bail!("{label} is oversized");
    }
    Ok(())
}

fn bounded_string(value: &str, label: &str, nonempty: bool) -> Result<()> {
    if (nonempty && value.is_empty()) || value.len() > MAX_STRING_BYTES {
        bail!("{label} is invalid or oversized");
    }
    if value.chars().any(char::is_control) {
        bail!("{label} contains control characters");
    }
    Ok(())
}

fn validate_opaque_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_ID_BYTES || value == "." || value == ".." {
        bail!("{label} is invalid");
    }
    if value
        .chars()
        .any(|character| !(character.is_ascii_alphanumeric() || ".:_-".contains(character)))
    {
        bail!("{label} is not opaque-safe");
    }
    Ok(())
}

fn reject_private_text(value: &str, label: &str) -> Result<()> {
    let lower = value.to_ascii_lowercase();
    if [
        "credential",
        "password",
        "secret",
        "token",
        "authorization",
        "rom/",
        "roms/",
        "bios/",
        "portmaster",
        "filename",
        "source bytes",
        "private key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || value.contains('/')
        || value.contains('\\')
    {
        bail!("{label} contains private provider data");
    }
    Ok(())
}

fn validate_safe_reason(reason: &str) -> Result<()> {
    if reason.is_empty() || reason.len() > 128 || reason.chars().any(char::is_control) {
        bail!("reason is invalid or oversized");
    }
    let lower = reason.to_ascii_lowercase();
    if reason.contains('/')
        || reason.contains('\\')
        || [
            "filename",
            "title",
            "path",
            "hash",
            "url",
            "query",
            "source",
            "rom",
            "credential",
            "secret",
            "token",
            "description",
        ]
        .iter()
        .any(|word| lower.contains(word))
    {
        bail!("reason contains private data marker");
    }
    Ok(())
}

fn safe_reason(reason: &str) -> String {
    let reason = reason
        .split(':')
        .next()
        .unwrap_or("provider-rejected")
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .take(64)
        .collect::<String>();
    if reason.is_empty() {
        "provider-rejected".to_string()
    } else {
        reason
    }
}

fn validate_https_url(value: &str) -> Result<()> {
    if value.len() > MAX_URL_BYTES
        || value.is_empty()
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || !value.starts_with("https://")
    {
        bail!("media URL must be a bounded HTTPS URL");
    }
    let authority_end = value[8..]
        .find(['/', '?', '#'])
        .map(|offset| offset + 8)
        .unwrap_or(value.len());
    let authority = &value[8..authority_end];
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        bail!("media URL authority is unsafe");
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| anyhow!("malformed IPv6 host"))?;
        let suffix = &rest[end + 1..];
        if !suffix.is_empty() && !suffix.starts_with(':') {
            bail!("malformed URL port");
        }
        (&rest[..end], suffix.strip_prefix(':'))
    } else if authority.matches(':').count() > 1 {
        bail!("unbracketed IPv6 host");
    } else {
        let mut parts = authority.split(':');
        let host = parts.next().unwrap_or_default();
        let port = parts.next();
        if parts.next().is_some() {
            bail!("malformed URL port");
        }
        (host, port)
    };
    if host.is_empty() {
        bail!("media URL host is missing");
    }
    if let Some(port) = port {
        if port.is_empty() || port.parse::<u16>().is_err() {
            bail!("media URL port is invalid");
        }
    }
    let lower_host = host.to_ascii_lowercase();
    if lower_host == "localhost" || lower_host.ends_with(".localhost") {
        bail!("localhost media URL is forbidden");
    }
    if let Ok(address) = lower_host.parse::<IpAddr>() {
        if unsafe_ip(address) {
            bail!("private or unspecified media URL address is forbidden");
        }
    } else if !valid_dns_name(&lower_host) {
        bail!("media URL host is malformed");
    }
    Ok(())
}

fn valid_dns_name(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
}

fn unsafe_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_multicast()
                || address == Ipv4Addr::BROADCAST
                || address.octets()[0] == 100 && (64..=127).contains(&address.octets()[1])
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            let mapped_v4 = segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff;
            address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address == Ipv6Addr::LOCALHOST
                || (mapped_v4
                    && unsafe_ip(IpAddr::V4(Ipv4Addr::new(
                        (segments[6] >> 8) as u8,
                        segments[6] as u8,
                        (segments[7] >> 8) as u8,
                        segments[7] as u8,
                    ))))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderDeclaration {
    pub id: String,
    pub enabled: bool,
    pub requires_credentials: bool,
    pub credential_configured: bool,
    pub priority: u8,
    pub max_concurrency: u8,
}

impl ProviderDeclaration {
    pub fn validate(&self) -> Result<()> {
        validate_opaque_id(&self.id, "provider ID")?;
        if !matches!(self.priority, 1..=3) || !matches!(self.max_concurrency, 1 | 2 | 4) {
            bail!("provider limits are outside bounds");
        }
        if !self.requires_credentials && self.credential_configured {
            bail!("anonymous provider cannot have credential status");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ScraperSettings {
    pub parallel_jobs: u8,
    pub providers: Vec<ProviderDeclaration>,
}

impl Default for ScraperSettings {
    fn default() -> Self {
        Self {
            parallel_jobs: 2,
            providers: registered_providers(),
        }
    }
}

impl ScraperSettings {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.parallel_jobs, 1 | 2 | 4) {
            bail!("parallel jobs must be 1, 2, or 4");
        }
        if self.providers.is_empty() || self.providers.len() > 3 {
            bail!("provider declarations are outside bounds");
        }
        let mut priorities = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for provider in &self.providers {
            provider.validate()?;
            if !ids.insert(provider.id.as_str()) || !priorities.insert(provider.priority) {
                bail!("provider IDs and priorities must be unique");
            }
        }
        Ok(())
    }

    pub fn enabled_in_priority(&self) -> Vec<&ProviderDeclaration> {
        let mut providers: Vec<_> = self
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .collect();
        providers.sort_by_key(|provider| provider.priority);
        providers
    }
}

pub fn registered_providers() -> Vec<ProviderDeclaration> {
    vec![
        ProviderDeclaration {
            id: "fixture-primary".into(),
            enabled: true,
            requires_credentials: true,
            credential_configured: false,
            priority: 1,
            max_concurrency: 1,
        },
        ProviderDeclaration {
            id: "fixture-secondary".into(),
            enabled: true,
            requires_credentials: false,
            credential_configured: false,
            priority: 2,
            max_concurrency: 2,
        },
        ProviderDeclaration {
            id: "fixture-tertiary".into(),
            enabled: true,
            requires_credentials: false,
            credential_configured: false,
            priority: 3,
            max_concurrency: 2,
        },
    ]
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BulkPhase {
    Searching,
    Retrying,
    FallingBack,
    DownloadingCover,
    Publishing,
    Done,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BulkJob {
    pub content_id: String,
    pub display_title: String,
    pub request: ScrapeRequest,
}

impl BulkJob {
    pub fn new(
        content_id: impl Into<String>,
        display_title: impl Into<String>,
        request: ScrapeRequest,
    ) -> Self {
        Self {
            content_id: content_id.into(),
            display_title: display_title.into(),
            request,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BulkRow {
    pub content_id: String,
    pub display_title: String,
    pub provider: Option<String>,
    pub phase: BulkPhase,
    pub fallback_transition: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BulkCounts {
    pub succeeded: usize,
    pub fallback: usize,
    pub not_found: usize,
    pub ambiguous: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BulkProgress {
    pub completed: usize,
    pub total: usize,
    pub percent: u8,
    pub active_slots: u8,
    pub rows: Vec<BulkRow>,
    pub counts: BulkCounts,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub background: bool,
}

#[derive(Clone, Debug)]
pub struct BulkItemResult {
    pub content_id: String,
    pub state: QueueState,
    pub result: Option<ScrapeResult>,
    pub reason: Option<String>,
    pub fallback: bool,
}

#[derive(Clone, Debug)]
pub struct BulkRun {
    pub progress: BulkProgress,
    pub results: Vec<BulkItemResult>,
}

pub trait BulkObserver: Send + Sync {
    fn progress(&self, progress: &BulkProgress);
}

type BulkCompletionCallback<'a> = &'a mut dyn FnMut(&BulkItemResult) -> Result<()>;

pub trait BulkProvider: Send + Sync {
    fn declaration(&self) -> &ProviderDeclaration;
    fn scrape(&self, request: &ScrapeRequest) -> ProviderResponse;
}

pub trait BulkMediaPublisher: Send + Sync {
    fn publish(&self, provider_id: &str, result: &ScrapeResult) -> Result<()>;
}

pub struct MediaCachePublisher {
    cache: Arc<media_cache::MediaCache>,
    transport: Arc<dyn media_cache::Transport>,
}

impl MediaCachePublisher {
    pub fn new(
        cache: Arc<media_cache::MediaCache>,
        transport: Arc<dyn media_cache::Transport>,
    ) -> Self {
        Self { cache, transport }
    }
}

impl BulkMediaPublisher for MediaCachePublisher {
    fn publish(&self, provider_id: &str, result: &ScrapeResult) -> Result<()> {
        for media in result
            .media
            .iter()
            .filter(|media| media.kind == MediaKind::BoxArt)
        {
            let reference = media_cache::MediaReference {
                content_id: result.content_id.clone(),
                kind: match media.kind {
                    MediaKind::BoxArt => media_cache::MediaKind::BoxArt,
                    MediaKind::Screenshot => media_cache::MediaKind::Screenshot,
                    MediaKind::TitleScreen => media_cache::MediaKind::TitleScreen,
                    MediaKind::Logo => media_cache::MediaKind::Logo,
                },
                url: media.url.as_str().to_owned(),
                region: Some(format!("{:?}", media.region).to_ascii_lowercase()),
                language: Some(format!("{:?}", media.language).to_ascii_lowercase()),
                provider: provider_id.to_owned(),
            };
            let profile = match media.kind {
                MediaKind::BoxArt => media_cache::Profile::Cover,
                MediaKind::Screenshot => media_cache::Profile::ScreenshotPreview,
                MediaKind::TitleScreen => media_cache::Profile::TitleScreen,
                MediaKind::Logo => media_cache::Profile::TransparentLogo,
            };
            self.cache
                .ingest(&reference, profile, self.transport.as_ref())?;
        }
        Ok(())
    }
}

pub struct FixtureBulkProvider {
    declaration: ProviderDeclaration,
    provider: Mutex<FixtureProvider>,
}

impl FixtureBulkProvider {
    pub fn new(declaration: ProviderDeclaration) -> Result<Self> {
        declaration.validate()?;
        Ok(Self {
            declaration,
            provider: Mutex::new(FixtureProvider::new()),
        })
    }
}

impl BulkProvider for FixtureBulkProvider {
    fn declaration(&self) -> &ProviderDeclaration {
        &self.declaration
    }

    fn scrape(&self, request: &ScrapeRequest) -> ProviderResponse {
        let id = self.declaration.id.as_str();
        if id == "fixture-primary" && request.content_id == "malformed" {
            let mut result = self
                .provider
                .lock()
                .expect("fixture provider lock")
                .success_result(request, "Synthetic Malformed Record".into());
            result.metadata.description = "private /roms provider payload".into();
            return ProviderResponse::Result(Box::new(result));
        }
        if id == "fixture-primary"
            && matches!(
                request.content_id.as_str(),
                "fallback-2" | "fallback-3" | "absent-cover" | "auth"
            )
        {
            if request.content_id == "auth" {
                return ProviderResponse::Failed {
                    reason: "auth-failure".into(),
                };
            }
            return ProviderResponse::Result(Box::new(ScrapeResult {
                status: MatchStatus::NotFound,
                ..self
                    .provider
                    .lock()
                    .expect("fixture provider lock")
                    .success_result(request, "Synthetic Missing Record".into())
            }));
        }
        if id == "fixture-secondary" && request.content_id == "fallback-2" {
            return ProviderResponse::Result(Box::new(
                self.provider
                    .lock()
                    .expect("fixture provider lock")
                    .success_result(request, "Synthetic Secondary Fallback".into()),
            ));
        }
        if id == "fixture-secondary" && request.content_id == "fallback-3" {
            return ProviderResponse::Result(Box::new(ScrapeResult {
                status: MatchStatus::NotFound,
                ..self
                    .provider
                    .lock()
                    .expect("fixture provider lock")
                    .success_result(request, "Synthetic Missing Record".into())
            }));
        }
        if id == "fixture-secondary" && matches!(request.content_id.as_str(), "auth" | "malformed")
        {
            return ProviderResponse::Result(Box::new(
                self.provider
                    .lock()
                    .expect("fixture provider lock")
                    .success_result(request, "Synthetic Auth Fallback".into()),
            ));
        }
        if id == "fixture-tertiary"
            && matches!(
                request.content_id.as_str(),
                "fallback-2" | "fallback-3" | "absent-cover"
            )
        {
            return ProviderResponse::Result(Box::new(
                self.provider
                    .lock()
                    .expect("fixture provider lock")
                    .success_result(request, "Synthetic Tertiary Fallback".into()),
            ));
        }
        let result = <FixtureProvider as MetadataProvider>::scrape(
            &mut self.provider.lock().expect("fixture provider lock"),
            request,
        );
        match result {
            ProviderResponse::Result(mut result)
                if id == "fixture-secondary" && request.content_id == "absent-cover" =>
            {
                result.media.clear();
                ProviderResponse::Result(result)
            }
            ProviderResponse::Result(result) => ProviderResponse::Result(result),
            ProviderResponse::Retry {
                reason,
                retry_after_secs,
            } => ProviderResponse::Retry {
                reason,
                retry_after_secs,
            },
            ProviderResponse::Failed { reason } => ProviderResponse::Failed { reason },
        }
    }
}

struct ProviderGate {
    active: Mutex<usize>,
    wake: Condvar,
    limit: usize,
}

impl ProviderGate {
    fn new(limit: usize) -> Self {
        Self {
            active: Mutex::new(0),
            wake: Condvar::new(),
            limit,
        }
    }
    fn enter(&self) -> ProviderPermit<'_> {
        let mut active = self.active.lock().expect("provider gate lock");
        while *active >= self.limit {
            active = self
                .wake
                .wait(active)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *active += 1;
        ProviderPermit { gate: self }
    }
}

struct ProviderPermit<'a> {
    gate: &'a ProviderGate,
}

impl Drop for ProviderPermit<'_> {
    fn drop(&mut self) {
        let mut active = self.gate.active.lock().expect("provider gate lock");
        *active -= 1;
        self.gate.wake.notify_one();
    }
}

struct ProviderSlot {
    provider: Arc<dyn BulkProvider>,
    auth_lock: Option<Mutex<()>>,
    gate: ProviderGate,
    unavailable: AtomicBool,
}

enum BulkEvent {
    Phase {
        index: usize,
        provider: Option<String>,
        phase: BulkPhase,
        transition: Option<String>,
    },
    Done {
        index: usize,
        result: Box<BulkItemResult>,
    },
}

pub fn scrape_bulk(
    jobs: Vec<BulkJob>,
    settings: &ScraperSettings,
    providers: Vec<Arc<dyn BulkProvider>>,
    observer: Option<&dyn BulkObserver>,
) -> Result<BulkRun> {
    scrape_bulk_with_worker_limit(
        jobs,
        settings,
        providers,
        settings.parallel_jobs,
        None,
        observer,
        None,
    )
}

pub fn scrape_bulk_with_media_publisher(
    jobs: Vec<BulkJob>,
    settings: &ScraperSettings,
    providers: Vec<Arc<dyn BulkProvider>>,
    publisher: Arc<dyn BulkMediaPublisher>,
    observer: Option<&dyn BulkObserver>,
) -> Result<BulkRun> {
    scrape_bulk_with_worker_limit(
        jobs,
        settings,
        providers,
        settings.parallel_jobs,
        Some(publisher),
        observer,
        None,
    )
}

fn scrape_bulk_with_worker_limit(
    jobs: Vec<BulkJob>,
    settings: &ScraperSettings,
    providers: Vec<Arc<dyn BulkProvider>>,
    worker_limit: u8,
    publisher: Option<Arc<dyn BulkMediaPublisher>>,
    observer: Option<&dyn BulkObserver>,
    mut on_completed: Option<BulkCompletionCallback<'_>>,
) -> Result<BulkRun> {
    settings.validate()?;
    if !matches!(worker_limit, 1 | 2 | 4) || worker_limit > settings.parallel_jobs {
        bail!("worker limit is outside configured bounds");
    }
    if jobs.len() > MAX_JOBS {
        bail!("bulk job count is too large");
    }
    let mut content_ids = std::collections::HashSet::new();
    for job in &jobs {
        if !content_ids.insert(job.content_id.as_str()) {
            bail!("duplicate bulk content ID");
        }
        if job.content_id != job.request.content_id {
            bail!("bulk job content ID does not match request");
        }
        job.request.validate()?;
        bounded_string(&job.display_title, "display title", true)?;
        reject_private_text(&job.display_title, "display title")?;
    }
    let configured: HashMap<_, _> = settings
        .providers
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();
    let mut ordered = Vec::new();
    let mut provider_ids = std::collections::HashSet::new();
    for provider in providers {
        let declaration = provider.declaration();
        declaration.validate()?;
        let declaration_id = declaration.id.clone();
        if !provider_ids.insert(declaration_id.clone()) {
            bail!("duplicate bulk provider ID");
        }
        let declaration_requires_credentials = declaration.requires_credentials;
        let declaration_max_concurrency = declaration.max_concurrency;
        let Some(setting) = configured.get(declaration_id.as_str()) else {
            continue;
        };
        if setting.enabled
            && declaration_requires_credentials == setting.requires_credentials
            && (!setting.requires_credentials || setting.credential_configured)
        {
            ordered.push((
                setting.priority,
                provider,
                setting.max_concurrency.min(declaration_max_concurrency),
            ));
        }
    }
    ordered.sort_by_key(|(priority, _, _)| *priority);
    let credential_missing = settings.providers.iter().any(|provider| {
        provider.enabled && provider.requires_credentials && !provider.credential_configured
    });
    let credential_fallback = settings
        .enabled_in_priority()
        .first()
        .zip(ordered.first())
        .is_some_and(|(first_enabled, first_runnable)| {
            credential_missing && first_runnable.0 > first_enabled.priority
        });
    let slots: Vec<_> = ordered
        .into_iter()
        .map(|(_, provider, max_concurrency)| ProviderSlot {
            auth_lock: provider
                .declaration()
                .requires_credentials
                .then(|| Mutex::new(())),
            gate: ProviderGate::new(max_concurrency as usize),
            provider,
            unavailable: AtomicBool::new(false),
        })
        .collect();
    let mut progress = BulkProgress {
        completed: 0,
        total: jobs.len(),
        percent: 0,
        active_slots: settings.parallel_jobs,
        rows: Vec::new(),
        counts: BulkCounts::default(),
        paused: false,
        paused_reason: None,
        background: false,
    };
    if let Some(observer) = observer {
        observer.progress(&progress);
    }
    if jobs.is_empty() {
        progress.percent = 100;
        if let Some(observer) = observer {
            observer.progress(&progress);
        }
        return Ok(BulkRun {
            progress,
            results: Vec::new(),
        });
    }
    let worker_count = worker_limit.min(jobs.len() as u8) as usize;
    let (work_tx, work_rx) = mpsc::sync_channel::<(usize, BulkJob)>(worker_count);
    let (event_tx, event_rx) = mpsc::channel::<BulkEvent>();
    let receiver = Arc::new(Mutex::new(work_rx));
    let slots = Arc::new(slots);
    let mut workers = Vec::new();
    for _ in 0..worker_count {
        let receiver = Arc::clone(&receiver);
        let event_tx = event_tx.clone();
        let slots = Arc::clone(&slots);
        let publisher = publisher.clone();
        workers.push(thread::spawn(move || loop {
            let work = receiver.lock().expect("bulk work lock").recv();
            let Ok((index, job)) = work else { break };
            let result = scrape_one(
                index,
                &job,
                &slots,
                publisher.as_deref(),
                &event_tx,
                credential_fallback,
            );
            let _ = event_tx.send(BulkEvent::Done {
                index,
                result: Box::new(result),
            });
        }));
    }
    for (index, job) in jobs.iter().cloned().enumerate() {
        work_tx
            .send((index, job))
            .map_err(|_| anyhow!("bulk worker stopped"))?;
    }
    drop(work_tx);
    drop(event_tx);
    let mut results: Vec<Option<BulkItemResult>> = vec![None; jobs.len()];
    let mut finalized = 0;
    while finalized < jobs.len() {
        match event_rx
            .recv()
            .map_err(|_| anyhow!("bulk event stream stopped"))?
        {
            BulkEvent::Phase {
                index,
                provider,
                phase,
                transition,
            } => {
                progress
                    .rows
                    .retain(|row| row.content_id != jobs[index].content_id);
                progress.rows.push(BulkRow {
                    content_id: jobs[index].content_id.clone(),
                    display_title: jobs[index].display_title.clone(),
                    provider,
                    phase,
                    fallback_transition: transition,
                });
                progress.rows.sort_by_key(|row| {
                    jobs.iter()
                        .position(|job| job.content_id == row.content_id)
                        .unwrap_or(usize::MAX)
                });
                progress.rows.truncate(worker_count);
                if let Some(observer) = observer {
                    observer.progress(&progress);
                }
            }
            BulkEvent::Done { index, result } => {
                let result = *result;
                progress
                    .rows
                    .retain(|row| row.content_id != jobs[index].content_id);
                update_counts(&mut progress.counts, &result);
                progress.completed += 1;
                progress.percent = ((progress.completed * 100) / progress.total).min(100) as u8;
                if let Some(on_completed) = on_completed.as_deref_mut() {
                    on_completed(&result)?;
                }
                results[index] = Some(result);
                finalized += 1;
                if let Some(observer) = observer {
                    observer.progress(&progress);
                }
            }
        }
    }
    for worker in workers {
        worker.join().map_err(|_| anyhow!("bulk worker panicked"))?;
    }
    progress.rows.clear();
    progress.percent = 100;
    if credential_missing && slots.is_empty() {
        progress.paused_reason = Some("credential-missing".into());
    }
    Ok(BulkRun {
        progress,
        results: results.into_iter().map(Option::unwrap).collect(),
    })
}

fn scrape_one(
    index: usize,
    job: &BulkJob,
    slots: &[ProviderSlot],
    publisher: Option<&dyn BulkMediaPublisher>,
    events: &mpsc::Sender<BulkEvent>,
    credential_fallback: bool,
) -> BulkItemResult {
    let mut fallback = credential_fallback;
    let mut transition = None;
    let mut matched_metadata = None;
    for slot in slots {
        let declaration = slot.provider.declaration();
        if slot.unavailable.load(Ordering::Acquire) {
            fallback = true;
            transition = Some(format!("{}: unavailable → next", declaration.id));
            continue;
        }
        let _ = events.send(BulkEvent::Phase {
            index,
            provider: Some(declaration.id.clone()),
            phase: BulkPhase::Searching,
            transition: transition.clone(),
        });
        let _permit = slot.gate.enter();
        let _auth_lock = slot
            .auth_lock
            .as_ref()
            .map(|lock| lock.lock().expect("provider auth lock"));
        if slot.unavailable.load(Ordering::Acquire) {
            fallback = true;
            transition = Some(format!("{}: unavailable → next", declaration.id));
            continue;
        }
        let mut attempts = 0;
        let response = loop {
            match slot.provider.scrape(&job.request) {
                ProviderResponse::Retry {
                    reason,
                    retry_after_secs,
                } if attempts < 3 => {
                    let retry_after = retry_after_secs.unwrap_or(0);
                    if retry_after > MAX_RETRY_AFTER_SECS {
                        break ProviderResponse::Failed {
                            reason: "retry-after-oversized".into(),
                        };
                    }
                    attempts += 1;
                    let _ = events.send(BulkEvent::Phase {
                        index,
                        provider: Some(declaration.id.clone()),
                        phase: BulkPhase::Retrying,
                        transition: None,
                    });
                    let backoff = 2_u64.saturating_pow(attempts.min(11));
                    let delay = retry_after.max(backoff).min(MAX_BACKOFF_SECS);
                    thread::sleep(Duration::from_secs(delay));
                    let _ = reason;
                }
                response => break response,
            }
        };
        match response {
            ProviderResponse::Result(result) => {
                let result = *result;
                if result.content_id != job.request.content_id
                    || result.system_id != job.request.system_id
                    || result.validate().is_err()
                {
                    fallback = true;
                    transition = Some(format!("{}: policy-invalid → next", declaration.id));
                    let _ = events.send(BulkEvent::Phase {
                        index,
                        provider: Some(declaration.id.clone()),
                        phase: BulkPhase::FallingBack,
                        transition: transition.clone(),
                    });
                    continue;
                }
                if result.status == MatchStatus::Ambiguous {
                    let _ = events.send(BulkEvent::Phase {
                        index,
                        provider: Some(declaration.id.clone()),
                        phase: BulkPhase::Done,
                        transition: transition.clone(),
                    });
                    return BulkItemResult {
                        content_id: job.content_id.clone(),
                        state: QueueState::Ambiguous,
                        result: Some(result),
                        reason: Some("ambiguous".into()),
                        fallback,
                    };
                }
                if result.status == MatchStatus::Matched
                    && result
                        .media
                        .iter()
                        .any(|media| media.kind == MediaKind::BoxArt)
                {
                    let mut result = result;
                    if let Some(metadata) = matched_metadata.take() {
                        result.metadata = metadata;
                    }
                    let _ = events.send(BulkEvent::Phase {
                        index,
                        provider: Some(declaration.id.clone()),
                        phase: BulkPhase::DownloadingCover,
                        transition: transition.clone(),
                    });
                    if let Some(publisher) = publisher {
                        if publisher.publish(&declaration.id, &result).is_err() {
                            let _ = events.send(BulkEvent::Phase {
                                index,
                                provider: Some(declaration.id.clone()),
                                phase: BulkPhase::Done,
                                transition: Some("media-publish-failed".into()),
                            });
                            return BulkItemResult {
                                content_id: job.content_id.clone(),
                                state: QueueState::Failed,
                                result: None,
                                reason: Some("media-publish-failed".into()),
                                fallback,
                            };
                        }
                    }
                    let _ = events.send(BulkEvent::Phase {
                        index,
                        provider: Some(declaration.id.clone()),
                        phase: BulkPhase::Publishing,
                        transition: transition.clone(),
                    });
                    let _ = events.send(BulkEvent::Phase {
                        index,
                        provider: Some(declaration.id.clone()),
                        phase: BulkPhase::Done,
                        transition: transition.clone(),
                    });
                    return BulkItemResult {
                        content_id: job.content_id.clone(),
                        state: QueueState::Succeeded,
                        result: Some(result),
                        reason: fallback.then_some("cover-found-via-fallback".into()),
                        fallback,
                    };
                }
                if result.status == MatchStatus::Matched {
                    matched_metadata = Some(result.metadata);
                    transition = Some(format!("{}: missing-cover-kind → next", declaration.id));
                } else {
                    transition = Some(format!("{}: not-found → next", declaration.id));
                }
                fallback = true;
            }
            ProviderResponse::Retry { reason, .. } => {
                fallback = true;
                transition = Some(format!(
                    "{}: {} → next",
                    declaration.id,
                    safe_reason(&reason)
                ));
            }
            ProviderResponse::Failed { reason } => {
                fallback = true;
                let reason = safe_reason(&reason);
                transition = Some(format!("{}: {} → next", declaration.id, reason));
                if reason.starts_with("auth") {
                    slot.unavailable.store(true, Ordering::Release);
                }
            }
        }
        let _ = events.send(BulkEvent::Phase {
            index,
            provider: Some(declaration.id.clone()),
            phase: BulkPhase::FallingBack,
            transition: transition.clone(),
        });
    }
    let _ = events.send(BulkEvent::Phase {
        index,
        provider: None,
        phase: BulkPhase::Done,
        transition,
    });
    BulkItemResult {
        content_id: job.content_id.clone(),
        state: QueueState::NotFound,
        result: None,
        reason: Some("all-providers-exhausted".into()),
        fallback,
    }
}

fn update_counts(counts: &mut BulkCounts, result: &BulkItemResult) {
    match result.state {
        QueueState::Succeeded if result.fallback => counts.fallback += 1,
        QueueState::Succeeded => counts.succeeded += 1,
        QueueState::NotFound => counts.not_found += 1,
        QueueState::Ambiguous => counts.ambiguous += 1,
        QueueState::Failed => counts.failed += 1,
        QueueState::Cancelled => counts.cancelled += 1,
        _ => {}
    }
}

pub fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
