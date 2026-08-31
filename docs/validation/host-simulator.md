# Host-native userspace simulator

This is the **host-native userspace simulator** lane. It is a deterministic Rust userspace test double for the synthetic TG4040 profile, not hardware emulation.

## Normal operation

Build without a host Rust installation, then run a complete deterministic catalog-to-session flow:

```sh
./scripts/sim build
RUN=$(mktemp -d)
./scripts/sim run --backend=dummy --run-dir "$RUN" --wait-ready 30
```

The caller-owned `$RUN` directory contains `logs/launcher.jsonl`, readiness, semantic route/selection, launch metadata, a strict `launch-request.json`, session and exit-status records, and logical PNG screenshots. The deterministic fixture traverses `library → systems → games`, selects one of four synthetic platform or PortMaster catalog entries, validates a typed `LaunchRequest`, and never starts an emulator. Docker identities are derived from the resolved physical checkout root by `scripts/docker-worktree.sh`; `TRIMUI_DOCKER_NAMESPACE` is the one explicit override and must be an exact 1–48 character lowercase alphanumeric/hyphen value, starting and ending alphanumeric, with no normalization. Images and containers carry the namespace, root fingerprint, source fingerprint, and cache-scope labels. Every run rejects a missing, foreign, stale, or source-mismatched image until that checkout rebuilds it. A persistent run can be started with `--detach` and stopped with:

```sh
./scripts/sim run --backend=dummy --run-dir "$RUN" --wait-ready 30 --detach
./scripts/sim stop --run-dir "$RUN"
```

Use `--backend=x11` only for the container-private SDL fixture lane. For headed acceptance, use an existing local X server with `--backend=host-x11 --display :N`. That mode mounts the host X11 socket read-only while retaining the read-only root, dropped capabilities, non-root UID, PID limit, and `--network none`; it never starts the private Xvfb. The SDL window follows the device profile (1024×768 for TG4040) and is titled `Host-native simulator acceptance — not physical TG4040 evidence`. Arrow keys, Return/Z/A, Escape/X/B, Space, Tab, Home, PageUp, and PageDown map to controller buttons. `clean-run` removes the simulator's known artifacts, including durable state/index data, in the external run directory before starting a new run. `run` rejects any existing managed evidence and structurally validates readiness and the first-frame event before returning success.

The launcher and `tools/sim/journeys/controller-route-coverage.py` consume the canonical 66-route graph at `sim/routes/controller-routes.json`. Home/Menu resets to Home, Up/Down selects a route, Primary enters it, and Secondary cancels. Coverage starts one fresh simulator for pass 1 and one for pass 2, reusing each instance for all 66 sequential routes; it never restarts per route or calls the direct `presentation` fixture command. Each pass runs the representative smoke subset first (Home, settings, diagnostics, PortMaster, and a launch/checkpoint/resume path), then exhaustive coverage. `--smoke-only` runs only that subset. Per-control (15s), per-route (60s), whole-pass (600s), startup/shutdown (45s), and shutdown verification (30s) bounds fail fast with the run directory and event log in the diagnostic. Shutdown verification is deliberately last because it terminates the simulator. Results compare normalized semantic route/event/presentation records only; run IDs, timestamps, container identity, output directories, and artifact filenames/paths are excluded.

## Contract and boundaries

- Catalog data is limited to `sim/fixtures/catalog.json`; every entry is generated and has zero content. The embedded strict launch catalog is `fixtures/launch-contract/generated-v1/catalog.synthetic.json`; its logical request paths are not host paths.
- The approved state-only profile is `sim/device/tg4040-host.json`. Its controls are semantic injected button events, and its battery, LED, audio, radio, suspend, and fault fields are fixture state.
- The platform trait is in `crates/platform-contract`; the host-independent catalog/session flow is in `crates/launcher/src/lib.rs`; `crates/host-platform` is the replaceable SDL/Xvfb or SDL/dummy backend. Shared route, catalog, launch, and session types are in `crates/domain` and have no simulator dependency.
- Runtime source is mounted read-only at `/src`, the process runs as UID/GID `10001:10001`, runtime state uses a `/tmp` tmpfs, networking is disabled, and `$RUN` is the only caller-owned writable host mount. The image is read-only and drops capabilities.
- No ROMs, BIOS, firmware, vendor blobs, archive files, private corpus, source paths, or proprietary dependencies are read or included.

The lane does **not** claim device, GPU, framebuffer, input, PowerVR, PMIC, radio, performance, ABI, or HIL fidelity. It does not prove A133P/board behavior, physical controls, battery/charging/power, suspend/resume, radio, physical audio/LED/rumble/USB, thermal behavior, or device performance. Screenshots are logical display output only.

## Validation

The practical checks are:

