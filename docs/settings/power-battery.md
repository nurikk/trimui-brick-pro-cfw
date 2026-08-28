# Power & Battery settings domain

This is a clean-room, domain-only MVP contract. It describes semantic fields and
interaction expectations; it does not define a global descriptor/schema,
persistence backend, capability registry, or hardware operation. All
observations may be unavailable. The fixture uses synthetic, non-physical data.

## Field contract

`power.*` IDs are stable semantic ID suggestions. Bounded sleep is a controller policy only: the RTC is a Linux-resume mechanism, not a shutdown decision or physical-power evidence. `cap.power.*` names are
power-domain capability-gate expectations, not a global registry.

| Stable semantic ID suggestion | Purpose | Field kind | Default | Constraints/options | Scope | Apply semantics | Sensitivity | Capability gate | Visibility rule | Failure behavior |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `power.sleep-timeout-seconds` | Idle time before sleep policy is eligible | Mutable policy | `300` seconds | Integer; `0` disables; `30..86400` otherwise | Device policy | Valid change becomes pending; confirm applies it; cancel discards it; reset proposes `300` | Low | `cap.power.sleep-policy` | Visible only when supported; hidden when unsupported | Invalid or unsupported change is rejected and current/pending values stay unchanged |
| `power.maximum-sleep-duration-minutes` | Maximum bounded sleep duration before deadline resume and orderly shutdown | Mutable policy | `5` minutes | Exactly `1`, `5`, `10`, `15`, `30`, or `60`; no indefinite value | Device policy | Valid change becomes pending; confirm applies it; cancel discards it; reset proposes `5` | Low | `cap.power.bounded-sleep` | Visible only when supported; hidden when unsupported | Invalid or unsupported change is rejected and current/pending values stay unchanged |
| `power.auto-shutdown-timeout-seconds` | Idle time before auto-shutdown policy is eligible | Mutable policy | `1800` seconds | Integer; `0` disables; `60..172800` otherwise | Device policy | Valid change becomes pending; confirm applies it; cancel discards it; reset proposes `1800` | Low | `cap.power.shutdown-policy` | Visible only when supported; hidden when unsupported | Invalid or unsupported change is rejected and current/pending values stay unchanged |
| `power.low-battery-warning-threshold-percent` | Charge level at which the warning policy becomes eligible | Mutable policy | `20` percent | Integer `1..50` | Device policy | Valid change becomes pending; confirm applies it; cancel discards it; reset proposes `20` | Low | `cap.power.low-battery-policy` | Visible only when supported; hidden when unsupported | Invalid or unsupported change is rejected and current/pending values stay unchanged |
| `power.low-battery-save-exit-policy` | Policy for saving and leaving a session at low battery | Mutable policy | `warn-only` | Exactly `warn-only`, `save-and-exit`, or `exit-without-save` | Device policy | Valid change becomes pending; confirm applies it; cancel discards it; reset proposes `warn-only` | Low | `cap.power.low-battery-policy` | Visible only when supported; hidden when unsupported | Invalid or unsupported change is rejected and current/pending values stay unchanged |
| `power.suspend-candidate` | Whether suspend is eligible as a policy candidate | Mutable policy | `true` | Boolean only; this is not a suspend command | Device policy | Valid change becomes pending; confirm applies it; cancel discards it; reset proposes `true` | Low | `cap.power.suspend-policy` | Visible only when supported; hidden when unsupported | Invalid or unsupported change is rejected and current/pending values stay unchanged |
| `power.performance-profile` | User-selected performance policy label | Mutable policy; policy-only | `balanced` | Exactly `quiet`, `balanced`, or `performance`; labels do not perform hardware operations | Device policy | Valid change becomes pending; confirm applies it; cancel discards it; reset proposes `balanced` | Low | `cap.power.performance-policy` | Visible only when supported; hidden when unsupported | Invalid or unsupported change is rejected and current/pending values stay unchanged |
| `power.charging-status` | Charging-state observation | Read-only observation | None; observation only | `charging`, `not-charging`, `full`, or `unknown` | Current device observation | Read/refresh only; never pending, confirmed, cancelled, or reset | Low | `cap.power.charging-observation` | Visible only when supported and an observation is available; otherwise hidden | Mutation/reset is rejected as read-only; unsupported observation is unavailable |
| `power.external-power-status` | External-power presence observation | Read-only observation | None; observation only | `connected`, `disconnected`, or `unknown` | Current device observation | Read/refresh only; never pending, confirmed, cancelled, or reset | Low | `cap.power.external-power-observation` | Visible only when supported and an observation is available; otherwise hidden | Mutation/reset is rejected as read-only; unsupported observation is unavailable |
| `power.battery-level-percent` | Current reported charge level | Read-only observation | None; observation only | Integer `0..100`, or `unknown` when unavailable | Current device observation | Read/refresh only; never pending, confirmed, cancelled, or reset | Low | `cap.power.battery-observation` | Visible only when supported and an observation is available; otherwise hidden | Mutation/reset is rejected as read-only; unsupported observation is unavailable |
| `power.battery-health` | Current reported battery health class | Read-only observation | None; observation only | `good`, `degraded`, or `unknown` | Current device observation | Read/refresh only; never pending, confirmed, cancelled, or reset | Low | `cap.power.battery-observation` | Visible only when supported and an observation is available; otherwise hidden | Mutation/reset is rejected as read-only; unsupported observation is unavailable |
| `power.battery-status` | Current coarse battery status | Read-only observation | None; observation only | `normal`, `low`, `critical`, or `unknown` | Current device observation | Read/refresh only; never pending, confirmed, cancelled, or reset | Low | `cap.power.battery-observation` | Visible only when supported and an observation is available; otherwise hidden | Mutation/reset is rejected as read-only; unsupported observation is unavailable |

## Interaction expectations

- Mutable policy fields are the only writable fields. A valid proposal creates a
  pending change and does not alter the currently applied value.
- Confirm applies the named pending policy change. Cancel removes that pending
  change and leaves the currently applied value unchanged. A pending change is
  not an instruction to perform a power operation.
- Reset on a mutable field proposes that field's documented default and follows
  the same pending/confirm/cancel flow. Reset on an observation is rejected as
  read-only.
- Invalid values are rejected without coercion and without changing current or
  pending state. Unsupported gates make the field unavailable; reads and writes
  must report unsupported rather than guessing a value or falling back.
- `power.performance-profile` is a policy label only. This contract includes no
  frequency, governor, thermal, suspend, shutdown, charging, or other physical
  operation and makes no hardware-behavior claim.
- The bounded flow is `awake → preparing-suspend → suspended → resumed-by-user | resumed-for-deadline → orderly-shutdown`. Before suspend it checkpoints, persists a bounded SHA-256 checksummed marker/journal, clears stale alarm state, arms and verifies one typed deadline, and fails closed to typed orderly shutdown if arming, verification, or alarm clearing fails.
- The host simulator proves only deterministic controller state, persisted boot-time/RTC deadline semantics, forward-clock-jump safety, typed request ordering, and redacted evidence. It does not prove TG4040 RTC/PMIC wake, `mem` suspend, current draw, clock drift, power-button behavior, save integrity, or physical power-off; those remain HIL-only after separate authorization.
