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

Use `--backend=x11` to exercise SDL through Xvfb inside the container; no host display forwarding is required. `clean-run` removes the simulator's known artifacts, including durable state/index data, in the external run directory before starting a new run. `run` rejects any existing managed evidence and structurally validates readiness and the first-frame event before returning success.

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
