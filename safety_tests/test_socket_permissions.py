#!/usr/bin/env python3
# safety_tests/test_socket_permissions.py
# Verification of control socket filesystem permission boundaries

import os
import sys
import stat
import glob
import subprocess
import time

def check_socket_permissions(path):
    print(f"[*] Inspecting control socket: {path}")
    try:
        st = os.stat(path)
    except OSError as e:
        print(f"❌ Failed to stat socket: {e}")
        return False

    # Get permission mode bits
    mode = st.st_mode
    is_socket = stat.S_ISSOCK(mode)
    
    if not is_socket:
        print(f"❌ Error: Path {path} is not a Unix domain socket.")
        return False

    # Extract permission bits
    perms = stat.S_IMODE(mode)
    print(f"[*] File permissions mode: {oct(perms)} (raw: {bin(perms)})")

    # Security check: Group and Other permissions must be completely empty (0)
    # The owner can have read/write (0o600) or read/write/execute (0o700)
    group_perms = (perms >> 3) & 0o7
    other_perms = perms & 0o7

    owner_read = bool(perms & stat.S_IRUSR)
    owner_write = bool(perms & stat.S_IWUSR)

    if group_perms != 0 or other_perms != 0:
        print(f"❌ SECURITY VULNERABILITY DETECTED!")
        print(f"   Socket permissions allow unauthorized access:")
        print(f"   Group permissions: {oct(group_perms)}")
        print(f"   Other permissions: {oct(other_perms)}")
        print(f"   Any local user on the machine could connect and hijack the session.")
        return False

    if not (owner_read and owner_write):
        print(f"⚠️ Warning: Socket owner lacks read or write permissions ({oct(perms)}).")
    
    # Path warning: check if socket is in public /tmp directory vs private runtime dir
    if path.startswith("/tmp/"):
        print(f"⚠️ Security Warning: Socket is located in public /tmp directory.")
        print(f"   Although file permissions are secure, hosting control sockets in /tmp increases")
        print(f"   the risk of socket hijacking or denial-of-service via socket path pre-emption.")
        print(f"   Recommended: Move socket to $XDG_RUNTIME_DIR (e.g. /run/user/1000/).")

    print("✅ Control socket permission validation: PASSED.")
    return True

def main():
    print("==================================================")
    print("  SECURITY TEST: CONTROL SOCKET BOUNDARY CHECKS   ")
    print("==================================================")

    # 1. Search for active sockets
    sockets = glob.glob("/tmp/hier-ctrl-wayland-*.sock")
    if not sockets:
        print("[*] No active compositor control sockets found in /tmp. Spawning a temporary instance...")
        
        env = os.environ.copy()
        env["LIBGL_ALWAYS_SOFTWARE"] = "1"
        
        try:
            proc = subprocess.Popen(
                ["target/debug/hier"],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                env=env,
                text=True
            )
            
            # Wait for socket path output
            socket_path = None
            start_time = time.time()
            while time.time() - start_time < 5.0:
                line = proc.stdout.readline()
                if "Control socket listening at:" in line:
                    socket_path = line.split("Control socket listening at:")[-1].strip()
                    break
                time.sleep(0.05)
            
            if not socket_path:
                # Fallback to searching
                time.sleep(1.5)
                sockets = glob.glob("/tmp/hier-ctrl-wayland-*.sock")
                if sockets:
                    socket_path = sockets[0]

            if socket_path and os.path.exists(socket_path):
                success = check_socket_permissions(socket_path)
                proc.terminate()
                proc.wait()
                if not success:
                    sys.exit(1)
            else:
                print("❌ Failed to start temporary compositor or find created control socket.")
                proc.terminate()
                sys.exit(1)

        except Exception as e:
            print(f"❌ Error spawning compositor: {e}")
            sys.exit(1)
    else:
        success = True
        for sock in sockets:
            if not check_socket_permissions(sock):
                success = False
        if not success:
            sys.exit(1)

if __name__ == "__main__":
    main()
