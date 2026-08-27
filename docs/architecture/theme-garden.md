# Theme Garden boundary

`theme-garden` is a clean-room, no-device Rust catalog and lifecycle fixture for the TrimUI Brick Pro `TG4040`. Its controller-first journey exposes Browse, Details, Preview, Installed, and Updates state; presentation code does not perform transport, archive, or trust work.

Catalog metadata is descriptive until `package-trust::TrustStore` verifies the delegated `themes` target and its exact catalog bytes. Package manifests and target bytes use the same delegated trust path before `package-manager::install_with_validation` atomically promotes a versioned package directory. `launcher-theme` remains the authoritative data-only Theme v1 parser and generated preview renderer.

The logical cache is `/data/cache/theme-garden` and interrupted acquisition staging is `/data/staging/themes/<id>/<version>.partial`. The journey always maps both beneath a caller-provided temporary synthetic root; it never opens or writes real `/data`. Verified catalog version and expiry, plus generated-neutral screenshots, are cached. Expiry permits inspection of installed themes but denies new install/update authorization.

Activation state is written only after package promotion and preview validation. Failed successors leave the prior active version intact. Removing an active non-default theme selects built-in `Artbook` first; `Artbook` is never removable. ROM, save, state, and settings fixture bytes are outside the package transaction and are checked byte-identically by the journey.

## Validation lane

Run the executable fixture journey with `cargo run --locked --release -p theme-garden -- journey --fixtures fixtures/theme-garden --root /tmp/theme-garden-root`. This is deterministic synthetic evidence only: no hardware, live networking, package publication, external catalog corpus, private signing material, or hardware claim is involved.
