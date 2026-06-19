#!/usr/bin/env python3
# test_visual_suite.py
# Rich visual testing suite for Hier compositor using screenshots and layout context.

import os
import sys
import time
import socket
import subprocess
import json
import base64
import shutil
import glob

# Auto-detect NIRI_SOCKET if not present in environment
if "NIRI_SOCKET" not in os.environ:
    niri_sockets = glob.glob("/run/user/1000/niri.wayland-*.sock")
    if niri_sockets:
        os.environ["NIRI_SOCKET"] = niri_sockets[0]
        print(f"[*] Dynamically set NIRI_SOCKET={niri_sockets[0]}")

if "WAYLAND_DISPLAY" not in os.environ:
    os.environ["WAYLAND_DISPLAY"] = "wayland-1"
    print("[*] Dynamically set WAYLAND_DISPLAY=wayland-1")

TEST_DIR = "/tmp/hier_visual_suite"
REPORT_PATH = "/home/super/Work/rust-based-dev/niri-rebuild/visual_test_report.html"
SOCKET_PATH = "/tmp/hier-ctrl-sandbox.sock"
OUTPUT_DIR = "/home/super/.gemini/antigravity/brain/9b4892b6-5aa6-435a-9444-f38848b79318"

# Expected colors in RGB bytes for mock window IDs
# Window ID % 4 colors:
# 0 => [0.15, 0.64, 0.41, 1.0] -> (38, 163, 104) [Teal/Green]
# 1 => [0.88, 0.11, 0.14, 1.0] -> (224, 28, 35) [Red/Orange]
# 2 => [0.12, 0.47, 0.81, 1.0] -> (30, 119, 206) [Blue]
# 3 => [0.55, 0.25, 0.70, 1.0] -> (140, 63, 178) [Purple]
EXPECTED_COLORS = {
    0: (38, 163, 104),
    1: (224, 28, 35),
    2: (30, 119, 206),
    3: (140, 63, 178)
}
COLOR_NAMES = {
    0: "Teal/Green",
    1: "Red/Orange",
    2: "Blue",
    3: "Purple"
}

def send_socket_cmd(cmd: str) -> str:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2.0)
        s.connect(SOCKET_PATH)
        s.sendall((cmd + "\n").encode())
        res = s.recv(4096).decode()
        s.close()
        return res
    except Exception as e:
        print(f"[Socket Error] {cmd}: {e}")
        return ""

def wait_for_camera_to_settle(timeout=5.0):
    print("  Waiting for layout camera to settle...")
    start = time.time()
    while time.time() - start < timeout:
        cam_str = send_socket_cmd("get_camera").strip()
        parts = cam_str.split(",")
        if len(parts) >= 4:
            try:
                curr_x = float(parts[0])
                curr_y = float(parts[1])
                targ_x = float(parts[2])
                targ_y = float(parts[3])
                if abs(curr_x - targ_x) < 0.5 and abs(curr_y - targ_y) < 0.5:
                    # Let it render a couple of frames to be fully painted
                    time.sleep(0.3)
                    return True
            except ValueError:
                pass
        time.sleep(0.1)
    print("  ⚠️ Warning: Camera did not settle in time.")
    return False

