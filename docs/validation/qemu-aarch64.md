# QEMU AArch64 validation

`scripts/qemu-aarch64 run --report /absolute/external/report.json` builds a fresh release, verifies its checksum, extracts a read-only system beside separate writable data, and runs the packaged launcher, broker, package, bootstrap, and recovery journeys. It is host/QEMU contract evidence only, not physical TG4040 evidence.

The run checks shell syntax, JSON validity, static ELF properties, protected writable-data sentinels, expected executables, A/B update safety, and cleanup of temporary release material. The caller-selected report is retained outside the worktree.