```sh
./scripts/sim build
RUN=$(mktemp -d)
./scripts/sim run --backend=dummy --run-dir "$RUN" --wait-ready 30
grep -q '"event":"ready"' "$RUN/logs/launcher.jsonl"
grep -q '"event":"first_frame"' "$RUN/logs/launcher.jsonl"
grep -q '"event":"route_selection".*"route":"library"' "$RUN/logs/launcher.jsonl"
grep -q '"event":"route_selection".*"route":"systems"' "$RUN/logs/launcher.jsonl"
grep -q '"event":"route_selection".*"route":"games"' "$RUN/logs/launcher.jsonl"
jsonschema -i "$RUN/launch-request.json" schemas/launch-request-v1.schema.json
test -s "$RUN"/screenshots/screen-*.png
./scripts/sim stop --run-dir "$RUN"
grep -q '"event":"clean_shutdown"' "$RUN/logs/launcher.jsonl"
. scripts/docker-worktree.sh
NS=$(trimui_docker_namespace "$PWD")
test "$(docker ps -q --filter label=org.trimui-brick-pro-cfw.simulator=host-native --filter label=org.trimui-brick-pro-cfw.worktree="$NS")" = ""

RUN_X11=$(mktemp -d)
./scripts/sim run --backend=x11 --run-dir "$RUN_X11" --wait-ready 30 --detach
./scripts/sim stop --run-dir "$RUN_X11"
grep -q '"event":"clean_shutdown"' "$RUN_X11/logs/launcher.jsonl"

RUN_HEADED=$(mktemp -d)
./scripts/sim run --backend=host-x11 --display :0 --run-dir "$RUN_HEADED" --wait-ready 30 --detach
./scripts/sim stop --backend=host-x11 --display :0 --run-dir "$RUN_HEADED"

SMOKE=$(mktemp -d)
rm -rf "$SMOKE"
tools/sim/journeys/controller-route-coverage.py --out "$SMOKE" --backend dummy --smoke-only
COVERAGE=$(mktemp -d)
rm -rf "$COVERAGE"
# Estimate 66 routes × 2 passes before launching this bounded exhaustive run.
tools/sim/journeys/controller-route-coverage.py --out "$COVERAGE" --backend dummy

STALE=$(mktemp -d)
printf '%s\n' '{}' > "$STALE/readiness.json"
if ./scripts/sim run --backend=dummy --run-dir "$STALE" --wait-ready 30; then
  exit 1
fi
./scripts/sim clean-run --backend=dummy --run-dir "$STALE" --wait-ready 30
./scripts/sim lifecycle-journey

dash -n scripts/sim
busybox ash -n scripts/sim
. scripts/docker-worktree.sh
NS=$(trimui_docker_namespace "$PWD")
IMAGE=trimui-brick-pro-cfw-simulator:$NS

docker run --rm --network none \
  --mount "type=bind,src=$PWD,dst=/workspace,readonly" \
  --tmpfs /tmp:rw,exec,nosuid,nodev \
  --user 10001:10001 \
  -w /workspace \
  --entrypoint cargo \
  "$IMAGE" \
  fmt --check

docker run --rm --network none \
  --mount "type=bind,src=$PWD,dst=/workspace,readonly" \
  --tmpfs /tmp:rw,exec,nosuid,nodev \
  --user 10001:10001 \
  -e CARGO_TARGET_DIR=/tmp/cargo-target \
  -w /workspace \
  --entrypoint cargo \
  "$IMAGE" \
  clippy --workspace -- -D warnings

git diff --check

# The executable journey is the bounded, non-test fixture check. It writes evidence under /tmp.
cargo run --release --locked -p sim-launcher --bin launcher-fixture-journey
./scripts/docker-isolation-journey

RUN_INSPECT=$(mktemp -d)
./scripts/sim run --backend=dummy --run-dir "$RUN_INSPECT" --wait-ready 30 --detach
CONTAINER=$(docker ps --filter label=org.trimui-brick-pro-cfw.simulator=host-native --filter label=org.trimui-brick-pro-cfw.worktree="$NS" -q)
docker inspect "$CONTAINER" --format 'user={{.Config.User}} readonly={{.HostConfig.ReadonlyRootfs}} network={{.HostConfig.NetworkMode}} tmpfs={{json .HostConfig.Tmpfs}} mounts={{range .Mounts}}{{.Destination}}:rw={{.RW}};{{end}}'
./scripts/sim stop --run-dir "$RUN_INSPECT"
```

`scripts/docker-isolation-journey` copies the checkout only into external temporary directories, builds both namespaced simulator images concurrently, interleaves control calls, checks exact labels/fingerprints, rejects a cross-worktree control attempt, selectively stops one run while the peer remains usable, and leaves zero owned containers. It does not add evidence or fixtures to the repository. The image build uses `containers/simulator/Dockerfile` and is based on the immutable x86_64 digest and records the toolchain and direct package versions in `sim/container-versions.txt`; `Cargo.lock` records all Rust dependency versions.
