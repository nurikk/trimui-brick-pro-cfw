#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
command -v grep >/dev/null 2>&1 || {
    printf '%s\n' 'session-broker HAL boundary scanner is unavailable' >&2
    exit 1
}
if grep -R -n -E '(/dev/|/sys/|ioctl|sh[[:space:]]+-c|bash[[:space:]]+-c|eval[[:space:]])' "$ROOT/crates/session-broker/src"; then
    printf '%s\n' 'session-broker HAL or shell boundary violated' >&2
    exit 1
else
    status=$?
    if [ "$status" -ne 1 ]; then
        printf '%s\n' 'session-broker HAL boundary scanner failed' >&2
        exit 1
    fi
fi
printf '%s\n' 'session-broker HAL and shell boundary passed'
