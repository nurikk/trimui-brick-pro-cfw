#!/bin/sh
set -eu

[ "$#" -eq 2 ] && [ "$1" = "--simulation-fixture-root" ] || exit 64
ROOT=$2
STATE=$ROOT/.brickpro/data/update
[ -d "$STATE" ] || exit 1
printf '%s\n' '{"schema":"brickpro-supervisor-handoff/v1","mode":"synthetic","handoff":"accepted","activating":false}' > "$STATE/supervisor-handoff.json"
printf '%s\n' 'b18f3e3b3f3a65d5b9d90ecf61001a224993c5a4af18423224d61e54748299ab' > "$STATE/supervisor-handoff.sha256"
