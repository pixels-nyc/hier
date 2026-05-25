#!/usr/bin/env python3
# scratch/test_visual_antigravity.py
# Visual integration test for antigravity-legacy graphical multiwindow layout and camera control.

import os
import sys
import time
import socket
import subprocess
import re
import json
import threading

CAPTURES_DIR = "/home/super/.gemini/antigravity/brain/fdd94233-ed82-4103-bdb9-36909c0906f3"

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

def send_cmd(socket_path: str, cmd: str) -> str:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(5.0)
        s.connect(socket_path)
        s.sendall((cmd + "\n").encode())
        res = s.recv(16384).decode()
        s.close()
        return res
    except Exception as e:
        return f"error: {e}"

def spawn_compositor(host_transform="Normal"):
    env = os.environ.copy()
    env["HIER_FULLSCREEN"] = "1"
    env["HIER_HOST_TRANSFORM"] = host_transform
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"

    proc = subprocess.Popen(
        ["./target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=env,
        text=True,
        cwd=os.getcwd()
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
            
    def drain():
        log_name = "/tmp/hier-test-antigravity.log"
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
        except Exception as e:
            print(f"Error in drain thread: {e}")
                
    t = threading.Thread(target=drain, daemon=True)
    t.start()
    
    return proc, display_name

def focus_and_center_window(nest_pid):
    print("[*] Binding and focusing Nest 0 window using Niri navigation...")
    start = time.time()
    while time.time() - start < 15.0:
        try:
            res_wins = subprocess.check_output(["niri", "msg", "--json", "windows"]).decode()
            wins = json.loads(res_wins)
            
            our_win = None
            for w in wins:
                if w.get("pid") == nest_pid or w.get("title") == "Smithay":
                    our_win = w
                    break
                    
            if not our_win:
                time.sleep(0.5)
                continue
                
            win_id = our_win["id"]
            workspace_id = our_win["workspace_id"]
            
            print(f"[*] Focusing window ID {win_id} via Niri IPC...")
            subprocess.run(["niri", "msg", "action", "focus-window", "--id", str(win_id)], check=True)
            time.sleep(0.5)
            
            print("[*] Centering window column on host compositor...")
            subprocess.run(["niri", "msg", "action", "center-column"], check=False)
            time.sleep(0.5)
            
            res_ws = subprocess.check_output(["niri", "msg", "--json", "workspaces"]).decode()
            workspaces = json.loads(res_ws)
            
            target_ws = None
            for ws in workspaces:
                if ws["id"] == workspace_id:
                    target_ws = ws
                    break
                    
            if not target_ws:
                time.sleep(0.5)
                continue
                
            output_name = target_ws["output"]
            
            res_outs = subprocess.check_output(["niri", "msg", "--json", "outputs"]).decode()
            outputs = json.loads(res_outs)
            
            output = outputs.get(output_name)
            if not output:
                time.sleep(0.5)
                continue
                
            logical = output["logical"]
            ox = int(logical["x"])
            oy = int(logical["y"])
            ow = int(logical["width"])
            oh = int(logical["height"])
            
            res_wins = subprocess.check_output(["niri", "msg", "--json", "windows"]).decode()
            wins = json.loads(res_wins)
            for w in wins:
                if w.get("id") == win_id:
                    our_win = w
                    break
            win_w, win_h = our_win["layout"]["window_size"]
            
            if win_w >= ow - 10 and win_h >= oh - 10:
                wx, wy, ww, wh = ox, oy, ow, oh
            else:
                wx = ox + (ow - win_w) // 2
                wy = oy + (oh - win_h) // 2
                ww = win_w
                wh = win_h
                
            print(f"✅ Bound successfully to screen region ({wx}, {wy}) size {ww}x{wh} on output {output_name} (win_id={win_id})")
            return wx, wy, ww, wh, output_name, win_id
        except Exception as e:
            print(f"  Warning during bind: {e}")
            time.sleep(0.5)
            
    print("⚠️ Nest 0 window navigation failed. Using default values.")
    return 0, 0, 1920, 1080, "HDMI-A-1", None

def focus_window_by_id(socket_path, target_win_id):
    for attempt in range(10):
        layout = send_cmd(socket_path, "get_layout_compact")
        target_col = None
        focused_col = None
        
        for line in layout.strip().split("\n"):
            if not line or line.startswith("error:"):
                continue
            parts = line.split(":", 5)
            if len(parts) < 4:
                continue
            try:
                col_idx = int(parts[1])
                win_id = int(parts[2])
                is_focused = parts[3].lower() == "true"
                
                if win_id == target_win_id:
                    target_col = col_idx
                if is_focused:
                    focused_col = col_idx
            except ValueError:
                continue
                
        if target_col is None:
            time.sleep(0.2)
            continue
            
        lines = layout.strip().split("\n")
        windows_in_target_col = []
        focused_win_in_target_col_id = None
        for line in lines:
            if not line or line.startswith("error:"):
                continue
            parts = line.split(":", 5)
            if len(parts) < 4:
                continue
            try:
                col_idx = int(parts[1])
                win_id = int(parts[2])
                is_focused = parts[3].lower() == "true"
                if col_idx == target_col:
                    windows_in_target_col.append(win_id)
                    if is_focused:
                        focused_win_in_target_col_id = win_id
            except ValueError:
                continue
                
        if focused_col is None:
            time.sleep(0.2)
            continue
            
        if target_col != focused_col:
            if target_col < focused_col:
                send_cmd(socket_path, "action focus-left")
            else:
                send_cmd(socket_path, "action focus-right")
            time.sleep(0.5)
            continue
            
        if focused_win_in_target_col_id == target_win_id:
            return True
            
        try:
            target_idx = windows_in_target_col.index(target_win_id)
            focused_idx = windows_in_target_col.index(focused_win_in_target_col_id)
            if target_idx < focused_idx:
                send_cmd(socket_path, "action focus-up")
            else:
                send_cmd(socket_path, "action focus-down")
        except ValueError:
            return False
            
        time.sleep(0.5)
    return False

def main():
    # Clean up stale configs from previous test runs to ensure a clean state
    import shutil
    for profile in ["default", "supervision"]:
        config_dir = f"/home/super/.config/antigravity-legacy-{profile}" if profile != "default" else "/home/super/.config/antigravity-legacy"
        if os.path.exists(config_dir):
            shutil.rmtree(config_dir, ignore_errors=True)
            
    os.makedirs(CAPTURES_DIR, exist_ok=True)
    
    print("[*] Rebuilding compositor...")
    subprocess.run(["cargo", "build"], check=True)
    
    host_transform = get_active_output_transform()
    print(f"[*] Detected active host monitor transform: {host_transform}")
    
    primary_display = get_primary_display_name()
    print(f"[*] Focusing primary display monitor {primary_display} via Niri IPC...")
    subprocess.run(["niri", "msg", "action", "focus-monitor", primary_display], check=False)
    time.sleep(0.5)
    
    print("[*] Spawning Nest 0 root nested compositor...")
    comp_proc, display_name = spawn_compositor(host_transform=host_transform)
    print(f"✅ Nest 0 started on display: {display_name}")
    socket_path = f"/tmp/hier-ctrl-{display_name}.sock"
    time.sleep(2.0)
    
    wx, wy, ww, wh, output_name, nest_win_id = focus_and_center_window(comp_proc.pid)
    
    # Launch default profile instance
    print("[*] Launching default profile window of Antigravity-Legacy...")
    env_clients = os.environ.copy()
    env_clients["WAYLAND_DISPLAY"] = display_name
    
    p_a = subprocess.Popen([
        "/home/super/.local/bin/antigravity-legacy",
        "--profile", "default",
        "--ozone-platform=wayland",
        "--enable-features=UseOzonePlatform",
        "--disable-gpu",
        "--disable-software-rasterizer",
        "--disable-dev-shm-usage",
        "--no-sandbox"
    ], env=env_clients)
    
    time.sleep(4.0)
    
    # Launch supervision profile instance
    print("[*] Launching supervision profile window of Antigravity-Legacy...")
    p_b = subprocess.Popen([
        "/home/super/.local/bin/antigravity-legacy",
        "--profile", "supervision",
        "--ozone-platform=wayland",
        "--enable-features=UseOzonePlatform",
        "--disable-gpu",
        "--disable-software-rasterizer",
        "--disable-dev-shm-usage",
        "--no-sandbox"
    ], env=env_clients)
    
    time.sleep(6.0)
    
    print("[*] Waiting for both windows to map in the layout...")
    mapped = False
    for attempt in range(30):
        layout = send_cmd(socket_path, "get_layout_compact")
        if layout and not layout.startswith("error:"):
            lines = [l for l in layout.strip().split("\n") if l]
            print(f"  Layout (attempt {attempt+1}): {lines}")
            if len(lines) >= 2:
                mapped = True
                break
        time.sleep(1.0)
        
    if not mapped:
        print("❌ Error: Both Antigravity windows did not map in the layout.")
        p_a.terminate()
        p_b.terminate()
        comp_proc.terminate()
        sys.exit(1)
        
    print("✅ Both windows mapped successfully!")
    print("[*] Waiting 15 seconds for Chromium/Electron webview pixel rendering to complete...")
    time.sleep(15.0)
    
    # State 1: Grid Mode (Default side-by-side tiling)
    print("[*] Setting Grid Tiling Mode and capturing...")
    send_cmd(socket_path, "action tiling-mode-grid")
    time.sleep(2.0)
    if nest_win_id:
        print("[*] Refocusing Nest 0 on host compositor...")
        subprocess.run(["niri", "msg", "action", "focus-window", "--id", str(nest_win_id)], check=False)
        time.sleep(2.0)
    img_grid = os.path.join(CAPTURES_DIR, "grid_layout.png")
    print(f"[*] Capturing output {output_name}: grim -o {output_name} -> {img_grid}")
    subprocess.run(["grim", "-o", output_name, img_grid], check=True)
    
    # State 2: Camera Focus (Focus on supervision window)
    print("[*] Focusing supervision window (ID 2)...")
    focused = focus_window_by_id(socket_path, 2)
    if focused:
        print("✅ Supervision window focused.")
    else:
        print("⚠️ Failed to focus supervision window, focusing right...")
        send_cmd(socket_path, "action focus-right")
    
    # Let's read camera target coords and set camera immediately
    camera = send_cmd(socket_path, "get_camera").strip()
    print(f"Camera state: {camera}")
    parts = camera.split(",")
    if len(parts) == 7:
        target_x, target_y = parts[2], parts[3]
        print(f"[*] Snapping camera to target ({target_x}, {target_y})...")
        send_cmd(socket_path, f"set_camera {target_x} {target_y} true")
        time.sleep(2.0)
        
    if nest_win_id:
        print("[*] Refocusing Nest 0 on host compositor...")
        subprocess.run(["niri", "msg", "action", "focus-window", "--id", str(nest_win_id)], check=False)
        time.sleep(2.0)
    img_focused = os.path.join(CAPTURES_DIR, "camera_focused.png")
    print(f"[*] Capturing output {output_name}: grim -o {output_name} -> {img_focused}")
    subprocess.run(["grim", "-o", output_name, img_focused], check=True)
    
    # State 3: Depth Tiling Mode
    print("[*] Activating Depth Tiling Mode...")
    send_cmd(socket_path, "action tiling-mode-depth")
    time.sleep(2.0)
    if nest_win_id:
        print("[*] Refocusing Nest 0 on host compositor...")
        subprocess.run(["niri", "msg", "action", "focus-window", "--id", str(nest_win_id)], check=False)
        time.sleep(2.0)
    img_depth = os.path.join(CAPTURES_DIR, "depth_layout.png")
    print(f"[*] Capturing output {output_name}: grim -o {output_name} -> {img_depth}")
    subprocess.run(["grim", "-o", output_name, img_depth], check=True)
    
    # State 4: Overview Mode
    print("[*] Activating Overview Tiling Mode...")
    print("[*] Resetting camera to (0,0) for overview presentation...")
    send_cmd(socket_path, "set_camera 0 0 true")
    send_cmd(socket_path, "action tiling-mode-overview")
    time.sleep(2.0)
    if nest_win_id:
        print("[*] Refocusing Nest 0 on host compositor...")
        subprocess.run(["niri", "msg", "action", "focus-window", "--id", str(nest_win_id)], check=False)
        time.sleep(2.0)
    img_overview = os.path.join(CAPTURES_DIR, "overview_layout.png")
    print(f"[*] Capturing output {output_name}: grim -o {output_name} -> {img_overview}")
    subprocess.run(["grim", "-o", output_name, img_overview], check=True)
    
    # Cleanup
    print("[*] Restoring grid mode and closing processes...")
    send_cmd(socket_path, "action tiling-mode-grid")
    time.sleep(0.5)
    
    p_a.terminate()
    p_b.terminate()
    p_a.wait()
    p_b.wait()
    comp_proc.terminate()
    comp_proc.wait()
    
    # Final cleanup of stale processes or locks
    subprocess.run(["pkill", "-f", "antigravity-legacy"], stderr=subprocess.DEVNULL)
    
    print("🎉 Visual integration test completed successfully!")

if __name__ == "__main__":
    main()
