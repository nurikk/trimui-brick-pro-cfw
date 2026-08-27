# TG4040 bootstrap candidate

This is a clean-room, no-device bootstrap candidate. It is not an activatable device payload. The authoritative hardware contract is [`tg4040.md`](tg4040.md); its runtime model files, firmware/kernel, framebuffer, evdev, storage mounts, ABI, stock entrypoint, and physical behavior remain unknown.

## Gate

`bootstrap-probe` has one executable fixture interface:

```text
bootstrap-probe --simulation-fixture-root <generated-fixture-root>
```

The root must contain the generated `fixture.json` marker and `profile.json` contract. The marker is synthetic-only, has exact `TG4040` identity, and explicitly disables real-device activation. The probe checks model identity before firmware, display, input, storage, or fingerprint state. It uses exact SKU equality: `TG3040`, missing identity, contradictory identity, and every non-exact `TG4040` value fail closed.

The checked-in profiles exercise these bounded reasons:

- `target-sku-mismatch`
- `firmware-unsupported`
- `framebuffer-missing` and `framebuffer-invalid`
- `input-missing` and `input-capability-missing`
- `storage-missing` and `storage-unsupported`
- `real-fingerprint-not-approved`

The successful synthetic profile means only that the generated contract is internally compatible with the candidate. It does not verify a physical display, input device, firmware, mount, ABI, or boot path.

## Scripts

`bootstrap/boot.sh` is POSIX `sh` orchestration. With no arguments it invokes the fixed candidate paths and fails into the real-device-denied recovery result. With the explicitly named `--simulation-fixture-root` interface it requires absolute `BRICKPRO_SIMULATION_PROBE`, `BRICKPRO_SIMULATION_RECOVERY`, and `BRICKPRO_SIMULATION_SUPERVISOR` paths. This override is for generated fixtures only; the binaries still reject an unapproved fixture marker.

A successful synthetic probe is the only case that writes `boot-context.json` below `.brickpro/data/update`; its adjacent fixed SHA-256 record is written with it. It records a bounded synthetic previous-release attempt and then `exec`s the project-owned supervisor command supplied by that simulation interface. It does not select or mount a real release, start a stock/user supervisor, mark boot healthy, migrate, update, or access hardware.

`bootstrap/recovery.sh` exposes the same explicit simulation boundary. `brick-recovery` always exposes exactly these three names:

1. `previous-userspace-release`
2. `safe-mode`
3. `stock-passthrough`

Selection is deterministic from `--select`, then the generated `.brickpro/data/recovery-next-boot` marker, then the generated `.brickpro/data/recovery-button-chord` marker. The markers are fixture files, not physical controls or a stock-resume chain. Stock passthrough is reported as a non-activating simulated outcome and never invokes a guessed stock binary.

Safe mode is a logical, non-activating policy only. Its deterministic presentation includes firmware/build, verified SKU, RAM, battery, temperature, storage, active/previous slots, active core, and last crash; unavailable values carry an explicit reason. It uses the built-in theme, conservative display/input, disabled network and third-party themes, no background indexing or automatic game launch, and no firmware, ROM, save, updater-record, raw-storage, or eMMC mutation. No physical semantics are claimed.

`brickpro-diagnostics --simulation-fixture-root <root> --export-support-bundle <sd-dir>` writes `trimui-support-bundle-v1/` below the selected, existing synthetic SD directory. The archive and checksum sidecar are fsynced in a staging directory, then published together by one directory rename; an existing bundle directory is never overwritten.

## Evidence boundary

Host builds, static AArch64 builds, shell parsing, JSON checks, and generated journeys prove only deterministic candidate behavior. They prove neither physical TG4040 boot, framebuffer/input compatibility, stock passthrough, button controls, storage mounts, nor eMMC behavior. The real-device fingerprint allowlist is intentionally empty and pending; no real device can activate this candidate in this phase.
