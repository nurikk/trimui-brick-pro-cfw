use std::{
    env, fs,
    panic::{catch_unwind, set_hook, take_hook, AssertUnwindSafe},
    path::PathBuf,
    process,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Barrier, Mutex,
    },
};

use anyhow::{bail, Context, Result};
use media_cache::{BodyReader, Limits, MediaCache, Response, Transport, ValidatedUrl};
use metadata_scraper::{
    scrape_bulk, BulkJob, BulkObserver, BulkProgress, BulkProvider, DiscoveryRecord,
    FixtureProvider, HttpsMediaUrl, Language, ProviderDeclaration, ProviderResponse, QueryKind,
    Queue, QueueState, SchedulingPolicy, ScrapeRequest, ScrapeResult,
};
use serde::Deserialize;
use serde_json::Value;

const FIXTURES: &[u8] = include_bytes!("../../../../fixtures/scraper/journeys.json");
const BULK_FIXTURES: &[u8] = include_bytes!("../../../../fixtures/scraper/bulk-v1.json");
const JPEG: &str = include_str!("../../../../fixtures/media-cache/jpeg.hex");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    #[serde(rename = "$schema")]
    schema_url: String,
    schema: String,
    synthetic: bool,
    provider: String,
    journeys: Vec<FixtureJourney>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureJourney {
    #[serde(rename = "contentId")]
    content_id: String,
    #[serde(rename = "systemId")]
    system_id: String,
    expected: String,
}

