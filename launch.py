#!/usr/bin/env python3
# launch.py
# Basic launch utility for the Hier nested Wayland compositor.

import os
import sys
import subprocess
import time
import shutil
import re

def get_terminal():
    for term in ["foot", "alacritty", "kitty", "xterm"]:
        if shutil.which(term):
            return term
    return None

def main():
    print("=== Hier Nested Compositor Launcher ===")
    
    # 1. Compile the project
    print("[*] Compiling Rust compositor...")
    try:
        subprocess.run(["cargo", "build"], check=True)
        print("✅ Compilation successful!")
    except subprocess.CalledProcessError:
        print("❌ Error: Compilation failed.")
        sys.exit(1)
        
    # 2. Check for optional --fullscreen/-f flag
    fullscreen = "--fullscreen" in sys.argv or "-f" in sys.argv
    env = os.environ.copy()
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    if fullscreen:
        env["HIER_FULLSCREEN"] = "1"
        print("[*] Fullscreen mode enabled.")
        
    # 3. Start the compositor process
    comp_bin = "./target/debug/hier"
    print(f"[*] Launching compositor from: {comp_bin}")
    comp_proc = subprocess.Popen(
        [comp_bin],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=env,
        text=True
    )
    
    # 4. Wait for compositor to declare its WAYLAND_DISPLAY socket name
    display_name = None
    start_time = time.time()
    while True:
        if time.time() - start_time > 15.0:
            print("❌ Error: Timeout waiting for WAYLAND_DISPLAY environment string.")
            comp_proc.terminate()
            sys.exit(1)
            
        line = comp_proc.stdout.readline()
        if not line:
            ret = comp_proc.poll()
            if ret is not None:
                print(f"❌ Error: Compositor exited with code {ret}")
                sys.exit(1)
            time.sleep(0.05)
            continue
            
        # Print compositor logs to terminal
        sys.stdout.write(f"[Compositor] {line}")
        sys.stdout.flush()
        
        # Parse WAYLAND_DISPLAY
        match = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
        if match:
            display_name = match.group(1)
            print(f"✅ Compositor is running on WAYLAND_DISPLAY={display_name}")
            break
            
    # Drain compositor stdout in the background to prevent blocking
    import threading
    def drain():
        try:
            while True:
                line = comp_proc.stdout.readline()
                if not line:
                    if comp_proc.poll() is not None:
                        break
                    time.sleep(0.1)
                    continue
                # Print compositor logs
                sys.stdout.write(f"[Compositor] {line}")
                sys.stdout.flush()
        except Exception:
            pass
            
    t = threading.Thread(target=drain, daemon=True)
    t.start()
    
    # 5. Spawn an initial terminal emulator client in the nested session
    terminal = get_terminal()
    if terminal:
        print(f"[*] Launching initial client terminal ({terminal}) on display {display_name}...")
        client_env = os.environ.copy()
        client_env["WAYLAND_DISPLAY"] = display_name
        client_env["LIBGL_ALWAYS_SOFTWARE"] = "1"
        
        # Launch terminal client
        subprocess.Popen([terminal], env=client_env)
    else:
        print("⚠️ Warning: No compatible terminal emulator (foot, alacritty, kitty, xterm) found to spawn.")
        
    print("\n--------------------------------------------------")
    print(f"Compositor running. Press Ctrl+C to terminate.")
    print("--------------------------------------------------\n")
    
    try:
        # Keep launcher alive until compositor exits or Ctrl+C is pressed
        comp_proc.wait()
    except KeyboardInterrupt:
        print("\n[*] Terminating compositor process...")
        comp_proc.terminate()
        comp_proc.wait()
        print("✅ Stopped.")

if __name__ == "__main__":
    main()
