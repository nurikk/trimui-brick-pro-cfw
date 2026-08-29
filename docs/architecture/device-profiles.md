# Device profiles

A selected `config/platform/<device>/compatibility.json` is the only device definition consumed by generic launcher configuration. It supplies `deviceId`, `targetSku`, framebuffer dimensions/format/stride, orientation, and `themeAspect`; `physicalPanel` may later supply `activeWidthMm`/`activeHeightMm` or `diagonalInches` for density scaling.

## Add a device

1. Add `config/platform/<device>/compatibility.json` with `schemaVersion: "1.0.0"`, a stable lowercase `deviceId`, a target SKU, and a valid display contract.
2. Add a HAL, boot, or storage adapter only where that hardware differs.
3. Select that compatibility file explicitly at build/runtime and construct `device_profile::DeviceProfile` from it.
4. Pass the parsed profile to display/theme/simulator consumers. Launcher/theme/settings/catalog code remains generic.

`themeAspect` is one of `4:3`, `16:9`, `3:2`, or `1:1` and must exactly match display dimensions. Theme selection turns it into `aspect-ratio-<ratio>.xml`; Brick Pro therefore selects `aspect-ratio-4-3.xml` from its device definition rather than a model branch.

The profile parser rejects invalid IDs, dimensions, stride, framebuffer format, orientation, aspect, and incomplete/non-positive physical panel values with configuration errors. Emulator/game framebuffer settings remain separate from launcher theme selection.
