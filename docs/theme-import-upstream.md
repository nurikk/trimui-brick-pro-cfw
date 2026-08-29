# Upstream themes

## Bundled offline default

The firmware bundles the complete Art Book Next (Batocera ES Edition) source at `themes/upstream/art-book-next-es/`.

- Source: https://github.com/anthonycaccese/art-book-next-es
- Pinned source commit: `9a50ef366e750aabfab29e6915a2867607212971`
- Author: Anthony Caccese
- License: CC-BY-NC-SA

The upstream `theme.xml`, `aspect-ratio-4-3.xml`, includes, images, SVGs, fonts, sounds, README, credits, and license are shipped unchanged. `config/platform/<device>/compatibility.json` selects the bundled theme and aspect; Brick Pro selects `art-book-next-es` and `4:3`. The runtime loads that profile-selected layout and actual upstream artwork. The only raster derivative is `themes/media/systems/genesis-wordmark.png`, generated from the bundled upstream Genesis SVG because the SDL renderer does not rasterize SVG.

Art Book Next is the non-removable offline fallback. `themes/default/` is only a project fixture and is not presented as Art Book Next.

## Optional Theme Garden sources

The catalog adapter consumes `https://batocera.org/upgrades/themes.json`. It preserves theme name, author, HTTPS source repository, update date, numeric size, upstream status, and a converted absolute screenshot URL. Aspect ratios and KNULLI compatibility remain unknown unless source data establishes them.

Theme Garden previews the catalog screenshot, then downloads bounded PNG source assets directly from the documented GitHub repository into temporary staging. It normalizes only those images needed by the supported data-only EmulationStation XML subset, validates and renders the local candidate, and atomically activates it under `/data/themes`. Interrupted or invalid candidates do not replace the active theme; the last catalog and previews remain cached offline.

The currently qualified direct-source profiles are:

- SimpleLife: https://github.com/DarrenCarol/Simple_Life at `b19995f3da751e71cb872610c4d40769c56754bf`; feed author DarrenCarol, source XML credits Mr. Overlay, no source license declaration found.
- Techdweeb: https://github.com/anthonycaccese/techdweeb-es at `4a27965ed279466c1b1d6f6e98ffc279e2d0f6d4`; TechDweeb / Anthony Caccese, CC-BY-NC-SA.

This is a deliberately bounded compatibility subset, not a claim that every Batocera theme is supported. Theme data is never executed. There is no signing, certificate, attestation, trust-tier, mirror, or repackaging framework.
