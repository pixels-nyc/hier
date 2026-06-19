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

# Parse option arguments
SOFTWARE_MODE=0
COOKIE_ID=""
NET_MODE="host"
NET_PARAM=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --cookie)
      COOKIE_ID="$2"
      shift 2
      ;;
    --net)
      NET_MODE="$2"
      if [ "$NET_MODE" = "vpn" ] || [ "$NET_MODE" = "proxy" ]; then
        NET_PARAM="$3"
        shift 3
      else
        shift 2
      fi
      ;;
    --software)
      SOFTWARE_MODE=1
      shift 1
      ;;
    *)
      break
      ;;
  esac
done

TARGET_CMD=("$@")
if [ ${#TARGET_CMD[@]} -eq 0 ]; then
  if command -v foot >/dev/null 2>&1; then
    TARGET_CMD=("/usr/bin/foot")
  else
    TARGET_CMD=("/usr/bin/alacritty")
  fi
fi
if [ -z "$HIER_HOST_TRANSFORM" ]; then
  if command -v niri >/dev/null 2>&1; then
    HIER_HOST_TRANSFORM=$(python3 -c '
import subprocess, json
try:
    ws = json.loads(subprocess.check_output(["niri", "msg", "--json", "workspaces"]).decode())
    active = next((w["output"] for w in ws if w.get("is_focused")), None) or next((w["output"] for w in ws if w.get("is_active")), None)
    if active:
        outs = json.loads(subprocess.check_output(["niri", "msg", "--json", "outputs"]).decode())
        print(outs.get(active, {}).get("logical", {}).get("transform", "Normal"))
    else:
        print("Normal")
except Exception:
    print("Normal")
' 2>/dev/null)
    export HIER_HOST_TRANSFORM
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
  --ro-bind "$PWD" /app
  --chdir /app
)

if [ -n "$COOKIE_ID" ]; then
  COOKIE_HOME="$HOME/.cache/hier/cookies/$COOKIE_ID/home"
  echo "[Sandbox] Using state cookie '$COOKIE_ID'. Home: $COOKIE_HOME"
  mkdir -p "$COOKIE_HOME"
  BWRAP_ARGS+=(--bind "$COOKIE_HOME" "$HOME")
else
  BWRAP_ARGS+=(--tmpfs "$HOME")
fi

BWRAP_ARGS+=(
  --bind-try /dev/shm /dev/shm
  --bind "$SANDBOX_DIR" "$SANDBOX_DIR"
  --bind "$HOST_XDG_RUNTIME_DIR/$HOST_WAYLAND_DISPLAY" "$SANDBOX_DIR/$HOST_WAYLAND_DISPLAY"
  --bind-try "$HOST_XDG_RUNTIME_DIR/pipewire-0" "$SANDBOX_DIR/pipewire-0"
  --bind-try "$HOST_XDG_RUNTIME_DIR/pulse" "$SANDBOX_DIR/pulse"
  --setenv WAYLAND_DISPLAY "$HOST_WAYLAND_DISPLAY"
  --setenv XDG_RUNTIME_DIR "$SANDBOX_DIR"
  --setenv PULSE_SERVER "unix:$SANDBOX_DIR/pulse/native"
  --setenv HIER_HOST_TRANSFORM "${HIER_HOST_TRANSFORM:-Normal}"
  --unshare-all
)

if [ "$NET_MODE" = "host" ]; then
  BWRAP_ARGS+=(--share-net)
elif [ "$NET_MODE" = "vpn" ]; then
  if [ -f "/var/run/netns/$NET_PARAM" ]; then
    echo "[Sandbox] Joining network namespace: $NET_PARAM"
    BWRAP_ARGS+=(--netns "/var/run/netns/$NET_PARAM")
  else
    echo "❌ Error: Network namespace '/var/run/netns/$NET_PARAM' does not exist."
    exit 1
  fi
elif [ "$NET_MODE" = "proxy" ]; then
  echo "[Sandbox] Proxy routing enabled via proxy endpoint: $NET_PARAM"
  # We share the net so we can reach the host proxy, but set proxy env vars
  BWRAP_ARGS+=(
    --share-net
    --setenv ALL_PROXY "$NET_PARAM"
    --setenv http_proxy "$NET_PARAM"
    --setenv https_proxy "$NET_PARAM"
  )
else
  echo "[Sandbox] Isolated network mode enabled (no network access)."
fi


if [ "$SOFTWARE_MODE" -eq 1 ] || [ -n "$LIBGL_ALWAYS_SOFTWARE" ]; then
  echo "[Sandbox] Software rendering mode (llvmpipe) FORCED."
  BWRAP_ARGS+=(--setenv LIBGL_ALWAYS_SOFTWARE 1)
else
  echo "[Sandbox] Hardware GPU/DRI acceleration enabled."
  echo "[Sandbox] Tip: If GPU driver errors occur inside sandbox (e.g. libEGL / VK_ERROR), restart with --software"
  BWRAP_ARGS+=(--dev-bind-try /dev/dri /dev/dri)
fi

bwrap "${BWRAP_ARGS[@]}" "${TARGET_CMD[@]}"

