#!/usr/bin/env python3
# test_fuzzel.py

import os
import time
import socket
import subprocess
import re
import sys
import threading

def send_cmd(socket_path: str, cmd: str) -> str:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2.0)
        s.connect(socket_path)
        s.sendall((cmd + "\n").encode())
        res = s.recv(4096).decode()
        s.close()
        return res
    except Exception as e:
        return f"error: {e}"

def main():
    print("=== Launching Fuzzel Overlay Geometry Test ===")
    
    # 1. Build compositor
    subprocess.run(["cargo", "build"], check=True)
    
    # 2. Launch nested compositor
    env = os.environ.copy()
    env["HIER_FULLSCREEN"] = "1"
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    comp_proc = subprocess.Popen(
        ["./target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=env,
        text=True
    )
    
    # 3. Read display socket name
    display = None
    start = time.time()
    while time.time() - start < 10.0:
        line = comp_proc.stdout.readline()
        match = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
        if match:
            display = match.group(1)
            break
    
    if not display:
        print("❌ Error: Compositor failed to start.")
        comp_proc.terminate()
        sys.exit(1)
        
    print(f"✅ Compositor started on display: {display}")
    
    # Background thread to drain compositor stdout and prevent deadlocks
    def drain():
        try:
            while True:
                line = comp_proc.stdout.readline()
                if not line:
                    if comp_proc.poll() is not None:
                        break
                    time.sleep(0.1)
                    continue
        except Exception:
            pass
            
    t = threading.Thread(target=drain, daemon=True)
    t.start()

    socket_path = f"/tmp/hier-ctrl-{display}.sock"
    time.sleep(2.0)
    
    # 4. Launch a client with "fuzzel" in the title
    # We set window title to "fuzzel" to trigger overlay layout engine classification.
    # We use "-e sleep 10" to prevent the shell from overriding the window title.
    client_env = os.environ.copy()
    client_env["WAYLAND_DISPLAY"] = display
    client_env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    client_proc = subprocess.Popen(
        ["alacritty", "-T", "fuzzel-menu-runner", "-e", "sleep", "10"], 
        env=client_env
    )
    time.sleep(3.0) # Allow window mapping
    
    # 5. Query layout compact
    layout = send_cmd(socket_path, "get_layout_compact")
    print(f"[*] Layout telemetry:\n{layout.strip()}")
    
    # 6. Parse and assert overlay bounds
    success = False
    for line in layout.strip().split("\n"):
        if "fuzzel" in line.lower():
            # Example compact line: 0:0:1:true:710,340,500,400:fuzzel-menu-runner
            parts = line.split(":")
            rect_str = parts[4]
            x, y, w, h = [float(val) for val in rect_str.split(",")]
            
            print(f"[*] Detected Fuzzel geometry: x={x}, y={y}, w={w}, h={h}")
            
            # Assert overlay constraints
            if w == 500.0 and h == 400.0:
                print("✅ Success: Fuzzel width is 500px and height is 400px!")
                success = True
            else:
                print(f"❌ Failure: Unexpected dimensions: {w}x{h}")
                
    # Cleanup
    client_proc.terminate()
    comp_proc.terminate()
    client_proc.wait()
    comp_proc.wait()
    
    if success:
        sys.exit(0)
    else:
        sys.exit(1)

if __name__ == "__main__":
    main()
