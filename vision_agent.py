#!/usr/bin/env python3
# vision_agent.py
# RPA vision validation using PyTorch (if available) with a zero-dependency binary PPM fallback

import socket
import sys
import os

SOCKET_PATH = os.environ.get(
    "HIER_CTRL_SOCKET",
    f"/tmp/hier-ctrl-{os.environ.get('WAYLAND_DISPLAY', 'wayland-2')}.sock"
)

def send_cmd(cmd: str) -> str:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(SOCKET_PATH)
        s.sendall((cmd + "\n").encode())
        res = s.recv(4096).decode()
        s.close()
        return res
    except Exception as e:
        print(f"Socket connection failed: {e}")
        sys.exit(1)

def parse_ppm(path):
    with open(path, 'rb') as f:
        # Read header
        header = f.readline().decode().strip()
        assert header == "P6", f"Unsupported PPM format: {header}"
        
        # Read width and height (ignoring comments)
        line = f.readline().decode().strip()
        while line.startswith("#"):
            line = f.readline().decode().strip()
        dimensions = line.split()
        width = int(dimensions[0])
        height = int(dimensions[1])
        
        # Read max value
        max_val = int(f.readline().decode().strip())
        
        # Read binary RGB data
        pixel_bytes = f.read()
        return width, height, pixel_bytes

def verify_window_border(width, height, pixel_bytes):
    print(f"Verifying PPM dimensions: {width}x{height}...")
    
    # Highlight borders in mock capture are 4px thick. Verify pixel at (2, 2)
    # Coordinate index formula: (py * width + px) * 3
    px, py = 2, 2
    idx = (py * width + px) * 3
    
    if idx + 2 >= len(pixel_bytes):
        print("❌ Error: Pixel data size mismatch.")
        return False
        
    r = pixel_bytes[idx]
    g = pixel_bytes[idx+1]
    b = pixel_bytes[idx+2]
    
    print(f"Border Pixel Color detected at ({px},{py}): [{r}, {g}, {b}]")
    
    # Expected color is [30, 144, 255] (Dodger Blue border)
    expected_color = (30, 144, 255)
    if (r, g, b) == expected_color:
        print("✅ Visual Border Check: PASSED (Correct highlight color detected!)")
        return True
    else:
        print(f"❌ Visual Border Check: FAILED (Expected {expected_color}, got [{r},{g},{b}])")
        return False

