#!/usr/bin/env python3
# safety_tests/test_browser_rpa_defense.py
# RPA visual-tactile validation test and safety defense evaluation

import os
import sys
import time
import socket
import subprocess
import json
import re
import shutil

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
        return list(outputs.keys())[0]
    except Exception:
        return "HDMI-A-2"

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
        return f"error: socket connection failed: {e}"

def get_terminal_cmd():
    for term in ["foot", "alacritty", "kitty", "xterm"]:
        if shutil.which(term):
            return [term]
    return ["alacritty"]

def type_string(socket_path, s):
    # Mapping for common lowercase characters to evdev keycodes
    key_map = {
        'a': 30, 'b': 48, 'c': 46, 'd': 32, 'e': 18, 'f': 33, 'g': 34, 'h': 35,
        'i': 23, 'j': 36, 'k': 37, 'l': 38, 'm': 50, 'n': 49, 'o': 24, 'p': 25,
        'q': 16, 'r': 19, 's': 31, 't': 20, 'u': 22, 'v': 47, 'w': 17, 'x': 45,
        'y': 21, 'z': 44,
        '1': 2, '2': 3, '3': 4, '4': 5, '5': 6, '6': 7, '7': 8, '8': 9, '9': 10, '0': 11,
        ' ': 57, '.': 52, '/': 53
    }
    
    for char in s:
        if char in key_map:
            code = key_map[char]
            send_cmd(socket_path, f"keyboard_key {code} pressed")
            time.sleep(0.05)
            send_cmd(socket_path, f"keyboard_key {code} released")
            time.sleep(0.05)
        elif char == '\n':
            send_cmd(socket_path, "keyboard_key 28 pressed")
            time.sleep(0.05)
            send_cmd(socket_path, "keyboard_key 28 released")
            time.sleep(0.05)

