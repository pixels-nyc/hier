#!/usr/bin/env python3
# mcp_hier_server.py
# Model Context Protocol (MCP) Server for Hier Nesting Compositor and RPA Display Management

import sys
import os
import json
import socket
import re
import traceback

# --- WAYLAND DISPLAY HIERARCHY SCANNER (from hier-tree) ---

def get_inode_to_socket_map():
    inode_to_socket = {}
    try:
        with open("/proc/net/unix", "r") as f:
            lines = f.readlines()
    except Exception:
        return inode_to_socket

    for line in lines[1:]:
        parts = line.strip().split()
        if len(parts) >= 8:
            inode = parts[6]
            path = parts[7]
            match = re.search(r"wayland-\d+$", path)
            if match:
                inode_to_socket[inode] = match.group(0)
    return inode_to_socket

def get_process_info(inode_to_socket):
    processes = {}
    try:
        pids = [d for d in os.listdir("/proc") if d.isdigit()]
    except Exception:
        return processes
    
    for pid_str in pids:
        try:
            pid = int(pid_str)
            proc_dir = f"/proc/{pid}"
            with open(f"{proc_dir}/comm", "r") as f:
                name = f.read().strip()
        except (IOError, ValueError):
            continue

        if name in ("systemd", "dbus-daemon", "dbus-broker", "pipewire", "wireplumber"):
            continue

        parent_display = None
        try:
            with open(f"{proc_dir}/environ", "rb") as f:
                environ_bytes = f.read()
            environ_strs = environ_bytes.decode("utf-8", errors="ignore").split("\x00")
            for entry in environ_strs:
                if entry.startswith("WAYLAND_DISPLAY="):
                    parent_display = entry.split("=", 1)[1]
                    break
        except Exception:
            pass

        owned_displays = []
        try:
            fd_dir = f"{proc_dir}/fd"
            for fd in os.listdir(fd_dir):
                fd_path = f"{fd_dir}/{fd}"
                try:
                    target = os.readlink(fd_path)
                    match = re.match(r"socket:\[(\d+)\]", target)
                    if match:
                        inode = match.group(1)
                        if inode in inode_to_socket:
                            display = inode_to_socket[inode]
                            if display not in owned_displays:
                                owned_displays.append(display)
                except Exception:
                    pass
        except Exception:
            pass

        processes[pid] = {
            "pid": pid,
            "name": name,
            "parent_display": parent_display,
            "owned_displays": owned_displays
        }
    return processes

def build_hierarchy():
    inode_map = get_inode_to_socket_map()
    procs = get_process_info(inode_map)
    
    display_owners = {}
    for pid, info in procs.items():
        for d in info["owned_displays"]:
            display_owners[d] = pid

    display_clients = {}
    for pid, info in procs.items():
        p_display = info["parent_display"]
        if p_display:
            display_clients.setdefault(p_display, []).append(pid)

    return procs, display_owners, display_clients

def build_ascii_tree():
    procs, display_owners, display_clients = build_hierarchy()
    
    all_displays = set(display_clients.keys()) | set(display_owners.keys())
    root_displays = sorted(list(all_displays - set(display_owners.keys())))
    if not root_displays and all_displays:
        root_displays = ["wayland-1"]

    lines = []
    lines.append("==================================================")
    lines.append("      WAYLAND NESTING DISPLAY HIERARCHY TREE      ")
    lines.append("==================================================")

    def render_display_node(display, prefix, is_last):
        owner_pid = display_owners.get(display)
        owner_info = f" (owned by PID {owner_pid}: {procs[owner_pid]['name']})" if owner_pid else " (Host Display)"
        lines.append(f"{prefix}{'└── ' if is_last else '├── '}[{display}]{owner_info}")
        
        clients = display_clients.get(display, [])
        child_display_procs = []
        flat_clients = []
        for c_pid in clients:
            c_info = procs.get(c_pid)
            if c_info and c_info["owned_displays"]:
                child_display_procs.append(c_pid)
            else:
                flat_clients.append(c_pid)

        child_prefix = prefix + ("    " if is_last else "│   ")
        for idx, c_pid in enumerate(flat_clients):
            c_is_last = (idx == len(flat_clients) - 1) and (len(child_display_procs) == 0)
            c_name = procs[c_pid]["name"]
            lines.append(f"{child_prefix}{'└── ' if c_is_last else '├── '}Process PID {c_pid}: {c_name}")

        for idx, c_pid in enumerate(child_display_procs):
            c_is_last = (idx == len(child_display_procs) - 1)
            for owned_d in procs[c_pid]["owned_displays"]:
                render_display_node(owned_d, child_prefix, c_is_last)

    for idx, root_d in enumerate(root_displays):
        render_display_node(root_d, "", idx == len(root_displays) - 1)
    
    return "\n".join(lines)

