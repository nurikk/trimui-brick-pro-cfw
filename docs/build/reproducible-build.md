# Reproducible TG4040 baseline build

This produces the first deterministic, non-activating TG4040 userspace baseline
candidate. It is a clean-room, no-device candidate, not a device-compatible
release. Never copy or activate it on a real device while the real-device
fingerprint and firmware allowlists are empty.

## Inputs and image

`provenance/components.json` is the source of truth for checked-in component,
package, license, path, type, mode, and obligation authorization. Exact output
identity is generated for each candidate in `manifest.json`; it is not frozen in
the checked-in authorization projection. The build image is based on the exact
digest-pinned Rust 1.85.1 image in
`containers/baseline/Dockerfile`; it records Rust, Cargo, the musl target,
components, and BusyBox. Build the labeled image once before a release build:

```sh
./scripts/build image
```

The helper computes a SHA-256 over the Dockerfile, `Cargo.lock`, Cargo
configuration, root manifest, and every workspace crate manifest, then passes
that digest as `BUILD_DEFINITION_SHA256`. The scripts also derive a deterministic
namespace from the resolved physical checkout root and label the image with the
namespace, root fingerprint, source fingerprint, and cache scope. A release
build fails closed when the local namespaced tag is missing, foreign, or has any
stale/missing fingerprint or build-definition label; it never silently rebuilds
or accepts a mutable tag. These runtime identities are deliberately absent from
release files, manifests, build-info, checksums, SPDX, and payload bytes.

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
audit. Supervisor fixture scenarios use a unique mode-0700 `mktemp` root under
`/tmp` so readiness socket paths stay below `SUN_LEN`; it is removed by the
build trap on success, failure, and signals. All release work remains under
`$OUT`. It produces exactly:

- `trimui-brick-pro-cfw-baseline.tar`
- `manifest.json`
- `SHA256SUMS`
- `brickpro-cfw.spdx.json`
- `THIRD_PARTY_NOTICES.md`
- `build-info.json`

The archive contains only static AArch64 `bootstrap-probe`, `brick-recovery`,
`brickpro-diagnostics`, `boot-state`, `update-agent`, and `userspace-supervisor`,
the existing POSIX bootstrap/recovery scripts, the TG4040 compatibility JSON,
and the generated license notice. It contains no manifest;
`manifest.json` is an external sidecar so it cannot hash itself. The candidate
manifest binds every staged file's path, type, mode, size, and SHA-256 to the
checked-in inventory and allowlist. The candidate SPDX document is generated
from that manifest identity and the authorized component/license records.

## Inspect and reproduce

```sh
(cd "$OUT" && sha256sum -c SHA256SUMS)
mkdir "$OUT/extracted"
tar -xf "$OUT/trimui-brick-pro-cfw-baseline.tar" -C "$OUT/extracted"
find "$OUT/extracted" -type f -print | sort
python3 -m json.tool "$OUT/manifest.json" >/dev/null
python3 -m json.tool "$OUT/build-info.json" >/dev/null
file "$OUT/extracted/usr/bin/brickpro-bootstrap-probe" \
     "$OUT/extracted/usr/bin/brickpro-recovery" \
     "$OUT/extracted/usr/bin/brickpro-diagnostics" \
     "$OUT/extracted/usr/bin/brickpro-boot-state" \
     "$OUT/extracted/usr/bin/brickpro-update-agent" \
     "$OUT/extracted/usr/bin/brickpro-userspace-supervisor"
for binary in "$OUT/extracted/usr/bin/brickpro-bootstrap-probe" \
              "$OUT/extracted/usr/bin/brickpro-recovery" \
              "$OUT/extracted/usr/bin/brickpro-diagnostics" \
              "$OUT/extracted/usr/bin/brickpro-boot-state" \
              "$OUT/extracted/usr/bin/brickpro-update-agent" \
              "$OUT/extracted/usr/bin/brickpro-userspace-supervisor"; do
    readelf -h "$binary"
    readelf -l "$binary"
    readelf -d "$binary"
done
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
state. The cleanup journey can exercise both successful and signal-interrupted
long-output releases without weakening socket validation:

```sh
./scripts/test-release-cleanup
```

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
  --inventory provenance/components.json \
  --manifest "$OUT/manifest.json" \
  --spdx "$OUT/brickpro-cfw.spdx.json" \
  --checksums "$OUT/SHA256SUMS"
for script in "$OUT/extracted"/bootstrap/*.sh; do
  dash -n "$script"
done
```

The audit also verifies the candidate manifest, candidate SPDX identity, and
checksum package before checking staged bytes. It rejects stale or tampered
identity records, missing/extra paths, mode/type changes, and source or
projection drift. The build additionally parses those scripts with BusyBox
`ash -n` from the pinned build image and parses every shipped JSON.

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

To remove the locally built image for this checkout without touching the repository:

```sh
. scripts/docker-worktree.sh
NS=$(trimui_docker_namespace "$PWD")
docker image rm "trimui-brick-pro-cfw-baseline:$NS"
```

There is no device rollback procedure for this candidate because no device
access or activation is permitted. If an external logical staging directory
was prepared, stop using it and remove that directory; do not copy anything
to a real device. Removing the external archive and sidecars is the complete
candidate removal action.
