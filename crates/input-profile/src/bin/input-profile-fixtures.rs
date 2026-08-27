use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use input_profile::{
    apply_transform, calibrate, load, normalize, save, Action, Capture, Catalog, CurveTransform,
    DpadPair, RawAxis, RawControl, RawSample, SamplePhase, StickPair, SyntheticIdentity,
    TransformOutput,
};
use serde_json::Value;

const CATALOG: &[u8] = include_bytes!("../../../../config/input/profiles.json");
const SCHEMA: &[u8] = include_bytes!("../../../../schemas/input-profile-v1.schema.json");

fn main() {
    if let Err(error) = run() {
        eprintln!("input-profile-fixtures: {error}");
        process::exit(1);
    }
    println!("input-profile-fixtures: catalog, resolution, transforms, calibration, and persistence journeys passed");
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    if arguments.next().as_deref() != Some("--fixture-journey")
        || arguments.next().as_deref() != Some("--fixture-root")
    {
        return Err("usage: input-profile-fixtures --fixture-journey --fixture-root DIR".into());
    }
    let root = PathBuf::from(arguments.next().ok_or("missing fixture root")?);
    if arguments.next().is_some() {
        return Err("unexpected argument".into());
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let schema: Value = serde_json::from_slice(SCHEMA).map_err(|e| format!("schema: {e}"))?;
    require(
        schema["$id"] == "trimui-input-profile-v1",
        "schema identity",
    )?;
    let catalog = Catalog::from_json(CATALOG).map_err(|e| e.to_string())?;
    require(
        catalog.canonical_json().map_err(|e| e.to_string())? == CATALOG,
        "catalog canonical bytes",
    )?;
    resolution(&catalog)?;
    transforms(&catalog)?;
    calibration(&root)?;
    Ok(())
}

fn require(condition: bool, label: &str) -> Result<(), String> {
    condition
        .then_some(())
        .ok_or_else(|| format!("{label} failed"))
}
fn rejected<T>(result: Result<T, input_profile::ProfileError>, label: &str) -> Result<(), String> {
    require(result.is_err(), label)
}
fn mapping_action(profile: &input_profile::Profile, control: RawControl) -> Result<Action, String> {
    profile
        .mappings
        .iter()
        .find(|mapping| mapping.control == control)
        .map(|mapping| mapping.action)
        .ok_or_else(|| format!("missing mapping for {control:?}"))
}
fn resolved(
    catalog: &Catalog,
    system: Option<&str>,
    game: Option<&str>,
    session: Option<&str>,
    id: &str,
    scope: input_profile::ResolutionScope,
) -> Result<(), String> {
    let result = catalog
        .resolve(system, game, session)
        .map_err(|e| e.to_string())?;
    require(
        result.profile_id == id && result.scope == scope,
        &format!("resolution {id}"),
    )
}

fn resolution(catalog: &Catalog) -> Result<(), String> {
    resolved(
        catalog,
        None,
        None,
        None,
        "default",
        input_profile::ResolutionScope::BuiltIn,
    )?;
    resolved(
        catalog,
        Some("arcade"),
        None,
        None,
        "southpaw",
        input_profile::ResolutionScope::System,
    )?;
    resolved(
        catalog,
        Some("arcade"),
        Some("space-quest"),
        None,
        "stick-to-dpad",
        input_profile::ResolutionScope::Game,
    )?;
    resolved(
        catalog,
        Some("puzzle"),
        None,
        Some("southpaw"),
        "southpaw",
        input_profile::ResolutionScope::Session,
    )?;
    resolved(
        catalog,
        Some("unconfigured"),
        None,
        None,
        "default",
        input_profile::ResolutionScope::BuiltIn,
    )?;
    resolved(
        catalog,
        Some("arcade"),
        Some("unconfigured-game"),
        None,
        "southpaw",
        input_profile::ResolutionScope::System,
    )?;
    resolved(
        catalog,
        Some("standalone"),
        Some("solo"),
        None,
        "southpaw",
        input_profile::ResolutionScope::Game,
    )?;
    rejected(
        catalog.resolve(None, Some("space-quest"), None),
        "game without system rejection",
    )?;
    let mut invalid_reference = catalog.clone();
    invalid_reference.selections.built_in = "missing".into();
    rejected(
        invalid_reference.validate(),
        "unknown profile reference rejection",
    )?;
    rejected(
        catalog.resolve(None, None, Some("missing")),
        "unknown session profile rejection",
    )?;
    let mut duplicate_mapping = catalog.clone();
    duplicate_mapping.profiles[0].mappings[13].control = RawControl::Up;
    rejected(
        duplicate_mapping.validate(),
        "duplicate mapping key rejection",
    )?;
    let standard = catalog.profile("default").map_err(|e| e.to_string())?;
    let actions = [
        RawControl::L3,
        RawControl::R3,
        RawControl::F1,
        RawControl::F2,
    ]
    .into_iter()
    .map(|control| mapping_action(standard, control))
    .collect::<Result<Vec<_>, _>>()?;
    require(
        actions[0] != actions[1]
            && actions[1] != actions[2]
            && actions[2] != actions[3]
            && actions[0] != actions[3],
        "four raw controls remain distinct",
    )?;
    let fn_action = mapping_action(standard, RawControl::Fn)?;
    let home_action = mapping_action(standard, RawControl::Home)?;
    require(
        fn_action == Action::Fn && home_action == Action::Home && fn_action != home_action,
        "Fn and Home remain distinct",
    )?;
    let capabilities = vec![
        "a".into(),
        "left-stick".into(),
        "right-stick".into(),
        "extra".into(),
    ];
    rejected(
        catalog.select_external("0123456789abcdef0123456789abcdef", &capabilities, None),
        "ambiguous external selection",
    )?;
    let selected = catalog
        .select_external(
            "0123456789abcdef0123456789abcdef",
            &capabilities,
            Some("external-controller"),
        )
        .map_err(|e| e.to_string())?;
    require(
        selected.profile_id == "external-controller",
        "explicit external selection",
    )?;
    rejected(
        catalog.select_external(
            "0123456789abcdef0123456789abcdee",
            &capabilities,
            Some("external-controller"),
        ),
        "exact GUID mismatch",
    )?;
    rejected(
        catalog.select_external(
            "0123456789abcdef0123456789abcdef",
            &["a".into()],
            Some("external-controller"),
        ),
        "capability mismatch",
    )?;
    Ok(())
}

fn transforms(catalog: &Catalog) -> Result<(), String> {
    let output = apply_transform(
        CurveTransform::Southpaw,
        DpadPair { x: 0, y: 0 },
        StickPair { x: 1.0, y: 0.0 },
        StickPair { x: 0.0, y: 1.0 },
    );
    require(
        output
            == TransformOutput::Sticks {
                left: StickPair { x: 0.0, y: 1.0 },
                right: StickPair { x: 1.0, y: 0.0 },
            },
        "southpaw transform",
    )?;
    let output = apply_transform(
        CurveTransform::DpadToStick,
        DpadPair { x: -1, y: 1 },
        StickPair { x: 0.0, y: 0.0 },
        StickPair { x: 0.0, y: 0.0 },
    );
    require(
        output
            == TransformOutput::Sticks {
                left: StickPair { x: -1.0, y: 1.0 },
                right: StickPair { x: 0.0, y: 0.0 },
            },
        "d-pad-to-stick transform",
    )?;
    let output = apply_transform(
        CurveTransform::StickToDpad,
        DpadPair { x: 0, y: 0 },
        StickPair { x: -0.8, y: 0.6 },
        StickPair { x: 0.0, y: 0.0 },
    );
    require(
        output
            == TransformOutput::Dpad {
                value: DpadPair { x: -1, y: 1 },
            },
        "stick-to-d-pad transform",
    )?;
    require(
        catalog
            .profile("southpaw")
            .map_err(|e| e.to_string())?
            .transform
            == CurveTransform::Southpaw,
        "southpaw catalog profile",
    )?;
    require(
        catalog
            .profile("dpad-to-stick")
            .map_err(|e| e.to_string())?
            .transform
            == CurveTransform::DpadToStick,
        "d-pad catalog profile",
    )?;
    require(
        catalog
            .profile("stick-to-dpad")
            .map_err(|e| e.to_string())?
            .transform
            == CurveTransform::StickToDpad,
        "stick catalog profile",
    )?;
    Ok(())
}

fn identity() -> SyntheticIdentity {
    SyntheticIdentity {
        id: "synthetic-hall-v1".into(),
        axes: vec![
            RawAxis::LeftX,
            RawAxis::LeftY,
            RawAxis::RightX,
            RawAxis::RightY,
        ],
    }
}
fn capture() -> Capture {
    let mut samples = Vec::new();
    for axis in [
        RawAxis::LeftX,
        RawAxis::LeftY,
        RawAxis::RightX,
        RawAxis::RightY,
    ] {
        let offset = samples.len() as u64;
        for (index, value) in [
            (-0.01, SamplePhase::Center),
            (0.0, SamplePhase::Center),
            (0.01, SamplePhase::Center),
            (0.0, SamplePhase::Center),
            (-1.0, SamplePhase::Minimum),
            (-0.98, SamplePhase::Minimum),
            (1.0, SamplePhase::Maximum),
            (0.98, SamplePhase::Maximum),
            (0.0, SamplePhase::Center),
        ]
        .into_iter()
        .enumerate()
        {
            samples.push(RawSample {
                sequence: offset + index as u64,
                axis,
                phase: value.1,
                value: value.0,
            });
        }
    }
    Capture {
        identity: identity(),
        samples,
    }
}

fn calibration(root: &Path) -> Result<(), String> {
    let expected = identity();
    let capture_data = capture();
    let result = calibrate(&expected, &capture_data).map_err(|e| e.to_string())?;
    require(result.axes.len() == 4, "four calibrated axes")?;
    for axis in &result.axes {
        require(
            (normalize(axis, axis.center).map_err(|e| e.to_string())? - 0.0).abs() < f64::EPSILON,
            "center normalization",
        )?;
        require(
            (normalize(axis, axis.maximum).map_err(|e| e.to_string())? - 1.0).abs() < 0.001,
            "maximum normalization",
        )?;
    }
    let mut smooth = result.axes[0].clone();
    smooth.curve = input_profile::Curve::Smooth;
    let linear = normalize(&result.axes[0], 0.5).map_err(|e| e.to_string())?;
    let nonlinear = normalize(&smooth, 0.5).map_err(|e| e.to_string())?;
    require(
        (linear - nonlinear).abs() > 0.1 && nonlinear > 0.0,
        "smooth response curve",
    )?;
    let path = root.join("hall-calibration.json");
    save(&path, &result, false).map_err(|e| e.to_string())?;
    let original = fs::read(&path).map_err(|e| e.to_string())?;
    require(load(&path, &expected).is_ok(), "valid calibration load")?;
    let mut checksum = original.clone();
    let position = checksum
        .windows(9)
        .position(|window| window == b"sha256\": ")
        .ok_or("checksum field missing")?
        + 9;
    checksum[position] = if checksum[position] == b'0' {
        b'1'
    } else {
        b'0'
    };
    fs::write(&path, checksum).map_err(|e| e.to_string())?;
    rejected(load(&path, &expected), "corrupt checksum rejection")?;
    let mut schema = original.clone();
    let position = schema
        .windows(b"trimui-hall-calibration".len())
        .position(|window| window == b"trimui-hall-calibration")
        .ok_or("schema field missing")?;
    schema[position] = b'x';
    fs::write(&path, schema).map_err(|e| e.to_string())?;
    rejected(load(&path, &expected), "corrupt schema rejection")?;
    let mut identity_bytes = original.clone();
    let position = identity_bytes
        .windows(b"synthetic-hall-v1".len())
        .position(|window| window == b"synthetic-hall-v1")
        .ok_or("identity field missing")?;
    identity_bytes[position] = b'x';
    fs::write(&path, identity_bytes).map_err(|e| e.to_string())?;
    rejected(load(&path, &expected), "corrupt identity rejection")?;
    fs::write(&path, &original).map_err(|e| e.to_string())?;
    let mut dropped = capture_data.clone();
    dropped.samples[3].sequence += 1;
    rejected(calibrate(&expected, &dropped), "dropped stream rejection")?;
    let mut nonfinite = capture_data.clone();
    nonfinite.samples[0].value = f64::NAN;
    rejected(
        calibrate(&expected, &nonfinite),
        "non-finite sample rejection",
    )?;
    let malformed = br#"{"identity":{"id":"synthetic-hall-v1","axes":["left","right"]},"samples":[],"extra":true}"#;
    require(
        serde_json::from_slice::<Capture>(malformed).is_err(),
        "malformed capture rejection",
    )?;
    let mut insufficient = capture_data.clone();
    insufficient.samples.truncate(10);
    rejected(
        calibrate(&expected, &insufficient),
        "insufficient capture rejection",
    )?;
    let mut noisy = capture_data.clone();
    noisy.samples[0].value = 0.2;
    rejected(calibrate(&expected, &noisy), "noisy center rejection")?;
    let mut degenerate = capture_data.clone();
    for sample in degenerate
        .samples
        .iter_mut()
        .filter(|sample| sample.phase == SamplePhase::Minimum)
    {
        sample.value = -0.1;
    }
    rejected(
        calibrate(&expected, &degenerate),
        "degenerate range rejection",
    )?;
    let mut changed = capture_data.clone();
    changed.samples[6].value = 0.8;
    let changed_calibration = calibrate(&expected, &changed).map_err(|e| e.to_string())?;
    let sentinel = root.join("external-sentinel");
    fs::write(&sentinel, b"sentinel").map_err(|e| e.to_string())?;
    let symlink = root.join("calibration-link");
    std::os::unix::fs::symlink(&sentinel, &symlink).map_err(|e| e.to_string())?;
    let sentinel_before = fs::read(&sentinel).map_err(|e| e.to_string())?;
    rejected(
        save(&symlink, &result, false),
        "calibration symlink rejection",
    )?;
    rejected(
        load(&symlink, &expected),
        "calibration symlink load rejection",
    )?;
    require(
        fs::read(&sentinel).map_err(|e| e.to_string())? == sentinel_before,
        "symlink sentinel preserved",
    )?;
    let directory = root.join("calibration-directory");
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    rejected(
        save(&directory, &result, false),
        "non-regular calibration rejection",
    )?;
    fs::write(&path, &original).map_err(|e| e.to_string())?;
    let before_failure = fs::read(&path).map_err(|e| e.to_string())?;
    rejected(
        save(&path, &changed_calibration, true),
        "injected publication failure",
    )?;
    require(
        fs::read(&path).map_err(|e| e.to_string())? == before_failure,
        "publication failure preserves bytes",
    )?;
    fs::write(&path, b"invalid old calibration").map_err(|e| e.to_string())?;
    let invalid_old = fs::read(&path).map_err(|e| e.to_string())?;
    rejected(
        save(&path, &changed_calibration, false),
        "invalid old content rejection",
    )?;
    require(
        fs::read(&path).map_err(|e| e.to_string())? == invalid_old,
        "invalid old content preserves bytes",
    )?;
    Ok(())
}
