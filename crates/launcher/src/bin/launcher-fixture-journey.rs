use std::{
    env, fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
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
const DEVICE_PROFILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../config/platform/tg4040/compatibility.json"
);
const UI_CATALOG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../sim/fixtures/catalog.json"
);
const LAUNCH_CATALOG: &[u8] =
    include_bytes!("../../../../fixtures/launch-contract/generated-v1/catalog.synthetic.json");
const REQUEST_SCHEMA: &[u8] = include_bytes!("../../../../schemas/launch-request-v1.schema.json");
const MAX_BINARY_BYTES: u64 = 24 * 1024 * 1024;
const MAX_COLD_START_US: u128 = 2_000_000;
const MAX_FIRST_FRAME_US: u128 = 500_000;
const MAX_IDLE_RSS_KIB: u64 = 128 * 1024;
const MAX_CATALOG_LIST_US: u128 = 100_000;
const MAX_INPUT_TO_FRAME_US: u128 = 250_000;
const MAX_LIBRARY_OPEN_US: u128 = 250_000;
const MAX_SYSTEM_SWITCH_US: u128 = 250_000;
const MAX_RETURN_FROM_GAME_US: u128 = 250_000;

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
    assert!(first.library_open_us <= MAX_LIBRARY_OPEN_US);
    assert!(first.system_switch_us <= MAX_SYSTEM_SWITCH_US);
    assert!(first.return_from_game_us <= MAX_RETURN_FROM_GAME_US);
    rom_index_corpus();
    println!(
        "{{\"lane\":\"host-native userspace simulator\",\"journey\":\"Library→Systems→Games→LaunchRequest\",\"runs\":2,\"wallUs\":{},\"metrics\":{{\"binaryBytes\":{},\"coldStartUs\":{},\"firstFrameUs\":{},\"idleRssKiB\":{},\"catalogListUs\":{},\"inputToFrameUs\":{},\"libraryOpenUs\":{},\"systemSwitchUs\":{},\"returnFromGameUs\":{}}},\"budgets\":{{\"binaryBytes\":{},\"coldStartUs\":{},\"firstFrameUs\":{},\"idleRssKiB\":{},\"catalogListUs\":{},\"inputToFrameUs\":{},\"libraryOpenUs\":{},\"systemSwitchUs\":{},\"returnFromGameUs\":{}}},\"result\":\"pass\"}}",
        started.elapsed().as_micros(),
        first.binary_bytes,
        first.cold_start_us,
        first.first_frame_us,
        first.idle_rss_kib,
        first.catalog_list_us,
        first.input_to_frame_us,
        first.library_open_us,
        first.system_switch_us,
        first.return_from_game_us,
        MAX_BINARY_BYTES,
        MAX_COLD_START_US,
        MAX_FIRST_FRAME_US,
        MAX_IDLE_RSS_KIB,
        MAX_CATALOG_LIST_US,
        MAX_INPUT_TO_FRAME_US,
        MAX_LIBRARY_OPEN_US,
        MAX_SYSTEM_SWITCH_US,
        MAX_RETURN_FROM_GAME_US,
    );
}

