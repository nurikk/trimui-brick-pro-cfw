# Simulator control protocol

The host-native TG4040 simulator exposes one semantic control socket per run at
`$RUN/control.sock`. The socket is a versioned newline-delimited JSON protocol
(`sim-control/v1`), specified by `sim/contracts/control.schema.json`. Request
frames are limited to 8192 bytes; responses are limited to 65536 bytes. Each
request read has a 500 ms total deadline and client responses have bounded waits.
Requests are typed JSON only: there is no shell, exec, subprocess, environment,
mount, source-path, catalog-path, or content-path operation.

## Host CLI

Use the host shim; it validates the resolved run directory, the exact simulator,
namespace, root/source-fingerprint, and cache-scope labels, `/evidence` mount,
and running container before using `docker exec --user 10001:10001`. A socket
from a peer worktree is rejected. It never weakens the socket permissions.

```sh
scripts/simctl --socket "$RUN/control.sock" wait-ready --timeout 30
scripts/simctl --socket "$RUN/control.sock" state
scripts/simctl --socket "$RUN/control.sock" button --button start --action press
scripts/simctl --socket "$RUN/control.sock" button --button down --action press
scripts/simctl --socket "$RUN/control.sock" button --button primary --action press
scripts/simctl --socket "$RUN/control.sock" adapter complete --status 0 --value 0
scripts/simctl --socket "$RUN/control.sock" hardware set battery.percent=5 storage.mode=full
scripts/simctl --socket "$RUN/control.sock" hardware set battery.available=false
scripts/simctl --socket "$RUN/control.sock" power battery-policy --warning 20 --critical 10 --action save-and-exit --charging-led false --charging-display true
scripts/simctl --socket "$RUN/control.sock" presentation --action settings-form
scripts/simctl --socket "$RUN/control.sock" presentation --action wifi-password
scripts/simctl --socket "$RUN/control.sock" presentation --action scraper-ambiguity
scripts/simctl --socket "$RUN/control.sock" lifecycle suspend --timeout 5
scripts/simctl --socket "$RUN/control.sock" clock advance --milliseconds 299000
scripts/simctl --socket "$RUN/control.sock" lifecycle resume --timeout 5 --source user
scripts/simctl --socket "$RUN/control.sock" fault set arm-fail
scripts/simctl --socket "$RUN/control.sock" fault clear arm-fail
scripts/simctl --socket "$RUN/control.sock" screenshot --name low-battery
scripts/simctl --socket "$RUN/control.sock" checkpoint --name after-launch
```

`button` accepts `up`, `down`, `left`, `right`, `primary`, `secondary`,
`start`, `select`, and `menu`, with `press` or `release`. Hardware assignments
are limited to typed battery availability/percent/charging/full/health/external-power, storage
`available|full`, radio enabled/connected, and suspend `active|suspended` plus
result `none|success|failed`. `clock advance` changes semantic monotonic and
boot time; `clock jump` changes only semantic boot/RTC time while retaining
monotonic time. Lifecycle compares the persisted boot-time deadline, not the
wake-source label; a forward clock jump at or beyond it terminates orderly.
Lifecycle sleep options are exactly `1`, `5`, `10`, `15`, `30`, and `60` minutes,
defaulting to `5`. Faults are the fixed allowlist `adapter-fail`,
`adapter-crash`, `input-drop`, `suspend-fail`, `checkpoint-fail`, `arm-fail`,
`verify-fail`, `clear-fail`, `crash-before-suspend`, `crash-armed-journal`,
`shutdown-fail`, and `hal-loss`.
Adapter actions are `complete`, `fail`, `exit`, or `crash`, with integer status
`0..255` and signed integer value. Artifact names are safe basenames only.

Successful CLI calls print exactly one JSON response on stdout. The response
has `version`, `id`, `ok`, `result`, and `error`; rejected requests have
`error.code=protocol_rejected`. Exit codes are stable: `2` usage/local
validation, `3` unavailable run/socket/container, `4` protocol rejection, and
`5` timeout or transport failure. The same rejection behavior applies to
malformed JSON, unknown fields/commands, empty or oversized frames, and
path-bearing or shell-like fields. With the repository's available validator,
`jsonschema -i accepted.json sim/contracts/control.schema.json` succeeds while
an `exec` or path-bearing request fails.

