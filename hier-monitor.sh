#!/usr/bin/env bash
# hier-monitor.sh
# Connect to the hierarchical compositor control socket and stream live messages.
# Usage: HIER_CTRL_SOCKET=/tmp/hier-ctrl-XXXX.sock ./hier-monitor.sh
# If HIER_CTRL_SOCKET is not set, defaults to /tmp/hier-ctrl.sock

set -euo pipefail

if [[ -n "${HIER_CTRL_SOCKET:-}" ]]; then
  SOCKET="$HIER_CTRL_SOCKET"
elif [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
  SOCKET="/tmp/hier-ctrl-${WAYLAND_DISPLAY}.sock"
else
  # Try to find any active hier-ctrl socket
  ACTIVE_SOCKETS=(/tmp/hier-ctrl-wayland-*.sock)
  if [[ -e "${ACTIVE_SOCKETS[0]:-}" ]]; then
    SOCKET="${ACTIVE_SOCKETS[0]}"
  else
    SOCKET="/tmp/hier-ctrl.sock"
  fi
fi

if [[ ! -e "$SOCKET" ]]; then
  echo "Error: Control socket $SOCKET does not exist." >&2
  echo "Please check if the compositor is running and verify your WAYLAND_DISPLAY environment variable." >&2
  exit 1
fi

# Connect via netcat (Unix domain socket)
# Print each line; exit when a line equals "quit"

nc -U "$SOCKET" | while IFS= read -r line; do
  echo "$line"
  if [[ "$line" == "quit" ]]; then
    echo "Received quit signal. Exiting monitor."
    break
  fi
done
