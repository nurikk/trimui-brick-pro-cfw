#!/bin/sh
set -eu

RECOVERY=/usr/bin/brickpro-recovery

if [ "$#" -eq 0 ]; then
    exec "$RECOVERY" --real-device-denied
fi

[ "$#" -ge 2 ] && [ "$1" = "--simulation-fixture-root" ] || exit 64
ROOT=$2
shift 2
case "${BRICKPRO_SIMULATION_RECOVERY-}" in
/*) RECOVERY=$BRICKPRO_SIMULATION_RECOVERY ;;
*) exit 64 ;;
esac

if [ "$#" -eq 0 ]; then
    exec "$RECOVERY" --simulation-fixture-root "$ROOT"
elif [ "$#" -eq 2 ] && [ "$1" = "--select" ]; then
    case "$2" in
    previous-userspace-release | safe-mode | stock-passthrough) ;;
    *) exit 64 ;;
    esac
    exec "$RECOVERY" --simulation-fixture-root "$ROOT" --select "$2"
else
    exit 64
fi
