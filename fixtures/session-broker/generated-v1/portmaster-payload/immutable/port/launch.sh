#!/bin/sh
read -r _ <&"$BROKER_BARRIER_FD" || exit 10
case "${2:-}" in
success) exit 0 ;;
nonzero) exit 7 ;;
signal) kill -ABRT "$$" ;;
timeout | cancel) while :; do :; done ;;
*) exit 9 ;;
esac
