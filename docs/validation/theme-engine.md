# Theme engine validation

Run from the repository root. Evidence is temporary and belongs under `/tmp`.
This checkout does not provide a Cargo executable, so Rust commands are
currently unsupported here.

```sh
cargo fmt --check
cargo clippy --locked -p launcher-theme -- -D warnings
cargo test --locked -p launcher-theme
cargo build --locked -p launcher-theme --release
```

The focused test boundary covers v1 fallback, native v2 PNG validation, XML
import and reload, catalog locator and remote-package loading, renderer PNG
dimensions, bounded traversal/XML work, and script/traversal rejection. The importer subset is deliberately small: UTF-8
XML, literal `formatVersion="4"`, `theme`, `view`, `image`, `text`, and
`textlist`; scalar `name`, `path`, `pos`, `size`, `color`, `fontSize`, and
`text`; integer pixel geometry; `#RRGGBB`; relative PNG assets. Unknown
 elements/properties, includes, entities, variables, scripts, commands, URLs,
archives, duplicate components/properties, symlinks, unsafe paths, unsupported
encodings, corrupt images, and resource exhaustion must fail.

`ThemesCatalog` also accepts the documented Batocera `themes.json` envelope
(`data` records with `theme`, `author`, `theme_url`, `last_update`,
`up_to_date`, `size`, and `screenshot`) plus the local fixture form. It validates metadata and locators. `DirectCatalogTransport` performs bounded
HTTPS retrieval with curl and maps GitHub repository locators to raw `main`
branch files; `ThemeGarden` remote selection fetches native `theme.json` and
its declared PNGs through this path. Local selection is bounded to a
caller-provided fixture root.

```sh
EVIDENCE_DIR="$(mktemp -d /tmp/launcher-theme.XXXXXX)"
cargo run --locked -p launcher-theme --release -- demo --output "$EVIDENCE_DIR"
cargo run --locked -p launcher-theme --release -- import \
  --theme fixtures/theme-import/owned-a --output "$EVIDENCE_DIR/owned-a"
cargo run --locked -p launcher-theme --release -- catalog \
  --catalog fixtures/theme-import/themes.json
```

The demo renders `themes/default` plus two distinct project-authored imported
fixtures. `preview` is the safe fallback path; `validate` and `import` are
strict. Inspect PNGs as 1024x768 images. The native renderer uses the validated
component scene and decoded PNG assets at the host presentation boundary; it
does not execute theme content or silently turn unsupported XML into diagnostic
rectangles.

Theme Garden validates native v2 and imported output with the same loader for
preview, activation, and fallback. Existing package lifecycle remains atomic.
Host-native screenshots are simulator evidence only: they do not establish
physical display, GPU, performance, input, board, device, or TG4040 HIL
behavior. No universal EmulationStation, Batocera, or KNULLI compatibility is
claimed.
