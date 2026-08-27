# TG4040 display-profile contract

`schemas/display-profile-v1.schema.json`, `crates/display-profile`, and the generated fixture catalog define a closed, versioned consumer contract for the TG4040 display at logical `1024x768`. It is project-authored metadata only.

## Consumer boundary

A future UI, launcher, or session owner may consume a resolved profile. This contract does not render, load, or apply a display mode. It does not implement a session broker, launch or broker integration, persistence, hardware operation, PowerVR integration, or HIL integration. It does not probe or detect devices.

Selections are declarative metadata: `scaling` is one of `integer`, `original-aspect`, `crop`, or `fullscreen`; overlay and shader values are bounded identifiers only. They are not paths, URLs, bytes, artwork, shader source, or renderer instructions. System and profile defaults select no overlay and no shader. A profile may explicitly select its own metadata-only identifiers.

## Resolution and precedence

The consumer supplies a channel, system ID, profile ID, and optional opaque game ID. Resolution is fail-closed unless the system and profile are in the requested channel, the target is exactly `TG4040`, and the logical output is exactly `1024x768`.

The effective selection is resolved in this order:

1. system `defaultSelection`;
2. profile `defaultSelection` for the selected system;
3. a matching profile `gameOverrides` entry, if present;
4. a `reset` game override returns exactly to the profile default selection (step 2).

A `set` game override replaces the profile default with its complete typed selection. Unknown game IDs do not invent a selection and therefore resolve to the profile default. Stable profiles may name only stable systems; experimental profiles may name only their explicit experimental systems. No stable-to-experimental fallback or leakage is permitted.

Warnings are closed, typed presentation metadata (`code`, `severity`, and `messageKey`). The contract requires warning projection for non-default scaling, crop, fullscreen, overlays, and non-none shaders. Warnings are not hardware or performance claims.

## Acceptance boundary

The deterministic fixture journey validates the checked-in positive catalog and fixture journey against this closed schema offline, classifies schema-invalid versus schema-valid typed-semantic negatives, and proves typed precedence and deterministic serialization using generated synthetic data. Physical PowerVR review and HIL review are a deferred acceptance gate and are unavailable here. They are not claimed and do not block this host/static contract work.
