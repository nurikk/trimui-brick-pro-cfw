# TG4040 AArch64 QEMU-user lane

This is the sole TG4040 QEMU-user compatibility command. It consumes the real
`scripts/build release` path, not the host simulator. The release contains the
static AArch64 launcher semantic entry point, `session-broker` and its helper,
`package-manager`, the bootstrap binaries, the two POSIX shell payloads, and
three deterministic generated-input archives.

## Prerequisites and command

Docker, Python 3, `dash`, `file`, `readelf`, `sha256sum`, `jq`, and `tar` must
be installed. The baseline image and QEMU-user image are built from the
repository's digest/version-pinned Dockerfiles. Their tags, cache scopes, and
ownership labels use the deterministic namespace from the resolved physical
checkout root. Set `TRIMUI_DOCKER_NAMESPACE` only to an exact 1–48 character
lowercase alphanumeric/hyphen value starting and ending alphanumeric; it is
rejected rather than normalized when invalid. Every run requires exact current
root/source fingerprints and the pinned build-definition or QEMU-lock
fingerprint, so a peer, stale tag, or source change fails closed until rebuilt.
Image acquisition is the only network-capable setup step:

```sh
./scripts/build image
./scripts/qemu-aarch64 image
```

Run from this checkout with a positive fixed epoch and a fresh report path
outside the repository:

```sh
REPORT=$(mktemp /tmp/trimui-tg4040-qemu-report.XXXXXX)
rm -f "$REPORT"
SOURCE_DATE_EPOCH=1700000000 ./scripts/qemu-aarch64 run --report "$REPORT"
jq -e '(.result == "pass" and .targetSku == "TG4040" and (.elf | all(.[]; .static and .hostLibraryFallback == false and .qemuUserExecution)))' "$REPORT"
```

The command builds a fresh external release, verifies `SHA256SUMS`, manifest,
archive, generated SPDX and payload closure, extracts a fresh system root,
expands only the three manifest-listed generated-input archives, and mounts
that system root read-only beside a separate writable data root. Docker runs
with `--network none`, no capabilities, read-only container storage, and a
pinned QEMU-user image. It removes release, extraction, data, wrapper, and
container-run material on success, failure, and interrupt. Only the caller's
report remains. The report path must not already exist.

The release build also runs its normal offline checks and staged provenance
audit. The QEMU command then:

- checks every manifest-listed `usr/bin` ELF with `file` and `readelf` for
  AArch64 ELF64, static/no interpreter/no `DT_NEEDED`, and no RPATH/RUNPATH;
- starts every packaged target ELF through the image's
  `/usr/bin/qemu-aarch64-static`;
- parses both packaged shell payloads with host `dash -n` and target AArch64
  BusyBox `ash -n`;
- runs the project-authored launcher semantic journey without SDL or a host
  platform, packaged `session-broker simulate` journeys including accepted and
  rejected requests, packaged `package-manager demo`, and the documented
  non-hardware bootstrap/recovery simulation modes; and
- checks the writable-data sentinel survives while the extracted system stays
  non-writable.

The broker and userspace-supervisor have a test-lane-only
`BRICKPRO_QEMU_USER` execution handoff so their real child AArch64 processes
also start through the pinned emulator. This is process execution plumbing,
not a replacement implementation or placeholder journey.

## Report scope

The deterministic report is
`trimui-tg4040-qemu-aarch64-evidence/v1`. It contains exact archive, manifest,
checksum and build-info SHA-256/size identities; ELF static/loader/dependency
and QEMU-start records; shell, filesystem, package and semantic-journey
results; generated-input names; runtime-only sanitized namespace/root/source
fingerprint, base/lock/toolchain identity; cleanup status; and the evidence-scope statement:

> QEMU user mode here proves aarch64 ISA/ABI process startup or static
> behavior, userspace filesystem/package/shell boundary and project semantic
> journeys.

`evidenceScope.nonFidelity` deliberately sets GPU/PowerVR,
display/framebuffer, physical input, PMIC, radio/xradio, suspend/resume,
thermal, timing, and performance to `false`. These are not proven by this
lane. `qemu-system` is deliberately out of scope: this release has no concrete
kernel init, device mount, or firmware requirement that this userspace
verification would uncover. Adding a qemu-system lane requires such a concrete
requirement first.

No ROM, BIOS, firmware, stock/vendor filesystem, proprietary binary, private
corpus, device node, mount, activation, update, or hardware operation is used.
The generated fixtures are the only semantic inputs. Delete the external report
when it is no longer needed:

```sh
rm -f -- "$REPORT"
```
