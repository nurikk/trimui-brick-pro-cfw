# PortMaster integration contract

This is a clean-room, no-device contract backed only by project-authored synthetic fixtures. It does not claim official PortMaster support, upstream compatibility certification, TG4040 hardware validation, or deployment.

## Lifecycle

The launch catalog approves the fixed `generated-portmaster` runner at version `1.0.0`. A PortMaster request carries only a typed package identity and version. The package manager accepts that package only after signed-target verification, manifest identity/version checks, normalized-path and symlink checks, and the PortMaster capability gate. Install and update publish one activation record transactionally; a rejected or interrupted successor leaves the previous activation selected. After a successor is published, cleanup errors never repoint activation to the previous version: the verified successor remains active and the cleanup error is surfaced for recovery. A completed update removes the previous package version. Remove validates the activation record and deletes only that package's private `.brickpro/packages/<id>/<version>` data.

The broker resolves the activation record immediately before launch. Only its immutable `immutable/port/launch.sh` entrypoint and matching private `runtime`/`runtime/lib` roots reach the child. Runtime roots are exposed through the broker's explicit child environment; no global linker state, `/system` mutation, caller command, shell text, URL, runtime path, environment field, or network permission is accepted.

ROMs and user saves/states remain outside package-manager lifecycle ownership. The repository's CFW settings vocabulary is `/data/settings.json`; the compatibility Save Vault vocabulary is `.brickpro/save-vault/`. Both are protected fixture boundaries and are not package manifest destinations or removal targets. No package operation installs, updates, caches, rolls back, or removes either boundary.

## Synthetic broker evidence

`portmaster` installs and removes `generated-portmaster` independently. `portmaster-success` installs and removes the distinct `generated-portmaster-success` package independently. Both use the catalog-owned runner and private runtime projection. Rejection journeys cover catalog/schema, package identity/version, symlink, request-shape, path, hash, and network-capability failures before broker launch.

Run the two deterministic journeys with:

```sh
cargo run --locked --release -p session-broker -- simulate --journeys portmaster,portmaster-success
```

These are host fixture results only. No real network, device, ROM/BIOS corpus, upstream checkout, private runtime, signing key, physical TG4040, or deployment lane is available or used.
