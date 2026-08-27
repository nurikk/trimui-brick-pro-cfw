# Platform HAL boundary

UI and launcher code use the generic `Platform` trait from
`crates/platform-contract`. The contract has typed state and operations for
display, input, Hall calibration, power, battery, suspend, radios, audio,
LEDs, rumble, USB, and logical storage, plus capability discovery and typed
unsupported/unavailable errors.

`sim-host-platform` is a synthetic userspace test double. It supplies fixture
state only; it is not hardware evidence. `UnavailableTg4040Platform` is the
no-device TG4040-facing boundary. It has no backend, performs no probing or
raw I/O, reports every domain unavailable, and rejects every operation.
