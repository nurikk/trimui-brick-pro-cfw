# Declarative theme engine

`launcher-theme` is an independent Rust crate. It has no dependency on the
launcher, platform adapters, simulator, SDL, or hardware APIs. A theme is a
strict `theme-v1` JSON document; it is data, not an extension mechanism.

## Model and boundary

The model contains project-authored MIT metadata, a fixed 1024x768 4:3 canvas,
semantic color tokens, four typed resource references, one bounded layout
preset, nine semantic regions, user settings, and a safe fallback policy.
`schemas/theme-v1.schema.json` describes the wire format. Rust structs use
`deny_unknown_fields`, and parsing performs a separate recursive duplicate-key
check because ordinary JSON object deserialization may otherwise keep only the
last duplicate value.

Resources are references only. The accepted references are the generated
controller mark, generated grid or flat field, a built-in generated sans
placeholder, and built-in silence. No resource bytes are loaded or executed.
The renderer creates neutral geometry from synthetic metadata and does not
render logos, screenshots, box art, fonts, or downloaded content.

The immutable `ValidatedTheme` is produced only after all checks pass. Public
operations are:

- `load_theme_dir` validates a directory containing only `theme.json`;
- `preview_path_or_fallback` returns a validated theme or the built-in Artbook
  theme with a stable `Reason`;
- `scene` returns semantic composition data;
- `render_png` emits deterministic 1024x768 RGBA PNG output.

No activation state, launcher state, or user data is persisted.

## Budgets and rejection

The parser rejects input before rendering or arbitrary image decoding:

| Item | Limit |
| --- | ---: |
| theme JSON | 131072 bytes |
| files in a theme directory | 32 (only `theme.json` is supported) |
| declared resource bytes | 65536 bytes total and per resource |
| rendered RGBA surface | 3145728 bytes |
| layout regions | 16 |
| visible games | 12 |
| text fields | 32 bytes for names/references, 64 for authors |
| font scale | 80–160 percent |

Absolute, current-directory, parent, backslash, NUL, symlink, archive/media,
unknown-file, URL-like, script-like, command-like, and unsupported resource
inputs are rejected. Region bounds, color tokens, enums, schema identity, and
metadata are checked before a `ValidatedTheme` exists. Errors contain the
stable machine-readable `Reason` enum and a human-readable message.

## Fallback

`preview_path_or_fallback` never returns a partial candidate. Missing,
malformed, duplicate-key, unknown-field, unsafe-path, symlink, unsupported-file,
setting, layout, resource, and budget failures all select the built-in safe
Artbook theme and expose the corresponding reason. The fallback uses a
neutral generated splash policy. The `validate` command remains strict and
returns nonzero for invalid input; `preview` renders the fallback and returns
zero so an integration can safely preview an untrusted selection.

## Clean-room and provenance

This implementation and the three checked-in JSON documents are
project-authored under MIT. They contain source-only geometry and synthetic
text. No third-party theme, font, image, audio, logo, or web/API material is
used. The theme directories intentionally contain no binary assets and no
artifact record is added to the distribution inventory.
