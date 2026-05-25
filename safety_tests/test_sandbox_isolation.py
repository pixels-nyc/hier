#!/usr/bin/env python3
# safety_tests/test_sandbox_isolation.py
# Verification of sandbox filesystem and socket namespace isolation

import os
import sys
import subprocess
import time
import glob

def run_isolated_sandbox_test(socket_path):
    print(f"[*] Verifying socket isolation for: {socket_path}")
    
    # We will invoke the sandbox runner to execute a python check inside the bwrap container
    # The check queries if the socket file is visible or connectable
    py_check_cmd = (
        "import os, glob, socket; "
        "socket_exists = os.path.exists('{path}'); "
        "socks = glob.glob('/tmp/hier-ctrl-*.sock'); "
        "print('SOCKETS_FOUND:' + str(socks)); "
        "print('EXISTS:' + str(socket_exists)); "
    ).format(path=socket_path)

    cmd = ["./sandbox_run.sh", "python3", "-c", py_check_cmd]
    
    # Set necessary env vars for the sandbox to boot
    env = os.environ.copy()
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    # Get display name from socket path
    # socket path is typically /tmp/hier-ctrl-wayland-X.sock
    display_name = "wayland-2"
    basename = os.path.basename(socket_path)
    if basename.startswith("hier-ctrl-") and basename.endswith(".sock"):
        display_name = basename[len("hier-ctrl-"):-len(".sock")]

    env["WAYLAND_DISPLAY"] = display_name
    
    print(f"[*] Launching sandbox checking command targeting display: {display_name}")
    try:
        proc = subprocess.run(
            cmd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=10.0
        )
        
        stdout = proc.stdout
        stderr = proc.stderr
        print(f"[*] Sandbox stdout:\n{stdout.strip()}")
        if stderr.strip():
            print(f"[*] Sandbox stderr:\n{stderr.strip()}")

        if "SOCKETS_FOUND:[]" in stdout and "EXISTS:False" in stdout:
            print("✅ Sandbox boundary verification: PASSED (Control sockets are completely isolated).")
            return True
        else:
            print("❌ Sandbox boundary verification: FAILED!")
            print("   The sandboxed application could see or interact with the host control socket.")
            return False

    except subprocess.TimeoutExpired:
        print("❌ Sandbox check timed out.")
        return False
    except Exception as e:
        print(f"❌ Error running sandbox test: {e}")
        return False

def main():
    print("==================================================")
    print("  SECURITY TEST: SANDBOX CONTAINER BOUNDARY CHECK ")
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
            
            socket_path = None
            start_time = time.time()
            while time.time() - start_time < 5.0:
                line = proc.stdout.readline()
                if "Control socket listening at:" in line:
                    socket_path = line.split("Control socket listening at:")[-1].strip()
                    break
                time.sleep(0.05)
            
            if not socket_path:
                time.sleep(1.5)
                sockets = glob.glob("/tmp/hier-ctrl-wayland-*.sock")
                if sockets:
                    socket_path = sockets[0]

            if socket_path and os.path.exists(socket_path):
                success = run_isolated_sandbox_test(socket_path)
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
            if not run_isolated_sandbox_test(sock):
                success = False
        if not success:
            sys.exit(1)

if __name__ == "__main__":
    main()
