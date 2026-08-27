# Synthetic storage migrations

Descriptors in this directory use `brickpro-storage-migration` schema version 1 and ordered data-version pairs. `storage-v1-to-v2.json` is generated-only and copies a generated configuration placeholder with a SHA-256 check.

A descriptor is safe only when it is ordered, idempotent, copy-on-write, checksum-verified, and journaled under `/data/meta/migrations`. `priorReleaseReadable` must be proven before activation; otherwise the simulator blocks before mutation. Descriptors may never name or target `/roms`/`Roms`.

The matching generated descriptor is copied into `fixtures/storage/v1/data/meta/migrations/` so the CLI can operate after the fixture is copied to a temporary root. No private, device, ROM, or BIOS data is used.
