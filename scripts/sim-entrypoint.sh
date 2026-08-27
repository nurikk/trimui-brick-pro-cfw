#!/bin/sh
set -eu

xvfb_pid=
launcher_pid=

stop_xvfb() {
    if [ -n "${xvfb_pid}" ]; then
        kill "$xvfb_pid" 2>/dev/null || true
        wait "$xvfb_pid" 2>/dev/null || true
        xvfb_pid=
    fi
}

forward_term() {
    kill -TERM "$launcher_pid" 2>/dev/null || true
    wait "$launcher_pid" 2>/dev/null || true
    stop_xvfb
    exit 0
}

if [ "${1:-}" = "--backend=x11" ]; then
    Xvfb :99 -screen 0 1024x768x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
    xvfb_pid=$!
    export DISPLAY=:99
    sleep 1
fi

trap forward_term TERM INT
/usr/local/bin/sim-launcher \
    --profile /src/sim/device/tg4040-host.json \
    --catalog /src/sim/fixtures/catalog.json \
    --evidence /evidence \
    "$@" &
launcher_pid=$!
wait "$launcher_pid"
status=$?
trap - TERM INT
stop_xvfb
exit "$status"