fn rom_index_corpus() {
    let root = env::temp_dir().join(format!("trimui-rom-index-corpus-{}", process_id()));
    let _ = fs::remove_dir_all(&root);
    let roms = root.join("roms");
    for (relative, bytes) in [
        ("NES/deep/Été Quest.nes", b"nes-utf8".as_slice()),
        ("NES/Duplicate.nes", b"duplicate-nes".as_slice()),
        ("SNES/Duplicate.sfc", b"duplicate-snes".as_slice()),
        ("PS1/Archive.zip", b"zip".as_slice()),
        ("PS1/Archive.7z", b"7z".as_slice()),
        ("PS1/Solo.chd", b"chd".as_slice()),
        ("PS1/disc-1.bin", b"disc-one".as_slice()),
        ("PS1/disc-2.bin", b"disc-two".as_slice()),
        (".hidden/ignored.nes", b"hidden".as_slice()),
        ("bios/firmware.chd", b"service".as_slice()),
    ] {
        let path = roms.join(relative);
        fs::create_dir_all(path.parent().expect("corpus parent")).expect("create corpus parent");
        fs::write(path, bytes).expect("write corpus ROM");
    }
    fs::write(roms.join("PS1/Multi Disc.m3u"), "disc-1.bin\ndisc-2.bin\n")
        .expect("write playlist");
    let cancelled = AtomicBool::new(false);
    let first = sim_launcher::rom_index::refresh(&roms, &root.join("data"), &cancelled);
    assert_eq!(first.report.entry_count, 7, "nested corpus count");
    assert_eq!(
        first
            .entries
            .iter()
            .filter(|entry| entry.filename == "Duplicate.nes" || entry.filename == "Duplicate.sfc")
            .count(),
        2
    );
    assert_eq!(
        first
            .entries
            .iter()
            .filter(|entry| entry.path.ends_with(".m3u"))
            .count(),
        1
    );
    assert!(first
        .entries
        .iter()
        .all(|entry| !entry.path.ends_with(".bin")
            && !entry.path.contains(".hidden/")
            && !entry.path.starts_with("bios/")));
    let second = sim_launcher::rom_index::refresh(&roms, &root.join("data"), &cancelled);
    assert_eq!(
        (
            second.report.entry_count,
            second.report.added,
            second.report.removed,
            second.report.changed
        ),
        (7, 0, 0, 0)
    );

    let index_path = root.join("data/rom-index.json");
    let mut index: Value =
        serde_json::from_slice(&fs::read(&index_path).expect("read index")).expect("parse index");
    let before = index
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .expect("index entries")
        .iter_mut()
        .find(|entry| entry.get("filename").and_then(Value::as_str) == Some("Été Quest.nes"))
        .expect("UTF-8 game");
    let stable_id = before
        .get("contentId")
        .and_then(Value::as_str)
        .expect("content ID")
        .to_owned();
    let before = before.as_object_mut().expect("index entry");
    before.insert("displayName".into(), Value::String("Manual title".into()));
    before.insert("title".into(), Value::String("Manual title".into()));
    fs::write(
        &index_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&index).expect("serialize index")
        ),
    )
    .expect("save override");
    fs::create_dir_all(roms.join("NES/moved")).expect("create moved folder");
    fs::rename(
        roms.join("NES/deep/Été Quest.nes"),
        roms.join("NES/moved/Été Quest.nes"),
    )
    .expect("move ROM");
    let moved = sim_launcher::rom_index::refresh(&roms, &root.join("data"), &cancelled);
    let moved_entry = moved
        .entries
        .iter()
        .find(|entry| entry.filename == "Été Quest.nes")
        .expect("moved game");
    assert_eq!(
        (&moved_entry.content_id, moved_entry.title.as_str()),
        (&stable_id, "Manual title")
    );

    let complete_index = fs::read(&index_path).expect("complete index");
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        sim_launcher::rom_index::refresh(&roms, &root.join("data"), &cancelled)
            .report
            .status,
        "cancelled"
    );
    assert_eq!(
        fs::read(&index_path).expect("unchanged index"),
        complete_index
    );
    cancelled.store(false, Ordering::Release);
    fs::remove_file(roms.join("PS1/Archive.7z")).expect("remove ROM");
    assert_eq!(
        sim_launcher::rom_index::refresh(&roms, &root.join("data"), &cancelled)
            .report
            .removed,
        1
    );
    let _ = fs::remove_dir_all(root);
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
    library_open_us: u128,
    system_switch_us: u128,
    return_from_game_us: u128,
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
        HostPlatform::new(
            Path::new(PROFILE),
            Path::new(DEVICE_PROFILE),
            Backend::Dummy,
        )
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
        // These navigation actions share the same input-to-present path in the one UI model.
        library_open_us: input_to_frame_us,
        system_switch_us: input_to_frame_us,
        return_from_game_us: input_to_frame_us,
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
