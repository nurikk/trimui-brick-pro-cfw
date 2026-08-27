# Reproducible TG4040 baseline build

This produces the first deterministic, non-activating TG4040 userspace baseline
candidate. It is a clean-room, no-device candidate, not a device-compatible
release. Never copy or activate it on a real device while the real-device
fingerprint and firmware allowlists are empty.

## Inputs and image

The source of truth for distributed bytes is `provenance/components.json`.
The build image is based on the exact digest-pinned Rust 1.85.1 image in
`containers/baseline/Dockerfile`; it records Rust, Cargo, the musl target,
components, and BusyBox. Build the labeled image once before a release build:

```sh
./scripts/build image
```

The helper computes a SHA-256 over the Dockerfile, `Cargo.lock`, Cargo
configuration, root manifest, and every workspace crate manifest, then passes
that digest as `BUILD_DEFINITION_SHA256`. The Dockerfile records both that
label and the immutable Rust base-image digest. A release build fails closed
when the local tag is missing or has either stale/missing label; it never
silently rebuilds or accepts a mutable tag.

That setup step may download the pinned Rust components and exact Debian
BusyBox package. The compilation itself is offline: the source is mounted
read-only, Docker networking is disabled, Cargo uses the image's fetched
locked registry in temporary `/tmp` state, and only the caller's external
output directory is writable.

## Build

Use an existing external directory (not `dist/` and not any path below this
repository):

```sh
OUT=$(mktemp -d /tmp/trimui-brick-pro-baseline.XXXXXX)
SOURCE_DATE_EPOCH=1700000000 ./scripts/build release --out "$OUT"
```

The command rejects missing/non-positive epochs, unknown arguments, relative
outputs, repository outputs, and non-empty output directories. It never
changes the pre-existing output-directory mode. It runs `cargo fmt --check`, locked offline
host release and static `aarch64-unknown-linux-musl` release builds, clippy,
host gate self-checks, JSON/shell checks, ELF checks, and the staged provenance
audit. It produces exactly:

- `trimui-brick-pro-cfw-baseline.tar`
- `manifest.json`
- `SHA256SUMS`
- `brickpro-cfw.spdx.json`
- `THIRD_PARTY_NOTICES.md`
- `build-info.json`

The archive contains only static AArch64 `bootstrap-probe` and
`brick-recovery`, the existing POSIX bootstrap/recovery scripts, the TG4040
compatibility JSON, and the generated license notice. It contains no manifest;
`manifest.json` is an external sidecar so it cannot hash itself.

## Inspect and reproduce

```sh
(cd "$OUT" && sha256sum -c SHA256SUMS)
mkdir "$OUT/extracted"
tar -xf "$OUT/trimui-brick-pro-cfw-baseline.tar" -C "$OUT/extracted"
find "$OUT/extracted" -type f -print | sort
python3 -m json.tool "$OUT/manifest.json" >/dev/null
python3 -m json.tool "$OUT/build-info.json" >/dev/null
file "$OUT/extracted/usr/bin/brickpro-bootstrap-probe" \
     "$OUT/extracted/usr/bin/brickpro-recovery"
readelf -h "$OUT/extracted/usr/bin/brickpro-bootstrap-probe"
readelf -l "$OUT/extracted/usr/bin/brickpro-bootstrap-probe"
readelf -d "$OUT/extracted/usr/bin/brickpro-bootstrap-probe"
```

The ELF checks prove only compiler/ISA output: AArch64, no interpreter, and no
`NEEDED` entries. They do not prove the target kernel ABI, loader, firmware,
board, peripherals, or physical boot compatibility.

Build a second clean external output with the same checked-out tree and epoch,
then compare every named artifact:

```sh
OUT2=$(mktemp -d /tmp/trimui-brick-pro-baseline.XXXXXX)
SOURCE_DATE_EPOCH=1700000000 ./scripts/build release --out "$OUT2"
for name in \
  trimui-brick-pro-cfw-baseline.tar manifest.json SHA256SUMS \
  brickpro-cfw.spdx.json THIRD_PARTY_NOTICES.md build-info.json; do
  cmp "$OUT/$name" "$OUT2/$name"
done
```

## Fixture and simulator evidence

The build runs the existing host-native generated-fixture journey against its
external, container-built host binaries before removing the temporary build
state:

```text
$ROOT/tools/sim/journeys/bootstrap-recovery.sh $WORK/host
```

This is host-native generated-fixture evidence only, not device or QEMU
evidence, and must not be relabeled as a physical launch. The build also
independently verifies that the default probe and recovery paths return
`real-fingerprint-not-approved`. The packaged scripts require the explicit
simulation interface for fixtures; their default no-argument path is
fail-closed and non-activating.

The archive may be extracted and audited without a device:

```sh
scripts/audit-dist "$OUT/extracted" \
  policy/distribution-allowlist.json \
  --inventory provenance/components.json
for script in "$OUT/extracted"/bootstrap/*.sh; do
  dash -n "$script"
done
```

The build additionally parses those scripts with BusyBox `ash -n` from the
pinned build image and parses every shipped JSON.

## SD staging policy

No physical SD, eMMC, block device, mount, flasher, updater, firmware, stock
ABI, loader, vendor library, ROM, BIOS, PortMaster, graphics, theme, font,
emulator, or private content is part of this candidate. If a later operator
reviews an external logical removable-SD staging plan, use a disposable
external directory and preserve the read-only logical system policy. Do not
write a device, infer a mount point, or activate this candidate. Activation
requires a future reviewed change that adds non-empty approved fingerprint and
firmware allowlists and closes the hardware contract.

## Cleanup and rollback/removal

Outputs are caller-owned. After inspection, remove only the external output:

```sh
rm -rf -- "$OUT" "$OUT2"
```

To remove a locally built image without touching the repository:

```sh
docker image rm trimui-brick-pro-cfw-baseline:local
```

There is no device rollback procedure for this candidate because no device
access or activation is permitted. If an external logical staging directory
was prepared, stop using it and remove that directory; do not copy anything
to a real device. Removing the external archive and sidecars is the complete
candidate removal action.
