#!/bin/sh
set -eu

PROBE=/usr/bin/brickpro-bootstrap-probe
RECOVERY=/usr/bin/brickpro-recovery
SUPERVISOR=/usr/bin/brickpro-supervisor

if [ "$#" -eq 0 ]; then
    if "$PROBE" --real-device; then
        exit 1
    else
        exec "$RECOVERY" --real-device-denied
    fi
fi

[ "$#" -eq 2 ] && [ "$1" = "--simulation-fixture-root" ] || exit 64
ROOT=$2
SIM_PROBE=${BRICKPRO_SIMULATION_PROBE-}
SIM_RECOVERY=${BRICKPRO_SIMULATION_RECOVERY-}
SIM_SUPERVISOR=${BRICKPRO_SIMULATION_SUPERVISOR-}
case "$SIM_PROBE:$SIM_RECOVERY:$SIM_SUPERVISOR" in
/*:/*:/*) ;;
*) exit 64 ;;
esac
PROBE=$SIM_PROBE
RECOVERY=$SIM_RECOVERY
SUPERVISOR=$SIM_SUPERVISOR

# A pending safe-mode request is handled before the normal supervisor handoff.
if [ -f "$ROOT/.brickpro/data/recovery-next-boot" ] &&
    [ "$(cat "$ROOT/.brickpro/data/recovery-next-boot")" = safe-mode ]; then
    exec "$RECOVERY" --simulation-fixture-root "$ROOT"
fi


if _PROBE_OUTPUT=$("$PROBE" --simulation-fixture-root "$ROOT"); then
    STATE=$ROOT/.brickpro/data/update
    [ -d "$STATE" ] || exit 1
    TEMP=$STATE/.boot-context.tmp
    trap 'rm -f "$TEMP"' EXIT HUP INT TERM
    printf '%s\n' '{"schema":"brickpro-boot-context/v1","targetSku":"TG4040","mode":"synthetic","selectedRelease":"previous-userspace-release","probe":"compatible","handoffEligible":true}' >"$TEMP"
    mv "$TEMP" "$STATE/boot-context.json"
    printf '%s\n' 'f195fe5e16fb911f990359ab4dfc5bfa961373bd97f6d9bce5b6177ff56cc05d' >"$STATE/boot-context.sha256"
    trap - EXIT HUP INT TERM
    exec "$SUPERVISOR" --simulation-fixture-root "$ROOT"
else
    exec "$RECOVERY" --simulation-fixture-root "$ROOT"
fi
