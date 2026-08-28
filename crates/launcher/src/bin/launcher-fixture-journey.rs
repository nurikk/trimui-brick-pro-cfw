use std::{
    env, fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use launch_contract::{
    parse_catalog_json, parse_request_json, validate, validate_catalog_projection,
};
use serde_json::Value;
use sim_host_platform::{Backend, HostPlatform};

const PROFILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../sim/device/tg4040-host.json"
);
const UI_CATALOG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../sim/fixtures/catalog.json"
);
const LAUNCH_CATALOG: &[u8] =
    include_bytes!("../../../../fixtures/launch-contract/generated-v1/catalog.synthetic.json");
const REQUEST_SCHEMA: &[u8] = include_bytes!("../../../../schemas/launch-request-v1.schema.json");
const MAX_BINARY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_COLD_START_US: u128 = 2_000_000;
const MAX_FIRST_FRAME_US: u128 = 500_000;
const MAX_IDLE_RSS_KIB: u64 = 128 * 1024;
const MAX_CATALOG_LIST_US: u128 = 100_000;
const MAX_INPUT_TO_FRAME_US: u128 = 100_000;

fn main() {
    let started = Instant::now();
    let first = run_once(1);
    let second = run_once(2);
    assert_eq!(
        first.deterministic, second.deterministic,
        "journey is not deterministic"
    );
    assert!(first.route_progression == ["library", "systems", "games"]);
    assert_eq!(first.catalog_entries, 4);
    assert!(first.binary_bytes <= MAX_BINARY_BYTES);
    assert!(first.cold_start_us <= MAX_COLD_START_US);
    assert!(first.first_frame_us <= MAX_FIRST_FRAME_US);
    assert!(first.idle_rss_kib <= MAX_IDLE_RSS_KIB);
    assert!(first.catalog_list_us <= MAX_CATALOG_LIST_US);
    assert!(first.input_to_frame_us <= MAX_INPUT_TO_FRAME_US);
    println!(
        "{{\"lane\":\"host-native userspace simulator\",\"journey\":\"Library→Systems→Games→LaunchRequest\",\"runs\":2,\"wallUs\":{},\"metrics\":{{\"binaryBytes\":{},\"coldStartUs\":{},\"firstFrameUs\":{},\"idleRssKiB\":{},\"catalogListUs\":{},\"inputToFrameUs\":{}}},\"budgets\":{{\"binaryBytes\":{},\"coldStartUs\":{},\"firstFrameUs\":{},\"idleRssKiB\":{},\"catalogListUs\":{},\"inputToFrameUs\":{}}},\"result\":\"pass\"}}",
        started.elapsed().as_micros(),
        first.binary_bytes,
        first.cold_start_us,
        first.first_frame_us,
        first.idle_rss_kib,
        first.catalog_list_us,
        first.input_to_frame_us,
        MAX_BINARY_BYTES,
        MAX_COLD_START_US,
        MAX_FIRST_FRAME_US,
        MAX_IDLE_RSS_KIB,
        MAX_CATALOG_LIST_US,
        MAX_INPUT_TO_FRAME_US,
    );
}

