# Launcher localization and readable-text contract

The launcher renders localized strings as text. It never bakes labels into theme images. The host renderer uses bundled Lato for Latin/Cyrillic, Droid CJK for Japanese/Chinese, and Nanum Barun Gothic for Korean glyphs. Text wraps by Unicode scalar and ellipsizes only after the available layout height is exhausted.

Text-size presets reuse the density-aware layout: `large` is 125% and `extra-large` is 150%. Lists retain a focused, visible controller item instead of fixing a row count or pixel coordinate.

Theme validation declares a 4.5:1 minimum for text/highlight labels against the background, requires the declared v2 assets, validates canvas/device resolution, and reserves ordered clock/Wi-Fi/battery space. A missing asset, incompatible resolution, invalid contrast, or invalid status layout selects the built-in safe Art Book theme and presents the recovery route.

`corpus/launcher-locales.json` is exercised by `launcher-presentation-journey` at 125% and 150%, including pseudolocalization and RU/JP/KR/ZH text. Glyph fallback is coverage-oriented only: SDL_ttf does not provide full complex-script or bidirectional shaping, so this launcher makes no claim of contextual shaping beyond the packaged precomposed glyphs.
