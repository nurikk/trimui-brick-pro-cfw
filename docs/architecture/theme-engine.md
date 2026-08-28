# Declarative theme engine

`launcher-theme` is a data-only Rust boundary. Native `theme-v2` is the current
contract and `theme-v1` remains readable for existing packages. Both contracts
use a fixed 1024x768 (4:3) canvas, semantic colors, bounded visual components,
and the bundled Lato Regular font under its OFL-1.1 license.

## Native v2

`schemas/theme-v2.schema.json` defines typography (`project-sans`), local PNG
assets, `image`/`text`/`textlist` components, semantic colors, layout regions,
and fallback behavior. Assets are loaded only from the selected theme
directory after path, file, size, image dimension, and decoded-color checks.
There is no plugin or execution mechanism. v1 generated resource references
remain compatibility-only and do not imply support for arbitrary fonts,
artwork, audio, URLs, archives, scripts, or commands.

## ES compatibility subset

`launcher-theme import --theme DIR --output DIR` accepts UTF-8 `theme.xml` with
root `<theme formatVersion="4">`, one or more `<view>` elements, and
`image`, `text`, and `textlist` children. Supported properties are `name`,
`path`, `pos`, `size`, `color`, `fontSize`, and `text`, either as attributes or
scalar child properties. Positions and sizes are two integer pixels; colors
are `#RRGGBB`; text is capped at 256 bytes. Image paths are normalized relative
POSIX paths and only local PNG files are supported.

Includes, entities, variables, custom components, scripts, commands, URLs,
archives, unknown elements/properties, duplicate components/properties,
symlinks, traversal/absolute/backslash paths, non-UTF-8 XML, and unsupported
encodings are rejected. The command writes a reloadable native-v2 directory: `theme.json`, copied PNG
assets, and `compatibility-report.json`. The report lists accepted and rejected
subset entries and is deterministic for the same input. This is an explicitly
bounded clean-room subset, not universal EmulationStation, Batocera, or KNULLI
compatibility.

## Limits and fallback

Theme JSON/XML is capped at 128 KiB; traversal stops at 32 entries per
level, 32 total files, and depth 8. XML parsing stops at 128 nodes, depth 16,
16 attributes per tag, and 256 bytes of text per node. A theme has at most 64
components. Each declared PNG is capped at 4 MiB and 4,194,304 pixels with
bounded decoded RGBA data. Unknown files, invalid dimensions/color formats,
corrupt assets, missing assets, and unsafe paths fail before rendering.
`preview` falls back to built-in Artbook; `validate` and `import` are strict.
Theme Garden uses the same loader for preview, activation, and fallback.

Checked-in visual fixtures are project-authored. The only bundled third-party
visual dependency is Lato Regular, distributed with its OFL-1.1 license in
`crates/host-platform/assets/fonts/Lato-OFL-1.1.txt`. No third-party themes,
logos, screenshots, or renderer code are shipped.
