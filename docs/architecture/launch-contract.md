# LaunchRequest contract

This document defines the clean-room, versioned handoff from a future catalog/UI owner to a future launch broker. It is a data contract only. The repository does not implement a broker, session supervisor, socket, daemon, runtime launch, package installation, signature verification, ROM discovery, renderer, theme, input HAL, or power HAL.

## v1 shape

`schemas/launch-request-v1.schema.json` and `crates/launch-contract` define the same closed JSON object. `schemas/launch-catalog-v1.schema.json` defines the separately supplied closed catalog projection. Serde rejects unknown fields, and both schemas use `additionalProperties: false` for every object. The Rust serializer emits the stable struct field order used by the generated canonical fixture.

A request contains:

- `schemaVersion: 1`, a request ID, one of `libretro`, `standalone`, or `port`, and an opaque content ID with a lower-case SHA-256;
- logical paths as `{root, relative}`. Content is read only below `roms`; saves and states are written only below `data/saves` and `data/states`;
- exact versioned runner identity, an optional exact versioned core identity, and a profile ID;
- typed resume, display, input, and power settings.

There is no executable path, command, shell fragment, argv, environment, working directory, redirection, or process option in the type or schema. The request JSON Schema rejects closed request shape, unknown kinds, fixed roots, and structural malformed fields; catalog membership (including an unknown runner) is necessarily rejected by the typed validator only after the separately validated installed/signed-catalog projection is supplied. The request schema alone cannot prove a dynamic allowlist, and this intentional split is fail-closed. The catalog schema and strict typed deserializer validate the projection's runner/core/profile shapes, IDs, versions, kinds, and capabilities; this layer does not verify package signatures or claim that catalog data came from a trusted signer.

The host-only validator accepts an explicitly supplied fixture root. It never probes a device or chooses a host path from request data. It rejects root/type mismatches, absolute or traversal relatives, empty or dot components, NULs, backslashes, Windows-reserved names, overlong components and paths, case-colliding aliases, and canonical paths that escape the supplied root. The generated journey uses only zero-byte synthetic placeholders and a runtime-created synthetic symlink escape.

## Compatibility and forward migration

V1 is closed and exact: a consumer accepts only the declared v1 schema URI, `format`, and `schemaVersion: 1`. Unknown versions are rejected rather than treated as v1. A v1 producer may add no fields without a new contract version.

A future v2 migration is selected explicitly by the producer/consumer negotiation outside this object: the consumer advertises a supported version, the producer selects exactly that version, and the consumer validates the complete v2 schema before interpreting it. No v2 field is silently ignored by a v1 reader. A migration adapter may translate a fully validated v1 object to v2 (or the reverse) only when that adapter is separately versioned, deterministic, and preserves the v1 safety invariants; it must reject unsupported combinations instead of guessing.

Future package/catalog ownership remains outside this card. A later catalog or trust contract may define signed package metadata and verification boundaries, but this crate accepts only an already-projected allowlist and does not inspect packages, signatures, archives, ROMs, BIOS data, or physical storage.

## Evidence boundary

`launch-contract-fixtures` parses and validates the generated catalog projection through the strict typed path, then runs the canonical generated request and every generated negative request. Its output is a bounded count, not a content name, private corpus path, ROM/BIOS byte report, or command line. This is host/static contract evidence only and makes no TG4040 hardware, stock ABI, loader, graphics, input, radio, suspend, thermal, RAM, boot, or runtime claim.
