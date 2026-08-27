# TG4040 userspace A/B updates

This is a clean-room, userspace-only simulator. It operates only below an explicit fixture root:

- `.brickpro/system/slots/A/system.squashfs` and `B/system.squashfs` are logical userspace slots;
- `.brickpro/data/update/staging/<release-id>` is the only staging location;
- `.brickpro/data/update/state.0.json` and `state.1.json` are redundant boot-state records;
- `roms`, `data/saves`, and `data/states` are protected and never written by update-agent.

A v1 manifest is closed and binds exact `TG4040`, a bounded release ID, increasing release sequence, stock-firmware window, userspace ABI, data-schema window, `squashfs-userspace` payload type, exact byte size, SHA-256, and a Minisign trusted comment. The signed comment is exactly `project=trimui-brick-pro-cfw; target=tg4040; release=<id>; sequence=<number>; payload-sha256=<digest>; manifest-sha256=<digest>`. The manifest digest is SHA-256 of canonical sorted JSON with `trustedComment` removed, avoiding a self-referential digest. Firmware versions are exactly three decimal `major.minor.patch` components, each bounded to `u16`; malformed or reversed windows are rejected and comparisons are numeric tuples, not strings. The payload must have a SquashFS header. `.awimg`, raw, block, partition, bootloader, eMMC, and other target claims are rejected.

The stage order is: parse/canonical-manifest validation, compatibility and prior-release-readable proof, payload size/header/hash, detached Minisign verification, protected-tree snapshot, inactive-slot staging and sync, inactive-slot sync, then state sync. Only the final state boundary makes the inactive slot pending. Interruption before or after any boundary leaves the old current slot selected; staged material is retained. Rollback never removes an activated staging directory. If both state records are checksum-valid at the same generation but contain different bytes/states, selection fails closed rather than choosing arbitrarily.

`boot-state select` increments pending attempts before handoff. The fourth selection after three unacknowledged pending boots selects the previous slot and persists `automatic-rollback` with attempts `3`. `boot-state mark-healthy` requires same-boot HAL self-check, broker readiness, launcher first frame, writable data, and readable ROMs; it then promotes pending to current, moves current to previous, records last-known-good, and resets attempts.

## Interfaces

`tools/package-release.sh --manifest FILE --payload FILE --signing-key EXTERNAL_PRIVATE_KEY --out DIR` signs and verifies a deterministic synthetic package before writing it; its only recovery actions are `boot-current`, `boot-previous`, and `discard-unactivated-staging`. It does not activate anything. `update-agent` has no physical-device interface.

The verifier uses the pinned zero-dependency pure-Rust `minisign-verify = =0.2.5` crate and the compiled-in `keys/update.pub` key. It cryptographically verifies the canonical manifest before inspecting the signed trusted comment; key IDs, release ID, sequence, target, payload digest, and manifest digest must all match. The release-host package tool may use an external Minisign CLI and ephemeral private key, but never places private signing material in the repository.

Physical kernel SquashFS loop-mount behavior is unknown. Versioned-release directories are a documented future fallback pending physical FAT/exFAT semantics and are not activated here.
