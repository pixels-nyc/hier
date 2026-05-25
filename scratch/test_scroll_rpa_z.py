#!/usr/bin/env python3
# test_scroll_rpa_z.py
# Verification of recursive Z-axis simulated scroll inputs (Nesting Dolls)
# Fully dynamic socket allocation to support pre-nested environments.

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
        print(f"Socket connection failed for socket '{socket_path}' cmd '{cmd}': {e}")
        return ""

def spawn_compositor(parent_display=None, host_transform="Normal", fullscreen=False):
    env = os.environ.copy()
    if parent_display:
        env["WAYLAND_DISPLAY"] = parent_display
    if fullscreen:
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
            
    # Drain output in background thread to avoid pipe capacity deadlock / Rust println panic
    import threading
    def drain():
        log_name = f"/tmp/hier-scroll-{'nest1' if parent_display else 'nest0'}.log"
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
    print("=== Starting Recursive Z-Axis Scroll & Nesting Dolls Testing ===")
    
    # Focus primary display monitor
    primary_display = get_primary_display_name()
    print(f"[*] Focusing primary display monitor {primary_display} via Niri IPC...")
    subprocess.run(["niri", "msg", "action", "focus-monitor", primary_display], check=False)
    time.sleep(0.5)

    # 1. Start Nest 0 (Root nested compositor)
    parent_wayland = os.environ.get("WAYLAND_DISPLAY", "wayland-1")
    print(f"[*] Launching root compositor (Nest 0, parent: {parent_wayland})...")
    host_transform = get_active_output_transform()
    comp_process0, display0, logs0 = spawn_compositor(host_transform=host_transform, fullscreen=True)
    print(f"✅ Nest 0 successfully initialized display: {display0}")
    socket_n0 = f"/tmp/hier-ctrl-{display0}.sock"
    time.sleep(2.0)
    
    # 2. Start Nest 1 nested inside Nest 0
    print(f"[*] Launching nested child compositor (Nest 1, parent: {display0})...")
    comp_process1, display1, logs1 = spawn_compositor(parent_display=display0, fullscreen=False)
    print(f"✅ Nest 1 successfully initialized display: {display1}")
    socket_n1 = f"/tmp/hier-ctrl-{display1}.sock"
    time.sleep(2.0)
    
    # 3. Spawn 2 windows on Nest 1
    print(f"[*] Spawning 2 client windows inside Nest 1 (display {display1})...")
    env_clients = os.environ.copy()
    env_clients["WAYLAND_DISPLAY"] = display1
    env_clients["LIBGL_ALWAYS_SOFTWARE"] = "1"
    term_cmd_w1 = get_terminal_cmd("Z_AXIS_1")
    w1 = subprocess.Popen(term_cmd_w1, env=env_clients)
    time.sleep(2.5)
    term_cmd_w2 = get_terminal_cmd("Z_AXIS_2")
    w2 = subprocess.Popen(term_cmd_w2, env=env_clients)
    time.sleep(2.5)

    # 4. Check Nest 1 layout before grouping
    layout_n1_init = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Initial Layout:\n{layout_n1_init.strip()}")
    
    # Group windows in Nest 1 into a tab stack
    print("[*] Focusing left and toggling tab group stack inside Nest 1...")
    send_cmd(socket_n1, "action focus-left")
    send_cmd(socket_n1, "action toggle-tab")
    time.sleep(1.5)

    layout_n1_stacked = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Stacked Layout (Expect stacked windows):\n{layout_n1_stacked.strip()}")

    # 5. Send Z-axis Scroll to Root Nest 0 (which should recursively forward it to Nest 1)
    print("\n==================================================")
    print(f"[*] Sending Z-Scroll to Root Nest 0 ({display0})...")
    print("==================================================")
    
    # Send scroll down to Nest 0
    res = send_cmd(socket_n0, "pointer_axis_z 1.0")
    print(f"Nest 0 Z-Scroll Response: {res.strip()}")
    time.sleep(2.0)
    
    # Check if Nest 1 received the forwarded scroll and shifted focus!
    layout_n1_scrolled = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Layout after Nest 0 Z-Scroll:\n{layout_n1_scrolled.strip()}")

    # Send scroll up to Nest 0 (propagates to Nest 1 to restore focus)
    print("\n[*] Sending Z-Scroll Up to Root Nest 0...")
    res = send_cmd(socket_n0, "pointer_axis_z -1.0")
    print(f"Nest 0 Z-Scroll Response: {res.strip()}")
    time.sleep(2.0)
    
    # Check if Nest 1 layout focus is restored
    layout_n1_restored = send_cmd(socket_n1, "get_layout_compact")
    print(f"[*] Nest 1 Layout after Nest 0 Z-Scroll Up:\n{layout_n1_restored.strip()}")

    # Cleanup processes
    print("\n[*] Cleaning up all compositor and client processes...")
    w1.terminate()
    w2.terminate()
    comp_process1.terminate()
    comp_process0.terminate()
    
    print("\n=== Nest 0 Compositor Output Logs ===")
    print("".join(logs0))
    try:
        remainder = comp_process0.stdout.read()
        if remainder:
            print(remainder)
    except Exception:
        pass
        
    print("\n=== Nest 1 Compositor Output Logs ===")
    print("".join(logs1))
    try:
        remainder = comp_process1.stdout.read()
        if remainder:
            print(remainder)
    except Exception:
        pass
        
    print("=== Recursive Z-Axis Scroll & Nesting Dolls Testing Completed ===")

if __name__ == "__main__":
    main()
