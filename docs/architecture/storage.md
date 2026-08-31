# Logical storage contract

This is a clean-room, no-device contract for TG4040 CFW storage. The hardware document remains authoritative: physical mount points, filesystem behavior, persistence, ABI, and loader behavior are unknown. This contract therefore never probes, mounts, opens, writes, partitions, formats, or flashes eMMC or block devices.

## Three logical boundaries

- **`/system`** is the immutable active release, mounted read-only. A release is selected from SD-card `.brickpro/system/slots/A` or `.brickpro/system/slots/B`; the installer may prepare an inactive slot but this contract does not implement installation.
- **`/data`** is CFW-owned persistent state. Mutable CFW writes are relative to a caller-provided fixture root in the simulator, never to a guessed device path.
- **`/roms`** is user-owned ROM and BIOS content. It is discovery-read-only. Installer, update, rollback, migration, and cache eviction never own, mutate, delete, or copy it.

The SD-card mapping is `.brickpro/system/slots/{A,B}`, `.brickpro/data`, and `Roms`. The mapping is logical documentation only; no executable here mounts or inspects an SD card or eMMC. The generated fixture uses a lowercase `roms` directory as a host-only logical stand-in.

## `/data` classes

| Subtree | Owner and class | Contract |
| --- | --- | --- |
| `saves` | CFW-owned **durable** | User progress; never removed by update, rollback, migration, or cache eviction. |
| `states` | CFW-owned **durable** | Emulator state; never removed by update, rollback, migration, or cache eviction. |
| `config` | CFW-owned **durable** | Typed CFW configuration. |
| `credentials` | CFW-owned **secret/private** | Private values; removable FAT/exFAT is not protected at rest. |
| `activity` | CFW-owned **immutable** records | Append-oriented activity records for a release; no recovery guarantee is inferred from unknown hardware behavior. |
| `calibration` | CFW-owned **durable** | Device calibration values, once a future device contract proves them. |
| `cache` | CFW-owned **disposable** | Rebuildable data; eviction must not touch user-owned trees. |
| `logs` | CFW-owned **disposable** | Bounded diagnostic output; never ROM/BIOS bytes, names, paths, or hashes. |
| `themes` | CFW-owned **immutable** release inputs | Release-owned theme data; replacing a release replaces this class. |
| `index` | CFW-owned **disposable** | Rebuildable discovery/index data. |
| `update` | CFW-owned **immutable** transaction metadata | Journal and generation metadata only; incomplete work is not activation. |
| `meta` | CFW-owned **immutable** layout and migration metadata | Contains `layout.json` and migration journals. |

`immutable` means the active release does not modify the class during normal use; `durable` means it must survive an update/rollback; `disposable` may be rebuilt; `secret` identifies confidentiality-sensitive values; and `user-owned` means the CFW must not own or mutate it.

## Removable-filesystem rules

The declared filesystem capabilities are part of `data/meta/layout.json`. Validation fails closed for unknown or unsupported behavior. FAT/exFAT handling must:

- check case-insensitive collisions before activation;
- reject Windows-forbidden names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, and `LPT1`–`LPT9`, including extensions and trailing dots/spaces);
- rely on no POSIX ownership, modes, symlinks, hard links, or permission semantics;
- enforce the FAT32 maximum file size of 4 GiB minus one byte;
- sync each file and its parent directory where the filesystem provides those operations;
- retain both generation and checksum records because rename alone is not sufficiently durable on removable media.

Writes are copy-on-write, checksum-verified, and journaled below `/data/meta/migrations`. A migration cannot activate unless the previous release is proven able to read the new form. Source data is not deleted until both releases can read it.

## First-run simulator contract

`storage-layout simulate-onboard` consumes a caller-owned, generated inventory and creates only the approved logical trees. It fails closed before mutation for an unsupported partition table or FAT32/exFAT declaration, less than 32 GiB capacity, invalid free space, read-only media, suspected counterfeit media, or a dirty filesystem. Dirty media produces a visible read-only recovery diagnostic; it never attempts repair or writes a recovery marker.

A two-card request stores SD2's stable UUID in SD1's `data/meta/layout.json`. If that UUID is absent or its fixture root is unavailable, onboarding returns recovery without creating `roms` on SD1. In a healthy two-card setup, `roms/BIOS` is created only on SD2; in a healthy one-card setup it is created on SD1. ROMs and BIOS remain user supplied and are never bundled or copied.

Formatting is simulation-only: `--format-device UUID --confirm-device UUID --confirm-format` must name the exact SD1 UUID twice and the inventory must report a remount. It then writes, syncs, reads, and CRC32-verifies a bounded fixture file before removing it. This models the confirmation and verification protocol without partitioning or formatting a block device. The inventory also models USB export: active users must be quiesced, local unmount and eject acknowledgement are required, and the resulting mode is MTP rather than unsafe simultaneous mounted mass storage. A bundle supplies only redistributable runtime, configuration, and theme files to fixed CFW-owned targets.

## Evidence boundary

The simulator consumes only generated fixture data. Its output is bounded and sanitized: it does not print per-ROM paths, names, contents, or hashes. Physical FAT/exFAT power-loss evidence, eMMC behavior, stock ABI/loader/runtime behavior, and hardware compatibility remain deferred; this is not a TG4040 device claim.
