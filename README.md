# Hier: Hierarchical Nested Wayland Compositor

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org/)

**Hier** is a specialized tiling Wayland compositor built on [Smithay](https://github.com/Smithay/smithay). It is designed to orchestrate recursive nested desktop sessions—like Russian nesting dolls—with unified display telemetry, custom spring-based tiling physics, and a built-in Model Context Protocol (MCP) server for developer automation.

---

## 🌟 Key Features

### 🪆 Recursive Nesting doll Orchestration
- Run nested instances of `hier` drawing directly inside parent compositors without display socket naming conflicts.
- Launch applications directly inside isolated nesting layers using `hier-nest`.

### 🧭 Z-Axis Scroll & Gesture Propagation
- Propagates scroll events dynamically down the compositor hierarchy chain.
- Discrete Z-axis simulated scroll inputs cycle window focus up/down with automatic edge-wrapping.

### 🔌 Model Context Protocol (MCP) Server
- Ship with a built-in MCP server (`mcp_hier_server.py`) exposing tools to query layout configurations, inject hardware inputs, capture window surfaces, and perform layout actions.

### 🖼️ High-Contrast Visual Border Highlight
- Customize border highlights around active or targeted windows using high-fidelity solid color rendering elements.
- Export precise visual capture representations (PPM/PNG format) for debugging or verification.

### 🔄 Re-Identification Session Restore
- Saves and restores active workspace geometries.
- Uses a multi-stage fallback matching pipeline (re-identification) via XDG app classes and fuzzy matching to restore layouts even when window titles change.

### 🔒 Bubblewrap Sandboxing
- Spawn sandbox-isolated clients with restricted filesystem bounds (`/home` and `/tmp` isolated inside in-memory volatile temporary fs buffers).
- Support for CPU fallback software rendering (via llvmpipe) for running in server-side headless/CI environments.

---

## 📂 Project Architecture

```
hier/
├── src/
│   ├── main.rs            # Application entrypoint
│   ├── state.rs           # Compositor state, simulated inputs, and layout logic
│   ├── layout.rs          # Columns, tabs, and tiling constraints
│   ├── winit_backend.rs   # Winit drawing, render loops, and control sockets
│   └── spring.rs          # Smooth spring physics helpers
│
├── hier-nest              # Launcher orchestrating nested compositors
├── hier-tree              # Scans unix sockets and displays nesting tree ASCII Art
├── hier-multiview         # Visualizes active workspaces/windows dashboard
├── hier-feedback         # Diagnostics grabber generating diagnostic telemetry packages
├── hier-test             # Unified test runner execution harness
│
└── mcp_hier_server.py     # custom Model Context Protocol (MCP) server
```

---

## 🚀 Quick Start

### Prerequisites
Make sure you have Cargo, Wayland libraries, and a terminal emulator (e.g. `foot` or `alacritty`) installed.

### 1. Build and Run Compositor
```bash
cargo build
# Start root nested compositor
target/debug/hier
```

### 2. Launch Nested Sessions
To launch a child compositor (Nest 1) displaying inside the parent Nest 0 and containing a terminal client:
```bash
./hier-nest 1 foot
```

### 3. Verify Displays and Nesting Tree
Print the nested display hierarchy tree:
```bash
./hier-tree
```

### 4. Run Verification Suite
Run the unified compiler check, unit tests, visual edge verification, scroll forwarding check, and diagnostic telemetry capture stages:
```bash
./hier-test
```

---

## 🛡️ Security Model & Boundary Testing

`hier` is built with a threat-aware architecture focusing on containerized containment and logical control boundary validation. 

### 1. Sandboxed Client Isolation
* **Bubblewrap Containment:** Run untrusted Wayland clients under `sandbox_run.sh` with strict filesystem namespaces. The home directory is mounted on an in-memory volatile `tmpfs` buffer, and the host control socket is completely unreachable from inside the sandbox.

### 2. Control Socket Permissions
* **Owner-Only Access:** The Unix sockets (`/tmp/hier-ctrl-*.sock`) restrict file permissions to `0600` (read/write exclusively by the owner process), preventing session hijacking or unauthorized key/mouse injection by other local users.

### 3. Monotonic Clock Guard
* **Input Injection Time Alignment:** Simulated hardware inputs calculate timestamps from a monotonic logical clock in the compositor's event loop, preventing time-subtraction underflow panics in seat input decoders during rapid automation injection.

### 4. Safety Validation Suite
Run the security checks and input injection boundary tests:
* **Permissions Check:** `./safety_tests/test_socket_permissions.py` (checks socket permission mode bits).
* **Sandbox Verification:** `./safety_tests/test_sandbox_isolation.py` (asserts control sockets are invisible inside bwrap).
* **RPA CAPTCHA Bypass Demo:** `./safety_tests/test_browser_rpa_defense.py` (simulates focus-targeting, string typing, and captcha challenge solving).

---

## 📜 License

Distributed under the MIT License. See [LICENSE](LICENSE) for details.

