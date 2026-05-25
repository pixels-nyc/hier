#!/usr/bin/env bash
# hier-dev-test-monitor.sh
# Wrapper to run a specified test script (e.g., a Python test) and display its output in the terminal
# with timestamps for easier debugging.
# Usage: ./hier-dev-test-monitor.sh path/to/test_script.py

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <test_script_path>"
  exit 1
fi

TEST_SCRIPT="$1"

if [[ ! -f "$TEST_SCRIPT" ]]; then
  echo "Error: Test script '$TEST_SCRIPT' not found." >&2
  exit 1
fi

# Run the test script and prefix each line with a timestamp
python3 "$TEST_SCRIPT" 2>&1 | while IFS= read -r line; do
  printf "[%s] %s\n" "$(date +%H:%M:%S)" "$line"
  # If the test script outputs a special "quit" line, stop monitoring
  if [[ "$line" == "quit" ]]; then
    echo "[monitor] Received quit signal – exiting."
    break
  fi
done