`state` returns `sim-state/v1` semantic state: current `library`, `systems`,
or `games` route, selected synthetic demo ID, active broker session and result,
modal/status, readiness generation, session frame step, typed virtual hardware,
semantic monotonic/wall clock, enabled named faults, and lifecycle evidence
including the armed deadline, wake source/reason, and typed orderly-shutdown
request. It also includes a `launcher-presentation/v1` Artbook screen. The screen
is built from generated `ui-model`, Artbook theme, `settings-ui`, and Wi-Fi
controller projections, including bounded ROM-index status and persisted recent
entries. `controllerRoute` reports the canonical navigator, selected index,
current route ID, and expected count. Home/Menu, Up/Down, Primary, and Secondary
drive it through the same button path as a local keyboard or controller.
`presentation --action` remains a narrow fixture command and is not used by the
controller acceptance runner. The state object is also written
beside each requested artifact. Validate it with the `state` definition in
`control.schema.json`; no pixels are inspected.

## Current supported flow and limits

The shipped synthetic launcher flow is `Library → Systems → Games`. It contains
four visible entries: `nebula-nes` on stable system `nes`, `mirror-ps1` on stable
system `ps1`, and the separately browsable PortMaster entries `orbit-garden` and
`signal-workshop`. `start` advances Library to Systems, `down` advances Systems
to Games, further directional presses select an entry, and `primary` strictly
validates and emits a typed `launch-contract::LaunchRequest` through the broker
boundary before starting a session. Platform entries use simulator-owned
generated runner/core metadata; no real ROM/core compatibility is claimed. The
request is written to `launch-request.json`; broker completion or failure is
reflected in `state`, `session.json`, and JSONL events. The canonical controller
graph covers generated systems, game details, settings, Wi-Fi, Theme Garden,
scraper, diagnostics, updater, fault, Game Switcher,
PortMaster, Save Vault/Save Sync, and shutdown surfaces. Coverage proves visible
controller reachability, while the existing focused journeys remain authoritative
for side effects. VNC/noVNC, real emulation, and real hardware are not implemented
or claimed.

Every screenshot or checkpoint creates a nonempty PNG and paired semantic JSON
under `$RUN/screenshots` or `$RUN/checkpoints`. The JSONL log contains only
filtered semantic data, a stable per-run `runId`, and monotonically increasing
`sequence` values; artifact events include relative artifact names and their
sequence. It contains no host paths, source/content paths, environment values,
private corpus references, vendor/firmware assets, or secrets.

## End-to-end generated-content check

```sh
./scripts/sim build
RUN=$(mktemp -d)
./scripts/sim run --backend=dummy --run-dir "$RUN" --wait-ready 30 --detach
./scripts/simctl --socket "$RUN/control.sock" wait-ready --timeout 30
./scripts/simctl --socket "$RUN/control.sock" button --button start --action press
./scripts/simctl --socket "$RUN/control.sock" button --button down --action press
./scripts/simctl --socket "$RUN/control.sock" button --button down --action press
./scripts/simctl --socket "$RUN/control.sock" button --button primary --action press
./scripts/simctl --socket "$RUN/control.sock" adapter complete --status 0 --value 7
./scripts/simctl --socket "$RUN/control.sock" screenshot --name generated-demo
./scripts/simctl --socket "$RUN/control.sock" state
./scripts/sim stop --run-dir "$RUN"
test -s "$RUN/screenshots/generated-demo.png"
test -s "$RUN/screenshots/generated-demo.json"
python3 -m json.tool sim/contracts/control.schema.json >/dev/null
python3 -m json.tool "$RUN/screenshots/generated-demo.json" >/dev/null
```

The run remains network-none, read-only-rootfs, capability-dropped, and
UID/GID 10001:10001. Regular evidence directories are caller-cleanable;
`control.sock` is explicitly mode 0660 and is not world-writable. `scripts/sim stop` verifies the container identity and
`/evidence` mount before stopping it, requires a clean shutdown record, and
requires that `control.sock` is gone. `clean-run` removes only known simulator
artifact directories/files and only the exact current-worktree container; it does
not perform global label cleanup.
