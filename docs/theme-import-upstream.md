# Art Book Next offline default

The firmware bundles the complete upstream Art Book Next repository at:
`themes/upstream/art-book-next-es/`.

- Source: <https://github.com/anthonycaccese/art-book-next-es>
- Pinned commit: `9a50ef366e750aabfab29e6915a2867607212971`
- Author: Anthony Caccese
- License: CC-BY-NC-SA

`theme.xml` and its real `aspect-ratio-4-3.xml`, variables/includes, images,
SVGs, fonts, sounds, README, and license/credit files are shipped unchanged.
The Brick Pro compatibility config selects `art-book-next-es` and `4:3`; the
runtime loads those values through `config/platform/<device>/compatibility.json`.

The launcher implements the bounded renderer seam needed by this product:
selected XML includes/variables, the 4:3 system artwork and game-artwork paths,
image/text/textlist layout roles, metadata visibility, and menu/status
components. Unsupported EmulationStation features such as video playback,
animations, SVG rasterization, and non-4:3 aspect variants are reported as a
renderer limitation; the required 4:3 routes still use the bundled PNG assets
and layout.

The Art Book renderer's one raster derivative is `themes/media/systems/genesis-wordmark.png`,
created from the unchanged upstream `_inc/systems/logos/genesis.svg` with
`gtk-encode-symbolic-svg` because SDL has no SVG renderer. It is loaded as the
`./_inc/systems/logos/genesis.png` asset; the upstream directory remains untouched.

## Bounded upstream Theme Garden imports

The two real feed entries below are kept as small data-only source slices under
`themes/imported/`. Their PNGs are bounded RGB/RGBA derivatives of the named
upstream files; the XML keeps the source identity and the importer uses only the
existing image/text/textlist subset.

- `SimpleLife`: <https://github.com/DarrenCarol/Simple_Life>@`b19995f3da751e71cb872610c4d40769c56754bf`, feed author DarrenCarol; the source XML identifies Mr. Overlay and declares no license.
- `Techdweeb`: <https://github.com/anthonycaccese/techdweeb-es>@`4a27965ed279466c1b1d6f6e98ffc279e2d0f6d4`, source notice credits TechDweeb and XML by Anthony Caccese, license CC-BY-NC-SA.

The public source feed is <https://batocera.org/upgrades/themes.json>.
Theme Garden cycles the existing Luma preview, then these two imported themes,
then returns to Art Book Next; the default remains non-removable.
