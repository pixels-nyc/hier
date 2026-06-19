#!/usr/bin/env python3
# safety_tests/test_cookie_net_isolation.py
# Verification of cookie home path isolation and network namespace unsharing

import os
import sys
import subprocess
import shutil

def test_cookie_filesystem_isolation():
    print("[*] Testing filesystem isolation with state cookie...")
    cookie_id = "test-fs-cookie"
    cookie_dir = os.path.expanduser(f"~/.cache/hier/cookies/{cookie_id}")
    
    # Ensure clean state
    if os.path.exists(cookie_dir):
        shutil.rmtree(cookie_dir)
        
    host_target = os.path.expanduser("~/hier-fs-isolation-test-file.txt")
    if os.path.exists(host_target):
        os.remove(host_target)

    # Launch nested command writing to home directory
    cmd = ["./hier-nest", "2", "--cookie", cookie_id, "python3", "-c", 
           "import os; f=open(os.path.expanduser('~/hier-fs-isolation-test-file.txt'), 'w'); f.write('sandbox-data'); f.close()"]
    
    print(f"[*] Running: {' '.join(cmd)}")
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    
    # Check if we got any compositor output or setup text, find stdout line
    lines = res.stdout.strip().split("\n")
    print(f"[*] Sandbox stdout lines: {lines}")
    if res.stderr.strip():
        print(f"[*] Sandbox stderr: {res.stderr.strip()}")
        
    # Check filesystem boundaries
    exists_on_host = os.path.exists(host_target)
    cookie_file_path = os.path.join(cookie_dir, "home/hier-fs-isolation-test-file.txt")
    exists_in_cookie = os.path.exists(cookie_file_path)
    
    print(f"[*] File exists on Host home: {exists_on_host}")
    print(f"[*] File exists in Cookie home: {exists_in_cookie}")
    
    success = False
    if exists_in_cookie and not exists_on_host:
        with open(cookie_file_path, "r") as f:
            content = f.read()
        if content == "sandbox-data":
            print("✅ Cookie filesystem isolation verification: PASSED")
            success = True
        else:
            print(f"❌ File content mismatch: {content}")
    else:
        print("❌ Cookie filesystem isolation: FAILED")
        
    # Clean up
    if os.path.exists(cookie_dir):
        shutil.rmtree(cookie_dir)
    return success

def test_network_namespace_isolation():
    print("[*] Testing network namespace isolation...")
    
    # 1. Test net none
    cmd_none = ["./hier-nest", "2", "--net", "none", "python3", "-c", 
                "import os; lines=open('/proc/net/dev').readlines(); print('NET_' + 'INTERFACES_LIST:' + str([l.split(':')[0].strip() for l in lines[2:] if l.strip()]))" ]
    print(f"[*] Running: {' '.join(cmd_none)}")
    res_none = subprocess.run(cmd_none, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    
    interfaces_none = []
    print(f"[*] Raw none stdout:\n{res_none.stdout}")
    if res_none.stderr.strip():
        print(f"[*] Raw none stderr:\n{res_none.stderr}")
    for line in res_none.stdout.split("\n"):
        if "NET_INTERFACES_LIST:" in line:
            interfaces_none = eval(line.split("NET_INTERFACES_LIST:")[-1].strip())
            break
            
    print(f"[*] Interfaces in 'none' mode: {interfaces_none}")
    
    # 2. Test net host
    cmd_host = ["./hier-nest", "2", "--net", "host", "python3", "-c", 
                "import os; lines=open('/proc/net/dev').readlines(); print('NET_' + 'INTERFACES_LIST:' + str([l.split(':')[0].strip() for l in lines[2:] if l.strip()]))" ]
    print(f"[*] Running: {' '.join(cmd_host)}")
    res_host = subprocess.run(cmd_host, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    
    interfaces_host = []
    print(f"[*] Raw host stdout:\n{res_host.stdout}")
    if res_host.stderr.strip():
        print(f"[*] Raw host stderr:\n{res_host.stderr}")
    for line in res_host.stdout.split("\n"):
        if "NET_INTERFACES_LIST:" in line:
            interfaces_host = eval(line.split("NET_INTERFACES_LIST:")[-1].strip())
            break
            
    print(f"[*] Interfaces in 'host' mode: {interfaces_host}")
    
    if len(interfaces_none) == 1 and "lo" in interfaces_none and len(interfaces_host) > 1:
        print("✅ Network namespace isolation verification: PASSED")
        return True
    else:
        print("❌ Network namespace isolation: FAILED")
        return False

def main():
    print("==================================================")
    print("  SECURITY TEST: COOKIE PERSISTENCE & NET ISOLATION")
    print("==================================================")
    fs_ok = test_cookie_filesystem_isolation()
    net_ok = test_network_namespace_isolation()
    print("==================================================")
    
    if fs_ok and net_ok:
        print("🎉 ALL ISOLATION TESTS PASSED SUCCESSFULLY!")
        sys.exit(0)
    else:
        print("❌ SOME ISOLATION TESTS FAILED.")
        sys.exit(1)

if __name__ == "__main__":
    main()
