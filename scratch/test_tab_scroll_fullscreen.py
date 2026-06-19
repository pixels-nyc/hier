#!/usr/bin/env python3
# scratch/test_tab_scroll_fullscreen.py
# Verify tab stack "Fullscreen Replacement" effect and scroll cycle.

import os
import socket
import sys
import time
import subprocess
import re
import json

def get_active_output_transform() -> str:
    try:
        res_ws = subprocess.check_output(["niri", "msg", "--json", "workspaces"]).decode()
        workspaces = json.loads(res_ws)
        active_output = None
        for ws in workspaces:
            if ws.get("is_focused") or ws.get("is_active"):
                active_output = ws.get("output")
                if ws.get("is_focused"):
                    break
        if not active_output:
            return "Normal"
        res_outs = subprocess.check_output(["niri", "msg", "--json", "outputs"]).decode()
        outputs = json.loads(res_outs)
        output = outputs.get(active_output)
        if output:
            return output["logical"].get("transform", "Normal")
    except Exception:
        pass
    return "Normal"

def get_primary_display_name() -> str:
    try:
        res = subprocess.check_output(["niri", "msg", "--json", "outputs"]).decode()
        outputs = json.loads(res)
        if not outputs:
            return "HDMI-A-2"
        for name, info in outputs.items():
            logical = info.get("logical", {})
            if logical.get("x") == 0 and logical.get("y") == 0:
                return name
        for name, info in outputs.items():
            logical = info.get("logical", {})
            if logical.get("transform") == "Normal":
                return name
        if "HDMI-A-2" in outputs:
            return "HDMI-A-2"
        return list(outputs.keys())[0]
    except Exception:
        return "HDMI-A-2"

def get_terminal_cmd(role="GENERIC"):
    import shutil
    term = "foot"
    if not shutil.which(term):
        term = "alacritty"
        if not shutil.which(term):
            term = "xterm"
            
    info_file = f"/home/super/Work/rust-based-dev/niri-rebuild/scratch/{role}.txt"
    if not os.path.exists(info_file):
        info_file = "/home/super/Work/rust-based-dev/niri-rebuild/scratch/GENERIC.txt"
        
    if term == "foot":
        return ["foot", "sh", "-c", f"cat {info_file}; exec $SHELL"]
    elif term == "alacritty":
        return ["alacritty", "-e", "sh", "-c", f"cat {info_file}; exec $SHELL"]
    else:
        return [term, "-e", "sh", "-c", f"cat {info_file}; exec $SHELL"]

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
        print(f"Socket connection failed: {e}")
        return ""

