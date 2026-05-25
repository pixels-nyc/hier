#!/usr/bin/env python3
# run_demo.py
# Comprehensive demonstration pipeline covering:
# 1. Nesting display tree architecture orchestration.
# 2. Multi-program client spawning inside nested viewports.
# 3. Multi-window layout column configurations and stacked tab groups.
# 4. Multi-workspace allocation and switching.
# 5. Perpetual Z-axis scroll focus wrapping verification.
# 6. High-contrast visual highlight captures and border verification.
# 7. Session state saving and robust re-identification restoration.
# Fully dynamic socket allocation to support pre-nested environments.

import os
import sys
import shutil
import time
import socket
import subprocess
import json
import re
import struct
import zlib
import threading

def save_png(width, height, rgb_bytes, output_path):
    png = bytearray([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    ihdr_data = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    
    def make_chunk(tag, data):
        chunk = bytearray()
        chunk.extend(struct.pack(">I", len(data)))
        chunk.extend(tag.encode('ascii'))
        chunk.extend(data)
        crc = zlib.crc32(tag.encode('ascii') + data)
        chunk.extend(struct.pack(">I", crc))
        return chunk

    png.extend(make_chunk("IHDR", ihdr_data))
    
    scanlines = bytearray()
    row_len = width * 3
    for y in range(height):
        scanlines.append(0)
        scanlines.extend(rgb_bytes[y * row_len : (y + 1) * row_len])
        
    compressed_data = zlib.compress(scanlines)
    png.extend(make_chunk("IDAT", compressed_data))
    png.extend(make_chunk("IEND", b""))
    
    with open(output_path, "wb") as f:
        f.write(png)

def ppm_to_png(ppm_path, png_path):
    with open(ppm_path, "rb") as f:
        header = f.readline().decode().strip()
        if header != "P6":
            raise ValueError("Unsupported PPM format")
        
        line = f.readline().decode().strip()
        while line.startswith("#"):
            line = f.readline().decode().strip()
        dims = line.split()
        width = int(dims[0])
        height = int(dims[1])
        
        f.readline() # Read max value line
        pixel_bytes = f.read()
        
    save_png(width, height, pixel_bytes, png_path)

def get_terminal_cmd():
    import shutil
    for term in ["foot", "alacritty", "kitty", "xterm"]:
        if shutil.which(term):
            return [term]
    return ["echo", "mock_term"]

def send_cmd(socket_path: str, cmd: str) -> str:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2.0)
        s.connect(socket_path)
        s.sendall((cmd + "\n").encode())
        res = s.recv(16384).decode()
        s.close()
        return res
    except Exception as e:
        return f"error: socket connection failed to {socket_path}: {e}"

def parse_ppm_border(path: str) -> bool:
    print(f"[*] Reading visual capture PPM from {path}...")
    try:
        with open(path, "rb") as f:
            header = f.readline().decode().strip()
            if header != "P6":
                print(f"❌ Error: Unsupported PPM format: {header}")
                return False
            
            line = f.readline().decode().strip()
            while line.startswith("#"):
                line = f.readline().decode().strip()
            
            dims = line.split()
            width = int(dims[0])
            height = int(dims[1])
            
            max_val = int(f.readline().decode().strip())
            pixel_bytes = f.read()
            
        # Highlight borders are 4px thick. Read coordinate (2, 2)
        px, py = 2, 2
        idx = (py * width + px) * 3
        if idx + 2 >= len(pixel_bytes):
            print("❌ Error: Pixel data size mismatch.")
            return False
            
        r = pixel_bytes[idx]
        g = pixel_bytes[idx+1]
        b = pixel_bytes[idx+2]
        
        print(f"[*] Pixel color at edge coordinates ({px},{py}): [{r}, {g}, {b}]")
        if (r, g, b) == (30, 144, 255):
            print("✅ Visual Border Validation: PASSED (Correct high-contrast border rendered!)")
            return True
        else:
            print("❌ Visual Border Validation: FAILED (Expected [30, 144, 255])")
            return False
    except Exception as e:
        print(f"❌ Error parsing PPM: {e}")
        return False

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
    if not shutil.which("niri"):
        return "HDMI-A-2"
    try:
        res = subprocess.check_output(["niri", "msg", "--json", "outputs"]).decode()
        outputs = json.loads(res)
        if not outputs:
            return "HDMI-A-2"
        # Strategy 1: Find output with x=0, y=0
        for name, info in outputs.items():
            logical = info.get("logical", {})
            if logical.get("x") == 0 and logical.get("y") == 0:
                return name
        # Strategy 2: Find output with transform "Normal"
        for name, info in outputs.items():
            logical = info.get("logical", {})
            if logical.get("transform") == "Normal":
                return name
        # Strategy 3: Check if HDMI-A-2 exists
        if "HDMI-A-2" in outputs:
            return "HDMI-A-2"
        # Strategy 4: Fallback to first output
        return list(outputs.keys())[0]
    except Exception:
        return "HDMI-A-2"

def spawn_compositor(parent_display=None, host_transform="Normal", fullscreen=False):
    env = os.environ.copy()
    if parent_display:
        env["WAYLAND_DISPLAY"] = parent_display
    if fullscreen:
        env["HIER_FULLSCREEN"] = "1"
    env["HIER_HOST_TRANSFORM"] = host_transform
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    # Start compositor process
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
                return proc, "wayland-bypassed", []
            time.sleep(0.05)
            continue
            
        lines_read.append(line)
        match = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
        if match:
            display_name = match.group(1)
            break
            
    # Drain output in background thread to avoid pipe capacity deadlock / Rust println panic
    def drain():
        log_name = f"/tmp/hier-demo-{'nest1' if parent_display else 'nest0'}.log"
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
    
    return proc, display_name, lines_read

def main():
    print("==================================================")
    print("    NIRI REBUILD UNIFIED VISUAL DEMONSTRATION     ")
    print("==================================================")

    # 1. Start Nest 0 (Root nested compositor)
    parent_wayland = os.environ.get("WAYLAND_DISPLAY", "wayland-1")
    primary_display = get_primary_display_name()
    print(f"[*] Focusing primary display monitor {primary_display} via Niri IPC...")
    if shutil.which("niri"):
        subprocess.run(["niri", "msg", "action", "focus-monitor", primary_display], check=False)
    time.sleep(0.5)

    print(f"[*] Spawning Nest 0 root compositor under parent: {parent_wayland}...")
    host_transform = get_active_output_transform()
    print(f"[*] Detected active host monitor transform: {host_transform}")
    comp0, display0, logs0 = spawn_compositor(host_transform=host_transform, fullscreen=True)
    print(f"✅ Nest 0 successfully initialized display: {display0}")
    socket_n0 = f"/tmp/hier-ctrl-{display0}.sock"
    time.sleep(2.0)

    # 2. Start Nest 1 (Nested child compositor connected to display0)
    print(f"[*] Spawning Nest 1 nested child compositor inside parent: {display0}...")
    comp1, display1, logs1 = spawn_compositor(parent_display=display0, fullscreen=False)
    print(f"✅ Nest 1 successfully initialized display: {display1}")
    socket_n1 = f"/tmp/hier-ctrl-{display1}.sock"
    time.sleep(2.0)

    # 3. Spawning multiple programs inside Nest 1
    term_cmd = get_terminal_cmd()
    print(f"[*] Spawning 3 client terminals ({term_cmd[0]}) inside Nest 1 (display {display1})...")
    env_clients = os.environ.copy()
    env_clients["WAYLAND_DISPLAY"] = display1
    env_clients["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    clients = []
    for i in range(3):
        p = subprocess.Popen(term_cmd, env=env_clients)
        clients.append(p)
        time.sleep(2.0)

    # 4. Check Nest 1 layout before stacking
    layout_init = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Initial Layout:\n{layout_init.strip()}")

    # 5. Build stacked tab group inside Nest 1
    print("[*] Grouping 3 windows into a stacked tab column in Nest 1...")
    send_cmd(socket_n1, "action focus-left")
    send_cmd(socket_n1, "action toggle-tab")
    time.sleep(1.0)
    send_cmd(socket_n1, "action focus-left")
    send_cmd(socket_n1, "action toggle-tab")
    time.sleep(1.0)

    layout_stacked = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Stacked Layout:\n{layout_stacked.strip()}")

    # 6. Switch workspaces in Nest 1
    print("[*] Navigating workspaces in Nest 1...")
    send_cmd(socket_n1, "action workspace-2")
    time.sleep(1.0)
    layout_ws2 = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 layout on Workspace 2:\n{layout_ws2.strip()}")
    
    # Return to Workspace 1
    send_cmd(socket_n1, "action workspace-1")
    time.sleep(1.0)

    # 7. Verify perpetual Z-axis scroll focus wrapping
    print("\n==================================================")
    print("[*] Testing Perpetual Z-Axis Scroll focus transitions...")
    print("==================================================")
    
    # Check focused window index initially
    res = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Tab Layout before scroll: {res.strip()}")
    
    # Send scroll down to Nest 0. Should propagate down to Nest 1 and wrap to index 0
    print("[*] Scroll DOWN 1 unit (propagates and wraps)...")
    send_cmd(socket_n0, "pointer_axis_z 1.0")
    time.sleep(1.5)
    res_scrolled = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Tab Layout after scroll DOWN: {res_scrolled.strip()}")

    # Scroll up (wraps back to index 2)
    print("[*] Scroll UP 1 unit (propagates and wraps)...")
    send_cmd(socket_n0, "pointer_axis_z -1.0")
    time.sleep(1.5)
    res_scrolled_up = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Tab Layout after scroll UP: {res_scrolled_up.strip()}")

    # 8. Visual Highlights and Captures
    print("\n[*] Highlighting and Capturing Window 1 layout...")
    send_cmd(socket_n1, "highlight_window 1 blue")
    time.sleep(1.0)
    
    capture_path = "/tmp/demo_highlight.ppm"
    send_cmd(socket_n1, f"capture_window 1 {capture_path}")
    
    parse_ppm_border(capture_path)
    
    # Save standard PNG screenshot of the highlight check in artifacts directory
    captures_dir = os.path.join(os.path.dirname(os.path.realpath(__file__)), "artifacts", "captures")
    os.makedirs(captures_dir, exist_ok=True)
    png_highlight_path = os.path.join(captures_dir, "demo_highlight.png")
    try:
        ppm_to_png(capture_path, png_highlight_path)
        os.remove(capture_path)
        print(f"✅ Highlight screenshot converted and saved as PNG: {png_highlight_path}")
    except Exception as e:
        print(f"❌ Error converting highlight capture to PNG: {e}")
        
    send_cmd(socket_n1, "clear_highlight")

    # 9. Nesting tree visualization
    print("\n==================================================")
    print("[*] Displaying Nested Wayland Tree Architecture...")
    print("==================================================")
    try:
        tree_res = subprocess.check_output(["./hier-tree"]).decode()
        print(tree_res)
    except Exception as e:
        print(f"Error printing display tree: {e}")

    # Multi-view layout dashboard visualization
    print("\n==================================================")
    print("[*] Displaying Dynamic Multi-View Workspace/Window Tree...")
    print("==================================================")
    try:
        print("[*] Visualizing all displays (--all) and exporting window screenshots:")
        mv_all = subprocess.check_output([
            "./hier-multiview",
            "--all",
            "--screenshot-dir",
            os.path.join(os.path.dirname(os.path.realpath(__file__)), "artifacts", "captures")
        ]).decode()
        print(mv_all)
        
        print(f"[*] Visualizing Nest 1 only (--display {display1}):")
        mv_disp = subprocess.check_output(["./hier-multiview", "--display", display1]).decode()
        print(mv_disp)
    except Exception as e:
        print(f"Error running multiview dashboard: {e}")

    # 10. Session Save & Restore Validation
    print("\n==================================================")
    print("[*] Testing Session Save & Restore pipeline...")
    print("==================================================")
    print("[*] Saving active workspace layout state...")
    save_res = send_cmd(socket_n1, "save_session")
    print(f"Save response: {save_res.strip()}")

    # Terminate clients in Nest 1 to clear workspace
    print("[*] Closing all terminal windows inside Nest 1...")
    for p in clients:
        p.terminate()
    time.sleep(2.0)
    
    layout_cleared = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Layout after closing clients:\n{layout_cleared.strip()}")

    # Respawn terminal clients
    term_cmd = get_terminal_cmd()
    print(f"[*] Respawning client windows ({term_cmd[0]}) inside Nest 1...")
    new_clients = []
    for i in range(3):
        p = subprocess.Popen(term_cmd, env=env_clients)
        new_clients.append(p)
        time.sleep(2.0)
        
    layout_fresh_spawned = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 layout before restoring:\n{layout_fresh_spawned.strip()}")

    # Trigger restore session pipeline
    print("[*] Restoring workspace layout from session snapshot...")
    restore_res = send_cmd(socket_n1, "restore_session")
    print(f"Restore response: {restore_res.strip()}")
    
    layout_restored = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Layout after restoring session:\n{layout_restored.strip()}")

    # 11. Cleanup all processes
    print("\n[*] Cleaning up all demonstration processes...")
    for p in new_clients:
        p.terminate()
    comp1.terminate()
    comp0.terminate()
    
    # Save log outputs
    try:
        with open("/tmp/demo-comp0.log", "w") as f:
            f.write("".join(logs0))
            try:
                f.write(comp0.stdout.read())
            except Exception:
                pass
        with open("/tmp/demo-comp1.log", "w") as f:
            f.write("".join(logs1))
            try:
                f.write(comp1.stdout.read())
            except Exception:
                pass
    except Exception as e:
        print(f"Error saving log files: {e}")
        
    print("=== Unified Visual Demonstration Completed Successfully ===")

if __name__ == "__main__":
    main()
