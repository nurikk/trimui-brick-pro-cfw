# Storage migrations

Storage migrations are ordered, idempotent, copy-on-write, checksum-verified transactions. A descriptor has an explicit format and schema version and an ordered `from.dataVersion`/`to.dataVersion` pair. Descriptors are data-only: they must not name or target `/roms` (or `Roms`) and may not contain device, block, mount, or eMMC operations.

## Activation protocol

1. Validate the v1 closed layout object, filesystem limits, path names, and capabilities.
2. Prove that the previous release can read the new representation. If that proof is absent, activation is blocked before any mutation.
3. Write a `prepared` journal record under `/data/meta/migrations`, sync the record and its parent directory, and retain the source.
4. Copy into a staging path, sync each file, verify its checksum, then atomically promote the staged form. Keep both generation and checksum records; rename alone is not a sufficient removable-media durability proof.
5. Write the new layout generation and a `committed` journal record only after verification. A restart replays a prepared transaction or completes a committed layout update without repeating unsafe work.
6. Do not delete source data until the old and new releases both read the new form. Update and rollback never delete or mutate `roms`, saves, or states.

The synthetic descriptor in [`migrations/storage/storage-v1-to-v2.json`](../../migrations/storage/storage-v1-to-v2.json) copies a generated configuration placeholder to a distinct data-only target while retaining the original source. This exercises the journal, checksum, copy-on-write, restart, and rollback rules without carrying private or user content. `simulate-migrate --interrupt-after-journal` deterministically stops after the prepared generation record; rerunning the normal command safely resumes it.

## Rollback

Rollback is allowed only from a checksum-verified committed migration whose `priorReleaseReadable` proof is true. It changes the active data version and completed-migration list back to the ordered `from` version, retains the data and journal, and records `rolledBack`. The retained representation is consumable by the prior release because the descriptor explicitly proves readability. Journal records carry both a generation and checksum-verification state. A missing proof, missing journal, checksum mismatch, or incomplete transaction blocks rollback/activation rather than guessing.

## FAT/exFAT boundary

Removable FAT/exFAT storage is case-insensitive and has no contract-level POSIX ownership, modes, symlinks, or atomic-permission semantics. Validate case collisions and Windows-forbidden names before mutation. FAT32 limits a single file to 4 GiB minus one byte. File and parent-directory sync are required by this simulator when declared available. The simulator declares these capabilities in the layout and fails closed when the required migration durability capabilities are unavailable.

This is host/static-AArch64 compiler and ISA evidence only. The digest-pinned build image does not claim a TG4040 stock ABI, dynamic loader, sysroot, runtime, mount behavior, filesystem power-loss behavior, or hardware compatibility. No device, eMMC, block device, private corpus, ROM, or BIOS bytes are accessed.
