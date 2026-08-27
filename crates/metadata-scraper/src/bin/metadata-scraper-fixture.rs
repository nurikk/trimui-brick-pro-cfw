use std::{env, fs, path::PathBuf, process};

use anyhow::{bail, Context, Result};
use metadata_scraper::{
    DiscoveryRecord, FixtureProvider, HttpsMediaUrl, Language, QueryKind, Queue, QueueState,
    SchedulingPolicy, ScrapeRequest, ScrapeResult,
};
use serde::Deserialize;

const FIXTURES: &[u8] = include_bytes!("../../../../fixtures/scraper/journeys.json");

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
    println!("fixture journey passed: 6 journeys, queue recovery/retry/rate-limit/cancel, policy, privacy, and URL rejection");
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
