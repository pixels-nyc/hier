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
    res = s.recv(16384).decode()
    s.close()
    return res

def launch_compositor():
    env = os.environ.copy()
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    proc = subprocess.Popen(
        ["target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
        text=True
    )
    
    display_name = None
    socket_path = None
    start_time = time.time()
    
    while time.time() - start_time < 8.0:
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
        print("❌ Failed to start compositor.")
        proc.terminate()
        proc.wait()
        sys.exit(1)
        
    return proc, display_name, socket_path

def main():
    print("=== Starting Multi-Display Z-Axis Cut Promotion Test ===")
    
    proc_parent = None
    proc_child1 = None
    proc_child2 = None
    
    try:
        # 1. Launch Parent Compositor Z
        print("\n[*] Launching Parent Compositor Z...")
        proc_parent, parent_disp, parent_sock = launch_compositor()
        print(f"✅ Parent Z started: Display={parent_disp}, Socket={parent_sock}")
        time.sleep(1.0)
        
        # 2. Launch Child Compositor 1
        print("\n[*] Launching Child Compositor 1...")
        proc_child1, child1_disp, child1_sock = launch_compositor()
        print(f"✅ Child 1 started: Display={child1_disp}, Socket={child1_sock}")
        time.sleep(1.0)
        
        # 3. Launch Child Compositor 2
        print("\n[*] Launching Child Compositor 2...")
        proc_child2, child2_disp, child2_sock = launch_compositor()
        print(f"✅ Child 2 started: Display={child2_disp}, Socket={child2_sock}")
        time.sleep(1.0)
        
        # 4. Spawn a window inside Child 1
        print(f"\n[*] Spawning window inside Child 1 ({child1_disp})...")
        send_cmd(child1_sock, "action spawn-terminal")
        time.sleep(0.5)
        
        # 5. Spawn a window inside Child 2
        print(f"\n[*] Spawning window inside Child 2 ({child2_disp})...")
        send_cmd(child2_sock, "action spawn-terminal")
        time.sleep(0.5)
        
        # Get and print initial layouts
        layout_c1 = json.loads(send_cmd(child1_sock, "get_layout"))
        win1_title = layout_c1["workspaces"][0]["columns"][0]["windows"][0]["title"]
        print(f"Child 1 active window: {win1_title}")
        
        layout_c2 = json.loads(send_cmd(child2_sock, "get_layout"))
        win2_title = layout_c2["workspaces"][0]["columns"][0]["windows"][0]["title"]
        print(f"Child 2 active window: {win2_title}")
        
        # 6. Perform Z-Cut from Child 1 on Parent Z
        print(f"\n[*] Promoting window 1 from Child 1 ({child1_disp}) to Parent Z...")
        res_cut1 = send_cmd(parent_sock, f"cut_window {child1_disp} 1").strip()
        print(f"Parent Z cut response: {res_cut1}")
        assert "ok: promoted window" in res_cut1, f"Expected successful promotion from Child 1, got: {res_cut1}"
        
        # 7. Perform Z-Cut from Child 2 on Parent Z
        print(f"\n[*] Promoting window 1 from Child 2 ({child2_disp}) to Parent Z...")
        res_cut2 = send_cmd(parent_sock, f"cut_window {child2_disp} 1").strip()
        print(f"Parent Z cut response: {res_cut2}")
        assert "ok: promoted window" in res_cut2, f"Expected successful promotion from Child 2, got: {res_cut2}"
        
        # 8. Verify both windows are now active in Parent Z
        layout_p = json.loads(send_cmd(parent_sock, "get_layout"))
        promoted_count = 0
        for ws in layout_p["workspaces"]:
            for col in ws["columns"]:
                for win in col["windows"]:
                    if "[Custom Access Promoted]" in win["title"]:
                        promoted_count += 1
                        print(f"Found promoted window in Parent Z layout: {win['title']}")
                        
        print(f"\nTotal promoted windows in Parent Z: {promoted_count}")
        assert promoted_count == 2, f"Expected 2 promoted windows in Parent Z layout, found: {promoted_count}"
        print("✅ Multi-display Z-axis window promotion verified successfully!")
        
    finally:
        print("\n[*] Cleaning up all compositor processes...")
        for proc in (proc_child2, proc_child1, proc_parent):
            if proc:
                try:
                    proc.terminate()
                    proc.wait()
                except Exception:
                    pass
        print("✅ Cleanup complete. All processes terminated.")

if __name__ == "__main__":
    main()
