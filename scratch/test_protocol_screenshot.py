#!/usr/bin/env python3
import subprocess
import socket
import time
import os
import sys
import re

def send_cmd(socket_path: str, cmd: str) -> str:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(socket_path)
    s.sendall((cmd + "\n").encode())
    res = s.recv(4096).decode()
    s.close()
    return res

def main():
    print("[*] Starting compositor capture protocol verification...")
    
    env = os.environ.copy()
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    # Spawn compositor
    comp_proc = subprocess.Popen(
        ["target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
        text=True
    )
    
    display_name = None
    socket_path = None
    
    try:
        # Parse display & socket path
        start_time = time.time()
        while time.time() - start_time < 5.0:
            line = comp_proc.stdout.readline()
            if not line:
                time.sleep(0.05)
                continue
            match_display = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
            if match_display:
                display_name = match_display.group(1)
            match_socket = re.search(r"Control socket listening at: (/tmp/hier-ctrl-wayland-\d+\.sock)", line)
            if match_socket:
                socket_path = match_socket.group(1)
            if display_name and socket_path:
                break
                
        if not display_name or not socket_path:
            print("❌ Error: Failed to initialize compositor or parse sockets.")
            comp_proc.terminate()
            sys.exit(1)
            
        print(f"✅ Compositor running on display: {display_name}")
        print(f"✅ Control socket path: {socket_path}")
        time.sleep(1.0)
        
        # Spawn client terminal inside the nested display
        print("[*] Spawning terminal client inside compositor...")
        import shutil
        term = "foot"
        if not shutil.which(term):
            term = "alacritty"
            if not shutil.which(term):
                term = "xterm"
        client_env = os.environ.copy()
        client_env["WAYLAND_DISPLAY"] = display_name
        client_env["LIBGL_ALWAYS_SOFTWARE"] = "1"
        client_proc = subprocess.Popen([term], env=client_env)
        time.sleep(3.0) # Allow window to map
        
        # Test: Query layout to check if client exists
        layout = send_cmd(socket_path, "get_layout_compact").strip()
        print(f"[*] Compositor compact layout:\n{layout}")
        if not layout or "error" in layout.lower():
            print("❌ Error: No mapped windows found.")
            client_proc.terminate()
            comp_proc.terminate()
            sys.exit(1)
            
        # Parse window ID (should be 1)
        win_id = None
        for line in layout.split("\n"):
            parts = line.split(":")
            if len(parts) >= 3:
                win_id = parts[2]
                break
                
        if not win_id:
            print("❌ Error: Could not determine window ID.")
            client_proc.terminate()
            comp_proc.terminate()
            sys.exit(1)
            
        # Trigger protocol capture_window command
        temp_ppm = "/tmp/protocol_verify.ppm"
        if os.path.exists(temp_ppm):
            os.remove(temp_ppm)
            
        print(f"[*] Sending screenshot protocol command: capture_window {win_id} {temp_ppm}")
        res = send_cmd(socket_path, f"capture_window {win_id} {temp_ppm}").strip()
        print(f"[*] Protocol capture response: {res}")
        
        # Verify the PPM file was successfully created
        if not os.path.exists(temp_ppm):
            print("❌ Error: PPM screenshot file was not created.")
            client_proc.terminate()
            comp_proc.terminate()
            sys.exit(1)
            
        # Read PPM header info and print details
        with open(temp_ppm, "rb") as f:
            header = f.readline().decode().strip()
            dim_line = f.readline().decode().strip()
            max_val = f.readline().decode().strip()
            pixel_data = f.read()
            
        print("\n==================================================")
        print("          PROTOCOL SCREENSHOT DETAILS")
        print("==================================================")
        print(f"  Format:           {header}")
        print(f"  Dimensions:       {dim_line} (pixels)")
        print(f"  Max Color Value:  {max_val}")
        print(f"  Total Data Bytes: {len(pixel_data)} bytes")
        
        # Validate border pixels (which are set to [30, 144, 255] by compositor logic)
        width, height = map(int, dim_line.split())
        border_idx = (2 * width + 2) * 3
        r = pixel_data[border_idx]
        g = pixel_data[border_idx + 1]
        b = pixel_data[border_idx + 2]
        print(f"  Border Pixel (2,2): R={r}, G={g}, B={b}")
        
        if (r, g, b) == (30, 144, 255):
            print("  Border Validation: ✅ PASSED (Matching high-contrast blue border!)")
        else:
            print("  Border Validation: ❌ FAILED")
        print("==================================================\n")
        
        # Clean up PPM
        os.remove(temp_ppm)
        
        # Cleanup processes
        client_proc.terminate()
        client_proc.wait()
        print("[*] Client terminal terminated.")
        
    finally:
        comp_proc.terminate()
        comp_proc.wait()
        print("[*] Compositor terminated.")

if __name__ == "__main__":
    main()
