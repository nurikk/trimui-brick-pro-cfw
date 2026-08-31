# TG4040 power-profile validation

`config/platform/tg4040/power-policies.json` is the canonical Eco, Balanced, and Performance policy fixture. It is intentionally simulator-only: `hardwareVerified` is `false`, real-device operations are denied, and none of its synthetic clock values authorize sysfs, vendor-daemon, or device-node access.

The policy keeps the launcher, suspend, wake, normal-exit, crash-exit, and safe-mode-reset states on Eco. A game receives the global Balanced baseline, an emulator-system default when present, then a declarative game override, and finally an optional temporary user override. Returning to the launcher removes the temporary policy. The fixed 75 C thermal limit always wins and degrades the effective profile to Eco with 5 C hysteresis; no preset or command can disable throttling. All profiles request the synthetic `1024x768@60` panel mode for 60 Hz content.

## Trade-off evidence

`fixtures/power-policy/benchmark-matrix.json` records FPS, p99 frame time, temperature, and power together for representative emulator and PortMaster workloads. These are deterministic fixture readings for the **host-native userspace simulator**, not physical measurements. They make the policy choice explainable while preserving the hardware-evidence boundary. Physical TG4040 validation remains required before any clock, thermal, power, or frame-pacing claim can be promoted to hardware-verified.

The supplied community reports motivate low launcher clocks, system-wide profiles, and conservative thermal handling, but they are anecdotal and may describe different SKUs. Their values are not copied into this TG4040 fixture:

- <https://www.reddit.com/r/trimui/comments/1hjpov2/crossmixos_v130_released/>
- <https://www.reddit.com/r/SBCGaming/comments/1j41n5e/trimui_brick_the_first_vertical_pocketable/>
- <https://github.com/LoveRetro/NextUI/issues/705>
- <https://github.com/cizia64/CrossMix-OS/issues/12>
- <https://github.com/spruceUI/spruceOS/issues/1481>

Run `tools/sim/journeys/performance-profiles.sh` after `scripts/sim build`. The journey uses two simulator instances total: a representative smoke pass first, then the lifecycle/crash/thermal matrix on a second fresh root.