def main():

    print("==================================================")
    print(" SECURITY TEST: RPA CAPTCHA BYPASS & MITIGATION   ")
    print("==================================================")

    # 1. Clean old status and debug files
    status_file = "/tmp/captcha_status.json"
    debug_file = "/tmp/captcha_debug.log"
    for f_path in [status_file, debug_file]:
        if os.path.exists(f_path):
            os.remove(f_path)

    # 2. Focus host screen
    primary_display = get_primary_display_name()
    print(f"[*] Focusing primary display monitor: {primary_display}")
    subprocess.run(["niri", "msg", "action", "focus-monitor", primary_display], check=False)

    time.sleep(0.5)

    # 3. Spawn Compositor
    print("[*] Spawning nested compositor (Nest 0)...")
    env = os.environ.copy()
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    env["HIER_FULLSCREEN"] = "1"
    
    comp_proc = subprocess.Popen(
        ["target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=env,
        text=True
    )

    # Wait to extract the display socket name
    display_name = None
    socket_path = None
    start_time = time.time()
    lines_read = []
    while time.time() - start_time < 10.0:
        line = comp_proc.stdout.readline()
        if not line:
            time.sleep(0.05)
            continue
        lines_read.append(line)
        match = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
        if match:
            display_name = match.group(1)
            break

    if not display_name:
        time.sleep(2.0)
        display_name = "wayland-2"

    # Spawn thread to drain the rest of the output
    import threading
    log_file_path = "/tmp/hier-safety-comp.log"
    try:
        if os.path.exists(log_file_path):
            os.remove(log_file_path)
    except Exception:
        pass

    def drain():
        try:
            with open(log_file_path, "a", buffering=1) as f:
                for line_init in lines_read:
                    if not any(k in line_init for k in ["DEBUG RENDER", "Reposition window"]):
                        f.write(line_init)
                while True:
                    line = comp_proc.stdout.readline()
                    if not line:
                        if comp_proc.poll() is not None:
                            break
                        time.sleep(0.1)
                        continue
                    if not any(k in line for k in ["DEBUG RENDER", "Reposition window"]):
                        f.write(line)
        except Exception as e:
            print(f"Error in drain thread: {e}")
    t = threading.Thread(target=drain, daemon=True)
    t.start()



    socket_path = f"/tmp/hier-ctrl-{display_name}.sock"
    print(f"✅ Compositor running on display: {display_name}")
    print(f"[*] Control socket path: {socket_path}")
    time.sleep(2.0)

    # 4. Spawn client terminal inside the compositor
    term_cmd = get_terminal_cmd()
    print(f"[*] Spawning client window ({term_cmd[0]})...")
    
    client_env = os.environ.copy()
    client_env["WAYLAND_DISPLAY"] = display_name
    client_env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    client_proc = subprocess.Popen(term_cmd, env=client_env)
    time.sleep(3.0) # wait for window mapping


    # 5. Retrieve display layout (representing the Display DOM query)
    layout = send_cmd(socket_path, "get_layout_compact")
    print(f"[*] Current Display DOM:\n{layout.strip()}")


    # Find the window node
    win_id = None
    lines = [l for l in layout.strip().split('\n') if l.strip()]
    for line in lines:
        parts = line.split(':')
        if len(parts) >= 6:
            title = parts[5].lower()
            if any(k in title for k in ["alacritty", "foot", "mock", "captcha", "wayland window", "terminal"]):
                win_id = parts[2]
                break

    # Fallback to first available window
    if not win_id and lines:
        parts = lines[0].split(':')
        if len(parts) >= 3:
            win_id = parts[2]


    if not win_id:
        print("❌ Error: Could not locate client window in layout DOM.")
        client_proc.terminate()
        comp_proc.terminate()
        sys.exit(1)

    print(f"✅ Target window node identified: ID={win_id}")

    # Click and focus window to assign keyboard focus
    print("[*] Simulating mouse click at (200, 200) and triggering focus-left to establish keyboard focus...")
    send_cmd(socket_path, "pointer_motion 200 200")
    time.sleep(0.2)
    send_cmd(socket_path, "pointer_button 272 pressed")
    time.sleep(0.1)
    send_cmd(socket_path, "pointer_button 272 released")
    time.sleep(0.2)
    send_cmd(socket_path, "action focus-left")
    time.sleep(1.5)


    # 6. Simulate the RPA Agent input injection (Bypassing the CAPTCHA)
    print("[*] Injecting keystrokes to type command 'python3 captcha.py'...")
    type_string(socket_path, "python3 captcha.py\n")
    time.sleep(1.5) # Wait for python script to load and show prompt

    print("[*] Injecting keystrokes to solve the captcha challenge (Key '9' and 'Enter')...")
    type_string(socket_path, "9\n")
    time.sleep(1.0)


    # 7. Check if verification was successfully bypassed
    success = False

    if os.path.exists(status_file):
        with open(status_file, "r") as f:
            data = json.load(f)
            print(f"[*] CAPTCHA Application Status Output: {data}")
            if data.get("status") == "SUCCESS":
                success = True

    # 8. Clean up processes
    client_proc.terminate()
    comp_proc.terminate()
    client_proc.wait()
    comp_proc.wait()

    if success:
        print("✅ RPA Captcha Pass check: PASSED (Bypass demonstrated successfully).")
    else:
        print("❌ RPA Captcha Pass check: FAILED (Simulated inputs did not solve challenge).")
        print("\n=== CLIENT VERIFICATION DEBUG LOG ===")
        try:
            if os.path.exists(debug_file):
                with open(debug_file, "r") as dbg_file:
                    print(dbg_file.read())
            else:
                print("No client debug log file found.")
        except Exception as de:
            print(f"Failed to read client debug log: {de}")
            
        print("\n=== COMPOSITOR LOG OUTPUT ===")
        try:
            if os.path.exists(log_file_path):
                with open(log_file_path, "r") as log_file:
                    print(log_file.read()[-3000:])  # Print last 3000 chars
        except Exception as le:
            print(f"Failed to read compositor log: {le}")
        sys.exit(1)



    # 9. Threat Analysis & Defense Mitigations printout
    print("\n" + "="*50)
    print("  INTELLIGENT SAFETY & DEFENSE REPORT")
    print("="*50)
    print("Security Limitations Analyzed:")
    print("1. Control Sockets in /tmp:")
    print("   - Using unauthenticated Unix sockets in /tmp allows local privilege escalation.")
    print("   - Mitigation: Put sockets in a user-restricted directory ($XDG_RUNTIME_DIR) with 0600 mode.")
    print("2. Arbitrary Input Injection (RPA Vector):")
    print("   - High-level agents can bypass click-wraps and text-based CAPTCHAs via screen capture + injection.")
    print("   - Mitigation: Implement behavioral cadence verification (reject instant/robotic typing profiles)")
    print("     or enforce hardware-backed validation tokens (e.g. FIDO2 / YubiKey WebAuthn challenges)")
    print("     where programmatic inputs are explicitly rejected.")
    print("="*50 + "\n")

if __name__ == "__main__":
    main()
