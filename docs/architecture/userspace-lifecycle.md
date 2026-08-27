# Userspace lifecycle

This document describes the no-device bootstrap candidate, not a device boot chain. Physical paths, stock entrypoints, process supervisors, framebuffer/input nodes, mounts, ABI, and eMMC behavior are unknown and intentionally unspecified.

## Bootstrap

1. The candidate performs a read-only synthetic probe only when explicitly invoked with `--simulation-fixture-root` and an approved generated fixture marker.
2. Exact `TG4040` identity is checked before any other compatibility check. Missing, malformed, contradictory, `TG3040`, and non-exact identities fail closed.
3. Firmware, the synthetic 1024x768 framebuffer contract, semantic input capabilities, and logical SD boundaries are checked. Failure returns a bounded reason and enters recovery.
4. A successful synthetic probe permits one bounded `boot-context.json` record and its adjacent fixed SHA-256 record under the fixture's `.brickpro/data/update` boundary. The record is an attempt/context record, not activation or health.
5. The candidate `exec`s a project-owned supervisor command through the explicit simulation interface. No stock or guessed supervisor is started.

The no-argument candidate path is denied because the approved real-device fingerprint list is empty. It cannot activate a physical device until a later reviewed hardware change adds approved fingerprints and verified facts.

## Recovery

Recovery returns exactly three choices:

- `previous-userspace-release`
- `safe-mode`
- `stock-passthrough`

A generated next-boot marker or generated button-chord marker may deterministically select one choice inside a fixture root. These are not physical button evidence and do not establish a stock-resume chain. Stock passthrough is a simulated, non-activating result.

Safe mode means read-only logical system, normal logical data saves, disposable cache ignored, radios disabled, fallback theme, and no migration/update. These are logical policy semantics only; they do not assert physical radio, filesystem, mount, or power behavior.

## Storage and mutation boundary

The fixture contract represents only logical `.brickpro/system/slots/A`, `.brickpro/system/slots/B`, `.brickpro/data`, and `roms` boundaries. Bootstrap and recovery do not mount, probe, or mutate hardware storage. The only candidate write is the bounded context record after a successful synthetic probe; recovery reads only generated markers.

## What validation does not prove

The host and static AArch64 builds, locked dependency builds, shell parsing, static source gates, JSON validation, and journey evidence prove deterministic clean-room software behavior only. They do not prove physical TG4040 boot, framebuffer or input compatibility, stock passthrough, physical button controls, storage mounts, filesystem persistence, ABI compatibility, or eMMC behavior.
