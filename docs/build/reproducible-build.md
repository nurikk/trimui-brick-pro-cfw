# Reproducible build

The release build runs in the pinned baseline container with a clean external output directory and the existing worktree/image identity checks. It builds the host journeys and static `aarch64-unknown-linux-musl` payload, validates shell and JSON inputs, and rejects dynamic ELF dependencies.

The archive contains the expected CFW executables, bootstrap scripts, compatibility data, generated fixture archives, and `THIRD_PARTY_NOTICES.md`. `manifest.json` records staged regular files, sizes, modes, and SHA-256 values. `build-info.json` records the build inputs and toolchain. `SHA256SUMS` covers the emitted archive and release metadata.

Run:

```sh
SOURCE_DATE_EPOCH=1700000000 scripts/build release --out /tmp/trimui-release
python3 scripts/audit-dist /tmp/trimui-release/.baseline-work/stage /tmp/trimui-release/manifest.json /tmp/trimui-release/SHA256SUMS
scripts/test-release-cleanup
```

The release audit checks notices, expected executables, staged manifest hashes, forbidden private corpus paths/assets, archive readability, and the emitted archive checksum. It uses only the staged manifest and checksum file.