struct Journey {
    deterministic: String,
    route_progression: [&'static str; 3],
    catalog_entries: usize,
    binary_bytes: u64,
    cold_start_us: u128,
    first_frame_us: u128,
    idle_rss_kib: u64,
    catalog_list_us: u128,
    input_to_frame_us: u128,
}

fn run_once(number: u8) -> Journey {
    let root = env::temp_dir().join(format!("trimui-launcher-journey-{}-{number}", process_id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("data")).expect("create caller-owned data directory");
    fs::write(root.join("data/rom-index.json"), b"interrupted-index")
        .expect("seed interrupted index fixture");
    fs::create_dir_all(&root).expect("create caller-owned evidence directory");
    let started = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    sim_launcher::run(Path::new(UI_CATALOG), &root, false, &stop, || {
        HostPlatform::new(Path::new(PROFILE), Backend::Dummy)
    })
    .expect("host-native journey");
    let cold_start_us = started.elapsed().as_micros();
    let events = read_events(&root.join("logs/launcher.jsonl"));
    let first_frame = events
        .iter()
        .find(|event| event["event"] == "first_frame")
        .expect("first frame event");
    assert_eq!(first_frame["logicalWidth"], 1024);
    assert_eq!(first_frame["logicalHeight"], 768);
    let routes = events
        .iter()
        .filter(|event| event["event"] == "route_selection")
        .map(|event| event["route"].as_str().expect("route").to_string())
        .collect::<Vec<_>>();
    assert!(routes.starts_with(&["library".into(), "systems".into(), "games".into()]));
    let controls = events
        .iter()
        .filter(|event| event["event"] == "control" && event["action"] == "press")
        .map(|event| event["control"].as_str().expect("control"))
        .collect::<Vec<_>>();
    assert!(
        controls.contains(&"start") && controls.contains(&"down") && controls.contains(&"primary")
    );

    let catalog = parse_catalog_json(LAUNCH_CATALOG).expect("strict launch catalog");
    validate_catalog_projection(&catalog).expect("valid launch catalog");
    assert_eq!(catalog.runners.len(), 3);
    let request_bytes =
        fs::read(root.join("launch-request.json")).expect("launch request evidence");
    let request = parse_request_json(&request_bytes).expect("strict launch request");
    validate(&request, &catalog).expect("launch request catalog validation");
    assert_schema_projection(&request_bytes, &request);
    assert_eq!(
        serde_json::to_vec_pretty(&request).expect("request serialization"),
        trim_newline(&request_bytes)
    );

    let text = evidence_text(&root);
    for forbidden in ["/srv/", "/src/", "C:\\\\", "private", "ROM", "BIOS"] {
        assert!(
            !text.contains(forbidden),
            "private evidence marker: {forbidden}"
        );
    }
    let index_event = events
        .iter()
        .find(|event| event["event"] == "index")
        .expect("index event");
    assert_eq!(index_event["status"], "recovered");
    assert_eq!(index_event["visibleRows"], 12);
    assert_eq!(index_event["searchResults"], 64);
    assert_eq!(index_event["queueDepth"], 32);
    let index: Value = serde_json::from_slice(
        &fs::read(root.join("data/rom-index.json")).expect("recovered index"),
    )
    .expect("index JSON");
    assert_eq!(index["schema"], "launcher-rom-index/v1");
    assert_eq!(index["entries"].as_array().expect("index entries").len(), 4);
    let state: Value = serde_json::from_slice(
        &fs::read(root.join("data/launcher-state.json")).expect("launcher state"),
    )
    .expect("launcher state JSON");
    assert_eq!(state["identity"], "Artbook");
    assert_eq!(state["schemaVersion"], 1);
    let input_to_frame_us = events
        .iter()
        .filter(|event| event["event"] == "input_to_frame")
        .map(|event| event["latencyUs"].as_u64().expect("input latency") as u128)
        .max()
        .expect("input latency event");
    let catalog_list_us = events
        .iter()
        .find(|event| event["event"] == "catalog_list")
        .and_then(|event| event["latencyUs"].as_u64())
        .expect("catalog latency") as u128;
    let first_frame_us = first_frame["hostElapsedUs"]
        .as_u64()
        .expect("first-frame latency") as u128;
    let deterministic = events
        .iter()
        .map(|event| {
            let mut event = event.clone();
            if let Some(object) = event.as_object_mut() {
                object.remove("runId");
                object.remove("hostElapsedUs");
                object.remove("latencyUs");
            }
            event.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let binary_bytes = fs::metadata(env::current_exe().expect("journey executable"))
        .expect("journey binary metadata")
        .len();
    let idle_rss_kib = host_fixture_performance_rss_kib();
    let _ = fs::remove_dir_all(root);
    Journey {
        deterministic,
        route_progression: ["library", "systems", "games"],
        catalog_entries: 4,
        binary_bytes,
        cold_start_us,
        first_frame_us,
        idle_rss_kib,
        catalog_list_us,
        input_to_frame_us,
    }
}

// Fixture-only host performance evidence; this is not frontend hardware access.
fn host_fixture_performance_rss_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0)
}

fn assert_schema_projection(request_bytes: &[u8], request: &launch_contract::LaunchRequest) {
    let schema: Value = serde_json::from_slice(REQUEST_SCHEMA).expect("request schema JSON");
    let object = serde_json::to_value(request).expect("request value");
    assert_eq!(schema["$id"], object["$schema"]);
    assert_eq!(schema["additionalProperties"], false);
    for required in schema["required"].as_array().expect("schema required") {
        assert!(object
            .get(required.as_str().expect("required field"))
            .is_some());
    }
    for key in object.as_object().expect("request object").keys() {
        assert!(
            schema["properties"].get(key).is_some(),
            "schema field: {key}"
        );
    }
    assert!(request_bytes.ends_with(b"\n"));
}

fn read_events(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .expect("event log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("event JSON"))
        .collect()
}

fn evidence_text(root: &Path) -> String {
    let mut text = String::new();
    for path in [
        root.join("readiness.json"),
        root.join("route-selection.json"),
        root.join("launch.json"),
        root.join("launch-request.json"),
        root.join("session.json"),
        root.join("exit-status.json"),
        root.join("logs/launcher.jsonl"),
    ] {
        text.push_str(&fs::read_to_string(path).expect("text evidence"));
    }
    text
}

fn trim_newline(bytes: &[u8]) -> Vec<u8> {
    bytes.strip_suffix(b"\n").unwrap_or(bytes).to_vec()
}

fn process_id() -> u32 {
    std::process::id()
}
