# Input profiles and synthetic Hall calibration

This contract is a clean-room userspace configuration boundary for the TG4040 checkout. `config/input/profiles.json` is the production declarative catalog; `input-profile` validates it, resolves built-in, system, game, and session selections in that exact precedence order, and applies named semantic mappings. An absent system or game override falls back to the lower layer; a game selection may exist without a system selection. L3, R3, F1, F2, Fn, and Home are distinct controls/actions. External profiles require an exact SDL GUID and every declared capability; compatible ambiguity requires an explicit profile choice.

Axis response curves are deterministic: `linear` preserves the deadzone-adjusted value, while `smooth` returns `sign(value) * abs(value)^2` after the same deadzone. The catalog exercises the smooth curve.

The calibration model accepts only caller-supplied typed synthetic samples and an exact synthetic identity. Each capture must provide center/minimum/maximum samples for all four axes: left-x, left-y, right-x, and right-y. It rejects incomplete, unstable, non-finite, or degenerate captures. A valid record is canonical JSON with an identity-bound SHA-256 checksum. Publication uses a caller-supplied path, a restrictive same-directory temporary file, sync, and atomic replacement; rejected input and publication failure leave the previous bytes unchanged.

The checked-in catalog, schema, and fixture journey prove deterministic host userspace behavior only. They do not observe or claim physical Hall sensors, controls, input nodes, SDL devices, target firmware, event devices, buses, NVRAM, persistent target storage, or hardware calibration. No runtime default persistence path is defined.