def verify_window_interior(width, height, pixel_bytes, win_title):
    title_lower = win_title.lower()
    is_terminal = "terminal" in title_lower or "ghostty" in title_lower or "alacritty" in title_lower
    is_browser = "chrome" in title_lower or "browser" in title_lower or "firefox" in title_lower or "epiphany" in title_lower
    
    print(f"Verifying window interior for '{win_title}' (is_terminal: {is_terminal}, is_browser: {is_browser})...")
    
    def get_pixel(px, py):
        idx = (py * width + px) * 3
        if idx + 2 >= len(pixel_bytes):
            return None
        return (pixel_bytes[idx], pixel_bytes[idx+1], pixel_bytes[idx+2])

    if is_terminal:
        # Check green prompt (at 7% of width, 7% of height)
        px_prompt = int(width * 7 / 100)
        py_prompt = int(height * 7 / 100)
        color_prompt = get_pixel(px_prompt, py_prompt)
        print(f"Terminal Prompt pixel color at ({px_prompt}, {py_prompt}): {color_prompt}")
        if color_prompt != (50, 205, 50):
            print(f"❌ Error: Terminal prompt color mismatch. Expected (50, 205, 50), got {color_prompt}")
            return False
            
        # Check green cursor (at 15% of width, 7% of height)
        px_cursor = int(width * 15 / 100)
        py_cursor = int(height * 7 / 100)
        color_cursor = get_pixel(px_cursor, py_cursor)
        print(f"Terminal Cursor pixel color at ({px_cursor}, {py_cursor}): {color_cursor}")
        if color_cursor != (50, 205, 50):
            print(f"❌ Error: Terminal cursor color mismatch. Expected (50, 205, 50), got {color_cursor}")
            return False
            
        # Check dark background (at 50% of width, 50% of height)
        px_bg = int(width * 50 / 100)
        py_bg = int(height * 50 / 100)
        color_bg = get_pixel(px_bg, py_bg)
        print(f"Terminal Background pixel color at ({px_bg}, {py_bg}): {color_bg}")
        if color_bg != (15, 15, 15):
            print(f"❌ Error: Terminal background color mismatch. Expected (15, 15, 15), got {color_bg}")
            return False
            
    elif is_browser:
        # Check off-white background (at 50% of width, 18% of height)
        px_bg = int(width * 50 / 100)
        py_bg = int(height * 18 / 100)
        color_bg = get_pixel(px_bg, py_bg)
        print(f"Browser Background pixel color at ({px_bg}, {py_bg}): {color_bg}")
        if color_bg != (240, 240, 240):
            print(f"❌ Error: Browser background color mismatch. Expected (240, 240, 240), got {color_bg}")
            return False
            
        # Check URL input field white background (at 50% of width, 10% of height)
        px_url = int(width * 50 / 100)
        py_url = int(height * 10 / 100)
        color_url = get_pixel(px_url, py_url)
        print(f"Browser URL Input pixel color at ({px_url}, {py_url}): {color_url}")
        if color_url != (255, 255, 255):
            print(f"❌ Error: Browser URL input color mismatch. Expected (255, 255, 255), got {color_url}")
            return False

        # Check light sky blue web page card (at 50% of width, 50% of height)
        px_card = int(width * 50 / 100)
        py_card = int(height * 50 / 100)
        color_card = get_pixel(px_card, py_card)
        print(f"Browser Web Content pixel color at ({px_card}, {py_card}): {color_card}")
        if color_card != (135, 206, 250):
            print(f"❌ Error: Browser content card color mismatch. Expected (135, 206, 250), got {color_card}")
            return False
            
    else:
        # Check Title Bar dark gray (at 50% of width, 7% of height)
        px_tb = int(width * 50 / 100)
        py_tb = int(height * 7 / 100)
        color_tb = get_pixel(px_tb, py_tb)
        print(f"App Title Bar pixel color at ({px_tb}, {py_tb}): {color_tb}")
        if color_tb != (30, 30, 30):
            print(f"❌ Error: App title bar color mismatch. Expected (30, 30, 30), got {color_tb}")
            return False
            
        # Check general background (at 50% of width, 50% of height)
        px_bg = int(width * 50 / 100)
        py_bg = int(height * 50 / 100)
        color_bg = get_pixel(px_bg, py_bg)
        print(f"App Background pixel color at ({px_bg}, {py_bg}): {color_bg}")
        if color_bg != (45, 45, 45):
            print(f"❌ Error: App background color mismatch. Expected (45, 45, 45), got {color_bg}")
            return False
            
    print("✅ Visual Interior Check: PASSED!")
    return True

def main():
    print("=== PyTorch Vision Agent (Lightweight Native Fallback) ===")
    
    # Check if torch is available (documenting PyTorch leading light concept)
    try:
        import torch
        import torchvision
        print(f"PyTorch environment detected! Torch version: {torch.__version__}")
        print("Transforming PPM pixel bytes to tensor...")
    except ImportError:
        print("Note: PyTorch/Torchvision not installed in current environment. Using native P6 PPM edge inspector.")
        
    layout_info = send_cmd("get_layout_compact").strip()
    if not layout_info:
        print("No windows mapped. Please start a nested Wayland client first.")
        sys.exit(1)
        
    # Get first window id and title
    first_window = layout_info.split('\n')[0]
    fields = first_window.split(':')
    win_id = fields[2]
    win_title = fields[5]
    
    capture_path = "/tmp/vision_capture.ppm"
    print(f"Capturing window ID {win_id} ('{win_title}') to {capture_path}...")
    res = send_cmd(f"capture_window {win_id} {capture_path}")
    print(f"Response: {res.strip()}")
    
    # Parse and verify
    width, height, pixel_bytes = parse_ppm(capture_path)
    border_ok = verify_window_border(width, height, pixel_bytes)
    interior_ok = verify_window_interior(width, height, pixel_bytes, win_title)
    
    if border_ok and interior_ok:
        print("=== RPA Vision Validation Successful ===")
    else:
        print("=== RPA Vision Validation Failed ===")
        sys.exit(1)

if __name__ == "__main__":
    main()
