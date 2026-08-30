use display_profile::{
    canonical_json, parse, validate_fixture_manifest, Catalog, FixtureManifest, ResolutionRequest,
    SCHEMA,
};
use std::process;

const CATALOG: &[u8] =
    include_bytes!("../../../../fixtures/display-profile/generated-v1/catalog.json");
const JOURNEY: &[u8] =
    include_bytes!("../../../../fixtures/display-profile/generated-v1/journey.json");
const SCHEMA_JSON: &[u8] = include_bytes!("../../../../schemas/display-profile-v1.schema.json");
const BRICK_PRO_DEVICE: &[u8] =
    include_bytes!("../../../../config/platform/tg4040/compatibility.json");
const SYNTHETIC_WIDE_DEVICE: &[u8] =
    include_bytes!("../../../../fixtures/platform/synthetic-wide/compatibility.json");
const MAX_DOCUMENT_BYTES: usize = 512 * 1024;

const NEGATIVES: &[(&str, &[u8])] = &[
    ("unknown-field", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/unknown-field.json")),
    ("non-tg4040-sku", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/non-tg4040-sku.json")),
    ("wrong-dimensions", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/wrong-dimensions.json")),
    ("unknown-mode", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/unknown-mode.json")),
    ("shader-missing-warning", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/shader-missing-warning.json")),
    ("duplicate-identifier", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/duplicate-identifier.json")),
    ("invalid-identifier", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/invalid-identifier.json")),
    ("channel-leakage", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/channel-leakage.json")),
    ("invalid-reset-override", include_bytes!("../../../../fixtures/display-profile/generated-v1/negative/invalid-reset-override.json")),
];

fn main() {
    if let Err(error) = run() {
        eprintln!("display-profile-fixtures: {error}");
        process::exit(1);
    }
    println!("display-profile-fixtures: 8 resolutions, 9 negatives (5 schema-invalid, 4 typed-semantic), and profile-driven schema validation passed");
}

fn run() -> Result<(), String> {
    schema_check()?;
    let device = device_profile::DeviceProfile::from_json(BRICK_PRO_DEVICE)
        .map_err(|error| format!("Brick Pro device profile: {error}"))?;
    let catalog: Catalog = parse(CATALOG).map_err(|error| format!("catalog parse: {error}"))?;
    catalog
        .validate(&device)
        .map_err(|error| format!("catalog validation: {error}"))?;
    let manifest: FixtureManifest =
        parse(JOURNEY).map_err(|error| format!("journey parse: {error}"))?;
    validate_fixture_manifest(&manifest).map_err(|error| format!("journey validation: {error}"))?;
    if manifest.cases.len() != 7 {
        return Err("unexpected journey count".into());
    }

    for case in &manifest.cases {
        let request = ResolutionRequest {
            schema: SCHEMA.into(),
            format: "trimui-display-profile".into(),
            schema_version: 1,
            kind: display_profile::RequestKind::ResolutionRequest,
            channel: case.channel.clone(),
            system_id: case.system_id.clone(),
            profile_id: case.profile_id.clone(),
            game_id: case.game_id.clone(),
        };
        let resolved = catalog
            .resolve(&device, &request)
            .map_err(|error| format!("{} resolution: {error}", case.id))?;
        if resolved.selection.scaling != case.expected_scaling
            || resolved
                .selection
                .overlay_selection
                .as_ref()
                .map(|item| item.id.as_str())
                != case.expected_overlay.as_deref()
            || resolved
                .selection
                .shader_selection
                .as_ref()
                .map(|item| item.id.as_str())
                != case.expected_shader.as_deref()
        {
            return Err(format!("{} resolved unexpected selection", case.id));
        }
        if case.id == "overlay-and-shader" && resolved.selection.warnings.len() != 2 {
            return Err("overlay and shader warning projection is incomplete".into());
        }
        if case.id == "reset" && resolved.selection != catalog.profiles[0].default_selection {
            return Err("reset did not return the profile default exactly".into());
        }
        let first = canonical_json(&resolved).map_err(|error| error.to_string())?;
        let second = canonical_json(
            &catalog
                .resolve(&device, &request)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        if first != second {
            return Err(format!("{} serialization is not deterministic", case.id));
        }
    }

    let synthetic_device = device_profile::DeviceProfile::from_json(SYNTHETIC_WIDE_DEVICE)
        .map_err(|error| format!("synthetic device profile: {error}"))?;
    let synthetic_catalog_bytes = String::from_utf8(CATALOG.to_vec())
        .map_err(|error| format!("synthetic catalog UTF-8: {error}"))?
        .replace("TG4040", "SYNTHETIC-WIDE")
        .replace("1024", "1280")
        .replace("768", "720");
    let synthetic_catalog: Catalog = parse(synthetic_catalog_bytes.as_bytes())
        .map_err(|error| format!("synthetic catalog parse: {error}"))?;
    synthetic_catalog
        .validate(&synthetic_device)
        .map_err(|error| format!("synthetic catalog validation: {error}"))?;
    let synthetic_request = ResolutionRequest {
        schema: SCHEMA.into(),
        format: "trimui-display-profile".into(),
        schema_version: 1,
        kind: display_profile::RequestKind::ResolutionRequest,
        channel: display_profile::Channel::Stable,
        system_id: "tg4040".into(),
        profile_id: "tg4040-default".into(),
        game_id: None,
    };
    if synthetic_catalog
        .resolve(&synthetic_device, &synthetic_request)
        .map_err(|error| format!("synthetic resolution: {error}"))?
        .logical_output
        != (display_profile::LogicalOutput {
            width: 1280,
            height: 720,
        })
    {
        return Err("synthetic device did not select its logical viewport".into());
    }

    Ok(())
}

fn schema_check() -> Result<(), String> {
    let schema: serde_json::Value = serde_json::from_slice(SCHEMA_JSON)
        .map_err(|error| format!("schema JSON parse: {error}"))?;
    if schema["$schema"] != "https://json-schema.org/draft/2020-12/schema"
        || schema["$id"] != SCHEMA
        || schema["$defs"]["catalog"]["additionalProperties"] != false
        || schema["$defs"]["selection"]["additionalProperties"] != false
    {
        return Err("schema is not the expected closed draft-2020-12 contract".into());
    }

    validate_document(CATALOG, &schema).map_err(|error| format!("catalog schema: {error}"))?;
    validate_document(JOURNEY, &schema).map_err(|error| format!("journey schema: {error}"))?;
    let semantic_only = [
        "non-tg4040-sku",
        "wrong-dimensions",
        "shader-missing-warning",
        "channel-leakage",
    ];
    let mut schema_invalid = 0;
    let mut typed_semantic = 0;
    for (name, bytes) in NEGATIVES {
        let schema_result = validate_document(bytes, &schema);
        if semantic_only.contains(name) {
            if let Err(error) = schema_result {
                return Err(format!("{name} should be schema-valid: {error}"));
            }
            let device = device_profile::DeviceProfile::from_json(BRICK_PRO_DEVICE)
                .map_err(|error| format!("Brick Pro device profile: {error}"))?;
            let typed_result =
                parse::<Catalog>(bytes).and_then(|catalog| catalog.validate(&device));
            if typed_result.is_ok() {
                return Err(format!("typed semantic negative accepted: {name}"));
            }
            typed_semantic += 1;
        } else if schema_result.is_ok() {
            return Err(format!("schema negative accepted: {name}"));
        } else {
            schema_invalid += 1;
        }
    }
    if schema_invalid != 5 || typed_semantic != 4 {
        return Err(format!(
            "negative classification was {schema_invalid} schema-invalid and {typed_semantic} typed-semantic"
        ));
    }
    Ok(())
}

fn validate_document(bytes: &[u8], schema: &serde_json::Value) -> Result<(), String> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err("document exceeds size bound".into());
    }
    let document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("document JSON parse: {error}"))?;
    validate_schema(&document, schema, schema, 0)
}

fn validate_schema(
    instance: &serde_json::Value,
    schema: &serde_json::Value,
    root: &serde_json::Value,
    depth: usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err("schema nesting exceeds bound".into());
    }
    if let Some(object) = schema.as_object() {
        const SUPPORTED: &[&str] = &[
            "$defs",
            "$id",
            "$ref",
            "$schema",
            "additionalProperties",
            "anyOf",
            "const",
            "description",
            "enum",
            "items",
            "kind",
            "maxLength",
            "minItems",
            "minLength",
            "oneOf",
            "pattern",
            "properties",
            "required",
            "title",
            "type",
            "uniqueItems",
        ];
        if object.keys().any(|key| !SUPPORTED.contains(&key.as_str())) {
            return Err("schema contains an unsupported keyword".into());
        }
    }
    if let Some(reference) = schema.get("$ref") {
        let reference = reference
            .as_str()
            .ok_or_else(|| "schema reference is not a string".to_string())?;
        let name = reference
            .strip_prefix("#/$defs/")
            .ok_or_else(|| "external schema reference is not permitted".to_string())?;
        let definition = root["$defs"][name]
            .as_object()
            .ok_or_else(|| format!("schema definition is missing: {name}"))?;
        return validate_schema(
            instance,
            &serde_json::Value::Object(definition.clone()),
            root,
            depth + 1,
        );
    }
    if let Some(branches) = schema.get("oneOf").and_then(serde_json::Value::as_array) {
        let matches = branches
            .iter()
            .filter(|branch| validate_schema(instance, branch, root, depth + 1).is_ok())
            .count();
        if matches != 1 {
            return Err(format!("oneOf matched {matches} branches"));
        }
    }
    if let Some(branches) = schema.get("anyOf").and_then(serde_json::Value::as_array) {
        if !branches
            .iter()
            .any(|branch| validate_schema(instance, branch, root, depth + 1).is_ok())
        {
            return Err("anyOf matched no branches".into());
        }
    }
    if let Some(expected) = schema.get("const") {
        if instance != expected {
            return Err("const value does not match".into());
        }
    }
    if let Some(values) = schema.get("enum").and_then(serde_json::Value::as_array) {
        if !values.iter().any(|value| value == instance) {
            return Err("enum value is not permitted".into());
        }
    }
    if let Some(kind) = schema.get("type").and_then(serde_json::Value::as_str) {
        let matches = match kind {
            "object" => instance.is_object(),
            "array" => instance.is_array(),
            "string" => instance.is_string(),
            "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
            "boolean" => instance.is_boolean(),
            "null" => instance.is_null(),
            _ => return Err(format!("unsupported schema type: {kind}")),
        };
        if !matches {
            return Err(format!("expected {kind}"));
        }
    }
    if let Some(object) = instance.as_object() {
        if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
            for name in required.iter().filter_map(serde_json::Value::as_str) {
                if !object.contains_key(name) {
                    return Err(format!("required property is missing: {name}"));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
            let properties = schema["properties"]
                .as_object()
                .ok_or_else(|| "closed schema object has no properties".to_string())?;
            if object.keys().any(|name| !properties.contains_key(name)) {
                return Err("unknown property in closed object".into());
            }
        }
        if let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (name, property_schema) in properties {
                if let Some(value) = object.get(name) {
                    validate_schema(value, property_schema, root, depth + 1)?;
                }
            }
        }
    }
    if let Some(items) = schema.get("items") {
        if let Some(array) = instance.as_array() {
            if let Some(minimum) = schema.get("minItems").and_then(serde_json::Value::as_u64) {
                if array.len() < minimum as usize {
                    return Err("array is shorter than minItems".into());
                }
            }
            if schema.get("uniqueItems") == Some(&serde_json::Value::Bool(true)) {
                // ponytail: bounded O(n²) fixture uniqueness; use hashed canonical items if fixture size grows.
                if array
                    .iter()
                    .enumerate()
                    .any(|(index, value)| array[..index].contains(value))
                {
                    return Err("array items are not unique".into());
                }
            }
            for value in array {
                validate_schema(value, items, root, depth + 1)?;
            }
        }
    }
    if let Some(value) = instance.as_str() {
        if let Some(minimum) = schema.get("minLength").and_then(serde_json::Value::as_u64) {
            if value.chars().count() < minimum as usize {
                return Err("string is shorter than minLength".into());
            }
        }
        if let Some(maximum) = schema.get("maxLength").and_then(serde_json::Value::as_u64) {
            if value.chars().count() > maximum as usize {
                return Err("string is longer than maxLength".into());
            }
        }
        if let Some(pattern) = schema.get("pattern").and_then(serde_json::Value::as_str) {
            if !matches_pattern(value, pattern) {
                return Err("string does not match schema pattern".into());
            }
        }
    }
    Ok(())
}

fn matches_pattern(value: &str, pattern: &str) -> bool {
    match pattern {
        "^[a-z][a-z0-9-]{0,63}$" => is_identifier(value, 64, |byte| byte == b'-'),
        "^[a-z][a-z0-9.-]{0,95}$" => is_identifier(value, 96, |byte| byte == b'.' || byte == b'-'),
        _ => false,
    }
}

fn is_identifier(value: &str, maximum: usize, extra: impl Fn(u8) -> bool) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && byte.is_ascii_lowercase())
                || (index > 0
                    && (byte.is_ascii_lowercase() || byte.is_ascii_digit() || extra(byte)))
        })
}
