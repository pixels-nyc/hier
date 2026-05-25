#!/usr/bin/env python3
# scratch/test_chromium_rpa.py

import os
import sys
import time
import socket
import subprocess
import re
import threading

def drain_output(stream):
    try:
        while True:
            line = stream.readline()
            if not line:
                break
            # Just consume it to prevent blocking the OS pipe buffer
    except Exception:
        pass

def send_cmd(socket_path: str, cmd: str) -> str:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(0.2)
        s.connect(socket_path)
        s.sendall((cmd + "\n").encode())
        res = ""
        while True:
            try:
                chunk = s.recv(4096).decode()
                if not chunk:
                    break
                res += chunk
            except socket.timeout:
                break
        s.close()
        return res
    except Exception as e:
        return f"error: {e}"

def main():
    print("=== Starting Chromium RPA Test Script ===")
    base_dir = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
    comp_bin = os.path.join(base_dir, "target/debug/hier")
    
    if not os.path.exists(comp_bin):
        print(f"❌ Error: Compositor binary not found at: {comp_bin}")
        print("Please run 'cargo build' first.")
        sys.exit(1)

    print("[*] Launching compositor...")
    comp_env = os.environ.copy()
    comp_env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    comp_proc = subprocess.Popen(
        [comp_bin],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=comp_env,
        cwd=base_dir,
        text=True
    )

    # Parse Wayland display name
    display_name = None
    start_time = time.time()
    while time.time() - start_time < 5.0:
        line = comp_proc.stdout.readline()
        if not line:
            time.sleep(0.05)
            continue
        match = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
        if match:
            display_name = match.group(1)
            break

    if not display_name:
        print("❌ Error: Failed to start compositor or read WAYLAND_DISPLAY.")
        comp_proc.terminate()
        sys.exit(1)

    print(f"✅ Compositor running on: {display_name}")
    
    # Start the daemon thread to drain compositor stdout and prevent deadlock
    t = threading.Thread(target=drain_output, args=(comp_proc.stdout,), daemon=True)
    t.start()

    socket_path = f"/tmp/hier-ctrl-{display_name}.sock"
    time.sleep(2.0) # allow display server socket creation

    # Launch Chromium in Wayland ozone mode
    print("[*] Launching Chromium inside nested display...")
    client_env = os.environ.copy()
    client_env["WAYLAND_DISPLAY"] = display_name
    client_env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    chromium_cmd = [
        "chromium",
        "--ozone-platform=wayland",
        "--enable-features=UseOzonePlatform",
        "--user-data-dir=/tmp/chromium-rpa-user-data",
        "--no-first-run",
        "--no-default-browser-check",
        "about:blank"
    ]
    
    client_proc = subprocess.Popen(
        chromium_cmd,
        env=client_env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )

    # Wait for Chromium window to map
    print("[*] Waiting for Chromium window mapping...")
    layout_str = ""
    win_id = None
    for _ in range(40):
        time.sleep(0.5)
        layout_str = send_cmd(socket_path, "get_layout_compact")
        if layout_str and not layout_str.startswith("error:"):
            # Check if chromium is in layout output
            if "chromium" in layout_str.lower() or "about:blank" in layout_str.lower() or "wayland window" in layout_str.lower():
                lines = layout_str.strip().split("\n")
                for line in lines:
                    if not line:
                        continue
                    parts = line.split(":", 5)
                    if len(parts) >= 3:
                        win_id = parts[2]
                break

    if not win_id:
        print(f"❌ Error: Chromium window not detected in layout. Layout: {layout_str}")
        client_proc.terminate()
        comp_proc.terminate()
        sys.exit(1)

    print(f"✅ Chromium mapped successfully! Window ID: {win_id}")
    print(f"Layout output:\n{layout_str.strip()}")

    # Highlight the window to verify layout highlight functionality
    print(f"[*] Highlighting Chromium Window {win_id}...")
    res = send_cmd(socket_path, f"highlight_window {win_id} #FF00FF")
    print(f"Highlight Response: {res.strip()}")

    # Simulate mouse motion and clicks to verify focus interaction
    print("[*] Simulating pointer clicks to focus Chromium window...")
    # Move to coordinates (100, 100) inside the window
    send_cmd(socket_path, "pointer_motion 100 100")
    time.sleep(0.1)
    
    # Send simulated button click (left button = 272)
    res_press = send_cmd(socket_path, "pointer_button 272 pressed")
    res_release = send_cmd(socket_path, "pointer_button 272 released")
    print(f"Click Response: press={res_press.strip()}, release={res_release.strip()}")

    # Verify if focus is correct
    layout_after = send_cmd(socket_path, "get_layout_compact")
    print(f"Layout after click:\n{layout_after.strip()}")

    # Clear highlight
    send_cmd(socket_path, "clear_highlight")
    
    # Cleanup
    print("[*] Cleaning up processes...")
    client_proc.terminate()
    comp_proc.terminate()
    
    # Wait for completion
    client_proc.wait()
    comp_proc.wait()
    print("🎉 Chromium RPA test completed successfully!")

if __name__ == "__main__":
    main()
