#!/usr/bin/env python3
import subprocess
import socket
import time
import os
import sys
import json
import re

def send_cmd(socket_path: str, cmd: str) -> str:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(socket_path)
    s.sendall((cmd + "\n").encode())
    res = s.recv(4096).decode()
    s.close()
    return res

def main():
    print("[*] Starting integration test for extended camera & tiling mode commands...")
    
    # Start compositor in software rendering mode
    env = os.environ.copy()
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    # We run the compositor target/debug/hier
    proc = subprocess.Popen(
        ["target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
        text=True
    )
    
    display_name = None
    socket_path = None
    
    try:
        # Parse display and socket info
        start_time = time.time()
        while time.time() - start_time < 5.0:
            line = proc.stdout.readline()
            if not line:
                time.sleep(0.05)
                continue
            print(f"[Compositor Output] {line.strip()}")
            match_display = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
            if match_display:
                display_name = match_display.group(1)
            match_socket = re.search(r"Control socket listening at: (/tmp/hier-ctrl-wayland-\d+\.sock)", line)
            if match_socket:
                socket_path = match_socket.group(1)
            if display_name and socket_path:
                break
                
        if not display_name or not socket_path:
            print("❌ Failed to parse display/socket path from compositor startup.")
            proc.terminate()
            sys.exit(1)
            
        print(f"✅ Compositor started: Display={display_name}, Socket={socket_path}")
        time.sleep(1.0)
        
        # Test 1: get_camera
        res = send_cmd(socket_path, "get_camera").strip()
        print(f"[Test 1] get_camera output: {res}")
        parts = res.split(",")
        assert len(parts) == 7, "get_camera output should have 7 parts"
        assert parts[6] == "Grid", f"Initial tiling mode should be Grid, got: {parts[6]}"
        print("✅ Test 1 Passed: get_camera returns correct structure and initial mode.")
        
        # Test 2: set_camera immediate
        res = send_cmd(socket_path, "set_camera 150 250 true").strip()
        print(f"[Test 2] set_camera output: {res}")
        assert res == "ok", f"Expected set_camera to return ok, got: {res}"
        
        res = send_cmd(socket_path, "get_camera").strip()
        print(f"[Test 2] get_camera after set_camera: {res}")
        parts = res.split(",")
        assert float(parts[0]) == 150.0, f"Expected X coordinate to be 150.0, got: {parts[0]}"
        assert float(parts[1]) == 250.0, f"Expected Y coordinate to be 250.0, got: {parts[1]}"
        print("✅ Test 2 Passed: set_camera immediate successfully moves camera.")
        
        # Test 3: change tiling mode via action
        res = send_cmd(socket_path, "action tiling-mode-depth").strip()
        print(f"[Test 3] action tiling-mode-depth: {res}")
        assert res == "ok", f"Expected ok, got: {res}"
        
        res = send_cmd(socket_path, "get_layout").strip()
        # Parse json layout to verify tiling_mode field
        layout = json.loads(res)
        print(f"[Test 3] tiling_mode in get_layout JSON: {layout.get('tiling_mode')}")
        assert layout.get("tiling_mode") == "Depth", f"Expected Depth, got: {layout.get('tiling_mode')}"
        print("✅ Test 3 Passed: get_layout JSON contains tiling_mode.")
        
        # Test 4: hier-multiview CLI --get-camera
        print("[*] Testing hier-multiview --get-camera...")
        mv_res = subprocess.check_output(["./hier-multiview", "-d", display_name, "-c"]).decode()
        print(f"[Test 4] hier-multiview -c output:\n{mv_res}")
        assert "Current Position: (0.0, 0.0)" in mv_res, "Expected X and Y to snap/recenter to 0.0 in Depth mode"
        assert "Tiling Mode:      Depth" in mv_res, "Expected tiling mode Depth"
        print("✅ Test 4 Passed: hier-multiview prints camera telemetry.")
        
        # Test 5: hier-multiview CLI --set-tiling
        print("[*] Testing hier-multiview --set-tiling overview...")
        mv_res = subprocess.check_output(["./hier-multiview", "-d", display_name, "--set-tiling", "overview"]).decode()
        print(f"[Test 5] hier-multiview --set-tiling output:\n{mv_res}")
        
        res = send_cmd(socket_path, "get_camera").strip()
        parts = res.split(",")
        assert parts[6] == "Overview", f"Expected Overview mode, got: {parts[6]}"
        print("✅ Test 5 Passed: hier-multiview --set-tiling successfully changes tiling mode.")

        # Test 6: hier-multiview CLI --set-camera
        print("[*] Testing hier-multiview --set-camera...")
        mv_res = subprocess.check_output(["./hier-multiview", "-d", display_name, "--set-camera", "300", "400", "true"]).decode()
        print(f"[Test 6] hier-multiview --set-camera output:\n{mv_res}")
        
        res = send_cmd(socket_path, "get_camera").strip()
        parts = res.split(",")
        assert float(parts[0]) == 300.0, f"Expected X coordinate to be 300.0, got: {parts[0]}"
        assert float(parts[1]) == 400.0, f"Expected Y coordinate to be 400.0, got: {parts[1]}"
        print("✅ Test 6 Passed: hier-multiview --set-camera successfully modifies camera position.")
        
    finally:
        print("[*] Cleaning up compositor process...")
        proc.terminate()
        proc.wait()
        print("[*] Compositor terminated.")

if __name__ == "__main__":
    main()