def spawn_compositor(host_transform="Normal"):
    env = os.environ.copy()
    env["HIER_FULLSCREEN"] = "1"
    env["HIER_HOST_TRANSFORM"] = host_transform
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    proc = subprocess.Popen(
        ["target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=env,
        text=True
    )
    
    display_name = None
    start_time = time.time()
    lines_read = []
    while True:
        if time.time() - start_time > 15.0:
            print("❌ Error: Timeout waiting for compositor initialization.")
            proc.terminate()
            sys.exit(1)
            
        line = proc.stdout.readline()
        if not line:
            ret = proc.poll()
            if ret is not None:
                print(f"❌ Error: Compositor exited early with code {ret}")
                sys.exit(1)
            time.sleep(0.05)
            continue
            
        lines_read.append(line)
        match = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
        if match:
            display_name = match.group(1)
            break
            
    import threading
    def drain():
        log_name = f"/tmp/hier-tab-scroll-test.log"
        try:
            with open(log_name, "w", buffering=1) as f:
                for l in lines_read:
                    f.write(l)
                f.flush()
                while True:
                    line = proc.stdout.readline()
                    if not line:
                        if proc.poll() is not None:
                            break
                        time.sleep(0.1)
                        continue
                    f.write(line)
                    f.flush()
        except Exception:
            pass
            
    t = threading.Thread(target=drain, daemon=True)
    t.start()
    
    return proc, display_name

def main():
    print("=== Starting Custom Tab Scroll & Fullscreen Replacement Test ===")
    
    # Rebuild compositor
    subprocess.run(["cargo", "build"], check=True)
    
    # Focus primary display monitor
    primary_display = get_primary_display_name()
    print(f"[*] Focusing primary display monitor {primary_display} via Niri IPC...")
    subprocess.run(["niri", "msg", "action", "focus-monitor", primary_display], check=False)
    time.sleep(0.5)

    host_transform = get_active_output_transform()
    comp_proc, display = spawn_compositor(host_transform=host_transform)
    print(f"✅ Compositor started on display: {display}")
    socket_path = f"/tmp/hier-ctrl-{display}.sock"
    time.sleep(2.0)
    
    # Spawn two terminals displaying WINDOW_1.txt and WINDOW_2.txt
    print("[*] Spawning terminal windows...")
    env_clients = os.environ.copy()
    env_clients["WAYLAND_DISPLAY"] = display
    env_clients["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    term1 = subprocess.Popen(get_terminal_cmd("WINDOW_1"), env=env_clients)
    time.sleep(2.0)
    term2 = subprocess.Popen(get_terminal_cmd("WINDOW_2"), env=env_clients)
    time.sleep(3.0)

    # Inspect initial side-by-side tiling layout
    layout_init = send_cmd(socket_path, "get_layout_compact")
    print(f"[*] Initial Tiled Layout:\n{layout_init.strip()}")
    
    # Focus the left column (w1)
    print("[*] Focusing first window and toggling tab stack...")
    send_cmd(socket_path, "action focus-left")
    time.sleep(0.5)
    
    # Merge them into a tab stack (simulates Logo+c via toggle-tab action)
    send_cmd(socket_path, "action toggle-tab")
    time.sleep(1.0)
    
    # Get layout after tab stacking
    layout_stacked = send_cmd(socket_path, "get_layout_compact")
    print(f"[*] Stacked Tab Layout:\n{layout_stacked.strip()}")
    
    # Check window geometry to verify Fullscreen Replacement (x = 10, w = 1900 or similar based on monitor viewport size)
    lines = [l for l in layout_stacked.strip().split("\n") if l]
    assert len(lines) >= 2, "Failed to get mapped windows in tab stack"
    
    # Parse bounds of active window in tab stack
    # Example format: 0:0:2:true:20,20,960,1040:Smithay
    # Let's inspect the active window coordinates. Since it is tabbed, it should fill the viewport.
    print("[*] Verifying window geometry is maximized to the viewport (Fullscreen Replacement)...")
    success = False
    for line in lines:
        parts = line.split(":")
        if len(parts) >= 5 and parts[3].lower() == "true":
            rect_str = parts[4]
            # rect_str is in form x,y,w,h
            rect = [float(val) for val in rect_str.split(",")]
            print(f"Active tab window rect: {rect}")
            
            # Fetch viewport dimensions via get_camera
            camera_res = send_cmd(socket_path, "get_camera").strip()
            print(f"Compositor Camera telemetry: {camera_res}")
            cam_parts = camera_res.split(",")
            viewport_w = float(cam_parts[4])
            viewport_h = float(cam_parts[5])
            
            # Fullscreen replacement size is viewport - 2 * outer_margin (outer_margin = 0 in run_winit_compositor)
            # In run_winit_compositor: outer_margin = 0.0
            # So expected size is viewport_w x viewport_h
            if abs(rect[2] - viewport_w) <= 20.0 and abs(rect[3] - viewport_h) <= 20.0:
                print("✅ Success: Tab stack window occupies the full viewport dimensions!")
                success = True
            else:
                print(f"❌ Error: Window width {rect[2]} or height {rect[3]} does not match viewport {viewport_w}x{viewport_h}")
    
    # 6. Test scrolling to cycle tabs
    print("[*] Testing tab scroll cycling...")
    # Get ID of the currently focused window before scroll
    focused_before = None
    for line in lines:
        parts = line.split(":")
        if len(parts) >= 4 and parts[3].lower() == "true":
            focused_before = parts[2]
            break
            
    print(f"Focused window before scroll: ID {focused_before}")
    
    # Send pointer axis Z scroll (simulates mouse wheel scroll on tabbed column)
    send_cmd(socket_path, "pointer_axis_z 1.0")
    time.sleep(1.5)
    
    layout_after = send_cmd(socket_path, "get_layout_compact")
    lines_after = [l for l in layout_after.strip().split("\n") if l]
    
    focused_after = None
    for line in lines_after:
        parts = line.split(":")
        if len(parts) >= 4 and parts[3].lower() == "true":
            focused_after = parts[2]
            break
            
    print(f"Focused window after scroll: ID {focused_after}")
    
    if focused_before != focused_after and focused_after is not None:
        print("✅ Success: Mouse wheel Z-scroll successfully cycled window focus in the tab stack!")
    else:
        print("❌ Error: Mouse wheel scroll did not cycle window focus in the tab stack.")
        success = False
        
    # Cleanup
    print("[*] Cleaning up processes...")
    term1.terminate()
    term2.terminate()
    comp_proc.terminate()
    
    if success:
        print("🎉 ALL VERIFICATION CHECKS PASSED!")
        sys.exit(0)
    else:
        print("🚨 TEST FAILED.")
        sys.exit(1)

if __name__ == "__main__":
    main()