# --- COMPOSITOR CONTROL COMMUNICATIONS ---

def send_command(display: str, cmd: str) -> str:
    if not display:
        display = os.environ.get("WAYLAND_DISPLAY", "wayland-1")
    socket_path = f"/tmp/hier-ctrl-{display}.sock"
    if not os.path.exists(socket_path):
        socket_path = "/tmp/hier-ctrl.sock"
    
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2.0)
        s.connect(socket_path)
        s.sendall((cmd + "\n").encode())
        res = s.recv(32768).decode()
        s.close()
        return res
    except Exception as e:
        return f"error: socket connection failed to {socket_path}: {e}\n"

# --- JSON-RPC / MCP SERVER PROTOCOL ENGINE ---

def respond(msg_id, result=None, error=None):
    response = {"jsonrpc": "2.0"}
    if msg_id is not None:
        response["id"] = msg_id
    if error:
        response["error"] = error
    else:
        response["result"] = result
    
    sys.stdout.write(json.dumps(response) + "\n")
    sys.stdout.flush()

def handle_tools_list():
    tools = [
        {
            "name": "get_display_tree",
            "description": "Get the hierarchical ASCII tree of all nested Wayland compositor displays and their running client processes.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "get_layout",
            "description": "Get the workspace and window layout details from the targeted compositor instance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "display": {
                        "type": "string",
                        "description": "Optional Wayland display name (e.g. 'wayland-2'). If omitted, uses the current WAYLAND_DISPLAY."
                    }
                }
            }
        },
        {
            "name": "get_scrolling_position",
            "description": "Get the scrolling position (column index, window tile index, and z-axis progress) of a specific window or the focused window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window_id": {
                        "type": "integer",
                        "description": "Optional window ID. If omitted, uses the currently focused window."
                    },
                    "display": {
                        "type": "string",
                        "description": "Optional Wayland display name."
                    }
                }
            }
        },
        {
            "name": "reposition_window",
            "description": "Reposition a window to a specific workspace index, column index, and window tile index.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window_id": {"type": "integer", "description": "The ID of the window to reposition."},
                    "workspace_idx": {"type": "integer", "description": "The target workspace index."},
                    "column_idx": {"type": "integer", "description": "The target column index within the workspace."},
                    "tile_idx": {"type": "integer", "description": "The target window tile index within the column."},
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                },
                "required": ["window_id", "workspace_idx", "column_idx", "tile_idx"]
            }
        },
        {
            "name": "cut_window",
            "description": "Perform Z-axis 'cut' window promotion: take a window out of a nested child display and place it in the parent compositor (Z) with custom access properties.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "child_display": {"type": "string", "description": "The nested child display name (e.g. 'wayland-2')."},
                    "window_id": {"type": "integer", "description": "The ID of the window to cut/promote from the child compositor."},
                    "display": {"type": "string", "description": "Optional parent display name (e.g. 'wayland-1')."}
                },
                "required": ["child_display", "window_id"]
            }
        },
        {
            "name": "perform_action",
            "description": "Trigger a window layout action (e.g. focus-left, focus-right, focus-up, focus-down, toggle-tab, spawn-terminal, workspace-1, restore-nest-0, fresh-nest, quit).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "description": "The layout action to trigger."},
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                },
                "required": ["action"]
            }
        },
        {
            "name": "inject_input",
            "description": "Inject a raw simulated hardware input command (e.g. keyboard_key, pointer_motion, pointer_button, pointer_axis, pointer_axis_z).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The exact input command string (e.g. 'pointer_axis_z 1.0', 'pointer_motion 100 200')."},
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                },
                "required": ["command"]
            }
        },
        {
            "name": "highlight_window",
            "description": "Set high-contrast highlight border around a window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window_id": {"type": "integer", "description": "The ID of the window to highlight."},
                    "color": {"type": "string", "description": "The highlight color (e.g. 'red', 'green', '#FF00FF')."},
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                },
                "required": ["window_id", "color"]
            }
        },
        {
            "name": "clear_highlight",
            "description": "Clear all active window highlights.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                }
            }
        },
        {
            "name": "capture_window",
            "description": "Export specified window geometry rendering to a PPM image file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window_id": {"type": "integer", "description": "The ID of the window to capture."},
                    "path": {"type": "string", "description": "The absolute file path where the PPM image will be saved."},
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                },
                "required": ["window_id", "path"]
            }
        },
        {
            "name": "save_session",
            "description": "Save current active windows and layout schema state to /tmp/hier-session.json.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                }
            }
        },
        {
            "name": "restore_session",
            "description": "Restore window positions from /tmp/hier-session.json using robust re-identification.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                }
            }
        },
        {
            "name": "get_camera",
            "description": "Get current camera/viewport offsets, dimensions, and tiling mode.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                }
            }
        },
        {
            "name": "set_camera",
            "description": "Move the camera/viewport to specified coordinates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": {"type": "number", "description": "The target x coordinate."},
                    "y": {"type": "number", "description": "The target y coordinate."},
                    "immediate": {"type": "boolean", "description": "Whether to snap immediately (true) or smooth scroll (false)."},
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                },
                "required": ["x", "y"]
            }
        },
        {
            "name": "inject_viewport_input",
            "description": "Inject an input command with viewport-relative mouse coordinates automatically translated to global compositor space.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The input command (e.g. 'pointer_motion 100 200')."},
                    "display": {"type": "string", "description": "Optional Wayland display name."}
                },
                "required": ["command"]
            }
        }
    ]
    return {"tools": tools}

