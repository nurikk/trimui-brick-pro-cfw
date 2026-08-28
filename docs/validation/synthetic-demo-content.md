# Synthetic demo-content journey

The clean-room journey exercises four distinct project-authored identities: two stable-platform catalog demos (`Nebula Notes` on `nes` and `Mirror Museum` on `ps1`) and two separately signed PortMaster demos (`Orbit Garden` and `Signal Workshop`). Platform content is inert synthetic text and uses only simulator-owned `generated-libretro`/`generated-core` adapter metadata; it makes no ROM, BIOS, core, or hardware compatibility claim.

The PortMaster entries are browsed as separate `portmaster` catalog items, installed from signed repository metadata, launched through their private runtimes and immutable entrypoints, resumed, and independently identified. The journey uninstalls `orbit-garden` and verifies that synthetic saves, state, settings, and all four content files remain protected.

Run the broker package journeys with:

```sh
cargo run --locked --release -p session-broker -- simulate --journeys portmaster,portmaster-success,portmaster-rejection,portmaster-mismatch,portmaster-symlink,portmaster-injection,portmaster-nonzero
```

The bounded output validates typed requests, package trust and private entrypoint handling, session completion, persistence, and fail-closed rejection cases without exposing paths or payloads.

For headed 1024x768 X11 evidence, run:

```sh
./scripts/demo-content-x11
```

The script drives all four visible catalog entries, captures three changing PNG frames after semantic controls for each launched identity, checks dimensions and content differences, and verifies clean shutdown. All writes stay in the temporary caller-owned run directory.

No ROMs, BIOS files, third-party payloads, private signing keys, device paths, or release binaries are used or claimed.
