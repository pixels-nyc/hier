#!/usr/bin/env python3
# z_axis_demo.py
# Standalone test/demo for the specific Z-axis scrolling effect and fullscreen replacement.

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
        log_name = f"/tmp/hier-z-axis-demo.log"
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
    print("=== Launching Standalone Z-Axis Demo ('z_axis_demo.py') ===")
    
    # 1. Compile compositor
    print("[*] Building Hier compositor...")
    subprocess.run(["cargo", "build"], check=True)
    
    # 2. Focus monitor
    primary_display = get_primary_display_name()
    print(f"[*] Focus primary display: {primary_display}")
    subprocess.run(["niri", "msg", "action", "focus-monitor", primary_display], check=False)
    time.sleep(0.5)

    # 3. Spawn compositor
    host_transform = get_active_output_transform()
    comp_proc, display = spawn_compositor(host_transform=host_transform)
    print(f"✅ Compositor started on display: {display}")
    socket_path = f"/tmp/hier-ctrl-{display}.sock"
    time.sleep(2.0)
    
    # 4. Spawn 2 terminal windows with specific text contents
    print("[*] Spawning terminal windows (WINDOW_1.txt & WINDOW_2.txt)...")
    env_clients = os.environ.copy()
    env_clients["WAYLAND_DISPLAY"] = display
    env_clients["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    term1 = subprocess.Popen(get_terminal_cmd("WINDOW_1"), env=env_clients)
    time.sleep(2.0)
    term2 = subprocess.Popen(get_terminal_cmd("WINDOW_2"), env=env_clients)
    time.sleep(3.0)

    # Print initial layout
    layout_init = send_cmd(socket_path, "get_layout_compact")
    print(f"\n[*] Initial side-by-side layout:\n{layout_init.strip()}")
    
    # Focus left window
    print("\n[*] Focusing left window and merging columns into a Tab Stack...")
    send_cmd(socket_path, "action focus-left")
    time.sleep(0.5)
    
    # Merge windows into a tab stack
    send_cmd(socket_path, "action toggle-tab")
    time.sleep(1.5)
    
    # Print layout after merging
    layout_stacked = send_cmd(socket_path, "get_layout_compact")
    print(f"\n[*] Layout after merge (Tab Stack):\n{layout_stacked.strip()}")
    
    # Verify window geometry is fullscreen
    lines = [l for l in layout_stacked.strip().split("\n") if l]
    if len(lines) < 2:
        print("❌ Error: Both windows did not map properly in the layout.")
        term1.terminate()
        term2.terminate()
        comp_proc.terminate()
        sys.exit(1)
        
    camera_res = send_cmd(socket_path, "get_camera").strip()
    cam_parts = camera_res.split(",")
    viewport_w = float(cam_parts[4])
    viewport_h = float(cam_parts[5])
    
    success = False
    for line in lines:
        parts = line.split(":")
        if len(parts) >= 5 and parts[3].lower() == "true":
            rect_str = parts[4]
            rect = [float(val) for val in rect_str.split(",")]
            print(f"\nActive tab window dimensions: {rect[2]}x{rect[3]} (at position x={rect[0]}, y={rect[1]})")
            print(f"Viewport dimensions: {viewport_w}x{viewport_h}")
            
            if abs(rect[2] - viewport_w) <= 20.0 and abs(rect[3] - viewport_h) <= 20.0:
                print("✅ Success: Window successfully expanded to full screen viewport width and height!")
                success = True
            else:
                print("❌ Error: Window dimensions do not match viewport.")
                
    # 5. Cycle tab stack using Z-axis scroll
    focused_before = None
    for line in lines:
        parts = line.split(":")
        if len(parts) >= 4 and parts[3].lower() == "true":
            focused_before = parts[2]
            break
            
    print(f"\n[*] Sending simulated mouse wheel scroll (Z-axis scroll) to cycle tabs...")
    send_cmd(socket_path, "pointer_axis_z 1.0")
    time.sleep(2.0)
    
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
        print("✅ Success: Z-axis scroll command successfully cycled visible window in the tab stack!")
    else:
        print("❌ Error: Focus did not cycle.")
        success = False
        
    # 6. Cycle tab back to original
    print("\n[*] Sending Z-Scroll Up to cycle back...")
    send_cmd(socket_path, "pointer_axis_z -1.0")
    time.sleep(2.0)
    
    layout_restored = send_cmd(socket_path, "get_layout_compact")
    lines_restored = [l for l in layout_restored.strip().split("\n") if l]
    
    focused_restored = None
    for line in lines_restored:
        parts = line.split(":")
        if len(parts) >= 4 and parts[3].lower() == "true":
            focused_restored = parts[2]
            break
            
    print(f"Focused window after restoring scroll: ID {focused_restored}")
    
    if focused_restored == focused_before:
        print("✅ Success: Successfully restored original focused window!")
    else:
        print("❌ Error: Failed to restore original focus.")
        success = False
        
    # Cleanup
    print("\n[*] Cleaning up demo processes...")
    term1.terminate()
    term2.terminate()
    comp_proc.terminate()
    term1.wait()
    term2.wait()
    comp_proc.wait()
    
    if success:
        print("\n🎉 Z-AXIS DEMO AND FULLSCREEN REPLACEMENT CHECKS ALL PASSED!")
        sys.exit(0)
    else:
        print("\n🚨 DEMO CHECKS FAILED.")
        sys.exit(1)

if __name__ == "__main__":
    main()
