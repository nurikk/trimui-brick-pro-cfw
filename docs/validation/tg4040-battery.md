# TG4040 battery and charging validation

## Evidence status

No physical TG4040 battery run is checked in. The host simulator proves controller behavior only; it cannot close PMIC, fuel-gauge, charging, shutdown-threshold, LED, display-wake, or unplug-after-power-off claims. Keep `docs/hardware/tg4040.md` at `unknown` until a sanitized physical record is reviewed.

Community reports motivate the failure cases but are not TG4040 calibration data:

- <https://github.com/spruceUI/spruceOS/issues/1592>
- <https://github.com/knulli-cfw/distribution/issues/472>
- <https://github.com/christianhaitian/arkos/issues/1278>
- <https://github.com/OnionUI/Onion/discussions/1893>

Do not copy their thresholds, battery curves, or chemistry assumptions.

## Instrumented physical run

Use an exact TG4040 with reviewed firmware identity and the platform HAL. UI and validation code must consume the HAL observation; they must not open guessed sysfs paths. Record one row at boot, every 60 seconds, at each percentage/state transition, and immediately before shutdown:

```text
timestamp_utc,elapsed_s,phase,hal_percent,hal_charging,hal_full,hal_external_power,hal_health,hal_level,reference_voltage_v,reference_current_a,reference_energy_wh,event
```

1. Start powered off and unplugged. Confirm it remains off for five minutes.
2. Attach the inline reference meter and charger. Record whether attach boots or wakes the unit; it must not do so unless the documented user policy requests display feedback.
3. Charge uninterrupted through the HAL `charging` to `full` transition. Record any plateau, rollback, jump, external-power contradiction, LED state, and display state.
4. Reboot once while connected, then unplug while awake. Confirm policy and observations survive and no save checkpoint, reboot, or suspend transition is created by either event.
5. Discharge with a fixed representative workload until the platform's orderly critical shutdown. Record warning, save-and-exit, checkpoint generation, shutdown request, final HAL percentage, and reference readings.
6. Leave the unit unplugged for five minutes after shutdown. It must remain off. Reconnect and verify the last checkpoint once; no repeated low/critical action may appear.
7. Repeat the low/critical boundary once after suspend/resume and once after an update boot to confirm persisted policy.

For this run only, compute run-relative gauge error from the reference meter's accumulated energy between the observed full and shutdown endpoints. Report maximum absolute percentage-point error, median error, largest one-sample jump, charge/full transition readings, warning percentage, critical action percentage, and physical power-off percentage. This is empirical run error, not a battery-capacity or chemistry claim.

A sanitized evidence record must identify device SKU, firmware/build digest, meter model, meter calibration date, operator, UTC interval, sample count, raw-log SHA-256, summarized values, anomalies, and pass/fail results. Keep raw device identifiers and full logs outside the repository. Only then add a `verified` record to `docs/hardware/evidence/tg4040/index.json` and update the hardware fact table.

## Canonical simulator readback

After `./scripts/sim build`, run:

```sh
tools/sim/journeys/power-lifecycle.sh
scripts/simctl --socket "$RUN/control.sock" state
```

The canonical `sim-state/v1` readback is `battery.policy`, `battery.decision`, `battery.actionCount`, `hardware.battery`, `hardware.externalPower`, `lifecycle`, and `presentation.affordances`. Null hardware fields and `unknown` classifications are unavailable observations; clients must not replace them with zero or a remembered value.