def handle_tools_call(name, args):
    display = args.get("display")
    
    if name == "get_display_tree":
        tree = build_ascii_tree()
        return {
            "content": [{"type": "text", "text": tree}]
        }
        
    elif name == "get_layout":
        res = send_command(display, "get_layout")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "get_scrolling_position":
        win_id = args.get("window_id")
        cmd = "get_scrolling_position"
        if win_id is not None:
            cmd = f"get_scrolling_position {win_id}"
        res = send_command(display, cmd)
        return {
            "content": [{"type": "text", "text": res.strip()}]
        }
        
    elif name == "reposition_window":
        win_id = args["window_id"]
        ws_idx = args["workspace_idx"]
        col_idx = args["column_idx"]
        tile_idx = args["tile_idx"]
        res = send_command(display, f"reposition_window {win_id} {ws_idx} {col_idx} {tile_idx}")
        return {
            "content": [{"type": "text", "text": res.strip()}]
        }
        
    elif name == "cut_window":
        child_display = args["child_display"]
        win_id = args["window_id"]
        res = send_command(display, f"cut_window {child_display} {win_id}")
        return {
            "content": [{"type": "text", "text": res.strip()}]
        }
        
    elif name == "perform_action":
        action = args["action"]
        res = send_command(display, f"action {action}")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "inject_input":
        cmd = args["command"]
        res = send_command(display, cmd)
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "highlight_window":
        win_id = args["window_id"]
        color = args["color"]
        res = send_command(display, f"highlight_window {win_id} {color}")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "clear_highlight":
        res = send_command(display, "clear_highlight")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "capture_window":
        win_id = args["window_id"]
        path = args["path"]
        res = send_command(display, f"capture_window {win_id} {path}")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "save_session":
        res = send_command(display, "save_session")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "restore_session":
        res = send_command(display, "restore_session")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "get_camera":
        res = send_command(display, "get_camera")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "set_camera":
        x = args["x"]
        y = args["y"]
        imm_val = "true" if args.get("immediate") else "false"
        res = send_command(display, f"set_camera {x} {y} {imm_val}")
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    elif name == "inject_viewport_input":
        cmd = args["command"]
        # Check if this is a pointer_motion command with coordinates
        match = re.match(r"^\s*pointer_motion\s+([-\d.]+)\s+([-\d.]+)\s*$", cmd)
        if match:
            # We need to offset the local mouse coordinates using current camera viewport position
            cam_res = send_command(display, "get_camera")
            cam_parts = cam_res.strip().split(",")
            if len(cam_parts) >= 2:
                try:
                    cam_x = float(cam_parts[0])
                    cam_y = float(cam_parts[1])
                    local_x = float(match.group(1))
                    local_y = float(match.group(2))
                    global_x = local_x + cam_x
                    global_y = local_y + cam_y
                    cmd = f"pointer_motion {global_x} {global_y}"
                except ValueError:
                    pass
        res = send_command(display, cmd)
        return {
            "content": [{"type": "text", "text": res}]
        }
        
    else:
        raise ValueError(f"Unknown tool name: {name}")

def main():
    # Stdin/Stdout processing loop
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            respond(None, error={"code": -32700, "message": "Parse error"})
            continue
        
        msg_id = req.get("id")
        method = req.get("method")
        
        if method == "initialize":
            respond(msg_id, result={
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "hier-mcp",
                    "version": "0.1.0"
                }
            })
            
        elif method == "notifications/initialized":
            # No response required for notifications
            continue
            
        elif method == "tools/list":
            res = handle_tools_list()
            respond(msg_id, result=res)
            
        elif method == "tools/call":
            params = req.get("params", {})
            name = params.get("name")
            args = params.get("arguments", {})
            try:
                result = handle_tools_call(name, args)
                respond(msg_id, result=result)
            except Exception as e:
                respond(msg_id, error={"code": -32603, "message": str(e), "data": traceback.format_exc()})
                
        else:
            respond(msg_id, error={"code": -32601, "message": f"Method not found: {method}"})

if __name__ == "__main__":
    main()
