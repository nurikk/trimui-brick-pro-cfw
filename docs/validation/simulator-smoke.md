# Bounded simulator smoke journey

`tools/sim/journeys/simulator-smoke.sh` is the ticket-scoped host-native,
dummy-backend smoke check. It runs two fresh bounded journeys, covering the
four generated launcher identities, strict `LaunchRequest` and completed
session evidence, one signed package progression plus corrupt-target retry,
suspend/resume, checkpoint-failure gating, and accepted/runner-mismatch resume
decisions. It does not run headed/X11, hardware, private-corpus, or exhaustive
menu coverage.

Builds and evidence use the exact Docker namespace
`t867a27e7-smoke`. The caller must provide an existing empty directory outside
the checkout; all generated files remain there:

```sh
mkdir -p /tmp/brickpro-smoke-evidence
TRIMUI_DOCKER_NAMESPACE=t867a27e7-smoke \
  tools/sim/journeys/simulator-smoke.sh --out /tmp/brickpro-smoke-evidence
python3 -m json.tool /tmp/brickpro-smoke-evidence/summary.json >/dev/null
```

The command rejects nonempty, repository, relative, and unsafe evidence roots,
rebuilds the namespace-labelled simulator, validates control/presentation/
`LaunchRequest` JSON with Python schemas, compares normalized deterministic
coverage between runs, and exits nonzero on missing evidence, failed recovery,
privacy markers, dirty shutdown, or any remaining simulator container with the
namespace labels. Each run is stopped through `scripts/sim stop`; its control
socket must be gone and its `exit-status.json` must record clean shutdown.

This is a userspace simulator check and makes no hardware-emulation claim.
