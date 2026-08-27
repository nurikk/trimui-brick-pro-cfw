# Brick Pro package trust model

This clean-room package foundation is a TG4040-only, host-fixture/static-AArch64 contract. It has no device, boot, launcher, vendor, ROM, BIOS, or PortMaster corpus access.

## Tiers

- **Built-in** is repository-owned release content. It is immutable, has no arbitrary runtime, and is never installed from an untrusted source.
- **Verified** is package content authorized by the public bootstrap root, the TUF top-level roles, and an allowed scoped delegation. It may be activated only after metadata, manifest, capability, size, path, and content checks.
- **Community** is declarative signed content by default. It may not contain an executable, script, shell hook, argv, symlink, absolute path, traversal, or privileged capability. Themes are JSON data only and are never executed.
- **Developer** requires explicit local enablement and a locally trusted key, is non-root only, and uses the same package-private namespace. It is never presented as Verified.

The public bootstrap root is the only trust anchor represented here. Private signing material is out of tree and is not required at runtime or committed in fixtures. The committed fixture contains public keys and signatures generated from temporary material that was not stored.

## TUF policy

`package-trust` is a narrow adapter over released `sigstore-tuf = 0.11.0` with default network features disabled. Its shipped documentation and source identify a transport-free `TrustedMetadataSet` and implement root chaining, signature thresholds, expiry, anti-rollback, length/hash pinning, and delegated-target discovery with ordered path-scoped roles. The adapter additionally requires TUF 1.x consistent snapshots, exactly the four top-level roles, SHA-256 target pins, bounded clock uncertainty, and persisted highest-seen metadata versions.

Root rotations are sequential: each candidate must be exactly the next version and satisfy both the old and new root thresholds. Delegated metadata is verified against the delegator's keys and threshold; the package role in the fixture is scoped to `packages/*/*.json`. Timestamp, snapshot, root, and targets expiry, signature, freeze/integrity, rollback, corrupt trusted state, and clock uncertainty produce explicit recovery statuses before a target is accepted. A corrupt or incomplete trusted state is not repaired automatically. Recovery is to retain the public bootstrap root, discard untrusted non-root metadata, and retry only with a fresh valid chain; no package bytes enter staging on failure.

The signed target bytes are verified inside the same operation, before the advanced metadata state is atomically published. State publication uses a collision-safe same-directory file, file sync, atomic rename, and parent-directory sync; injected publication failure leaves the prior state byte-for-byte unchanged. This adapter does not implement network transport, key generation, signing, or private-key recovery. Those are intentionally unsupported rather than hand-rolled. The current fixture runner supplies already-fetched metadata bytes and exercises the offline verification path.

## Package boundary

Manifests are versioned and deny unknown fields. They name exact SHA-256/length records, typed capabilities, private runtime dependencies, declarative entrypoints (with the sole verified PortMaster launch entrypoint constrained to a private immutable script), license/provenance, and the SPDX SBOM reference. Install validates everything before creating `.brickpro/packages/.staging`; it copies only regular files into a normalized private namespace, rechecks every length/hash, rejects runnable/binary content, and promotes only a complete version to an activation record. It never executes a package or invokes a shell.

Uninstall reads only a validated package-owned activation record and removes only that package's immutable/runtime/cache/staging namespace. `/roms`, `/data/saves`, and `/data/states` are protected user/durable data and are never copied, updated, rolled back, cached, or deleted. Invalid manifests, invalid activation records, and interrupted transactions fail closed without touching those paths. There is no arbitrary root shell, privileged capability, host path, mutable system path, or runtime outside the package-private namespace.

## Evidence and limits

Run the no-device evidence harness with:

```sh
docker build --network=default -f containers/package-trust/Dockerfile -t brickpro-package-trust .
docker run --rm --network=none brickpro-package-trust
```

It demonstrates signed delegated progression, target integrity and retry, capability/path/case-collision/symlink rejection, unsigned/expired/rollback/freeze/clock-uncertainty/corrupt-state rejection, install/uninstall, interrupted transactions, state-publication safety, and byte preservation of generated protected fixtures. It is not evidence about physical TG4040 behavior, ABI, PowerVR, input, radio, suspend, thermal, RAM, boot, recovery, or vendor services.
