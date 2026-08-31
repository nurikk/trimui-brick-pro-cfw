# Input profiles and synthetic Hall calibration

This contract is a clean-room userspace configuration boundary for the TG4040 checkout. `config/input/profiles.json` is the baseline physical-to-logical catalog; `input-profile` resolves built-in, system, game, and session selections in that exact precedence order. Persisted mapping layers are applied global → system → game and reset removes only the selected layer, restoring the known-good catalog baseline. The resolved bindings and hotkeys are the sole launch projection for the launcher, RetroArch, standalone, and PortMaster adapters.

F1, F2, slider, Fn, and Home are independently configurable. The `Fn+Select` escape chord is reserved and always restored if a user layer omits it. Duplicate physical bindings and duplicate chords are rejected. Load-state and quit require hold or confirmation; they cannot be immediate actions. Player assignment covers P1–P4 and disconnecting the assigned external P1 immediately falls back to `built-in`, without a frontend restart.

Axis response curves are deterministic: `linear` preserves the deadzone-adjusted value, while `smooth` returns `sign(value) * abs(value)^2` after the same deadzone. The catalog exercises the smooth curve.

The calibration model accepts only caller-supplied typed synthetic samples and an exact synthetic identity. Each capture must provide center/minimum/maximum samples for all four axes: left-x, left-y, right-x, and right-y. It rejects incomplete, unstable, non-finite, or degenerate captures. A valid record is canonical JSON with an identity-bound SHA-256 checksum. Publication uses a caller-supplied path, a restrictive same-directory temporary file, sync, and atomic replacement; rejected input and publication failure leave the previous bytes unchanged.

The tester reports supplied raw values for both sticks, calibrated center, deadzone-adjusted saturation, inversion, and direct D-pad navigation (analog drift never becomes D-pad input). Descriptor checks cover supplied built-in, USB, and Bluetooth-shaped fixture metadata only; they do not claim target Bluetooth compatibility or physical-device support.

The checked-in catalog, schema, persistence code, and fixture journey prove deterministic host userspace behavior only. They do not observe or claim physical Hall sensors, controls, input nodes, SDL devices, target firmware, event devices, buses, NVRAM, or mechanical-stick health.
