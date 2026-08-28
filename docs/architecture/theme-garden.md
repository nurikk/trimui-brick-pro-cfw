# Theme Garden boundary

`theme-garden` is a no-device Rust catalog and lifecycle fixture for the
TrimUI Brick Pro `TG4040`. Its controller-first journey exposes Browse,
Details, Preview, Installed, and Updates state. Theme data is never executed.

The `launcher-theme::ThemesCatalog` adapter accepts the project
`themes-catalog-v1` shape and the documented Batocera feed shape (`data` with
`theme`, `author`, `theme_url`, `last_update`, `up_to_date`, `size`, and
`screenshot`). IDs, three-part versions, HTTPS/GitHub locators, and local
`fixture:<id>` locators are bounded and validated. `DirectCatalogTransport`
performs bounded HTTPS GETs with curl (no shell and no downloaded-content
execution); GitHub repository locators resolve to the repository's `main`
branch raw files. A remote selection fetches `theme.json` and each declared
PNG, validates them through `launcher-theme`, and a local selection resolves a
caller-provided fixture root through the same loader. `CatalogTransport`
remains available for deterministic tests and alternate callers. Unknown fields, duplicate
IDs, unsafe locators, invalid versions, and unsafe screenshot paths reject.

`fixtures/theme-garden/themes.json` and `fixtures/theme-import/themes.json`
are project-authored catalog fixtures. They contain no third-party theme
content. The XML adapter supports only the explicit data subset documented in
`docs/architecture/theme-engine.md`.

Existing package lifecycle behavior remains atomic: candidates are validated
and previewed before activation, failed successors leave the active theme
alone, and removal falls back to built-in Artbook. The logical cache is
`/data/cache/theme-garden`; interrupted staging is
`/data/staging/themes/<id>/<version>.partial`. Journeys map both beneath a
caller-provided temporary root and preserve ROM/save/state/settings fixture
bytes.

## Validation lane

```sh
cargo run --locked --release -p theme-garden -- journey \
  --fixtures fixtures/theme-garden --root /tmp/theme-garden-root
```

This is synthetic evidence only: no hardware, package publication, live
network, external catalog corpus, or physical-device claim is involved. Remote
selection is limited to native `theme.json` packages rooted at an HTTPS URL or
GitHub repository; archive extraction and branch discovery are intentionally not
part of this contract.
