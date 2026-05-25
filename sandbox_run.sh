#!/bin/bash
# sandbox_run.sh
# Run a Wayland client securely sandboxed via Bubblewrap

HOST_XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/1000}"
HOST_WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-2}"

SANDBOX_DIR="/tmp/niri-sandbox"
mkdir -p "$SANDBOX_DIR"
touch "$SANDBOX_DIR/$HOST_WAYLAND_DISPLAY"

echo "[Sandbox] Initializing Bubblewrap client container..."
echo "[Sandbox] Binding client display: $HOST_XDG_RUNTIME_DIR/$HOST_WAYLAND_DISPLAY -> $SANDBOX_DIR/$HOST_WAYLAND_DISPLAY"

# Parse software mode option
SOFTWARE_MODE=0
if [ "$1" = "--software" ]; then
  SOFTWARE_MODE=1
  shift
fi

TARGET_CMD=("$@")
if [ ${#TARGET_CMD[@]} -eq 0 ]; then
  if command -v foot >/dev/null 2>&1; then
    TARGET_CMD=("/usr/bin/foot")
  else
    TARGET_CMD=("/usr/bin/alacritty")
  fi
fi

BWRAP_ARGS=(
  --ro-bind /usr /usr
  --ro-bind /lib /lib
  --ro-bind /lib64 /lib64
  --ro-bind /bin /bin
  --ro-bind-try /etc/fonts /etc/fonts
  --ro-bind-try /usr/share/fonts /usr/share/fonts
  --dev /dev
  --proc /proc
  --tmpfs /tmp
  --tmpfs /home
  --bind-try /dev/shm /dev/shm
  --bind "$SANDBOX_DIR" "$SANDBOX_DIR"
  --bind "$HOST_XDG_RUNTIME_DIR/$HOST_WAYLAND_DISPLAY" "$SANDBOX_DIR/$HOST_WAYLAND_DISPLAY"
  --setenv WAYLAND_DISPLAY "$HOST_WAYLAND_DISPLAY"
  --setenv XDG_RUNTIME_DIR "$SANDBOX_DIR"
  --unshare-all
  --share-net
)

if [ "$SOFTWARE_MODE" -eq 1 ] || [ -n "$LIBGL_ALWAYS_SOFTWARE" ]; then
  echo "[Sandbox] Software rendering mode (llvmpipe) FORCED."
  BWRAP_ARGS+=(--setenv LIBGL_ALWAYS_SOFTWARE 1)
else
  echo "[Sandbox] Hardware GPU/DRI acceleration enabled."
  echo "[Sandbox] Tip: If GPU driver errors occur inside sandbox (e.g. libEGL / VK_ERROR), restart with --software"
  BWRAP_ARGS+=(--dev-bind-try /dev/dri /dev/dri)
fi

bwrap "${BWRAP_ARGS[@]}" "${TARGET_CMD[@]}"

