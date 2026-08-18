#!/bin/sh

# Copy to ~/.deepcode/openmicro-notify.sh, make it executable, and set the
# `notify` key in ~/.deepcode/settings.json to that absolute path.
OPENMICRO_BIN="/Applications/OpenMicro.app/Contents/MacOS/OpenMicro"

case "${STATUS:-}" in
  completed) light_status="success" ;;
  failed) light_status="error" ;;
  *) exit 0 ;;
esac

"$OPENMICRO_BIN" status "$light_status" "deep-code:${TITLE:-default}" >/dev/null 2>&1 || true
