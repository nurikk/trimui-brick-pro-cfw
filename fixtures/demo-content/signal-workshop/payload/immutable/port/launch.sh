#!/bin/sh
read -r _ <&"$BROKER_BARRIER_FD" || exit 10
scenario=${2:-}
save=
state=
resume=false
while [ "$#" -gt 0 ]; do
    case "$1" in
    --save)
        save=${2:-}
        shift 2
        ;;
    --state)
        state=${2:-}
        shift 2
        ;;
    --resume)
        resume=true
        shift
        ;;
    *) shift ;;
    esac
done
case "$scenario" in
success)
    [ -n "$save" ] && [ -n "$state" ] || exit 11
    if [ "$resume" = true ]; then
        [ "$(cat "$save" 2>/dev/null)" = signal-workshop-save-v1 ] || exit 12
        [ "$(cat "$state" 2>/dev/null)" = signal-workshop-state-v1 ] || exit 12
        printf '%s\n' signal-workshop-resumed >"$PORTMASTER_RUNTIME_ROOT/lib/session.marker"
    else
        printf '%s\n' signal-workshop-save-v1 >"$save"
        printf '%s\n' signal-workshop-state-v1 >"$state"
        printf '%s\n' signal-workshop-frame >"$PORTMASTER_RUNTIME_ROOT/lib/session.marker"
    fi
    exit 0
    ;;
nonzero) exit 7 ;;
signal) kill -ABRT "$$" ;;
timeout | cancel) while :; do :; done ;;
*) exit 9 ;;
esac