fn main() {
    let result = match env::args().nth(1).as_deref() {
        Some("--fixture-journey") => run(),
        _ => {
            eprintln!("usage: metadata-scraper-fixture --fixture-journey");
            process::exit(2);
        }
    };
    if let Err(error) = result {
        eprintln!("fixture journey failed: {error:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let manifest: FixtureManifest =
        serde_json::from_slice(FIXTURES).context("parse fixture manifest")?;
    if !manifest
        .schema_url
        .ends_with("trimui-brick-scrape-fixtures-v1.schema.json")
        || manifest.schema != "metadata-scraper-fixtures/v1"
        || !manifest.synthetic
        || manifest.provider != "fixture"
        || manifest.journeys.len() != 6
    {
        bail!("fixture manifest identity is invalid");
    }
    let bulk: Value = serde_json::from_slice(BULK_FIXTURES).context("parse bulk fixture")?;
    if bulk["schema"] != "metadata-scraper-bulk/v1"
        || bulk["synthetic"] != true
        || bulk["parallelJobs"] != 2
        || bulk["providers"].as_array().map_or(0, Vec::len) != 3
        || bulk.to_string().contains("credentialRef")
    {
        bail!("bulk fixture manifest identity is invalid");
    }

    let mut provider = FixtureProvider::new();
    for (index, journey) in manifest.journeys.iter().enumerate() {
        let root = fixture_root(&format!("journey-{index}"));
        let mut queue = Queue::open(&root)?;
        let request = if journey.expected == "manual-search" {
            ScrapeRequest::new(&journey.content_id, &journey.system_id)
                .with_manual_query("synthetic manual")
        } else {
            ScrapeRequest::new(&journey.content_id, &journey.system_id)
        };
        queue.enqueue(request, false, false)?;
        queue.dispatch_at(&mut provider, SchedulingPolicy::default(), 0)?;
        if journey.expected.contains("then") {
            let retry_at = queue
                .get(&journey.content_id)
                .context("missing retry journey")?
                .next_attempt_at;
            queue.dispatch_at(&mut provider, SchedulingPolicy::default(), retry_at)?;
            require_state(&queue, &journey.content_id, QueueState::Succeeded)?;
        } else if journey.expected == "manual-search" {
            require_state(&queue, &journey.content_id, QueueState::Succeeded)?;
            if provider.last_query_kind() != Some(QueryKind::Manual) {
                bail!("manual query was not selected");
            }
        } else {
            require_state(&queue, &journey.content_id, state_for(&journey.expected)?)?;
        }
        remove_fixture_root(&root)?;
    }

    query_order_and_result_validation(&mut provider)?;
    queue_controls_and_recovery(&mut provider)?;
    reject_unsafe_inputs(&mut provider)?;
    bulk_provider_and_scheduler()?;
    bulk_recovery_after_coordinator_failure()?;
    println!("fixture journey passed: 6 journeys, bounded bulk fallback 1/2/4, queue recovery/retry/rate-limit/cancel, policy, privacy, and URL rejection");
    Ok(())
}

fn query_order_and_result_validation(provider: &mut FixtureProvider) -> Result<()> {
    let request = ScrapeRequest::new("hash-success", "nes")
        .with_hash("synthetic-hash")
        .with_hash_lookup(true)
        .with_filename("Synthetic Quest.rom")
        .with_title("Synthetic Quest")
        .with_manual_query("synthetic manual")
        .with_priorities(
            vec![metadata_scraper::Region::Europe],
            vec![Language::French],
        );
    let result = provider.scrape(&request)?;
    if provider.query_history()
        != [
            QueryKind::Hash,
            QueryKind::NormalizedFilenameOrTitle,
            QueryKind::Manual,
        ]
        || result.metadata.region != Some(metadata_scraper::Region::Europe)
        || result.media[0].language != Language::French
    {
        bail!("query order or priority selection is invalid");
    }
    let json = result.to_json()?;
    ScrapeResult::from_json(json.as_bytes())?;
    Ok(())
}

fn queue_controls_and_recovery(provider: &mut FixtureProvider) -> Result<()> {
    let root = fixture_root("controls");
    let mut queue = Queue::open(&root)?;
    let policy = SchedulingPolicy::default();
    queue.enqueue_discovered(DiscoveryRecord::new("success", "nes"), policy, true, false)?;
    queue.enqueue(
        ScrapeRequest::new("manual", "nes").with_manual_query("synthetic manual"),
        false,
        false,
    )?;
    queue.enqueue(ScrapeRequest::new("low-confidence", "nes"), false, false)?;
    let mut systems = Queue::open(root.join("systems"))?;
    if systems.enqueue_systems(
        [
            DiscoveryRecord::new("selected-nes", "nes"),
            DiscoveryRecord::new("selected-snes", "snes"),
        ],
        &["snes".into()],
        policy,
        false,
        false,
    )? != 1
        || systems.get_for("selected-snes", "snes").is_none()
        || systems.enqueue_systems(
            [
                DiscoveryRecord::new("all-nes", "nes"),
                DiscoveryRecord::new("all-snes", "snes"),
            ],
            &[],
            policy,
            false,
            false,
        )? != 2
    {
        bail!("system selection did not queue the expected records");
    }
    let checkpoint = fs::read_to_string(root.join("scraper-queue.json"))?;
    if ["synthetic manual", "romHash", "filename", "manualQuery"]
        .iter()
        .any(|value| checkpoint.contains(value))
    {
        bail!("queue checkpoint exposed provider input");
    }
    queue.pause()?;
    if !matches!(
        queue.dispatch_at(provider, policy, 0)?,
        metadata_scraper::DispatchOutcome::None
    ) {
        bail!("paused queue dispatched");
    }
    queue.resume()?;
    queue.dispatch_at(provider, policy, 0)?;
    queue.dispatch_at(provider, policy, 0)?;
    queue.dispatch_at(provider, policy, 0)?;
    require_state(&queue, "low-confidence", QueueState::Ambiguous)?;
    if queue
        .get("low-confidence")
        .and_then(|job| job.reason.as_deref())
        != Some("confidence-review")
    {
        bail!("low-confidence match was applied without review");
    }
    queue.enqueue(ScrapeRequest::new("cancelled", "nes"), false, false)?;
    queue.cancel("cancelled")?;
    if queue
        .get("cancelled")
        .context("missing cancelled job")?
        .state
        != QueueState::Cancelled
    {
        bail!("cancel did not persist");
    }
    if queue
        .automatic_enqueue(
            ScrapeRequest::new("blocked", "nes"),
            SchedulingPolicy {
                wifi_available: false,
                ..policy
            },
            false,
            false,
        )
        .is_ok()
    {
        bail!("policy gate accepted unavailable Wi-Fi");
    }
    remove_fixture_root(&root)?;

    let root = fixture_root("recovery");
    let mut queue = Queue::open(&root)?;
    queue.enqueue(ScrapeRequest::new("success", "nes"), false, false)?;
    queue.mark_running("success")?;
    drop(queue);
    let queue = Queue::open(&root)?;
    require_state(&queue, "success", QueueState::Retry)?;
    remove_fixture_root(&root)?;
    Ok(())
}

fn reject_unsafe_inputs(provider: &mut FixtureProvider) -> Result<()> {
    for url in [
        "http://example.invalid/a",
        "https://user@example.invalid/a",
        "https://127.0.0.1/a",
        "https://10.0.0.1/a",
        "https://169.254.1.1/a",
        "https://[::1]/a",
        "https://[fc00::1]/a",
        "https://localhost/a",
        "https:///missing-host",
    ] {
        if HttpsMediaUrl::parse(url).is_ok() {
            bail!("unsafe URL was accepted: {url}");
        }
    }
    if HttpsMediaUrl::parse(format!("https://example.invalid/{}", "x".repeat(2049))).is_ok() {
        bail!("oversized URL was accepted");
    }
    if provider
        .scrape(&ScrapeRequest::new("success", "nes").with_title("x".repeat(4097)))
        .is_ok()
    {
        bail!("oversized provider input was accepted");
    }
    let mut result = provider.scrape(&ScrapeRequest::new("success", "nes"))?;
    result.metadata.description = "private /roms source bytes".to_string();
    if result.validate().is_ok() {
        bail!("private provider data was accepted");
    }
    let mut value = serde_json::to_value(result)?;
    value["privateField"] = serde_json::Value::String("private".to_string());
    if ScrapeResult::from_json(&serde_json::to_vec(&value)?).is_ok() {
        bail!("unknown result field was accepted");
    }
    Ok(())
}

struct ProgressCollector(Mutex<Vec<BulkProgress>>);

impl BulkObserver for ProgressCollector {
    fn progress(&self, progress: &BulkProgress) {
        self.0.lock().expect("progress lock").push(progress.clone());
    }
}

struct BarrierProvider {
    declaration: ProviderDeclaration,
    barrier: Barrier,
    active: AtomicUsize,
    maximum: AtomicUsize,
    provider: Mutex<FixtureProvider>,
}

impl BarrierProvider {
    fn new(slots: usize) -> Self {
        Self {
            declaration: ProviderDeclaration {
                id: "fixture-secondary".into(),
                enabled: true,
                requires_credentials: false,
                credential_configured: false,
                priority: 1,
                max_concurrency: 4,
            },
            barrier: Barrier::new(slots),
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            provider: Mutex::new(FixtureProvider::new()),
        }
    }

    fn maximum(&self) -> usize {
        self.maximum.load(Ordering::Acquire)
    }
}

impl BulkProvider for BarrierProvider {
    fn declaration(&self) -> &ProviderDeclaration {
        &self.declaration
    }

    fn scrape(&self, request: &ScrapeRequest) -> ProviderResponse {
        let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
        self.maximum.fetch_max(active, Ordering::AcqRel);
        self.barrier.wait();
        self.active.fetch_sub(1, Ordering::AcqRel);
        let mut provider = self.provider.lock().expect("fixture provider lock");
        let result = if request.content_id.starts_with("parallel-") {
            let mut result = provider.scrape(&ScrapeRequest::new("success", "nes"));
            if let Ok(result) = result.as_mut() {
                result.content_id = request.content_id.clone();
                result.system_id = request.system_id.clone();
            }
            result
        } else {
            provider.scrape(request)
        };
        match result {
            Ok(result) => ProviderResponse::Result(Box::new(result)),
            Err(error) => ProviderResponse::Failed {
                reason: error.to_string(),
            },
        }
    }
}

struct FixtureBody {
    bytes: Vec<u8>,
    offset: usize,
}

impl BodyReader for FixtureBody {
    fn read(&mut self, buffer: &mut [u8], _deadline: std::time::Instant) -> Result<usize> {
        let count = (self.bytes.len() - self.offset).min(buffer.len());
        buffer[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

struct FixtureTransport {
    bytes: Vec<u8>,
}

impl Transport for FixtureTransport {
    fn fetch(&self, _url: &ValidatedUrl, _deadline: std::time::Instant) -> Result<Response> {
        Ok(Response {
            status: 200,
            content_type: "image/jpeg".into(),
            redirect: None,
            body: Box::new(FixtureBody {
                bytes: self.bytes.clone(),
                offset: 0,
            }),
        })
    }
}

fn jpeg_bytes() -> Result<Vec<u8>> {
    (0..JPEG.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&JPEG[index..index + 2], 16).context("decode JPEG fixture"))
        .collect()
}

fn bulk_recovery_after_coordinator_failure() -> Result<()> {
    let root = fixture_root("bulk-coordinator-recovery");
    let (release_tx, release_rx) = mpsc::channel();
    let provider = Arc::new(InterruptProvider::new(release_rx));
    let settings = metadata_scraper::ScraperSettings {
        parallel_jobs: 1,
        ..Default::default()
    };
    let mut queue = Queue::open(&root)?;
    queue.enqueue_bulk(
        [
            ScrapeRequest::new("success", "nes"),
            ScrapeRequest::new("unfinished", "nes"),
        ],
        false,
        false,
    )?;
    let panic_hook = take_hook();
    set_hook(Box::new(|_| {}));
    let interrupted = catch_unwind(AssertUnwindSafe(|| {
        queue.dispatch_bulk(
            &settings,
            vec![provider.clone() as Arc<dyn BulkProvider>],
            Some(&InterruptObserver),
        )
    }));
    set_hook(panic_hook);
    if interrupted.is_ok() {
        bail!("coordinator interruption did not interrupt the batch");
    }
    release_tx
        .send(())
        .map_err(|_| anyhow::anyhow!("unfinished worker did not wait for release"))?;
    drop(queue);

    let mut reopened = Queue::open(&root)?;
    require_state(&reopened, "success", QueueState::Succeeded)?;
    let pending = reopened.pending_requests();
    if pending.len() != 1 || pending[0].content_id != "unfinished" {
        bail!("coordinator recovery requeued completed work");
    }
    let resumed = reopened.dispatch_bulk(&settings, fixture_bulk_providers()?, None)?;
    if resumed.results.len() != 1 || resumed.results[0].content_id != "unfinished" {
        bail!("coordinator recovery restarted completed work");
    }
    require_state(&reopened, "success", QueueState::Succeeded)?;
    require_state(&reopened, "unfinished", QueueState::NotFound)?;
    remove_fixture_root(&root)?;
    Ok(())
}

struct InterruptObserver;

impl BulkObserver for InterruptObserver {
    fn progress(&self, progress: &BulkProgress) {
        if progress.completed == 1 {
            panic!("synthetic coordinator interruption");
        }
    }
}

struct InterruptProvider {
    declaration: ProviderDeclaration,
    release: Mutex<mpsc::Receiver<()>>,
    provider: Mutex<FixtureProvider>,
}

impl InterruptProvider {
    fn new(release: mpsc::Receiver<()>) -> Self {
        Self {
            declaration: ProviderDeclaration {
                id: "fixture-secondary".into(),
                enabled: true,
                requires_credentials: false,
                credential_configured: false,
                priority: 2,
                max_concurrency: 1,
            },
            release: Mutex::new(release),
            provider: Mutex::new(FixtureProvider::new()),
        }
    }
}

impl BulkProvider for InterruptProvider {
    fn declaration(&self) -> &ProviderDeclaration {
        &self.declaration
    }

    fn scrape(&self, request: &ScrapeRequest) -> ProviderResponse {
        if request.content_id == "unfinished" {
            self.release
                .lock()
                .expect("release lock")
                .recv()
                .expect("release signal");
        }
        match self
            .provider
            .lock()
            .expect("interrupt provider lock")
            .scrape(request)
        {
            Ok(result) => ProviderResponse::Result(Box::new(result)),
            Err(error) => ProviderResponse::Failed {
                reason: error.to_string(),
            },
        }
    }
}

fn fixture_bulk_providers() -> Result<Vec<Arc<dyn BulkProvider>>> {
    metadata_scraper::registered_providers()
        .into_iter()
        .map(|declaration| {
            metadata_scraper::FixtureBulkProvider::new(declaration)
                .map(|provider| Arc::new(provider) as Arc<dyn BulkProvider>)
        })
        .collect()
}

fn bulk_provider_and_scheduler() -> Result<()> {
    for slots in [1usize, 2, 4] {
        let provider = Arc::new(BarrierProvider::new(slots));
        let jobs = (0..(slots * 2))
            .map(|index| {
                let id = format!("parallel-{index}");
                BulkJob::new(
                    &id,
                    format!("Synthetic Game {index}"),
                    ScrapeRequest::new(&id, "nes"),
                )
            })
            .collect();
        let mut settings = metadata_scraper::ScraperSettings {
            parallel_jobs: slots as u8,
            ..Default::default()
        };
        settings.providers[0].enabled = false;
        settings.providers[1].max_concurrency = 4;
        let observer = ProgressCollector(Mutex::new(Vec::new()));
        let run = scrape_bulk(jobs, &settings, vec![provider.clone()], Some(&observer))?;
        if provider.maximum() != slots
            || run.progress.percent != 100
            || run.results.len() != slots * 2
        {
            bail!("barrier scheduler did not honor exactly {slots} slots");
        }
        let snapshots = observer
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("progress lock poisoned"))?;
        if snapshots
            .windows(2)
            .any(|pair| pair[1].percent < pair[0].percent)
            || (slots > 1
                && !snapshots
                    .iter()
                    .any(|progress| progress.rows.len() == slots))
        {
            bail!("bulk progress was not monotonic and overlapping");
        }
        if run
            .results
            .iter()
            .any(|result| result.state != QueueState::Succeeded)
        {
            bail!("parallel fixture did not finalize every job");
        }
    }

    let provider = Arc::new(BarrierProvider::new(1));
    let jobs = (0..4)
        .map(|index| {
            let id = format!("provider-cap-{index}");
            BulkJob::new(
                &id,
                format!("Provider Cap Game {index}"),
                ScrapeRequest::new(&id, "nes"),
            )
        })
        .collect();
    let mut cap_settings = metadata_scraper::ScraperSettings {
        parallel_jobs: 4,
        ..Default::default()
    };
    cap_settings.providers[0].enabled = false;
    cap_settings.providers[1].max_concurrency = 1;
    let cap_run = scrape_bulk(jobs, &cap_settings, vec![provider.clone()], None)?;
    if provider.maximum() != 1 || cap_run.progress.percent != 100 {
        bail!("provider-local concurrency cap was exceeded");
    }

    let providers = fixture_bulk_providers()?;
    let mut settings = metadata_scraper::ScraperSettings::default();
    settings.providers[0].credential_configured = true;
    settings.parallel_jobs = 2;
    let jobs = [
        ("fallback-2", "Fallback Two"),
        ("fallback-3", "Fallback Three"),
        ("absent-cover", "Absent Cover"),
        ("not-found", "Missing Game"),
        ("retry", "Retry Game"),
        ("rate-limit", "Rate Limited Game"),
        ("ambiguous", "Ambiguous Game"),
        ("malformed", "Malformed Game"),
    ]
    .into_iter()
    .map(|(id, title)| BulkJob::new(id, title, ScrapeRequest::new(id, "nes")))
    .collect();
    let run = scrape_bulk(jobs, &settings, providers, None)?;
    let by_id = |id: &str| {
        run.results
            .iter()
            .find(|result| result.content_id == id)
            .ok_or_else(|| anyhow::anyhow!("missing bulk result: {id}"))
    };
    if by_id("fallback-2")?.state != QueueState::Succeeded
        || !by_id("fallback-2")?.fallback
        || by_id("fallback-3")?.state != QueueState::Succeeded
        || !by_id("fallback-3")?.fallback
        || by_id("not-found")?.state != QueueState::NotFound
        || by_id("retry")?.state != QueueState::Succeeded
        || by_id("rate-limit")?.state != QueueState::Succeeded
        || by_id("ambiguous")?.state != QueueState::Ambiguous
        || by_id("malformed")?.state != QueueState::Succeeded
    {
        bail!("provider priority or fallback result is invalid");
    }
    let auth_providers = fixture_bulk_providers()?;
    let auth = scrape_bulk(
        vec![BulkJob::new(
            "auth",
            "Auth Fallback",
            ScrapeRequest::new("auth", "nes"),
        )],
        &settings,
        auth_providers,
        None,
    )?;
    if auth.results[0].state != QueueState::Succeeded {
        bail!("auth provider was not skipped for fallback");
    }

    let missing = metadata_scraper::ScraperSettings::default();
    let missing = scrape_bulk(
        vec![BulkJob::new(
            "fallback-2",
            "Skip Game",
            ScrapeRequest::new("fallback-2", "nes"),
        )],
        &missing,
        fixture_bulk_providers()?,
        None,
    )?;
    if !missing.results[0].fallback || missing.results[0].state != QueueState::Succeeded {
        bail!("missing credential did not preserve anonymous fallback");
    }

    let mut reordered = settings.clone();
    reordered.providers[0].priority = 2;
    reordered.providers[1].priority = 3;
    reordered.providers[2].priority = 1;
    let reordered = scrape_bulk(
        vec![BulkJob::new(
            "fallback-3",
            "Reordered Providers",
            ScrapeRequest::new("fallback-3", "nes"),
        )],
        &reordered,
        fixture_bulk_providers()?,
        None,
    )?;
    if reordered.results[0].fallback
        || reordered.results[0]
            .result
            .as_ref()
            .map(|result| result.metadata.canonical_title.as_str())
            != Some("Synthetic Tertiary Fallback")
    {
        bail!("provider reorder was not respected");
    }

    let mut disabled = settings.clone();
    disabled.providers[1].enabled = false;
    let provider = metadata_scraper::FixtureBulkProvider::new(disabled.providers[0].clone())?;
    let provider3 = metadata_scraper::FixtureBulkProvider::new(disabled.providers[2].clone())?;
    let result = scrape_bulk(
        vec![BulkJob::new(
            "fallback-2",
            "Fallback Two",
            ScrapeRequest::new("fallback-2", "nes"),
        )],
        &disabled,
        vec![Arc::new(provider), Arc::new(provider3)],
        None,
    )?;
    if result.results[0].state != QueueState::Succeeded {
        bail!("disabled provider was still required");
    }

    let root = fixture_root("bulk-restart");
    let mut queue = Queue::open(&root)?;
    queue.enqueue_bulk(
        [
            ScrapeRequest::new("fallback-2", "nes"),
            ScrapeRequest::new("not-found", "nes"),
        ],
        false,
        false,
    )?;
    let requests = queue.pending_requests();
    let restart_providers = fixture_bulk_providers()?;
    if requests.len() != 2 {
        bail!("restart fixture did not retain pending jobs");
    }
    let restart = queue.dispatch_bulk(&settings, restart_providers, None)?;
    if restart
        .results
        .iter()
        .map(|result| result.content_id.as_str())
        .collect::<Vec<_>>()
        != ["fallback-2", "not-found"]
        || restart.progress.percent != 100
    {
        bail!("bulk restart dispatch did not preserve deterministic output ordering");
    }
    queue.finalize_bulk(&restart)?;
    drop(queue);
    let reopened = Queue::open(&root)?;
    let summary = reopened.progress();
    if !reopened.pending_requests().is_empty() || summary.succeeded != 1 || summary.not_found != 1 {
        bail!("finalized bulk jobs were not durable exactly once across restart");
    }
    remove_fixture_root(&root)?;

    let root = fixture_root("bulk-policy-gate");
    let mut queue = Queue::open(&root)?;
    queue.enqueue(ScrapeRequest::new("policy-gated", "nes"), false, false)?;
    let gated_settings = metadata_scraper::ScraperSettings {
        parallel_jobs: 1,
        ..settings.clone()
    };
    let rejected = queue.dispatch_bulk_with_policy(
        &gated_settings,
        fixture_bulk_providers()?,
        SchedulingPolicy {
            active_jobs: 1,
            max_concurrency: 2,
            ..SchedulingPolicy::default()
        },
        None,
    );
    if rejected.is_ok()
        || queue
            .get("policy-gated")
            .is_none_or(|job| job.state != QueueState::Pending)
    {
        bail!("global policy gate did not preserve pending work");
    }
    remove_fixture_root(&root)?;

    let root = fixture_root("bulk-media-cache");
    let cache_root = root.join("cache");
    let temporary_root = root.join("tmp");
    fs::create_dir_all(&temporary_root)?;
    let cache = Arc::new(MediaCache::open_with_temp_root(
        &cache_root,
        &temporary_root,
        Limits::default(),
    )?);
    let transport = Arc::new(FixtureTransport {
        bytes: jpeg_bytes()?,
    });
    let publisher = Arc::new(metadata_scraper::MediaCachePublisher::new(
        cache.clone(),
        transport,
    ));
    let settings = metadata_scraper::ScraperSettings::default();
    let mut queue = Queue::open(root.join("queue"))?;
    queue.enqueue(ScrapeRequest::new("fallback-2", "nes"), false, false)?;
    let published = queue.dispatch_bulk_with_media_publisher(
        &settings,
        fixture_bulk_providers()?,
        publisher.clone(),
        None,
    )?;
    if published.results[0].state != QueueState::Succeeded {
        bail!("durable queue media cache publication failed");
    }
    let cleanup = cache.cleanup_orphans(&[], false)?;
    if cleanup.candidates != 1 || cleanup.deleted != 0 {
        bail!("unconfirmed orphan cleanup removed cache data");
    }
    cache.protect_manual_artwork("fallback-2")?;
    if !cache.manual_artwork_is_protected("fallback-2")
        || cache.cleanup_orphans(&[], true)?.deleted != 0
        || !cache_root.join("index/fallback-2.json").is_file()
    {
        bail!("manual artwork was not protected from confirmed cleanup");
    }
    let published = metadata_scraper::scrape_bulk_with_media_publisher(
        vec![BulkJob::new(
            "fallback-2",
            "Cache Game",
            ScrapeRequest::new("fallback-2", "nes"),
        )],
        &settings,
        fixture_bulk_providers()?,
        publisher,
        None,
    )?;
    if published.results[0].state != QueueState::Succeeded {
        bail!("media cache publication failed");
    }
    let object_count = fs::read_dir(cache_root.join("objects"))?
        .filter_map(Result::ok)
        .map(|shard| fs::read_dir(shard.path()).map(|entries| entries.count()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .sum::<usize>();
    if object_count != 1 || !cache_root.join("index/fallback-2.json").is_file() {
        bail!("media cache publication did not atomically deduplicate");
    }
    remove_fixture_root(&root)?;
    Ok(())
}

fn state_for(expected: &str) -> Result<QueueState> {
    match expected {
        "succeeded" => Ok(QueueState::Succeeded),
        "not-found" => Ok(QueueState::NotFound),
        "ambiguous" => Ok(QueueState::Ambiguous),
        _ => bail!("unknown fixture expectation: {expected}"),
    }
}

fn require_state(queue: &Queue, content_id: &str, expected: QueueState) -> Result<()> {
    let actual = queue
        .get(content_id)
        .context("missing queue job")?
        .state
        .clone();
    if actual != expected {
        bail!(
            "{content_id} state was {}, expected {}",
            actual.as_str(),
            expected.as_str()
        );
    }
    Ok(())
}

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from("/var/tmp").join(format!("metadata-scraper-fixture-{name}-{}", process::id()))
}

fn remove_fixture_root(root: &PathBuf) -> Result<()> {
    fs::remove_dir_all(root).context("remove synthetic fixture root")?;
    Ok(())
}
