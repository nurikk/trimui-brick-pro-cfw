# Clean-room emulator catalog

`emulator-catalog-v1` is a closed, declarative metadata boundary for the TG4040. It is not a launcher, broker, downloader, emulator implementation, firmware script, ROM index, or BIOS store. The catalog contains no executable, BIOS, ROM, artwork, or copied launcher/configuration payload.

## Authoring

Each JSON document in `catalog/systems`, `catalog/runners`, `catalog/cores`, `catalog/profiles`, and `catalog/channels` carries the schema URL, format, version, kind, and an explicit channel. Rust parsing uses `serde(deny_unknown_fields)` and validation rejects malformed IDs, versions, hashes, URLs, logical paths, duplicate IDs, duplicate normalized extensions, dangling references, unsupported targets, and unpinned artifacts. The JSON schema conditionals enforce stable hash-bearing BIOS records and experimental candidate/hash-free BIOS shapes per document; cross-document runner/channel/core/path relationships remain runtime-only checks.

Extensions are stored as lowercase, dotless, case-folded values (`gb`, not `.GB`). Paths are logical rooted paths only: `roms`, `bios`, `data/saves`, or `data/states`; they are never host paths. Save and state roots are part of the system metadata. The stable core-pack permits shared `zip` only under its explicit typed-system routing policy; extension-only routing and every other normalized cross-system collision are rejected.

`Runner` and `Core` entries carry exact versions, target architecture, support scope, capabilities, license/provenance URL, channel, and an artifact identifier plus lowercase SHA-256 pin. The pilot artifact identifiers and digests are synthetic contract metadata; no corresponding binary is included or claimed.

## Channels and promotion

`catalog/channels/stable.json` and `experimental.json` contain disjoint IDs and versions. A stable selection requires a pinned runner artifact, provenance URL, TG4040 synthetic smoke evidence ID, complete BIOS requirement records, and an empty runtime-requirement list. A stable-only resolution cannot select an experimental runner, core, system, profile, or extension. Experimental entries remain isolated until independently reviewed and promoted by changing their catalog metadata and channel membership. The TG4040 experimental lane currently contains four separately pinned, metadata-only candidates: N64/Mupen64Plus-Next, Dreamcast/Flycast, PSP/PPSSPP, and Nintendo DS/melonDS DS. Their source refs and license URLs are public provenance pointers only; no source, executable, package, BIOS, ROM, or artifact hash is included.

`catalog/core-packs/stable.json` is a separate stable-pack contract for the public 8/16-bit, Neo Geo/arcade, and PS1 planning scope. It is intentionally blocked: every package, runner, and core identity is exact and target-pinned, but manifest/artifact hashes are null until separately sourced, licensed, signed, and approved. It is not an installable selection and has no upstream package mapping.

The smoke evidence ID in this pilot is **host/static contract evidence only, not hardware evidence**. No physical TG4040 smoke test is represented. Each experimental profile uses a 1024x768 baseline, native/1x internal-rendering intent, no post-processing, zero frame skip, and disabled speedhack. Per-game deltas are additive, reversible, explicitly unverified, and keyed only by opaque `contentId`; they reference the exact experimental system, runner, and core pins.

Experimental resolution requires both the experimental channel and `experimentalOptIn: true`. Missing content-free BIOS requirements return `bios_missing`; any requested renderer is rejected as `unsupported_renderer` because renderer and device capability behavior are unverified. There is no fallback to stable metadata. Public-source, host/static, simulator, and AArch64/QEMU evidence may document contract or userspace behavior only; TG4040 performance, PowerVR, renderer, controls, thermal, timing, compatibility, save/resume, and runtime success remain unverified. No candidate is known-good without physical TG4040 evidence. `package-manager` continues to reject every core package before extraction or activation, including experimental attempts; stable core IDs, logical paths, package records, and protected data are not replacement targets.

## Resolution

`resolve --root <catalog-root> --case <fixture>` resolves only typed, known settings. It applies deltas in exactly this order:

`device -> system -> core -> folder ancestors (root-to-leaf) -> game -> session`

The output includes every effective setting and its winning layer. The resolver rejects unknown fields, path escapes, case-colliding folder ancestors, unavailable runner/core/profile/extension selections, unsupported capabilities, and display settings beyond device limits. It does not accept shell commands, executable paths, or runtime directives from a fixture.

## BIOS audit and privacy boundary

`bios-audit` (also accepted as `audit`) takes `--catalog <catalog-root> --bios-root <fixture-filesystem-root> --channel stable|experimental` (`audit` also accepts the legacy `--root` spelling). The BIOS root is supplied separately from the catalog directory. Candidate locations remain logical `bios/...` paths. The audit streams each candidate through SHA-256 and emits only requirement IDs, present/missing/mismatch counts, and status. It never copies, uploads, indexes, prints, persists, or logs candidate filenames, host paths, discovered hashes, or bytes. A present BIOS with no approved hash is `unverified` and is a nonzero, fail-closed CLI outcome; it is not launchable. The repository fixtures intentionally contain no BIOS payload or private corpus. Experimental BIOS records are requirement IDs and logical locations only; they carry no BIOS hash and an explicit `required-unverified` status.

## Provenance

A stable entry needs an independently reviewable license/provenance URL and evidence for the modeled fact. When evidence is insufficient, omit the entry or keep it experimental. This pilot uses clean-room, synthetic metadata and public documentation links as provenance pointers; it does not translate third-party firmware data. Host builds and fixture journeys demonstrate parser and contract behavior only and must not be described as device/runtime validation.

## Fixture evidence

Run `emulator-catalog schema-validation-journey` for strict positive-document and unknown-field checks, `emulator-catalog journey` for the deterministic precedence and fail-closed negative journeys, and `emulator-catalog core-pack-journey` for the generated synthetic blocked-pack and BIOS-boundary journey. The tracked cases cover stable/experimental isolation, extension collision, missing BIOS, invalid runner/core versions, capability and device-limit rejection, every precedence layer, path escape, unknown fields, TG3040 rejection, unpinned artifacts, and channel leaks.
