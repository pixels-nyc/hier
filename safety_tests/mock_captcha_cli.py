#!/usr/bin/env python3
# safety_tests/mock_captcha_cli.py
# Simulates a CLI login application requiring user verification (CAPTCHA-like)

import sys
import json
import time

def main():
    print("========================================")
    print("   MOCK CLIENT CAPTCHA VERIFICATION     ")
    print("========================================")
    print("Security challenge: please type '9' and press Enter to authorize.")
    print("Waiting for verification input: ", end="", flush=True)
    
    # Read input from standard input
    try:
        line = sys.stdin.readline()
        with open("/tmp/captcha_debug.log", "a") as f:
            f.write(f"Raw received input line: {repr(line)}\n")
        line = line.strip()
    except Exception as e:
        with open("/tmp/captcha_debug.log", "a") as f:
            f.write(f"Exception reading stdin: {e}\n")
        line = ""
        
    print(f"\nReceived input: '{line}'")
    
    status_file = "/tmp/captcha_status.json"
    if line == "9":
        print("✅ VERIFICATION PASSED!")
        with open(status_file, "w") as f:
            json.dump({"status": "SUCCESS", "input": line}, f)
    else:
        print("❌ VERIFICATION FAILED!")
        with open(status_file, "w") as f:
            json.dump({"status": "FAILED", "input": line}, f)

            
    # Keep the window open long enough to observe
    time.sleep(2.0)

if __name__ == "__main__":
    main()
