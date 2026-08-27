# Decision: native Rust launcher spike

## Verdict

**Conditionally retain the host/core Rust spike.** The host-native journey passed, but TG4040 viability and physical acceptance remain pending and unproven. No device was available or authorized for this work.

## What was exercised

The launcher uses the existing generic `Platform` boundary and the pinned host SDL2 software backend (`rust-sdl2 0.37`, simulator image `libsdl2-dev=2.26.5+dfsg-1`). The deterministic journey is exactly:

`Library → Systems → Games → typed launch-contract::LaunchRequest`

It renders a neutral 1024×768 logical frame, consumes semantic fixture controls, validates the generated strict launch catalog and request through `launch-contract`, and emits the request as `launch-request.json`. It does not spawn an emulator, invoke a shell, access device nodes, scan real paths, read ROM/BIOS/private content, scrape, use the network, or load vendor binaries.

The host lane is a test double only. SDL2 is not a vendor/device ABI compatibility claim.

## Budgets and measured host-only results

The executable journey runs two deterministic release executions with caller-owned evidence under `/tmp`, checks the first run against these budgets, and compares the semantic event results across both runs. Units are bytes, microseconds, and KiB.

| Metric | Budget | Observed | Result |
| --- | ---: | ---: | --- |
| fixture journey binary size | ≤ 8,388,608 bytes | 1,654,952 bytes | pass |
| cold process-to-clean-journey proxy | ≤ 2,000,000 µs | 31,666 µs | pass |
| first frame | ≤ 500,000 µs | 1,156 µs | pass |
| idle RSS | ≤ 131,072 KiB | 8,692 KiB | pass |
| catalog/list parse | ≤ 100,000 µs | 72 µs | pass |
| input-to-frame | ≤ 100,000 µs | 969 µs | pass |

These are one host-only observation in the pinned Rust 1.85.1 simulator environment, not TG4040 performance evidence. The public-reference comparison is not closed: no verified public reference values were available and no live/network research was authorized.

## Lanes and remaining evidence

- **Host-native userspace simulator:** passed the synthetic journey and strict request evidence above.
- **Static AArch64 compiler/ISA lane:** `aarch64-unknown-linux-musl` was preinstalled and the optimized pure `sim-launcher` library build passed. `file` reported the produced `.rlib` as an ar archive. This is compiler/ISA evidence, not device ABI evidence; no target was installed by this task.
- **Physical TG4040 hardware-in-loop:** unavailable and unproven. Kernel, ABI, SDL/EGL/GLES, framebuffer/DRM, evdev, vendor libraries, physical controls, power, storage, audio, LED, radio, suspend, thermal, and device-performance facts remain unknown.

Physical acceptance requires an approved HIL setup with the actual TG4040, observed runtime/ABI/display/input evidence, and separately defined device-performance measurements. Neither the host SDL lane nor the static-AArch64 lane can substitute for that prerequisite.

## Reproduction

```sh
cargo run --release --locked -p sim-launcher --bin launcher-fixture-journey
RUN=$(mktemp -d)
./scripts/sim run --backend=dummy --run-dir "$RUN" --wait-ready 30
jsonschema -i "$RUN/launch-request.json" schemas/launch-request-v1.schema.json
```

The temporary `RUN` directory is caller-owned and must remain outside the checkout. Do not stage or commit generated evidence.
