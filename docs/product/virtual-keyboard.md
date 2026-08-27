# Controller-first virtual keyboard

`virtual-keyboard` is a pure Rust state machine with a render-neutral scene model for the 1024×768 launcher surface. It is intentionally not wired into settings, networking, persistence, the HAL, or the launcher.

## Public behavior

`FieldPolicy` supplies the field kind (`Text`, `Secret`, or `Numeric`), allowed character classes, initial value, placeholder, validation state, and both UTF-8 byte and Unicode-scalar limits. `Keyboard::press` accepts controller buttons and returns `InputResult`; `Start` returns a typed `Text`, `Secret`, or `Numeric` value, while `Secondary` and `Menu` restore the original value and return `Cancelled`.

D-pad buttons move focus. `Primary` activates the focused cell. `LeftShoulder` and `RightShoulder` move the cursor across Unicode scalar boundaries. `WrapMode` explicitly controls horizontal and vertical edge wrapping. The scene exposes large hit cells, a visible focus marker, a help strip, and semantic token roles (`focus`, `error`, and so on) so meaning does not depend on color.

## Layouts

`key_grid` deterministically generates IDs and cells for lowercase QWERTY, one-shot uppercase shift, uppercase caps, symbols, a numeric keypad, and a constrained URL-safe character set. The keyboard also provides backspace, delete, clear, space where allowed, and layout-switch cells.

## Redaction and bounds

Secret working values never enter scenes, semantic snapshots, semantic events, or `Debug` output. Secret scenes expose only `*` masks, scalar length, cursor position, layout, and focus. The typed secret is available only in the `TypedValue::Secret` returned directly to the caller on confirmation. Invalid characters and byte/scalar overflows are rejected without mutation; numeric confirmation requires an unsigned integer.

The committed JSON fixtures are semantic 1024×768 scenes and controller-journey evidence. They contain no secret field value. No clipboard, shell, filesystem, network, or credential persistence behavior exists in this component.
