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

def main():
    print("[*] Starting integration test for scrolling position layout API...")
    
    # Start compositor in software rendering mode
    env = os.environ.copy()
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    
    # Run parent compositor
    proc = subprocess.Popen(
        ["target/debug/hier"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
        text=True
    )
    
    display_name = None
    socket_path = None
    proc_child = None
    
    try:
        # Parse display and socket info
        start_time = time.time()
        while time.time() - start_time < 5.0:
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
            print("❌ Failed to parse display/socket path from compositor startup.")
            proc.terminate()
            sys.exit(1)
            
        print(f"✅ Compositor started: Display={display_name}, Socket={socket_path}")
        time.sleep(1.0)
        
        # 1. Get initial layout (which automatically maps mock windows in sandbox/headless mode if any, or wait let's spawn a couple of windows)
        print("[*] Spawning a few mock windows via action spawn-terminal...")
        send_cmd(socket_path, "action spawn-terminal")
        time.sleep(0.5)
        send_cmd(socket_path, "action spawn-terminal")
        time.sleep(0.5)
        
        # Query get_layout JSON
        res_layout = send_cmd(socket_path, "get_layout").strip()
        print(f"[Test 1] get_layout JSON output:\n{res_layout}")
        
        layout = json.loads(res_layout)
        
        # Verify the custom scrolling position fields inside windows
        found_window = False
        for ws in layout.get("workspaces", []):
            for col in ws.get("columns", []):
                for win in col.get("windows", []):
                    found_window = True
                    print(f"Checking Window {win['id']} - {win['title']}")
                    assert "scrolling_position" in win, "Window JSON missing 'scrolling_position'"
                    assert "scrolling_position_formatted" in win, "Window JSON missing 'scrolling_position_formatted'"
                    assert "z_axis" in win, "Window JSON missing 'z_axis'"
                    
                    sp = win["scrolling_position"]
                    assert "column" in sp, "scrolling_position missing 'column'"
                    assert "tile" in sp, "scrolling_position missing 'tile'"
                    assert "z_axis" in sp, "scrolling_position missing 'z_axis'"
                    
                    formatted = win["scrolling_position_formatted"]
                    print(f"Formatted value: {formatted}")
                    # Format: column(col_idx) ; tile)win_idx ; z axis(win_z)
                    match = re.match(r"^column\(\d+\) ; tile\)\d+ ; z axis\([-+]?[0-9]*\.?[0-9]+\)$", formatted)
                    assert match, f"scrolling_position_formatted does not match expected format: {formatted}"
        
        assert found_window, "No windows found in layout to verify"
        print("✅ Test 1 Passed: get_layout JSON contains correct scrolling position structures and template strings.")
        
        # 2. Test get_scrolling_position command
        res_sp = send_cmd(socket_path, "get_scrolling_position").strip()
        print(f"[Test 2] get_scrolling_position (active) output: {res_sp}")
        match = re.match(r"^Scrolling Position: column\(\d+\) ; tile\)\d+ ; z axis\([-+]?[0-9]*\.?[0-9]+\)$", res_sp)
        assert match, f"get_scrolling_position output does not match expected format: {res_sp}"
        print("✅ Test 2 Passed: get_scrolling_position command returned the correct format.")
        
        # 3. Test get_scrolling_position <window_id> command
        # Let's get the first window id from layout
        first_win_id = layout["workspaces"][0]["columns"][0]["windows"][0]["id"]
        res_sp_id = send_cmd(socket_path, f"get_scrolling_position {first_win_id}").strip()
        print(f"[Test 3] get_scrolling_position {first_win_id} output: {res_sp_id}")
        match = re.match(r"^Scrolling Position: column\(\d+\) ; tile\)\d+ ; z axis\([-+]?[0-9]*\.?[0-9]+\)$", res_sp_id)
        assert match, f"get_scrolling_position <id> output does not match expected format: {res_sp_id}"
        print("✅ Test 3 Passed: get_scrolling_position <window_id> command returned the correct format.")
        
        # 4. Test reposition_window command
        # Move window 1 (which starts at ws 0, col 0, tile 0) to workspace 0, column 0, tile 0 (it should stack as a tab group in column 0!)
        print("[*] Repositioning Window 1 to Workspace 0, Column 0, Tile 0...")
        res_rep = send_cmd(socket_path, "reposition_window 1 0 0 0").strip()
        print(f"[Test 4] reposition_window output: {res_rep}")
        assert res_rep == "ok", f"Expected reposition_window to return 'ok', got: {res_rep}"
        
        # Query get_layout JSON to verify window positions
        res_layout_rep = send_cmd(socket_path, "get_layout").strip()
        print(f"[Test 4] get_layout JSON output after reposition:\n{res_layout_rep}")
        layout_rep = json.loads(res_layout_rep)
        
        # Verify Window 1 is now in Workspace 0, Column 1 (tiled/stacked with Window 2!)
        ws0 = layout_rep["workspaces"][0]
        # Since column 0 became empty when window 1 was removed, it should have been deleted!
        # So workspace 0 should now have only 1 column, containing both Window 1 and Window 2 stacked!
        print(f"Columns count after reposition: {len(ws0['columns'])}")
        assert len(ws0["columns"]) == 1, "Expected 1 column (column 0 deleted, leaving column 1 which becomes the new column 0)"
        
        column = ws0["columns"][0]
        assert len(column["windows"]) == 2, f"Expected 2 windows stacked as tabs, got: {len(column['windows'])}"
        
        # Check window IDs inside the column (ordered)
        win_ids = [w["id"] for w in column["windows"]]
        print(f"Window IDs in column: {win_ids}")
        assert 1 in win_ids and 2 in win_ids, "Expected window 1 and 2 to be in the column"
        
        # Check that scrolling positions reflect the new positions
        res_sp_id_after = send_cmd(socket_path, "get_scrolling_position 1").strip()
        print(f"[Test 4] get_scrolling_position 1 after reposition: {res_sp_id_after}")
        assert "column(0) ; tile)0" in res_sp_id_after or "column(0) ; tile)1" in res_sp_id_after, f"Expected Window 1 position to update, got: {res_sp_id_after}"
        print("✅ Test 4 Passed: reposition_window successfully moved window and updated layout dynamically.")

        # 5. Test Z-Axis cut promotion from nested child compositor to parent
        print("\n[*] Starting Test 5: Z-Axis window promotion cut from nested compositor...")
        env_child = os.environ.copy()
        env_child["LIBGL_ALWAYS_SOFTWARE"] = "1"
        proc_child = subprocess.Popen(
            ["target/debug/hier"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=env_child,
            text=True
        )
        
        child_disp = None
        child_sock = None
        start_time_c = time.time()
        while time.time() - start_time_c < 5.0:
            line = proc_child.stdout.readline()
            if not line:
                time.sleep(0.05)
                continue
            match_disp = re.search(r"WAYLAND_DISPLAY=(wayland-\d+)", line)
            if match_disp:
                child_disp = match_disp.group(1)
            match_sock = re.search(r"Control socket listening at: (/tmp/hier-ctrl-wayland-\d+\.sock)", line)
            if match_sock:
                child_sock = match_sock.group(1)
            if child_disp and child_sock:
                break
                
        if not child_disp or not child_sock:
            print("❌ Failed to start child compositor.")
            proc_child.terminate()
            proc_child.wait()
            sys.exit(1)
            
        print(f"✅ Child compositor started: Display={child_disp}, Socket={child_sock}")
        time.sleep(1.0)
        
        # Spawn window inside child compositor
        send_cmd(child_sock, "action spawn-terminal")
        time.sleep(0.5)
        
        # Check child layout to verify Window 1 is in the child
        layout_c = json.loads(send_cmd(child_sock, "get_layout"))
        win_title_c = layout_c["workspaces"][0]["columns"][0]["windows"][0]["title"]
        print(f"Child window title: {win_title_c}")
        
        # Invoke cut_window on parent compositor to pull window 1 from child compositor
        print(f"[*] Invoking cut_window on parent compositor to pull window 1 from child display {child_disp}...")
        res_cut = send_cmd(socket_path, f"cut_window {child_disp} 1").strip()
        print(f"cut_window response: {res_cut}")
        assert "ok: promoted window" in res_cut, f"Expected cut_window success message, got: {res_cut}"
        
        # Verify window is removed from child compositor (moved to 999 999 999 index)
        layout_c_after = json.loads(send_cmd(child_sock, "get_layout"))
        assert len(layout_c_after["workspaces"][0]["columns"]) == 0, "Expected window 1 to be removed from child active layout"
        print("✅ Window successfully removed from child active workspace columns.")
        
        # Verify window is spawned in parent compositor with Custom Access title and highlight border
        layout_p_after = json.loads(send_cmd(socket_path, "get_layout"))
        found_promoted = False
        for ws in layout_p_after["workspaces"]:
            for col in ws["columns"]:
                for win in col["windows"]:
                    if "[Custom Access Promoted]" in win["title"]:
                        found_promoted = True
                        print(f"Found promoted window in parent: {win['title']}")
                        
        assert found_promoted, "Expected to find promoted window in parent compositor layout"
        print("✅ Promoted window successfully found in parent layout.")
        print("✅ Test 5 Passed: Z-Axis cut promotion works flawlessly.")
        
    finally:
        print("[*] Cleaning up compositor processes...")
        if proc_child:
            proc_child.terminate()
            proc_child.wait()
        proc.terminate()
        proc.wait()
        print("[*] Compositors terminated.")

if __name__ == "__main__":
    main()
