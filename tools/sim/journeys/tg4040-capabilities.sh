#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd -P)
command -v timeout >/dev/null 2>&1 || { printf '%s\n' 'tg4040 capability journey: timeout is required' >&2; exit 2; }
RUN=$(mktemp -d "${TMPDIR:-/tmp}/tg4040-capabilities.XXXXXX")
cleanup() {
    "$ROOT/scripts/sim" stop --run-dir "$RUN" >/dev/null 2>&1 || true
    rm -rf "$RUN" 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM
ctl() {
    status=0
    timeout 15 "$ROOT/scripts/simctl" --socket "$RUN/control.sock" "$@" || status=$?
    if [ "$status" -eq 124 ]; then
        printf '%s\n' "tg4040 capability journey: control timed out; removing simulator" >&2
        "$ROOT/scripts/sim" clean-run --run-dir "$RUN" >/dev/null 2>&1 || true
        exit 1
    fi
    return "$status"
}
state() { ctl state; }
assert_state() { printf '%s\n' "$1" | python3 -c "$2"; }

# One fresh simulator root covers the bounded lifecycle and reconnect loops.
"$ROOT/scripts/sim" run --backend=dummy --profile sim/device/tg4040-alphabet.json --run-dir "$RUN" --wait-ready 30 --detach

assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert t["capabilities"]["led_zones"] == ["shoulder-lr", "middle", "f1", "f2", "rear"], t
assert t["capabilities"]["led_effects"] == ["off", "low-battery"], t
assert t["capabilities"]["motor_voltage_path"] == "/sys/class/motor/voltage", t
assert t["capabilities"]["motor_enable_path"] == "/sys/class/gpio/gpio227/value", t
assert t["capabilities"]["bluetooth_ready_path"] == "/sys/class/bluetooth/hci0", t
assert t["capabilities"]["input_signals"] == ["gpio243"], t
assert t["capabilities"]["battery_impact"] == "Unmeasured", t
assert s["platformState"]["leds"] == {"on": False, "brightness_percent": 0}, s
'

ctl tg4040 led --enabled true --brightness-percent 42 >/dev/null
ctl tg4040 reboot >/dev/null
ctl tg4040 input --signal gpio243 >/dev/null
assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert t["persisted_led"] == {"enabled": True, "brightness_percent": 42}, t
assert t["observed_inputs"] == ["gpio243"], t
assert s["platformState"]["leds"] == {"on": True, "brightness_percent": 42}, s
'

for role in controller audio; do
    i=0
    while [ "$i" -lt 20 ]; do
        ctl tg4040 scan --role "$role" >/dev/null
        ctl tg4040 pair >/dev/null
        ctl tg4040 paired >/dev/null
        ctl tg4040 connected >/dev/null
        ctl tg4040 reboot >/dev/null
        ctl tg4040 reconnect >/dev/null
        ctl tg4040 connected >/dev/null
        i=$((i + 1))
    done
    assert_state "$(state)" "
import json, sys
s = json.load(sys.stdin)[\"result\"]
t = s[\"tg4040\"]
assert t[\"bluetooth\"] == {\"role\": \"$role\", \"phase\": \"connected\", \"local_input_enabled\": True}, t
assert s[\"platformState\"][\"radios\"][\"bluetooth\"] == {\"enabled\": True, \"connected\": True}, s
"
done

# Controller route graph: fresh-home → mirror session.
ctl button --button primary --action press >/dev/null
ctl button --button down --action press >/dev/null
ctl button --button down --action press >/dev/null
ctl button --button primary --action press >/dev/null
assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
assert s["activeSession"] is not None, s
'

# 50 actual suspend/resume control cycles: LEDs and rumble quiesce in sleep,
# then the persisted LED setting is restored.
i=0
while [ "$i" -lt 50 ]; do
    ctl tg4040 rumble --active true >/dev/null
    ctl lifecycle suspend --timeout 5 >/dev/null
    assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert s["lifecycle"]["phase"] == "suspended", s
assert t["effective_led_enabled"] is False and t["effective_led_effect"] == "off", t
assert t["rumble_active"] is False, t
assert s["platformState"]["leds"]["on"] is False, s
assert s["platformState"]["rumble"]["active"] is False, s
'
    ctl lifecycle resume --timeout 5 --source user >/dev/null
    assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert t["persisted_led"] == {"enabled": True, "brightness_percent": 42}, t
assert t["effective_led_enabled"] is True and t["effective_led_effect"] == "off", t
assert s["platformState"]["leds"] == {"on": True, "brightness_percent": 42}, s
'
    i=$((i + 1))
done

ctl tg4040 low-battery --active true >/dev/null
assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert t["low_battery_override"] is True and t["effective_led_effect"] == "low-battery", t
'
ctl tg4040 low-battery --active false >/dev/null
assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert t["low_battery_override"] is False and t["effective_led_effect"] == "off", t
assert t["persisted_led"] == {"enabled": True, "brightness_percent": 42}, t
'

ctl tg4040 rumble --active true >/dev/null
ctl adapter crash --status 1 --value 0 >/dev/null
assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert s["activeSession"] is None, s
assert t["rumble_active"] is False and t["effective_led_effect"] == "off", t
assert s["platformState"]["rumble"]["active"] is False, s
'

ctl tg4040 led --enabled false --brightness-percent 0 >/dev/null
ctl tg4040 reset >/dev/null
assert_state "$(state)" '
import json, sys
s = json.load(sys.stdin)["result"]
t = s["tg4040"]
assert t["persisted_led"] == {"enabled": False, "brightness_percent": 100}, t
assert t["effective_led_enabled"] is False and t["effective_led_effect"] == "off", t
assert t["rumble_active"] is False and t["ownership_active"] is False, t
assert t["bluetooth"] == {"role": None, "phase": "idle", "local_input_enabled": True}, t
assert t["observed_inputs"] == [], t
assert s["platformState"]["leds"] == {"on": False, "brightness_percent": 0}, s
assert s["platformState"]["radios"]["bluetooth"] == {"enabled": False, "connected": False}, s
'
printf '%s\n' 'tg4040 capability journey: PASS (one simulator root; 50 lifecycle cycles; GPIO 243; controller/audio reconnect; source-derived only)'
