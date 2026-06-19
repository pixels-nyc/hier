#!/usr/bin/env python3
# gen_sandbox_animations.py
# Automated recording and generation script for Hier visual sandbox animations.

import os
import sys
import time
import socket
import subprocess
import json
import threading
import shutil

FRAMES_DIR = "/tmp/sandbox_frames"
OUTPUT_DIR = "/home/super/.gemini/antigravity/brain/6c3c92ad-16e9-4675-b655-b6144025e7d0"
GIF_PATH = os.path.join(OUTPUT_DIR, "sandbox_animations.gif")
MP4_PATH = os.path.join(OUTPUT_DIR, "sandbox_animations.mp4")

def send_socket_cmd(cmd: str) -> str:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2.0)
        s.connect("/tmp/hier-ctrl-sandbox.sock")
        s.sendall((cmd + "\n").encode())
        res = s.recv(1024).decode()
        s.close()
        return res
    except Exception as e:
        print(f"[Socket Error] {cmd}: {e}")
        return ""

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
                        print(f"✅ Found Smithay window fullscreen on output {output_name} at ({ox}, {oy}) size {ow}x{oh}")
                        return (ox, oy, ow, oh)
        except Exception as e:
            print(f"  Retrying window detection: {e}")
        time.sleep(0.5)
    return None

def main():
    # Clean up any existing instances and socket files
    print("[*] Cleaning up existing hier processes and socket files...")
    subprocess.run(["pkill", "-f", "target/debug/hier"])
    if os.path.exists("/tmp/hier-ctrl-sandbox.sock"):
        os.remove("/tmp/hier-ctrl-sandbox.sock")
    if os.path.exists(FRAMES_DIR):
        shutil.rmtree(FRAMES_DIR)
    os.makedirs(FRAMES_DIR, exist_ok=True)
    os.makedirs(OUTPUT_DIR, exist_ok=True)

    print("[*] Rebuilding Hier compositor...")
    subprocess.run(["cargo", "build"], check=True)

    print("[*] Starting compositor in sandbox + fullscreen mode...")
    env = os.environ.copy()
    env["HIER_SANDBOX"] = "1"
    env["HIER_FULLSCREEN"] = "1"
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    comp_proc = subprocess.Popen(
        ["./target/debug/hier"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=env
    )
    
    # Wait for the control socket to open
    print("[*] Waiting for sandbox control socket to open...")
    start_time = time.time()
    socket_ready = False
    while time.time() - start_time < 10.0:
        if os.path.exists("/tmp/hier-ctrl-sandbox.sock"):
            socket_ready = True
            break
        time.sleep(0.1)
        
    if not socket_ready:
        print("❌ Error: Sandbox control socket did not open in time.")
        comp_proc.terminate()
        sys.exit(1)
        
    print("✅ Control socket ready.")
    
    geom = get_sandbox_window_geom()
    if not geom:
        print("❌ Error: Could not locate sandbox compositor window on host.")
        comp_proc.terminate()
        sys.exit(1)
        
    # Start the screen capture loop
    stop_capture = False
    captured_frames = []
    
    def capture_loop():
        x, y, w, h = geom
        frame_idx = 0
        print("[*] Started capturing frames...")
        while not stop_capture:
            t0 = time.time()
            frame_path = f"{FRAMES_DIR}/frame_{frame_idx:04d}.png"
            # grim -g "x,y wxh" output.png
            subprocess.run(
                ["grim", "-g", f"{x},{y} {w}x{h}", frame_path],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            captured_frames.append(frame_path)
            frame_idx += 1
            # Maintain stable 20 fps (50ms interval)
            elapsed = time.time() - t0
            sleep_time = max(0.005, 0.05 - elapsed)
            time.sleep(sleep_time)
        print(f"[*] Stopped capture loop. Captured {frame_idx} frames.")

    capture_thread = threading.Thread(target=capture_loop, daemon=True)
    capture_thread.start()
    
    # Sequence of layout transitions
    try:
        print("[*] Triggering Grid mode layout sequence (initial)...")
        time.sleep(2.0)
        
        print("[*] Action: Spawn mock window...")
        send_socket_cmd("action spawn-mock-window")
        time.sleep(1.5)
        
        print("[*] Action: Switch to tiling-mode-depth...")
        send_socket_cmd("action tiling-mode-depth")
        time.sleep(2.0)
        
        print("[*] Action: Cycle Tab focus Up (twice)...")
        send_socket_cmd("action focus-up")
        time.sleep(1.2)
        send_socket_cmd("action focus-up")
        time.sleep(1.2)
        
        print("[*] Action: Cycle Tab focus Down (once)...")
        send_socket_cmd("action focus-down")
        time.sleep(1.2)
        
        print("[*] Action: Switch to tiling-mode-diagonal...")
        send_socket_cmd("action tiling-mode-diagonal")
        time.sleep(2.0)
        
        print("[*] Action: Switch to tiling-mode-float...")
        send_socket_cmd("action tiling-mode-float")
        time.sleep(2.0)
        
        print("[*] Action: Switch to tiling-mode-overview...")
        send_socket_cmd("action tiling-mode-overview")
        time.sleep(2.5)
        
        print("[*] Action: Switch back to tiling-mode-grid...")
        send_socket_cmd("action tiling-mode-grid")
        time.sleep(2.0)
        
    finally:
        # Stop capture thread
        stop_capture = True
        capture_thread.join()
        
        # Terminate compositor
        print("[*] Terminating sandbox compositor...")
        comp_proc.terminate()
        comp_proc.wait()
        
    # Compile animated GIF and MP4 using ffmpeg
    if len(captured_frames) == 0:
        print("❌ Error: No frames captured.")
        sys.exit(1)
        
    print(f"[*] Compiling frames into high-quality GIF: {GIF_PATH}")
    # Compile scaled (800px wide) optimized GIF
    subprocess.run([
        "ffmpeg", "-y",
        "-framerate", "20",
        "-i", f"{FRAMES_DIR}/frame_%04d.png",
        "-vf", "fps=20,scale=800:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
        GIF_PATH
    ], check=True)
    
    print(f"[*] Compiling frames into MP4 video: {MP4_PATH}")
    # Compile H.264 MP4 with even dimension scaling
    subprocess.run([
        "ffmpeg", "-y",
        "-framerate", "20",
        "-i", f"{FRAMES_DIR}/frame_%04d.png",
        "-vf", "fps=20,scale=1280:-2:flags=lanczos",
        "-c:v", "libx264",
        "-pix_fmt", "yuv420p",
        MP4_PATH
    ], check=True)
    
    # Clean up temporary frames directory
    print("[*] Cleaning up temporary frame pngs...")
    shutil.rmtree(FRAMES_DIR)
    print("🎉 Animation generation completed successfully!")

if __name__ == "__main__":
    main()