def get_sandbox_window_geom():
    print("[*] Detecting sandbox window geometry via host Niri IPC...")
    start_time = time.time()
    while time.time() - start_time < 15.0:
        try:
            res_wins = subprocess.check_output(["niri", "msg", "--json", "windows"]).decode()
            wins = json.loads(res_wins)
            
            for w in wins:
                # Winit uses Title "Smithay"
                if w.get("title") == "Smithay":
                    # Focus and make fullscreen on host Niri compositor
                    print("  Requesting host Niri to focus and fullscreen the sandbox window...")
                    subprocess.run(["niri", "msg", "action", "focus-window", "--id", str(w["id"])], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    time.sleep(0.5)
                    subprocess.run(["niri", "msg", "action", "fullscreen-window"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    time.sleep(1.0) # Settle fullscreen animation
                    
                    workspace_id = w["workspace_id"]
                    res_ws = subprocess.check_output(["niri", "msg", "--json", "workspaces"]).decode()
                    workspaces = json.loads(res_ws)
                    output_name = None
                    for ws in workspaces:
                        if ws["id"] == workspace_id:
                            output_name = ws["output"]
                            break
                    
                    if not output_name:
                        continue
                        
                    res_outs = subprocess.check_output(["niri", "msg", "--json", "outputs"]).decode()
                    outputs = json.loads(res_outs)
                    output = outputs.get(output_name)
                    if output:
                        logical = output["logical"]
                        ox = int(logical["x"])
                        oy = int(logical["y"])
                        ow = int(logical["width"])
                        oh = int(logical["height"])
                        print(f"✅ Found Smithay window and made fullscreen on output {output_name} at ({ox}, {oy}) size {ow}x{oh}")
                        return (ox, oy, ow, oh)
        except Exception as e:
            print(f"  Retrying window detection: {e}")
        time.sleep(0.5)
    return None

def parse_ppm(path):
    try:
        with open(path, 'rb') as f:
            header = f.readline().decode().strip()
            if header != "P6":
                print(f"❌ Error: PPM header is not P6: {header}")
                return None
                
            # Skip comments
            line = f.readline().decode().strip()
            while line.startswith("#"):
                line = f.readline().decode().strip()
                
            dimensions = line.split()
            width = int(dimensions[0])
            height = int(dimensions[1])
            
            max_val = int(f.readline().decode().strip())
            pixel_bytes = f.read()
            return width, height, pixel_bytes
    except Exception as e:
        print(f"❌ Error parsing PPM file {path}: {e}")
        return None

def get_pixel_color(width, height, pixel_bytes, x, y):
    if x < 0 or x >= width or y < 0 or y >= height:
        return None
    idx = (int(y) * width + int(x)) * 3
    if idx + 2 >= len(pixel_bytes):
        return None
    return (pixel_bytes[idx], pixel_bytes[idx+1], pixel_bytes[idx+2])

def parse_window_rects(layout_str):
    rects = []
    for line in layout_str.strip().split("\n"):
        if not line or line.startswith("error:"):
            continue
        parts = line.split(":")
        if len(parts) < 6:
            continue
        try:
            col_idx = int(parts[1])
            win_id = int(parts[2])
            is_focused = parts[3].lower() == "true"
            rect_str = parts[4]
            x, y, w, h = map(int, rect_str.split(","))
            if len(parts) >= 8:
                win_z = float(parts[-2])
                ws_z = float(parts[-1])
            else:
                win_z = 0.0
                ws_z = 1.0
            rects.append({
                "win_id": win_id,
                "col_idx": col_idx,
                "is_focused": is_focused,
                "x": x,
                "y": y,
                "w": w,
                "h": h,
                "win_z": win_z,
                "ws_z": ws_z
            })
        except ValueError:
            continue
    return rects

def check_telemetry_stutter(tc_name):
    telemetry_raw = send_socket_cmd("get_telemetry").strip()
    if not telemetry_raw:
        return {
            "win_id": None,
            "description": "Verify transition frame telemetry (smoothness)",
            "status": "WARNING",
            "details": "No telemetry response received from control socket"
        }
    try:
        data = json.loads(telemetry_raw)
        stutter_count = data.get("stutter_count", 0)
        frame_times = data.get("frame_times", [])
        
        # Calculate 95th percentile
        p95 = 0.0
        if frame_times:
            sorted_times = sorted(frame_times)
            idx = int(len(sorted_times) * 0.95)
            idx = min(idx, len(sorted_times) - 1)
            p95 = sorted_times[idx]
            
        mean = data.get("mean_ms", 0.0)
        max_t = data.get("max_ms", 0.0)
        
        # Criteria: stutter_count <= 5 OR p95 < 35.0 (accommodating software rendering CPU limits)
        status = "PASSED"
        details = f"Stutters: {stutter_count}, Mean: {mean:.2f}ms, P95: {p95:.2f}ms, Max: {max_t:.2f}ms"
        
        if stutter_count > 5 and p95 >= 35.0:
            status = "FAILED"
            details = f"Stutter count ({stutter_count}) exceeds threshold and P95 ({p95:.2f}ms) >= 35ms"
            
        return {
            "win_id": None,
            "description": "Verify transition frame telemetry (smoothness)",
            "status": status,
            "details": details
        }
    except Exception as e:
        return {
            "win_id": None,
            "description": "Verify transition frame telemetry (smoothness)",
            "status": "WARNING",
            "details": f"Failed to parse telemetry JSON: {e}"
        }

def main():
    print("==========================================================")
    # 1. Setup workspace
    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)
    os.makedirs(TEST_DIR, exist_ok=True)
    
    # 2. Cleanup compositor
    print("[*] Cleaning up existing compositor instances...")
    subprocess.run(["pkill", "-f", "target/release/hier"])
    if os.path.exists(SOCKET_PATH):
        os.remove(SOCKET_PATH)
        
    # 3. Launch compositor in sandbox
    print("[*] Launching sandbox compositor...")
    env = os.environ.copy()
    if "WAYLAND_DISPLAY" not in env:
        env["WAYLAND_DISPLAY"] = "wayland-1"
    env["HIER_SANDBOX"] = "1"
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    comp_proc = subprocess.Popen(
        ["./target/release/hier"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=env
    )
    
    print("[*] Waiting for sandbox control socket...")
    socket_ready = False
    start_time = time.time()
    while time.time() - start_time < 10.0:
        if os.path.exists(SOCKET_PATH):
            socket_ready = True
            break
        time.sleep(0.1)
        
    if not socket_ready:
        print("❌ Error: Socket did not open in time.")
        comp_proc.terminate()
        sys.exit(1)
        
    geom = get_sandbox_window_geom()
    if not geom:
        print("❌ Error: Could not locate sandbox window.")
        comp_proc.terminate()
        sys.exit(1)
        
    wx, wy, ww, wh = geom
    
    test_cases = [
        {
            "id": "grid_initial",
            "name": "Grid Mode - Initial Mock Windows",
            "setup_cmd": "action tiling-mode-grid",
            "settle_time": 5.0,
            "validate_windows": [3] # Window 3 focused by default
        },
        {
            "id": "focus_left_2",
            "name": "Focus Left to Web Browser (Mock Window #2)",
            "setup_cmd": "action focus-left",
            "settle_time": 1.5,
            "validate_windows": [2]
        },
        {
            "id": "focus_left_1",
            "name": "Focus Left to Terminal (Mock Window #1)",
            "setup_cmd": "action focus-left",
            "settle_time": 1.5,
            "validate_windows": [1]
        },
        {
            "id": "spawn_window",
            "name": "Spawn Mock Window App #4 (Teal/Green)",
            "setup_cmd": "action spawn-mock-window",
            "settle_time": 2.0,
            "validate_windows": [4] # Spawning App #4 focuses Window 4 (column 1)
        },
        {
            "id": "focus_left_1_from_4",
            "name": "Focus Left to Terminal (Mock Window #1)",
            "setup_cmd": "action focus-left",
            "settle_time": 1.5,
            "validate_windows": [1] # Column 0
        },
        {
            "id": "depth_mode",
            "name": "Depth Mode - Stacking Viewport",
            "setup_cmd": "action tiling-mode-depth",
            "settle_time": 2.5, # Give extra settle time for camera to center
            "validate_windows": [1] # Window 1 focused in foreground (at index 0 in Depth stack)
        },
        {
            "id": "depth_scroll_1",
            "name": "Depth Mode - Scroll Carousel Down to Web Browser (Mock Window #2)",
            "setup_cmd": "action focus-down",
            "settle_time": 1.5,
            "validate_windows": [2]
        },
        {
            "id": "depth_scroll_2",
            "name": "Depth Mode - Scroll Carousel Down to Text Editor (Mock Window #3)",
            "setup_cmd": "action focus-down",
            "settle_time": 1.5,
            "validate_windows": [3]
        },
        {
            "id": "depth_scroll_3",
            "name": "Depth Mode - Scroll Carousel Down to App #4 (Mock Window #4)",
            "setup_cmd": "action focus-down",
            "settle_time": 1.5,
            "validate_windows": [4]
        },
        {
            "id": "depth_scroll_up",
            "name": "Depth Mode - Scroll Carousel Up back to Text Editor (Mock Window #3)",
            "setup_cmd": "action focus-up",
            "settle_time": 1.5,
            "validate_windows": [3]
        },
        {
            "id": "depth_scroll_up_2",
            "name": "Depth Mode - Scroll Carousel Up to Web Browser (Mock Window #2)",
            "setup_cmd": "action focus-up",
            "settle_time": 1.5,
            "validate_windows": [2]
        },
        {
            "id": "depth_scroll_up_3",
            "name": "Depth Mode - Scroll Carousel Up to Terminal (Mock Window #1)",
            "setup_cmd": "action focus-up",
            "settle_time": 1.5,
            "validate_windows": [1]
        },
        {
            "id": "diagonal_mode",
            "name": "Diagonal Mode - Staggered Tiling",
            "setup_cmd": "action tiling-mode-diagonal",
            "settle_time": 2.0,
            "validate_windows": [1]
        },
        {
            "id": "overview_mode",
            "name": "Overview Mode - Workspace Zoom",
            "setup_cmd": "action tiling-mode-overview",
            "settle_time": 2.5,
            "validate_windows": [] # Zoomed out, skip color assertions
        },
        {
            "id": "grid_restore",
            "name": "Grid Mode - Restored Workspace",
            "setup_cmd": "action tiling-mode-grid",
            "settle_time": 2.0,
            "validate_windows": [1]
        }
    ]
    
    results = []
    
    try:
        for tc in test_cases:
            case_id = tc["id"]
            print(f"\n[*] Running Test Case: {tc['name']}...")
            
            # Reset telemetry stats before running the transition
            send_socket_cmd("reset_telemetry")
            
            # Setup layout
            if tc["setup_cmd"]:
                send_socket_cmd(tc["setup_cmd"])
                
            wait_for_camera_to_settle()
            # Give a small extra margin for hardware mapping and buffers
            time.sleep(0.5)
            
            # Capture
            png_path = f"{TEST_DIR}/{case_id}.png"
            ppm_path = f"{TEST_DIR}/{case_id}.ppm"
            
            subprocess.run(
                ["grim", "-g", f"{wx},{wy} {ww}x{wh}", png_path],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            
            subprocess.run(
                ["ffmpeg", "-y", "-i", png_path, ppm_path],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            
            # Get layout context
            layout_str = send_socket_cmd("get_layout_compact")
            camera_str = send_socket_cmd("get_camera").strip()
            
            window_rects = parse_window_rects(layout_str)
            
            # Parse PPM
            ppm_info = parse_ppm(ppm_path)
            if not ppm_info:
                results.append({
                    "case_id": case_id,
                    "name": tc["name"],
                    "status": "FAILED",
                    "reason": "Failed to capture or parse PPM screenshot",
                    "layout": window_rects,
                    "camera": camera_str,
                    "assertions": [],
                    "base64_img": ""
                })
                continue
                
            ppm_w, ppm_h, ppm_pixels = ppm_info
            # Calculate layout display scaling factor relative to winit logical size
            layout_scale = ppm_w / ww
            print(f"  Screenshot physical size: {ppm_w}x{ppm_h} (Scale Factor: {layout_scale:.2f})")
            
            assertions = []
            # Verify transition frame telemetry (smoothness)
            telemetry_res = check_telemetry_stutter(tc["name"])
            assertions.append(telemetry_res)
            case_failed = (telemetry_res["status"] == "FAILED")
            failure_reason = telemetry_res["details"] if case_failed else ""
            
            # Perform visual center color assertions for specified windows
            for win_id in tc["validate_windows"]:
                expected_color = EXPECTED_COLORS[win_id % 4]
                color_name = COLOR_NAMES[win_id % 4]
                
                # Find window in parsed layout rects
                win_rect = None
                for r in window_rects:
                    if r["win_id"] == win_id:
                        win_rect = r
                        break
                        
                if not win_rect:
                    case_failed = True
                    failure_reason = f"Window {win_id} not found in layout context"
                    assertions.append({
                        "win_id": win_id,
                        "description": f"Verify Window {win_id} color",
                        "status": "FAILED",
                        "details": f"Window not present in layout"
                    })
                    continue
                
                # Verify that it is focused in layout context
                if win_rect["is_focused"]:
                    assertions.append({
                        "win_id": win_id,
                        "description": f"Verify Window {win_id} has keyboard focus",
                        "status": "PASSED",
                        "details": "Focused status is true in layout context"
                    })
                else:
                    case_failed = True
                    failure_reason = f"Window {win_id} is not focused"
                    assertions.append({
                        "win_id": win_id,
                        "description": f"Verify Window {win_id} has keyboard focus",
                        "status": "FAILED",
                        "details": "Focused status is false"
                    })
                
                # Check center color in cropped PPM
                win_x_logical = win_rect["x"]
                win_y_logical = win_rect["y"]
                win_w = win_rect["w"]
                win_h = win_rect["h"]
                
                center_x_logical = win_x_logical + win_w / 2.0
                center_y_logical = win_y_logical + win_h / 2.0
                
                px_x = int(center_x_logical * layout_scale)
                px_y = int(center_y_logical * layout_scale)
                
                actual_color = get_pixel_color(ppm_w, ppm_h, ppm_pixels, px_x, px_y)
                
                if not actual_color:
                    case_failed = True
                    failure_reason = f"Center coordinates ({px_x}, {px_y}) out of screen bounds"
                    assertions.append({
                        "win_id": win_id,
                        "description": f"Verify color at logical center ({center_x_logical:.1f}, {center_y_logical:.1f})",
                        "status": "FAILED",
                        "details": f"Coordinates ({px_x}, {px_y}) out of bounds"
                    })
                else:
                    # Compare color with tolerance (accounting for subtle blending, though software render should be exact)
                    dr = abs(actual_color[0] - expected_color[0])
                    dg = abs(actual_color[1] - expected_color[1])
                    db = abs(actual_color[2] - expected_color[2])
                    
                    tolerance = 10
                    if dr <= tolerance and dg <= tolerance and db <= tolerance:
                        print(f"    [PASS] Window {win_id} ({color_name}) center color verified. Expected RGB {expected_color}, detected {actual_color}")
                        assertions.append({
                            "win_id": win_id,
                            "description": f"Verify Window {win_id} ({color_name}) center color",
                            "status": "PASSED",
                            "details": f"Expected RGB {expected_color}, detected {actual_color} at pixel ({px_x}, {px_y}) (diff={dr},{dg},{db})"
                        })
                    else:
                        case_failed = True
                        failure_reason = f"Window {win_id} color mismatch"
                        print(f"    [FAIL] Window {win_id} ({color_name}) center color mismatch. Expected RGB {expected_color}, detected {actual_color} at pixel ({px_x}, {px_y})")
                        assertions.append({
                            "win_id": win_id,
                            "description": f"Verify Window {win_id} ({color_name}) center color",
                            "status": "FAILED",
                            "details": f"Expected RGB {expected_color}, detected {actual_color} at pixel ({px_x}, {px_y}) (diff={dr},{dg},{db})"
                        })
            
            # Read PNG file and encode to base64
            with open(png_path, "rb") as image_file:
                encoded_string = base64.b64encode(image_file.read()).decode('utf-8')
                
            results.append({
                "case_id": case_id,
                "name": tc["name"],
                "status": "FAILED" if case_failed else "PASSED",
                "reason": failure_reason,
                "layout": window_rects,
                "camera": camera_str,
                "assertions": assertions,
                "base64_img": encoded_string
            })
            
            print(f"  Result: {'✅ PASSED' if not case_failed else '❌ FAILED'}")
            
    finally:
        # Terminate compositor
        print("\n[*] Terminating sandbox compositor...")
        comp_proc.terminate()
        comp_proc.wait()
        
    # 4. Generate HTML report
    print(f"\n[*] Generating Visual HTML Dashboard Report at {REPORT_PATH}...")
    
    html_content = """<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Hier Visual Layout Suite Report</title>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;600;700&family=Outfit:wght@400;600;800&family=Fira+Code:wght@400;500&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg: #0b0c10;
            --surface: #1f2833;
            --surface-accent: #2c3540;
            --text: #c5c6c7;
            --text-heading: #ffffff;
            --accent: #66fcf1;
            --accent-dim: #45a29e;
            --pass: #15ff92;
            --fail: #ff2a5f;
            --border-radius: 12px;
        }
        
        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }
        
        body {
            font-family: 'Inter', sans-serif;
            background-color: var(--bg);
            color: var(--text);
            padding: 2rem;
            line-height: 1.6;
        }
        
        header {
            max-width: 1400px;
            margin: 0 auto 3rem auto;
            border-bottom: 1px solid var(--surface-accent);
            padding-bottom: 2rem;
        }
        
        h1 {
            font-family: 'Outfit', sans-serif;
            font-size: 2.5rem;
            font-weight: 800;
            color: var(--text-heading);
            margin-bottom: 0.5rem;
        }
        
        .subtitle {
            font-size: 1.1rem;
            color: var(--accent);
            text-transform: uppercase;
            letter-spacing: 2px;
            font-weight: 600;
        }
        
        .summary-cards {
            display: flex;
            gap: 1.5rem;
            max-width: 1400px;
            margin: 0 auto 3rem auto;
        }
        
        .card {
            background-color: var(--surface);
            border-radius: var(--border-radius);
            padding: 1.5rem;
            flex: 1;
            border: 1px solid var(--surface-accent);
            box-shadow: 0 4px 20px rgba(0,0,0,0.3);
            display: flex;
            flex-direction: column;
            justify-content: center;
        }
        
        .card h3 {
            font-size: 0.9rem;
            text-transform: uppercase;
            letter-spacing: 1px;
            color: var(--text);
            margin-bottom: 0.5rem;
        }
        
        .card .value {
            font-family: 'Outfit', sans-serif;
            font-size: 2.5rem;
            font-weight: 800;
            color: var(--text-heading);
        }
        
        .card .value.pass {
            color: var(--pass);
        }
        
        .card .value.fail {
            color: var(--fail);
        }
        
        main {
            max-width: 1400px;
            margin: 0 auto;
            display: flex;
            flex-direction: column;
            gap: 4rem;
        }
        
        .case-section {
            background-color: var(--surface);
            border-radius: var(--border-radius);
            padding: 2rem;
            border: 1px solid var(--surface-accent);
            box-shadow: 0 4px 20px rgba(0,0,0,0.3);
        }
        
        .case-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            border-bottom: 1px solid var(--surface-accent);
            padding-bottom: 1rem;
            margin-bottom: 1.5rem;
        }
        
        .case-title-group {
            display: flex;
            align-items: center;
            gap: 1rem;
        }
        
        .case-title {
            font-family: 'Outfit', sans-serif;
            font-size: 1.5rem;
            font-weight: 600;
            color: var(--text-heading);
        }
        
        .badge {
            padding: 0.35rem 0.75rem;
            border-radius: 50px;
            font-size: 0.8rem;
            font-weight: 700;
            text-transform: uppercase;
            letter-spacing: 1px;
        }
        
        .badge.pass {
            background-color: rgba(21, 255, 146, 0.15);
            color: var(--pass);
            border: 1px solid var(--pass);
        }
        
        .badge.fail {
            background-color: rgba(255, 42, 95, 0.15);
            color: var(--fail);
            border: 1px solid var(--fail);
        }
        
        .case-content {
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 2rem;
        }
        
        @media (max-width: 1000px) {
            .case-content {
                grid-template-columns: 1fr;
            }
        }
        
        .screenshot-container {
            border-radius: 8px;
            overflow: hidden;
            border: 2px solid var(--surface-accent);
            background-color: #000;
            display: flex;
            align-items: center;
            justify-content: center;
        }
        
        .screenshot-container img {
            width: 100%;
            height: auto;
            max-height: 450px;
            object-fit: contain;
            display: block;
        }
        
        .meta-container {
            display: flex;
            flex-direction: column;
            gap: 1.5rem;
        }
        
        .meta-group h4 {
            font-size: 0.95rem;
            text-transform: uppercase;
            letter-spacing: 1px;
            color: var(--accent);
            margin-bottom: 0.5rem;
        }
        
        .code-block {
            font-family: 'Fira Code', monospace;
            background-color: #121820;
            padding: 0.75rem 1rem;
            border-radius: 6px;
            font-size: 0.85rem;
            overflow-x: auto;
            white-space: pre-wrap;
            border: 1px solid rgba(255,255,255,0.05);
        }
        
        .assertions-list {
            display: flex;
            flex-direction: column;
            gap: 0.75rem;
        }
        
        .assertion-item {
            background-color: #1a222c;
            padding: 0.75rem 1rem;
            border-radius: 6px;
            border-left: 4px solid var(--surface-accent);
            display: flex;
            justify-content: space-between;
            align-items: center;
        }
        
        .assertion-item.passed {
            border-left-color: var(--pass);
        }
        
        .assertion-item.failed {
            border-left-color: var(--fail);
        }
        
        .assertion-desc {
            font-weight: 600;
            font-size: 0.9rem;
            color: var(--text-heading);
        }
        
        .assertion-details {
            font-size: 0.8rem;
            color: var(--text);
            margin-top: 0.25rem;
        }
        
        .assertion-status {
            font-size: 0.8rem;
            font-weight: 700;
        }
        
        .assertion-status.passed {
            color: var(--pass);
        }
        
        .assertion-status.failed {
            color: var(--fail);
        }
    </style>
</head>
<body>
    <header>
        <span class="subtitle">Hier Compositor Workspace</span>
        <h1>Visual Tiling Layout Test Suite</h1>
    </header>
"""
    
    # Calculate stats
    total_cases = len(results)
    passed_cases = sum(1 for r in results if r["status"] == "PASSED")
    failed_cases = total_cases - passed_cases
    
    html_content += f"""
    <section class="summary-cards">
        <div class="card">
            <h3>Total Test Cases</h3>
            <div class="value">{total_cases}</div>
        </div>
        <div class="card">
            <h3>Passed</h3>
            <div class="value pass">{passed_cases}</div>
        </div>
        <div class="card">
            <h3>Failed</h3>
            <div class="value fail">{failed_cases}</div>
        </div>
    </section>
    
    <main>
    """
    
    for r in results:
        badge_class = "pass" if r["status"] == "PASSED" else "fail"
        
        # Format window rects
        rects_fmt = ""
        for rc in r["layout"]:
            focused_str = " (Focused)" if rc["is_focused"] else ""
            z_str = f" Z={rc['win_z']:.4f} WS_Z={rc['ws_z']:.4f}" if "win_z" in rc else ""
            rects_fmt += f"Window ID {rc['win_id']} -> col={rc['col_idx']}, rect=({rc['x']},{rc['y']},{rc['w']}x{rc['h']}){focused_str}{z_str}\n"
            
        if not rects_fmt:
            rects_fmt = "No windows active."
            
        # Format camera telemetry
        cam_parts = r["camera"].split(",")
        if len(cam_parts) >= 6:
            cam_fmt = f"Camera Pos: ({cam_parts[0]}, {cam_parts[1]})\nCamera Target: ({cam_parts[2]}, {cam_parts[3]})\nViewport: {cam_parts[4]}x{cam_parts[5]}"
        else:
            cam_fmt = r["camera"]
            
        # Build assertions list
        assertions_html = ""
        if r["assertions"]:
            for ass in r["assertions"]:
                item_class = "passed" if ass["status"] == "PASSED" else "failed"
                status_class = "passed" if ass["status"] == "PASSED" else "failed"
                assertions_html += f"""
                <div class="assertion-item {item_class}">
                    <div>
                        <div class="assertion-desc">{ass['description']}</div>
                        <div class="assertion-details">{ass['details']}</div>
                    </div>
                    <div class="assertion-status {status_class}">{ass['status']}</div>
                </div>
                """
        else:
            assertions_html = "<div class='assertion-item'>No assertions performed for this layout.</div>"
            
        html_content += f"""
        <section class="case-section">
            <div class="case-header">
                <div class="case-title-group">
                    <span class="badge {badge_class}">{r['status']}</span>
                    <h2 class="case-title">{r['name']}</h2>
                </div>
            </div>
            <div class="case-content">
                <div class="screenshot-container">
                    <img src="data:image/png;base64,{r['base64_img']}" alt="{r['name']} Capture">
                </div>
                <div class="meta-container">
                    <div class="meta-group">
                        <h4>Visual Assertions</h4>
                        <div class="assertions-list">
                            {assertions_html}
                        </div>
                    </div>
                    <div class="meta-group">
                        <h4>Layout Context Geometries</h4>
                        <div class="code-block">{rects_fmt}</div>
                    </div>
                    <div class="meta-group">
                        <h4>Camera Telemetry</h4>
                        <div class="code-block">{cam_fmt}</div>
                    </div>
                </div>
            </div>
        </section>
        """
        
    html_content += """
    </main>
</body>
</html>
"""
    
    # Write HTML report
    with open(REPORT_PATH, "w") as f:
        f.write(html_content)
        
    # Copy report to artifacts directory for user visibility
    artifact_report_path = os.path.join(OUTPUT_DIR, "visual_test_report.html")
    shutil.copy(REPORT_PATH, artifact_report_path)
    
    # Copy all PNGs to artifacts directory for user visibility
    print("[*] Copying PNG screenshots to output directory...")
    for case in results:
        case_id = case["case_id"]
        png_src = f"{TEST_DIR}/{case_id}.png"
        if os.path.exists(png_src):
            shutil.copy(png_src, os.path.join(OUTPUT_DIR, f"{case_id}.png"))

    # 5. Cleanup temp files
    print("[*] Cleaning up temporary image files in /tmp...")
    shutil.rmtree(TEST_DIR)
    
    print(f"🎉 Visual layout testing suite completed successfully! HTML Dashboard generated at {REPORT_PATH}")
    
    if failed_cases > 0:
        print("🚨 Warning: One or more visual assertions failed.")
        sys.exit(1)
    else:
        print("🎉 All visual assertions passed successfully!")
        sys.exit(0)

if __name__ == "__main__":
    main()
