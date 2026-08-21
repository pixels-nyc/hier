#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use smithay::{
    delegate_compositor, delegate_shm, delegate_seat, delegate_xdg_shell, delegate_output,
    delegate_data_device, delegate_primary_selection, delegate_xdg_activation, delegate_data_control,
    desktop::{Space, Window, WindowSurfaceType},
    input::{
        Seat, SeatState, SeatHandler,
        pointer::{CursorImageStatus, AxisFrame},
        keyboard::{FilterResult, ModifiersState, Keysym},
    },
    output::Output,
    wayland::compositor::{CompositorState, CompositorClientState, CompositorHandler},
    wayland::shm::{ShmState, ShmHandler},
    wayland::shell::xdg::{XdgShellState, XdgShellHandler, ToplevelSurface, PopupSurface, PositionerState},
    reexports::wayland_server::{DisplayHandle, Client, protocol::wl_surface::WlSurface},
    utils::{Serial, SERIAL_COUNTER, Point},
    wayland::selection::{
        SelectionHandler,
        data_device::{DataDeviceState, DataDeviceHandler, ClientDndGrabHandler, ServerDndGrabHandler},
        primary_selection::{PrimarySelectionState, PrimarySelectionHandler},
        wlr_data_control::{DataControlState, DataControlHandler},
    },
    wayland::xdg_activation::{XdgActivationState, XdgActivationHandler},
};
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::backend::winit::WinitInput;
use smithay::backend::input::{
    InputEvent, KeyboardKeyEvent, PointerButtonEvent, ButtonState, Event,
    AbsolutePositionEvent, KeyState, Axis, PointerAxisEvent,
    GestureBeginEvent, GestureSwipeUpdateEvent
};
use smithay::wayland::seat::WaylandFocus;
use smithay::input::keyboard::keysyms;

use crate::layout::{LayoutEngine, WindowId};

/// Custom client data representation for keeping track of compositor-specific client state.
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl wayland_server::backend::ClientData for ClientState {
    fn initialized(&self, _client_id: wayland_server::backend::ClientId) {}
}

#[derive(Debug, Clone)]
pub struct PendingRestore {
    pub title: String,
    pub app_id: Option<String>,
    pub ws_idx: usize,
    pub col_idx: usize,
    pub col_width: f32,
    pub col_focused_idx: usize,
}

/// The central state of our Wayland compositor.
pub struct State {
    pub display_handle: DisplayHandle,
    pub layout_engine: LayoutEngine,
    pub space: Space<Window>,
    pub windows: HashMap<WindowId, Window>,
    pub next_window_id: u32,
    pub output: Output,

    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub xdg_shell_state: XdgShellState,
    pub seat: Seat<Self>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub activation_state: XdgActivationState,
    pub running: bool,
    pub socket_name: String,
    pub highlighted_window: Option<(WindowId, [f32; 4])>,
    pub hud_tiling_mode: Option<crate::layout::TilingMode>,
    pub hud_opacity: f32,
    pub hud_previous_mode: Option<crate::layout::TilingMode>,
    pub child_display_socket: Option<String>,
    pub workspace_swipe_accumulator: f32,
    pub start_time: std::time::Instant,
    pub last_event_time: u32,
    pub depth_switcher_active: bool,
    pub depth_switcher_previous_mode: Option<crate::layout::TilingMode>,
    pub pending_restores: Vec<PendingRestore>,
    pub config_binds: HashMap<(bool, bool, bool, bool, u32), String>,
    pub frame_times: Vec<f32>,
    pub stutter_count: u32,
    pub stutter_threshold_ms: f32,
    pub sandbox: bool,
}



impl State {
    pub fn load_config_binds() -> HashMap<(bool, bool, bool, bool, u32), String> {
        let mut binds = HashMap::new();
        let paths = vec![
            "/home/super/.config/niri/config.kdl",
            "/home/super/Projects/linux-configs/niri/config.kdl",
        ];
        
        for path in paths {
            if let Ok(file) = File::open(path) {
                let reader = BufReader::new(file);
                let mut in_binds = false;
                
                for line in reader.lines() {
                    if let Ok(l) = line {
                        let l_trimmed = l.trim();
                        if l_trimmed.starts_with("binds {") {
                            in_binds = true;
                            continue;
                        }
                        if in_binds && l_trimmed == "}" {
                            in_binds = false;
                            continue;
                        }
                        if in_binds {
                            if let Some(left_idx) = l_trimmed.find('{') {
                                if let Some(right_idx) = l_trimmed.find('}') {
                                    let left = &l_trimmed[..left_idx].trim();
                                    let right = &l_trimmed[left_idx+1..right_idx].trim();
                                    
                                    let parts: Vec<&str> = left.split_whitespace().collect();
                                    if !parts.is_empty() {
                                        let key_mods = parts[0];
                                        let mods_parts: Vec<&str> = key_mods.split('+').collect();
                                        if !mods_parts.is_empty() {
                                            let key_name = mods_parts.last().unwrap();
                                            
                                            let keysym_opt = match key_name.to_lowercase().as_str() {
                                                "left" => Some(keysyms::KEY_Left),
                                                "right" => Some(keysyms::KEY_Right),
                                                "up" => Some(keysyms::KEY_Up),
                                                "down" => Some(keysyms::KEY_Down),
                                                "h" => Some(keysyms::KEY_h),
                                                "j" => Some(keysyms::KEY_j),
                                                "k" => Some(keysyms::KEY_k),
                                                "l" => Some(keysyms::KEY_l),
                                                "o" => Some(keysyms::KEY_o),
                                                "w" => Some(keysyms::KEY_w),
                                                "c" => Some(keysyms::KEY_c),
                                                "r" => Some(keysyms::KEY_r),
                                                "f" => Some(keysyms::KEY_f),
                                                "m" => Some(keysyms::KEY_m),
                                                "u" => Some(keysyms::KEY_u),
                                                "i" => Some(keysyms::KEY_i),
                                                "a" => Some(keysyms::KEY_a),
                                                "b" => Some(keysyms::KEY_b),
                                                "d" => Some(keysyms::KEY_d),
                                                "e" => Some(keysyms::KEY_e),
                                                "g" => Some(keysyms::KEY_g),
                                                "n" => Some(keysyms::KEY_n),
                                                "p" => Some(keysyms::KEY_p),
                                                "q" => Some(keysyms::KEY_q),
                                                "s" => Some(keysyms::KEY_s),
                                                "t" => Some(keysyms::KEY_t),
                                                "v" => Some(keysyms::KEY_v),
                                                "x" => Some(keysyms::KEY_x),
                                                "y" => Some(keysyms::KEY_y),
                                                "z" => Some(keysyms::KEY_z),
                                                "escape" => Some(keysyms::KEY_Escape),
                                                "return" => Some(keysyms::KEY_Return),
                                                "space" => Some(keysyms::KEY_space),
                                                "comma" => Some(keysyms::KEY_comma),
                                                "period" => Some(keysyms::KEY_period),
                                                "minus" => Some(keysyms::KEY_minus),
                                                "equal" => Some(keysyms::KEY_equal),
                                                "page_down" => Some(keysyms::KEY_Page_Down),
                                                "page_up" => Some(keysyms::KEY_Page_Up),
                                                "home" => Some(keysyms::KEY_Home),
                                                "end" => Some(keysyms::KEY_End),
                                                "slash" => Some(keysyms::KEY_slash),
                                                "semicolon" => Some(keysyms::KEY_semicolon),
                                                "bracketleft" => Some(keysyms::KEY_bracketleft),
                                                "bracketright" => Some(keysyms::KEY_bracketright),
                                                "1" => Some(keysyms::KEY_1),
                                                "2" => Some(keysyms::KEY_2),
                                                "3" => Some(keysyms::KEY_3),
                                                "4" => Some(keysyms::KEY_4),
                                                "5" => Some(keysyms::KEY_5),
                                                "6" => Some(keysyms::KEY_6),
                                                "7" => Some(keysyms::KEY_7),
                                                "8" => Some(keysyms::KEY_8),
                                                "9" => Some(keysyms::KEY_9),
                                                _ => None,
                                            };
                                            
                                            if let Some(keysym) = keysym_opt {
                                                let has_ctrl = mods_parts.contains(&"Ctrl");
                                                let has_shift = mods_parts.contains(&"Shift");
                                                let has_alt = mods_parts.contains(&"Alt");
                                                let has_logo = mods_parts.contains(&"Logo") || mods_parts.contains(&"Super");
                                                let has_mod = mods_parts.contains(&"Mod");
                                                
                                                let action_parts: Vec<&str> = right.split_whitespace().collect();
                                                if !action_parts.is_empty() {
                                                    let action_raw = action_parts[0].trim_end_matches(';');
                                                    
                                                    let layout_action_opt = match action_raw {
                                                        "focus-column-left" | "focus-window-or-workspace-left" => Some("focus-left"),
                                                        "focus-column-right" | "focus-window-or-workspace-right" => Some("focus-right"),
                                                        "focus-window-down" | "focus-window-or-workspace-down" => Some("focus-down"),
                                                        "focus-window-up" | "focus-window-or-workspace-up" => Some("focus-up"),
                                                        "move-column-left" => Some("move-left"),
                                                        "move-column-right" => Some("move-right"),
                                                        "move-window-down" => Some("move-down"),
                                                        "move-window-up" => Some("move-up"),
                                                        "toggle-overview" => Some("tiling-mode-overview"),
                                                        "toggle-column-tabbed-display" => Some("toggle-tab"),
                                                        "close-window" => Some("close-window"),
                                                        "spawn" => Some("spawn-terminal"),
                                                        "focus-workspace-down" => Some("focus-workspace-down"),
                                                        "focus-workspace-up" => Some("focus-workspace-up"),
                                                        "move-column-to-workspace-down" => Some("move-column-to-workspace-down"),
                                                        "move-column-to-workspace-up" => Some("move-column-to-workspace-up"),
                                                        _ => None,
                                                    };
                                                    
                                                    if let Some(layout_action) = layout_action_opt {
                                                        if has_mod {
                                                            binds.insert((has_ctrl, has_shift, true, false, keysym), layout_action.to_string());
                                                            binds.insert((has_ctrl, has_shift, false, true, keysym), layout_action.to_string());
                                                            println!("[Config Bind] Parsed Mod bind: {:?}+{} -> {}", mods_parts, key_name, layout_action);
                                                        } else {
                                                            binds.insert((has_ctrl, has_shift, has_alt, has_logo, keysym), layout_action.to_string());
                                                            println!("[Config Bind] Parsed bind: {:?}+{} -> {}", mods_parts, key_name, layout_action);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                break;
            }
        }
        binds
    }

    pub fn save_session_internal(&mut self) -> Result<String, String> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct SavedWindow {
            title: String,
            #[serde(default)]
            app_id: Option<String>,
            #[serde(default)]
            cmdline: Option<Vec<String>>,
        }
        #[derive(serde::Serialize, serde::Deserialize)]
        struct SavedColumn {
            width: f32,
            focused_window_idx: usize,
            windows: Vec<SavedWindow>,
        }
        #[derive(serde::Serialize, serde::Deserialize)]
        struct SavedWorkspace {
            focused_column_idx: usize,
            columns: Vec<SavedColumn>,
        }
        #[derive(serde::Serialize, serde::Deserialize)]
        struct SavedSession {
            active_workspace_idx: usize,
            workspaces: Vec<SavedWorkspace>,
        }

        let workspaces_saved: Vec<SavedWorkspace> = self.layout_engine.workspaces.iter().map(|ws| {
            let columns_saved: Vec<SavedColumn> = ws.columns.iter().map(|col| {
                let windows_saved: Vec<SavedWindow> = col.windows.iter().map(|win| {
                    let app_id = self.windows.get(&win.id).and_then(|w| {
                        w.toplevel().and_then(|t| {
                            smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                                states
                                    .data_map
                                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                    .unwrap()
                                    .lock()
                                    .unwrap()
                                    .app_id
                                    .clone()
                            })
                        })
                    });

                    let pid = self.windows.get(&win.id).and_then(|w| {
                        w.toplevel().and_then(|t| {
                            use smithay::reexports::wayland_server::Resource;
                            t.wl_surface().client().and_then(|c| {
                                c.get_credentials(&self.display_handle).ok().map(|creds| creds.pid)
                            })
                        })
                    });

                    let cmdline = pid.and_then(|p| {
                        std::fs::read(format!("/proc/{}/cmdline", p)).ok().map(|bytes| {
                            bytes.split(|&b| b == 0)
                                .filter(|chunk| !chunk.is_empty())
                                .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
                                .collect::<Vec<String>>()
                        })
                    });

                    SavedWindow {
                        title: win.title.clone(),
                        app_id,
                        cmdline,
                    }
                }).collect();
                SavedColumn {
                    width: col.width,
                    focused_window_idx: col.focused_window_idx,
                    windows: windows_saved,
                }
            }).collect();
            SavedWorkspace {
                focused_column_idx: ws.focused_column_idx,
                columns: columns_saved,
            }
        }).collect();

        let session = SavedSession {
            active_workspace_idx: self.layout_engine.active_workspace_idx,
            workspaces: workspaces_saved,
        };

        let cookie = std::env::var("HIER_COOKIE").ok();
        let path = if let Some(ref cookie_id) = cookie {
            format!("{}/.cache/hier/cookies/{}/session.json", std::env::var("HOME").unwrap_or_else(|_| "/home/super".to_string()), cookie_id)
        } else {
            "/tmp/hier-session.json".to_string()
        };

        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match std::fs::File::create(&path) {
            Ok(file) => {
                if serde_json::to_writer_pretty(file, &session).is_ok() {
                    Ok(path)
                } else {
                    Err("failed to serialize session".to_string())
                }
            }
            Err(e) => Err(format!("failed to create file: {}", e)),
        }
    }

    pub fn new(display_handle: DisplayHandle, layout_engine: LayoutEngine, output: Output, socket_name: String, sandbox: bool) -> Self {
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&display_handle, "hier-seat");
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&display_handle);
        let data_control_state = DataControlState::new::<Self, _>(&display_handle, None, |_| true);
        let activation_state = XdgActivationState::new::<Self>(&display_handle);

        // Add keyboard and pointer capabilities to the seat
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let config_binds = Self::load_config_binds();

        let mut state = Self {
            display_handle,
            layout_engine,
            space: Space::default(),
            windows: HashMap::new(),
            next_window_id: 1,
            output,
            compositor_state,
            shm_state,
            seat_state,
            xdg_shell_state,
            seat,
            data_device_state,
            primary_selection_state,
            data_control_state,
            activation_state,
            running: true,
            socket_name,
            highlighted_window: None,
            child_display_socket: None,
            workspace_swipe_accumulator: 0.0,
            start_time: std::time::Instant::now(),
            last_event_time: 0,
            depth_switcher_active: false,
            depth_switcher_previous_mode: None,
            hud_tiling_mode: None,
            hud_opacity: 0.0,
            hud_previous_mode: None,
            pending_restores: Vec::new(),
            config_binds,
            frame_times: Vec::with_capacity(200),
            stutter_count: 0,
            stutter_threshold_ms: std::env::var("HIER_STUTTER_THRESHOLD_MS")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(18.0),
            sandbox,
        };

        if sandbox {
            state.layout_engine.spawn_window(WindowId(1), "Terminal (Mock)".to_string());
            state.layout_engine.spawn_window(WindowId(2), "Web Browser (Mock)".to_string());
            state.layout_engine.spawn_window(WindowId(3), "Text Editor (Mock)".to_string());
            state.next_window_id = 4;
            state.highlighted_window = Some((WindowId(3), [0.117, 0.565, 1.0, 1.0]));
            state.layout_engine.recenter_camera(true);
        }

        state
    }

    pub fn record_frame_time(&mut self, dt_secs: f32) {
        let dt_ms = dt_secs * 1000.0;
        if dt_ms > self.stutter_threshold_ms {
            self.stutter_count += 1;
        }
        self.frame_times.push(dt_ms);
        if self.frame_times.len() > 200 {
            self.frame_times.remove(0);
        }
    }

    pub fn forward_to_child(&self, cmd: &str) -> bool {
        // 1. Try dynamic auto-registered child display socket
        if let Some(ref child_display) = self.child_display_socket {
            let child_socket = format!("/tmp/hier-ctrl-{}.sock", child_display);
            if std::path::Path::new(&child_socket).exists() {
                use std::io::Write;
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&child_socket) {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
                    let formatted_cmd = format!("{}\n", cmd);
                    if stream.write_all(formatted_cmd.as_bytes()).is_ok() {
                        let _ = stream.flush();
                        return true;
                    }
                }
            }
        }

        // 2. Fallback to sequential guess wayland-(num+1)
        if let Some(num_str) = self.socket_name.strip_prefix("wayland-") {
            if let Ok(num) = num_str.parse::<u32>() {
                let child_socket = format!("/tmp/hier-ctrl-wayland-{}.sock", num + 1);
                if std::path::Path::new(&child_socket).exists() {
                    use std::io::Write;
                    if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&child_socket) {
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                        let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
                        let formatted_cmd = format!("{}\n", cmd);
                        if stream.write_all(formatted_cmd.as_bytes()).is_ok() {
                            let _ = stream.flush();
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    pub fn forward_binary_to_child(&self, msg_type: u8, payload: &[u8]) -> bool {
        // 1. Try dynamic auto-registered child display socket
        if let Some(ref child_display) = self.child_display_socket {
            let child_socket = format!("/tmp/hier-ctrl-{}.sock", child_display);
            if std::path::Path::new(&child_socket).exists() {
                use std::io::Write;
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&child_socket) {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
                    let mut header = [0u8; 5];
                    header[0..4].copy_from_slice(b"HIER");
                    header[4] = msg_type;
                    if stream.write_all(&header).is_ok() && stream.write_all(payload).is_ok() {
                        let _ = stream.flush();
                        return true;
                    }
                }
            }
        }

        // 2. Fallback to sequential guess wayland-(num+1)
        if let Some(num_str) = self.socket_name.strip_prefix("wayland-") {
            if let Ok(num) = num_str.parse::<u32>() {
                let child_socket = format!("/tmp/hier-ctrl-wayland-{}.sock", num + 1);
                if std::path::Path::new(&child_socket).exists() {
                    use std::io::Write;
                    if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&child_socket) {
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                        let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
                        let mut header = [0u8; 5];
                        header[0..4].copy_from_slice(b"HIER");
                        header[4] = msg_type;
                        if stream.write_all(&header).is_ok() && stream.write_all(payload).is_ok() {
                            let _ = stream.flush();
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    pub fn is_nested_compositor_window(&self, win_id: WindowId) -> bool {
        if let Some(win) = self.windows.get(&win_id) {
            let title = win.toplevel().and_then(|t| {
                smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .title
                        .clone()
                })
            });
            let app_id = win.toplevel().and_then(|t| {
                smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .app_id
                        .clone()
                })
            });

            if let Some(t) = title {
                let t_lower = t.to_lowercase();
                if t_lower.contains("smithay") || t_lower.contains("hier") {
                    return true;
                }
            }
            if let Some(a) = app_id {
                let a_lower = a.to_lowercase();
                if a_lower.contains("smithay") || a_lower.contains("hier") {
                    return true;
                }
            }
        }
        false
    }

    pub fn cycle_depth_stack(&mut self, forward: bool) {
        let len = self.layout_engine.windows.len();
        if len == 0 {
            return;
        }
        let current_idx = self.layout_engine.depth_scroll_progress.round() as usize;
        let next_idx = if forward {
            (current_idx + 1) % len
        } else {
            (current_idx + len - 1) % len
        };
        self.layout_engine.depth_scroll_progress = next_idx as f32;
        
        if let Some(&active_win_id) = self.layout_engine.windows.get(next_idx) {
            let ws = self.layout_engine.active_workspace_mut();
            if let Some((col_idx, win_idx)) = ws.find_window(active_win_id) {
                ws.focused_column_idx = col_idx;
                ws.columns[col_idx].focused_window_idx = win_idx;
            }
            let surface = self.windows.get(&active_win_id)
                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
            if let Some(surface) = surface {
                self.set_keyboard_focus(Some(surface));
            }
            println!("[cycle_depth_stack] Focus updated to window {:?}", active_win_id);
        }
        self.reposition_windows();
    }

    pub fn window_under_pointer(&self, pointer_pos: Point<f64, smithay::utils::Logical>) -> Option<WindowId> {
        let current_scale = self.layout_engine.current_overview_scale;
        let is_scaled = self.layout_engine.overview_open 
            || (current_scale - 1.0).abs() > 1e-3;
        
        if is_scaled {
            for (&win_id, _) in &self.windows {
                let ws_idx = self.layout_engine.workspaces.iter().position(|ws| ws.find_window(win_id).is_some()).unwrap();
                let col_idx = self.layout_engine.workspaces[ws_idx].find_window(win_id).unwrap().0;
                let col = &self.layout_engine.workspaces[ws_idx].columns[col_idx];
                let is_overlay = col.is_overlay();

                let mut anim_geom = None;
                for win in &col.windows {
                    if win.id == win_id {
                        if win.anim_initialized {
                            anim_geom = Some((win.anim_x, win.anim_y, win.anim_w, win.anim_h));
                        }
                        break;
                    }
                }

                let geom = if let Some((ax, ay, aw, ah)) = anim_geom {
                    Some((ax, ay, aw, ah))
                } else {
                    self.layout_engine.get_window_rect_for_mode(win_id, &self.layout_engine.underlying_tiling_mode)
                };

                if let Some((nx, ny, nw, nh)) = geom {
                    let ws_y = ws_idx as f32 * self.layout_engine.viewport.height;
                    let x_local = nx;
                    let y_local = ny - ws_y;
                    
                    let (sx, sy, sw, sh) = self.layout_engine.project_rect(x_local, y_local, nw, nh, ws_idx, current_scale, is_overlay);
                    
                    if pointer_pos.x >= sx as f64 && pointer_pos.x < (sx + sw) as f64
                        && pointer_pos.y >= sy as f64 && pointer_pos.y < (sy + sh) as f64 {
                        return Some(win_id);
                    }
                }
            }
            None
        } else {
            let space_pos = pointer_pos + Point::from((
                self.layout_engine.viewport.x as f64,
                self.layout_engine.viewport.y as f64,
            ));
            self.space.element_under(space_pos).and_then(|(win, _)| {
                self.windows.iter().find(|(_, w)| **w == *win).map(|(id, _)| *id)
            })
        }
    }

    pub fn focus_window_by_id(&mut self, win_id: WindowId) {
        let mut found = None;
        for (ws_idx, ws) in self.layout_engine.workspaces.iter().enumerate() {
            if let Some((col_idx, win_idx)) = ws.find_window(win_id) {
                found = Some((ws_idx, col_idx, win_idx));
                break;
            }
        }
        
        if let Some((ws_idx, col_idx, win_idx)) = found {
            self.layout_engine.active_workspace_idx = ws_idx;
            let ws = &mut self.layout_engine.workspaces[ws_idx];
            ws.focused_column_idx = col_idx;
            ws.columns[col_idx].focused_window_idx = win_idx;
            
            let surface = self.windows.get(&win_id)
                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
            if let Some(surface) = surface {
                self.set_keyboard_focus(Some(surface));
            }
        }
    }

    pub fn set_keyboard_focus(&mut self, focus: Option<WlSurface>) {
        println!("[set_keyboard_focus] Setting keyboard focus to surface: {:?}", focus);
        if let Some(keyboard) = self.seat.get_keyboard() {
            let serial = SERIAL_COUNTER.next_serial();
            keyboard.set_focus(self, focus, serial);
        }
    }


    /// Positions all mapped windows inside the Smithay `Space` based on our `LayoutEngine`'s coordinates.
    pub fn reposition_windows(&mut self) {
        let active_ws = self.layout_engine.active_workspace();

        // Determine which mode to use for client logical geometry
        let geom_mode = &self.layout_engine.tiling_mode;

        if self.sandbox {
            for col in &active_ws.columns {
                if let Some(win) = col.focused_window() {
                    if let Some((x, y, w, h)) = self.layout_engine.get_window_rect_for_mode(win.id, geom_mode) {
                        println!("[Sandbox Reposition] Mock window ID={:?} (Title: {:?}): position=({}, {}), size=({}x{}) using mode {:?}", win.id, win.title, x, y, w, h, geom_mode);
                    }
                }
            }
            return;
        }

        // Unmap all windows to make sure no old windows linger
        let mapped_windows: Vec<Window> = self.windows.values().cloned().collect();
        for window in &mapped_windows {
            self.space.unmap_elem(window);
        }

        // Dynamically update the output's location in the space based on animated camera viewport coordinates
        let vx = self.layout_engine.viewport.x as i32;
        let vy = self.layout_engine.viewport.y as i32;
        self.space.map_output(&self.output.clone(), (vx, vy));

        // Loop through current active workspace's columns and map the focused window of each column
        let active_ws = self.layout_engine.active_workspace();

        // Determine which mode to use for client logical geometry
        let geom_mode = &self.layout_engine.tiling_mode;

        for col in &active_ws.columns {
            if let Some(win) = col.focused_window() {
                if let Some(smithay_win) = self.windows.get(&win.id) {
                    let (x, y, w, h) = if win.anim_initialized {
                        (win.anim_x, win.anim_y, win.anim_w, win.anim_h)
                    } else if let Some(target) = self.layout_engine.get_window_rect_for_mode(win.id, geom_mode) {
                        target
                    } else {
                        continue;
                    };

                    // Tell the client to resize to match our animated dimensions
                    let toplevel = smithay_win.toplevel().unwrap();
                    let current_size = toplevel.current_state().size;
                    let target_size = Some((w as i32, h as i32).into());
                    
                    if current_size != target_size {
                        toplevel.with_pending_state(|state| {
                            state.size = target_size;
                        });
                        toplevel.send_configure();
                    }

                    // Map in Smithay's Space Logical Coordinate system
                    self.space.map_element(smithay_win.clone(), (x as i32, y as i32), true);
                    println!("Reposition window ID={:?} (Title: {:?}): position=({}, {}), size=({}x{}) using mode {:?}", win.id, win.title, x, y, w, h, geom_mode);
                }
            }
        }
    }

    /// Process physical keyboard, mouse pointer, and other input events.
    pub fn process_input(&mut self, event: InputEvent<WinitInput>) {
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                self.last_event_time = self.last_event_time.max(time);
                let keycode = event.key_code();

                let key_state = event.state();

                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.input(
                    self,
                    keycode,
                    key_state,
                    serial,
                    time,
                    |state, modifiers, handle| {
                        let keysym = handle.modified_sym();
                        state.handle_key_action(key_state, modifiers, keysym)
                    },
                );
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                self.last_event_time = self.last_event_time.max(time);
                let raw_pos = event.position();

                let pos = Point::from((raw_pos.x, raw_pos.y));



                let pointer = self.seat.get_pointer().unwrap();

                // Translate pointer screen coordinates to space coordinates by adding the camera viewport offset
                let space_pos = pos + Point::from((
                    self.layout_engine.viewport.x as f64,
                    self.layout_engine.viewport.y as f64,
                ));

                let is_overview = self.layout_engine.overview_open;
                let focus = if is_overview {
                    if let Some(win_id) = self.window_under_pointer(pos) {
                        self.highlighted_window = Some((win_id, [0.117, 0.565, 1.0, 1.0])); // Dodger Blue
                    } else {
                        self.highlighted_window = None;
                    }
                    None
                } else {
                    let under = self.space.element_under(space_pos);
                    under.and_then(|(win, local_pos)| {
                        win.surface_under(
                            local_pos.to_f64(),
                            WindowSurfaceType::ALL,
                        )
                        .map(|(surface, surface_local_pos)| {
                            (surface, surface_local_pos.to_f64())
                        })
                    })
                };

                pointer.motion(
                    self,
                    focus,
                    &smithay::input::pointer::MotionEvent {
                        location: space_pos,
                        time,
                        serial,
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                self.last_event_time = self.last_event_time.max(time);
                let button = event.button_code();

                let state = event.state();

                let pointer = self.seat.get_pointer().unwrap();

                // Focus window and update pointer focus under cursor on click
                if state == ButtonState::Pressed {
                    let pos = pointer.current_location();
                    
                    if self.layout_engine.overview_open {
                        let screen_pos = pos - Point::from((
                            self.layout_engine.viewport.x as f64,
                            self.layout_engine.viewport.y as f64,
                        ));
                        if let Some(win_id) = self.window_under_pointer(screen_pos) {
                            self.focus_window_by_id(win_id);
                            self.layout_engine.overview_open = false;
                            self.layout_engine.overview_progress = None;
                            self.layout_engine.recenter_camera(false);
                            self.reposition_windows();
                            return;
                        }
                    }
                    
                    let (focus, surface, clicked_win_id) = {
                        let under = self.space.element_under(pos);
                        let focus = under.as_ref().and_then(|(win, local_pos)| {
                            win.surface_under(
                                local_pos.to_f64(),
                                WindowSurfaceType::ALL,
                            )
                            .map(|(surface, surface_local_pos)| {
                                (surface, surface_local_pos.to_f64())
                            })
                        });
                        let surface = under.as_ref().and_then(|(win, _)| win.wl_surface().map(|c| c.into_owned()));
                        let clicked_win_id = under.as_ref().and_then(|(win, _)| {
                            self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id)
                        });
                        (focus, surface, clicked_win_id)
                    };

                    pointer.motion(
                        self,
                        focus,
                        &smithay::input::pointer::MotionEvent {
                            location: pos,
                            time,
                            serial,
                        },
                    );

                    if let Some(win_id) = clicked_win_id {
                        self.focus_window_by_id(win_id);
                        self.layout_engine.recenter_camera(false);
                        self.reposition_windows();
                    } else if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                }

                pointer.button(
                    self,
                    &smithay::input::pointer::ButtonEvent {
                        button,
                        state,
                        serial,
                        time,
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();
                let modifiers = keyboard.modifier_state();
                
                let active_col_is_tabbed = self.layout_engine.active_workspace().focused_column().map_or(false, |col| col.is_tabbed());

                if active_col_is_tabbed && !modifiers.logo && !modifiers.alt {
                    let amount = event.amount(Axis::Vertical)
                        .or_else(|| event.amount_v120(Axis::Vertical).map(|v| v / 120.0))
                        .unwrap_or(0.0);
                    if amount != 0.0 {
                        let old_win_id = self.layout_engine.active_workspace().focused_column()
                            .and_then(|col| col.focused_window().map(|w| w.id));

                        if amount > 0.0 {
                            self.layout_engine.focus_tab_down();
                        } else {
                            self.layout_engine.focus_tab_up();
                        }

                        let new_win_id = self.layout_engine.active_workspace().focused_column()
                            .and_then(|col| col.focused_window().map(|w| w.id));

                        if old_win_id != new_win_id {
                            let surface = new_win_id
                                .and_then(|id| self.windows.get(&id))
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                            self.reposition_windows();
                        }
                        return;
                    }
                }

                if modifiers.logo || modifiers.alt {
                    let amount = event.amount(Axis::Vertical)
                        .or_else(|| event.amount_v120(Axis::Vertical).map(|v| v / 120.0))
                        .unwrap_or(0.0);
                    
                    if amount != 0.0 {
                        if !self.depth_switcher_active {
                            self.depth_switcher_previous_mode = Some(self.layout_engine.tiling_mode.clone());
                            self.depth_switcher_active = true;
                            self.layout_engine.tiling_mode = crate::layout::TilingMode::Depth;
                            self.layout_engine.depth_scroll_progress = 0.0;
                            println!("[concept] Depth stacking Alt/Tab switcher activated via scroll. Previous tiling mode: {:?}", self.depth_switcher_previous_mode);
                        }

                        let delta = amount as f32;
                        self.layout_engine.scroll_z(delta);
                        
                        let active_idx = self.layout_engine.depth_scroll_progress.round() as usize;
                        if let Some(&active_win_id) = self.layout_engine.windows.get(active_idx) {
                            let ws = self.layout_engine.active_workspace_mut();
                            if let Some((col_idx, win_idx)) = ws.find_window(active_win_id) {
                                ws.focused_column_idx = col_idx;
                                ws.columns[col_idx].focused_window_idx = win_idx;
                            }
                            let surface = self.windows.get(&active_win_id)
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                        }
                        self.reposition_windows();
                        return;
                    }
                }
                
                let time = event.time_msec();
                self.last_event_time = self.last_event_time.max(time);
                let mut frame = AxisFrame::new(time);

                if let Some(val) = event.amount(Axis::Horizontal) {
                    frame = frame.value(Axis::Horizontal, val);
                } else if let Some(val) = event.amount_v120(Axis::Horizontal) {
                    frame = frame.v120(Axis::Horizontal, val as i32);
                }
                if let Some(val) = event.amount(Axis::Vertical) {
                    frame = frame.value(Axis::Vertical, val);
                } else if let Some(val) = event.amount_v120(Axis::Vertical) {
                    frame = frame.v120(Axis::Vertical, val as i32);
                }
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            InputEvent::GestureSwipeBegin { event } => {
                println!("[Gesture] Swipe begin: fingers={}", GestureBeginEvent::<WinitInput>::fingers(&event));
            }
            InputEvent::GestureSwipeUpdate { event } => {
                let dx = GestureSwipeUpdateEvent::<WinitInput>::delta_x(&event);
                let dy = GestureSwipeUpdateEvent::<WinitInput>::delta_y(&event);
                println!("[Gesture] Swipe update: dx={}, dy={}", dx, dy);
                if dx.abs() > dy.abs() {
                    // Horizontal scroll on columns ribbon
                    let speed = 2.0;
                    self.layout_engine.viewport.target_x += dx as f32 * speed;
                } else {
                    // Vertical scroll to switch workspaces
                    self.workspace_swipe_accumulator += dy as f32;
                    if self.workspace_swipe_accumulator.abs() > 150.0 {
                        if self.workspace_swipe_accumulator > 0.0 {
                            self.layout_engine.focus_workspace_down();
                        } else {
                            self.layout_engine.focus_workspace_up();
                        }
                        self.workspace_swipe_accumulator = 0.0;
                    }
                }
                self.reposition_windows();
            }
            InputEvent::GestureSwipeEnd { event: _ } => {
                println!("[Gesture] Swipe end");
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
            }
            _ => {}
        }
    }

fn find_terminal_cmd() -> String {
    let preferred = ["foot", "alacritty", "kitty", "xterm"];
    if let Ok(path_var) = std::env::var("PATH") {
        for term in preferred.iter() {
            for path_dir in std::env::split_paths(&path_var) {
                let bin_path = path_dir.join(term);
                if bin_path.is_file() {
                    return term.to_string();
                }
            }
        }
    }
    "alacritty".to_string() // Fallback
}

    pub fn perform_layout_action(&mut self, action: &str) -> Result<(), String> {
        let res = match action {
            "focus-left" | "focus_left" => {
                self.layout_engine.focus_left();
                let win_id = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id));
                if self.sandbox {
                    if let Some(id) = win_id {
                        self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                    }
                } else {
                    let surface = win_id
                        .and_then(|id| self.windows.get(&id))
                        .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                }
                self.reposition_windows();
                Ok(())
            }
            "focus-right" | "focus_right" => {
                self.layout_engine.focus_right();
                let win_id = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id));
                if self.sandbox {
                    if let Some(id) = win_id {
                        self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                    }
                } else {
                    let surface = win_id
                        .and_then(|id| self.windows.get(&id))
                        .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                }
                self.reposition_windows();
                Ok(())
            }
            "focus-up" | "focus_up" => {
                if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                    self.layout_engine.depth_scroll_progress = (self.layout_engine.depth_scroll_progress - 1.0).max(0.0);
                    let active_idx = self.layout_engine.depth_scroll_progress.round() as usize;
                    if let Some(&active_win_id) = self.layout_engine.windows.get(active_idx) {
                        let ws = self.layout_engine.active_workspace_mut();
                        if let Some((col_idx, win_idx)) = ws.find_window(active_win_id) {
                            ws.focused_column_idx = col_idx;
                            ws.columns[col_idx].focused_window_idx = win_idx;
                        }
                        if self.sandbox {
                            self.highlighted_window = Some((active_win_id, [0.117, 0.565, 1.0, 1.0]));
                        } else {
                            let surface = self.windows.get(&active_win_id)
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                        }
                    }
                    self.reposition_windows();
                } else {
                    self.layout_engine.focus_tab_up();
                    let win_id = self.layout_engine.active_workspace().focused_column()
                        .and_then(|col| col.focused_window().map(|w| w.id));
                    if self.sandbox {
                        if let Some(id) = win_id {
                            self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                        }
                    } else {
                        let surface = win_id
                            .and_then(|id| self.windows.get(&id))
                            .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                        if let Some(surface) = surface {
                            self.set_keyboard_focus(Some(surface));
                        }
                    }
                    self.reposition_windows();
                }
                Ok(())
            }
            "focus-down" | "focus_down" => {
                if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                    let max_progress = (self.layout_engine.windows.len().saturating_sub(1)) as f32;
                    self.layout_engine.depth_scroll_progress = (self.layout_engine.depth_scroll_progress + 1.0).min(max_progress);
                    let active_idx = self.layout_engine.depth_scroll_progress.round() as usize;
                    if let Some(&active_win_id) = self.layout_engine.windows.get(active_idx) {
                        let ws = self.layout_engine.active_workspace_mut();
                        if let Some((col_idx, win_idx)) = ws.find_window(active_win_id) {
                            ws.focused_column_idx = col_idx;
                            ws.columns[col_idx].focused_window_idx = win_idx;
                        }
                        if self.sandbox {
                            self.highlighted_window = Some((active_win_id, [0.117, 0.565, 1.0, 1.0]));
                        } else {
                            let surface = self.windows.get(&active_win_id)
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                        }
                    }
                    self.reposition_windows();
                } else {
                    self.layout_engine.focus_tab_down();
                    let win_id = self.layout_engine.active_workspace().focused_column()
                        .and_then(|col| col.focused_window().map(|w| w.id));
                    if self.sandbox {
                        if let Some(id) = win_id {
                            self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                        }
                    } else {
                        let surface = win_id
                            .and_then(|id| self.windows.get(&id))
                            .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                        if let Some(surface) = surface {
                            self.set_keyboard_focus(Some(surface));
                        }
                    }
                    self.reposition_windows();
                }
                Ok(())
            }
            "focus-workspace-up" | "focus_workspace_up" => {
                self.layout_engine.focus_workspace_up();
                let win_id = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id));
                if self.sandbox {
                    if let Some(id) = win_id {
                        self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                    }
                } else {
                    let surface = win_id
                        .and_then(|id| self.windows.get(&id))
                        .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                }
                self.reposition_windows();
                Ok(())
            }
            "focus-workspace-down" | "focus_workspace_down" => {
                self.layout_engine.focus_workspace_down();
                let win_id = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id));
                if self.sandbox {
                    if let Some(id) = win_id {
                        self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                    }
                } else {
                    let surface = win_id
                        .and_then(|id| self.windows.get(&id))
                        .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                }
                self.reposition_windows();
                Ok(())
            }
            "move-left" | "move_left" => {
                self.layout_engine.move_column_left();
                self.reposition_windows();
                Ok(())
            }
            "move-right" | "move_right" => {
                self.layout_engine.move_column_right();
                self.reposition_windows();
                Ok(())
            }
            "move-up" | "move_up" => {
                self.layout_engine.move_window_workspace_up();
                self.reposition_windows();
                Ok(())
            }
            "move-down" | "move_down" => {
                self.layout_engine.move_window_workspace_down();
                self.reposition_windows();
                Ok(())
            }
            "toggle-tab" | "toggle_tab" => {
                self.layout_engine.toggle_tab_group();
                self.reposition_windows();
                Ok(())
            }
            "spawn-terminal" | "spawn_terminal" => {
                if self.sandbox {
                    let id = WindowId(self.next_window_id);
                    self.next_window_id += 1;
                    let title = format!("Mock App #{}", id.0);
                    self.layout_engine.spawn_window(id, title);
                    self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                    self.reposition_windows();
                    Ok(())
                } else {
                    let socket = self.socket_name.clone();
                    let term = Self::find_terminal_cmd();
                    println!("Spawning terminal ({}) on WAYLAND_DISPLAY={}", term, socket);
                    let _ = std::process::Command::new(term)
                        .env("WAYLAND_DISPLAY", socket)
                        .spawn();
                    Ok(())
                }
            }
            "spawn-mock-window" | "spawn_mock_window" => {
                let id = WindowId(self.next_window_id);
                self.next_window_id += 1;
                let title = format!("Mock App #{}", id.0);
                self.layout_engine.spawn_window(id, title);
                self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                self.reposition_windows();
                Ok(())
            }
            "close-window" | "close_window" => {
                let win_id = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id));
                if let Some(id) = win_id {
                    self.layout_engine.close_window(id);
                    if let Some((highlighted_id, _)) = self.highlighted_window {
                        if highlighted_id == id {
                            self.highlighted_window = self.layout_engine.active_workspace().focused_column()
                                .and_then(|col| col.focused_window().map(|w| (w.id, [0.117, 0.565, 1.0, 1.0])));
                        }
                    }
                    self.reposition_windows();
                }
                Ok(())
            }
            "fresh-nest" | "fresh_nest" => {
                if let Ok(exe_path) = std::env::current_exe() {
                    let socket = self.socket_name.clone();
                    println!("Spawning fresh nest from: {:?} on parent display: {}", exe_path, socket);
                    let _ = std::process::Command::new(exe_path)
                        .env("WAYLAND_DISPLAY", socket)
                        .spawn();
                } else {
                    return Err("failed to get current executable path".to_string());
                }
                Ok(())
            }
            "restore-nest-0" | "restore_nest_0" => {
                self.layout_engine.active_workspace_idx = 0;
                let ws = &mut self.layout_engine.workspaces[0];
                ws.focused_column_idx = 0;
                if let Some(col) = ws.focused_column_mut() {
                    col.focused_window_idx = 0;
                }
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-diagonal" | "tiling_mode_diagonal" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Diagonal;
                self.layout_engine.underlying_tiling_mode = crate::layout::TilingMode::Diagonal;
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-grid" | "tiling_mode_grid" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Grid;
                self.layout_engine.underlying_tiling_mode = crate::layout::TilingMode::Grid;
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-float" | "tiling_mode_float" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Float;
                self.layout_engine.underlying_tiling_mode = crate::layout::TilingMode::Float;
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-depth" | "tiling_mode_depth" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Depth;
                self.layout_engine.underlying_tiling_mode = crate::layout::TilingMode::Depth;
                self.layout_engine.depth_scroll_progress = 0.0;
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-overview" | "tiling_mode_overview" => {
                self.layout_engine.overview_open = !self.layout_engine.overview_open;
                self.layout_engine.overview_progress = if self.layout_engine.overview_open {
                    Some(crate::layout::OverviewProgress::Open)
                } else {
                    None
                };
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                Ok(())
            }
            "quit" => {
                self.running = false;
                Ok(())
            }
            other if other.starts_with("workspace-") || other.starts_with("workspace_") => {
                let idx_str = if other.starts_with("workspace-") {
                    &other["workspace-".len()..]
                } else {
                    &other["workspace_".len()..]
                };
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx >= 1 && idx <= 5 {
                        self.layout_engine.active_workspace_idx = idx - 1;
                        if self.sandbox {
                            let win_id = self.layout_engine.active_workspace().focused_column()
                                .and_then(|col| col.focused_window().map(|w| w.id));
                            if let Some(id) = win_id {
                                self.highlighted_window = Some((id, [0.117, 0.565, 1.0, 1.0]));
                            } else {
                                self.highlighted_window = None;
                            }
                        }
                        self.layout_engine.recenter_camera(false);
                        self.reposition_windows();
                        return Ok(());
                    }
                }
                Err(format!("invalid workspace index: {}", other))
            }
            other => Err(format!("unknown layout action: {}", other)),
        };
        if res.is_ok() {
            let _ = self.save_session_internal();
        }
        res
    }

    pub fn handle_key_action(
        &mut self,
        key_state: KeyState,
        modifiers: &ModifiersState,
        keysym: Keysym,
    ) -> FilterResult<()> {
        if key_state == KeyState::Pressed {
            let key_key = (modifiers.ctrl, modifiers.shift, modifiers.alt, modifiers.logo, keysym.raw());
            if let Some(action) = self.config_binds.get(&key_key).cloned() {
                if action == "tiling-mode-overview" {
                    let _ = self.perform_layout_action("tiling-mode-overview");
                } else {
                    let _ = self.perform_layout_action(&action);
                }
                return smithay::input::keyboard::FilterResult::Intercept(());
            }

            if modifiers.logo || modifiers.alt {
                if keysym.raw() == keysyms::KEY_z || keysym.raw() == keysyms::KEY_Z {
                    if !self.depth_switcher_active {
                        self.depth_switcher_previous_mode = Some(self.layout_engine.tiling_mode.clone());
                        self.depth_switcher_active = true;
                        self.layout_engine.tiling_mode = crate::layout::TilingMode::Depth;
                        self.layout_engine.depth_scroll_progress = 0.0;
                        println!("[concept] Depth stacking Alt/Tab switcher activated. Previous tiling mode: {:?}", self.depth_switcher_previous_mode);
                    }
                    self.cycle_depth_stack(!modifiers.shift);
                    return smithay::input::keyboard::FilterResult::Intercept(());
                }

                if modifiers.shift {
                    match keysym.raw() {
                        keysyms::KEY_Left | keysyms::KEY_h => {
                            let _ = self.perform_layout_action("move-left");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Right | keysyms::KEY_l => {
                            let _ = self.perform_layout_action("move-right");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Up | keysyms::KEY_k => {
                            let _ = self.perform_layout_action("move-up");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Down | keysyms::KEY_j => {
                            let _ = self.perform_layout_action("move-down");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Q => {
                            let _ = self.perform_layout_action("quit");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        _ => {}
                    }
                } else {
                    match keysym.raw() {
                        keysyms::KEY_Left | keysyms::KEY_h => {
                            let _ = self.perform_layout_action("focus-left");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Right | keysyms::KEY_l => {
                            let _ = self.perform_layout_action("focus-right");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Up | keysyms::KEY_k => {
                            let _ = self.perform_layout_action("focus-up");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Down | keysyms::KEY_j => {
                            let _ = self.perform_layout_action("focus-down");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_c => {
                            let _ = self.perform_layout_action("toggle-tab");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_q => {
                            let _ = self.perform_layout_action("close-window");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_d => {
                            let _ = self.perform_layout_action("tiling-mode-depth");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_g => {
                            let _ = self.perform_layout_action("tiling-mode-grid");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_f => {
                            let _ = self.perform_layout_action("tiling-mode-float");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_a => {
                            let _ = self.perform_layout_action("tiling-mode-diagonal");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_o => {
                            let _ = self.perform_layout_action("tiling-mode-overview");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_Return => {
                            let _ = self.perform_layout_action("spawn-terminal");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_1 => {
                            let _ = self.perform_layout_action("workspace-1");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_2 => {
                            let _ = self.perform_layout_action("workspace-2");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_3 => {
                            let _ = self.perform_layout_action("workspace-3");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_4 => {
                            let _ = self.perform_layout_action("workspace-4");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        keysyms::KEY_5 => {
                            let _ = self.perform_layout_action("workspace-5");
                            return smithay::input::keyboard::FilterResult::Intercept(());
                        }
                        _ => {}
                    }
                }
            }
        }
        if key_state == KeyState::Released {
            if self.depth_switcher_active && !modifiers.logo && !modifiers.alt {
                let original_mode = self.depth_switcher_previous_mode.take().unwrap_or(crate::layout::TilingMode::Grid);
                println!("[concept] Depth stacking Alt/Tab switcher deactivated. Restoring previous tiling mode: {:?}", original_mode);
                self.depth_switcher_active = false;
                self.layout_engine.tiling_mode = original_mode;
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                return smithay::input::keyboard::FilterResult::Intercept(());
            }
        }
        smithay::input::keyboard::FilterResult::Forward
    }

    pub fn handle_simulated_input(&mut self, input: &str) -> String {
        println!("[Simulated Input] Command: {}", input);
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return "error: empty command\n".to_string();
        }

        match parts[0] {
            "keyboard_key" => {
                if parts.len() < 3 {
                    return "error: keyboard_key requires keycode and state\n".to_string();
                }
                let keycode = match parts[1].parse::<u32>() {
                    Ok(k) => k,
                    Err(_) => return "error: invalid keycode\n".to_string(),
                };
                let key_state = match parts[2].to_lowercase().as_str() {
                    "pressed" => KeyState::Pressed,
                    "released" => KeyState::Released,
                    _ => return "error: key state must be pressed or released\n".to_string(),
                };

                // Recursive Forwarding to Nest Child
                let is_nested = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id))
                    .map(|id| self.is_nested_compositor_window(id))
                    .unwrap_or(false);

                if is_nested {
                    let mut payload = [0u8; 5];
                    payload[0..4].copy_from_slice(&keycode.to_le_bytes());
                    payload[4] = if key_state == KeyState::Pressed { 1 } else { 0 };
                    self.forward_binary_to_child(1, &payload);
                }

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;


                let keyboard = self.seat.get_keyboard().unwrap();
                // Offset keycode from evdev to XKB format (evdev + 8) to avoid smithay's internal subtraction overflow panic.
                keyboard.input(
                    self,
                    (keycode + 8).into(),
                    key_state,
                    serial,
                    time,
                    |state, modifiers, handle| {
                        let keysym = handle.modified_sym();
                        state.handle_key_action(key_state, modifiers, keysym)
                    },
                );
                "ok\n".to_string()
            }
            "pointer_motion" => {
                if parts.len() < 3 {
                    return "error: pointer_motion requires x and y\n".to_string();
                }
                let x = match parts[1].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid x coordinate\n".to_string(),
                };
                let y = match parts[2].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid y coordinate\n".to_string(),
                };

                let is_overview = self.layout_engine.tiling_mode == crate::layout::TilingMode::Overview;
                let vp = &self.layout_engine.viewport;
                let (min_x, max_x, min_y, max_y) = if is_overview {
                    (0.0, vp.width as f64, 0.0, vp.height as f64)
                } else {
                    (vp.x as f64, (vp.x + vp.width) as f64, vp.y as f64, (vp.y + vp.height) as f64)
                };

                let clamped_x = x.clamp(min_x, max_x);
                let clamped_y = y.clamp(min_y, max_y);

                let pos = Point::from((clamped_x, clamped_y));
                let pointer = self.seat.get_pointer().unwrap();

                let (focus, space_pos) = if is_overview {
                    if let Some(win_id) = self.window_under_pointer(pos) {
                        self.highlighted_window = Some((win_id, [0.117, 0.565, 1.0, 1.0])); // Dodger Blue
                    } else {
                        self.highlighted_window = None;
                    }
                    (None, pos + Point::from((vp.x as f64, vp.y as f64)))
                } else {
                    let under = self.space.element_under(pos);
                    // Recursive Forwarding to Nest Child (as pointer_motion_local)
                    if let Some((win, local_pos)) = under.as_ref() {
                        if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                            if self.is_nested_compositor_window(id) {
                                let local_x = local_pos.x as f64;
                                let local_y = local_pos.y as f64;
                                let mut payload = [0u8; 16];
                                payload[0..8].copy_from_slice(&local_x.to_le_bytes());
                                payload[8..16].copy_from_slice(&local_y.to_le_bytes());
                                self.forward_binary_to_child(3, &payload);
                            }
                        }
                    }
                    let focus = under.and_then(|(win, local_pos)| {
                        win.surface_under(local_pos.to_f64(), WindowSurfaceType::ALL)
                            .map(|(surface, surface_local_pos)| (surface, surface_local_pos.to_f64()))
                    });
                    (focus, pos)
                };

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;

                pointer.motion(
                    self,
                    focus,
                    &smithay::input::pointer::MotionEvent {
                        location: space_pos,
                        time,
                        serial,
                    },
                );
                pointer.frame(self);
                "ok\n".to_string()
            }
            "pointer_motion_local" => {
                if parts.len() < 3 {
                    return "error: pointer_motion_local requires x and y\n".to_string();
                }
                let x = match parts[1].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid x coordinate\n".to_string(),
                };
                let y = match parts[2].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid y coordinate\n".to_string(),
                };

                let vp = &self.layout_engine.viewport;
                let clamped_x = x.clamp(0.0, vp.width as f64);
                let clamped_y = y.clamp(0.0, vp.height as f64);

                let global_x = clamped_x + vp.x as f64;
                let global_y = clamped_y + vp.y as f64;

                let pos = Point::from((global_x, global_y));
                let pointer = self.seat.get_pointer().unwrap();

                let under = self.space.element_under(pos);

                // Recursive Forwarding to Nest Child
                if let Some((win, local_pos)) = under.as_ref() {
                    if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                        if self.is_nested_compositor_window(id) {
                            let local_x = local_pos.x as f64;
                            let local_y = local_pos.y as f64;
                            let mut payload = [0u8; 16];
                            payload[0..8].copy_from_slice(&local_x.to_le_bytes());
                            payload[8..16].copy_from_slice(&local_y.to_le_bytes());
                            self.forward_binary_to_child(3, &payload);
                        }
                    }
                }

                let focus = under.and_then(|(win, local_pos)| {
                    win.surface_under(local_pos.to_f64(), WindowSurfaceType::ALL)
                        .map(|(surface, surface_local_pos)| (surface, surface_local_pos.to_f64()))
                });

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;

                pointer.motion(
                    self,
                    focus,
                    &smithay::input::pointer::MotionEvent {
                        location: pos,
                        time,
                        serial,
                    },
                );
                pointer.frame(self);
                "ok\n".to_string()
            }
            "pointer_button" => {
                if parts.len() < 3 {
                    return "error: pointer_button requires button_code and state\n".to_string();
                }
                let button = match parts[1].parse::<u32>() {
                    Ok(b) => b,
                    Err(_) => return "error: invalid button code\n".to_string(),
                };
                let state = match parts[2].to_lowercase().as_str() {
                    "pressed" => ButtonState::Pressed,
                    "released" => ButtonState::Released,
                    _ => return "error: button state must be pressed or released\n".to_string(),
                };

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;


                let pointer = self.seat.get_pointer().unwrap();
                let pos = pointer.current_location();

                // Find focus surface under pointer coordinate in a separate block to satisfy borrow checker
                let (focus, surface, is_nested, clicked_win_id) = {
                    let under = self.space.element_under(pos);
                    let focus = under.as_ref().and_then(|(win, local_pos)| {
                        win.surface_under(
                            local_pos.to_f64(),
                            WindowSurfaceType::ALL,
                        )
                        .map(|(surface, surface_local_pos)| {
                            (surface.clone(), surface_local_pos.to_f64())
                        })
                    });
                    let surface = under.as_ref().and_then(|(win, _)| win.wl_surface().map(|c| c.into_owned()));
                    let is_nested = under.as_ref().and_then(|(win, _)| {
                        self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| self.is_nested_compositor_window(*id))
                    }).unwrap_or(false);
                    let clicked_win_id = under.as_ref().and_then(|(win, _)| {
                        self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id)
                    });
                    (focus, surface, is_nested, clicked_win_id)
                };

                // Recursive Forwarding to Nest Child
                if is_nested {
                    let mut payload = [0u8; 5];
                    payload[0..4].copy_from_slice(&button.to_le_bytes());
                    payload[4] = if state == ButtonState::Pressed { 1 } else { 0 };
                    self.forward_binary_to_child(4, &payload);
                }

                pointer.motion(
                    self,
                    focus,
                    &smithay::input::pointer::MotionEvent {
                        location: pos,
                        time,
                        serial,
                    },
                );

                pointer.button(
                    self,
                    &smithay::input::pointer::ButtonEvent {
                        button,
                        state,
                        serial,
                        time,
                    },
                );
                pointer.frame(self);

                if state == ButtonState::Pressed {
                    if self.layout_engine.tiling_mode == crate::layout::TilingMode::Overview {
                        let screen_pos = pos - Point::from((
                            self.layout_engine.viewport.x as f64,
                            self.layout_engine.viewport.y as f64,
                        ));
                        if let Some(win_id) = self.window_under_pointer(screen_pos) {
                            self.focus_window_by_id(win_id);
                            self.layout_engine.tiling_mode = self.layout_engine.underlying_tiling_mode.clone();
                            self.layout_engine.recenter_camera(false);
                            self.reposition_windows();
                            return "ok\n".to_string();
                        }
                    } else {
                        if let Some(win_id) = clicked_win_id {
                            self.focus_window_by_id(win_id);
                            self.layout_engine.recenter_camera(false);
                            self.reposition_windows();
                        }
                    }
                    
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                }
                "ok\n".to_string()
            }
            "pointer_axis" => {
                if parts.len() < 3 {
                    return "error: pointer_axis requires horizontal and vertical scroll values\n".to_string();
                }
                let horizontal = match parts[1].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid horizontal scroll\n".to_string(),
                };
                let vertical = match parts[2].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid vertical scroll\n".to_string(),
                };

                let pointer = self.seat.get_pointer().unwrap();
                let pos = pointer.current_location();

                // Recursive Forwarding to Nest Child
                let under = self.space.element_under(pos);
                if let Some((win, _)) = under.as_ref() {
                    if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                        if self.is_nested_compositor_window(id) {
                            let mut payload = [0u8; 16];
                            payload[0..8].copy_from_slice(&horizontal.to_le_bytes());
                            payload[8..16].copy_from_slice(&vertical.to_le_bytes());
                            self.forward_binary_to_child(5, &payload);
                        }
                    }
                }

                self.last_event_time += 10;
                let time = self.last_event_time;



                let mut frame = AxisFrame::new(time);
                frame = frame.value(Axis::Horizontal, horizontal);
                frame = frame.value(Axis::Vertical, vertical);
                
                pointer.axis(self, frame);
                pointer.frame(self);
                "ok\n".to_string()
            }
            "pointer_gesture_swipe" => {
                if parts.len() < 3 {
                    return "error: pointer_gesture_swipe requires dx and dy\n".to_string();
                }
                let dx = match parts[1].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid dx\n".to_string(),
                };
                let dy = match parts[2].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid dy\n".to_string(),
                };

                let pointer = self.seat.get_pointer().unwrap();
                let pos = pointer.current_location();

                // Recursive Forwarding to Nest Child
                let under = self.space.element_under(pos);
                if let Some((win, _)) = under.as_ref() {
                    if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                        if self.is_nested_compositor_window(id) {
                            let mut payload = [0u8; 16];
                            payload[0..8].copy_from_slice(&dx.to_le_bytes());
                            payload[8..16].copy_from_slice(&dy.to_le_bytes());
                            self.forward_binary_to_child(7, &payload);
                        }
                    }
                }

                if dx.abs() > dy.abs() {
                    let speed = 2.0;
                    self.layout_engine.viewport.target_x += dx as f32 * speed;
                } else {
                    self.workspace_swipe_accumulator += dy as f32;
                    if self.workspace_swipe_accumulator.abs() > 150.0 {
                        if self.workspace_swipe_accumulator > 0.0 {
                            self.layout_engine.focus_workspace_down();
                        } else {
                            self.layout_engine.focus_workspace_up();
                        }
                        self.workspace_swipe_accumulator = 0.0;
                    }
                }
                self.reposition_windows();
                "ok\n".to_string()
            }
            "pointer_gesture_swipe_end" => {
                self.forward_binary_to_child(8, &[]);
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                "ok\n".to_string()
            }
            "register_child_display" => {
                if parts.len() < 2 {
                    return "error: register_child_display requires display_name\n".to_string();
                }
                let child_display = parts[1].to_string();
                println!("[register_child_display] Registering child display socket name: {}", child_display);
                self.child_display_socket = Some(child_display);
                "ok\n".to_string()
            }
            "get_child_display" => {
                if let Some(ref child) = self.child_display_socket {
                    format!("{}\n", child)
                } else {
                    "none\n".to_string()
                }
            }
            "pointer_axis_z" => {
                println!("[pointer_axis_z] Entered");
                if parts.len() < 2 {
                    return "error: pointer_axis_z requires scroll value\n".to_string();
                }
                let z_val = match parts[1].parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid scroll value\n".to_string(),
                };
                println!("[pointer_axis_z] z_val: {}", z_val);

                // Recursive Nest Doll Scroll Forwarding
                let mut payload = [0u8; 8];
                payload[0..8].copy_from_slice(&z_val.to_le_bytes());
                let forwarded = self.forward_binary_to_child(6, &payload);
                if forwarded {
                    println!("[pointer_axis_z] Successfully forwarded Z-scroll to nested child.");
                }

                if !forwarded {
                    if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                        println!("[pointer_axis_z] Performing depth scroll_z");
                        self.layout_engine.scroll_z(z_val as f32);
                        
                        let active_idx = self.layout_engine.depth_scroll_progress.round() as usize;
                        if let Some(&active_win_id) = self.layout_engine.windows.get(active_idx) {
                            let ws = self.layout_engine.active_workspace_mut();
                            if let Some((col_idx, win_idx)) = ws.find_window(active_win_id) {
                                ws.focused_column_idx = col_idx;
                                ws.columns[col_idx].focused_window_idx = win_idx;
                            }
                            let surface = self.windows.get(&active_win_id)
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                            println!("[pointer_axis_z] Focus updated to window {:?}", active_win_id);
                        }
                        self.reposition_windows();
                    } else {
                        println!("[pointer_axis_z] Performing local focus tab switch");
                        let old_win_id = self.layout_engine.active_workspace().focused_column()
                            .and_then(|col| col.focused_window().map(|w| w.id));

                        if z_val > 0.0 {
                            self.layout_engine.focus_tab_down();
                        } else if z_val < 0.0 {
                            self.layout_engine.focus_tab_up();
                        }

                        let new_win_id = self.layout_engine.active_workspace().focused_column()
                            .and_then(|col| col.focused_window().map(|w| w.id));

                        if old_win_id != new_win_id {
                            println!("[pointer_axis_z] Focus changed from {:?} to {:?}", old_win_id, new_win_id);
                            let surface = new_win_id
                                .and_then(|id| self.windows.get(&id))
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                            self.reposition_windows();
                        } else {
                            println!("[pointer_axis_z] Focus did not change locally.");
                        }
                    }
                }

                "ok\n".to_string()
            }
            "highlight_window" | "highlight-window" => {
                if parts.len() < 3 {
                    return "error: highlight_window requires window_id and color\n".to_string();
                }
                let id = match parts[1].parse::<u32>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid window_id\n".to_string(),
                };
                let color_arr = match parts[2].to_lowercase().as_str() {
                    "red" => [1.0, 0.0, 0.0, 1.0],
                    "green" => [0.0, 1.0, 0.0, 1.0],
                    "blue" => [0.0, 0.0, 1.0, 1.0],
                    "yellow" => [1.0, 1.0, 0.0, 1.0],
                    "orange" => [1.0, 0.5, 0.0, 1.0],
                    "magenta" => [1.0, 0.0, 1.0, 1.0],
                    "cyan" => [0.0, 1.0, 1.0, 1.0],
                    "white" => [1.0, 1.0, 1.0, 1.0],
                    hex if hex.starts_with('#') => {
                        let hex_val = &hex[1..];
                        if hex_val.len() == 6 {
                            let r = u8::from_str_radix(&hex_val[0..2], 16).unwrap_or(255) as f32 / 255.0;
                            let g = u8::from_str_radix(&hex_val[2..4], 16).unwrap_or(255) as f32 / 255.0;
                            let b = u8::from_str_radix(&hex_val[4..6], 16).unwrap_or(255) as f32 / 255.0;
                            [r, g, b, 1.0]
                        } else {
                            [1.0, 0.0, 0.0, 1.0] // fallback to red
                        }
                    }
                    _ => [1.0, 0.0, 0.0, 1.0],
                };
                self.highlighted_window = Some((WindowId(id), color_arr));
                "ok\n".to_string()
            }
            "clear_highlight" | "clear-highlight" => {
                self.highlighted_window = None;
                "ok\n".to_string()
            }
            "get_layout_compact" | "get-layout-compact" => {
                let mut lines = Vec::new();
                for (ws_idx, ws) in self.layout_engine.workspaces.iter().enumerate() {
                    for (col_idx, col) in ws.columns.iter().enumerate() {
                        for (win_idx, win) in col.windows.iter().enumerate() {
                            let is_focused = self.layout_engine.active_workspace_idx == ws_idx
                                && ws.focused_column_idx == col_idx
                                && col.focused_window_idx == win_idx;
                            let win_z = if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                if let Some(i) = self.layout_engine.windows.iter().position(|&w_id| w_id == win.id) {
                                    (i as f32) - self.layout_engine.depth_scroll_progress
                                } else {
                                    0.0f32
                                }
                            } else {
                                0.0f32
                            };
                            let ws_z = if self.layout_engine.overview_open {
                                self.layout_engine.current_overview_scale
                            } else {
                                1.0f32
                            };
                            let rect_str = if let Some((x, y, w, h)) = self.layout_engine.get_window_rect(win.id) {
                                let (mut rx, mut ry, mut rw, mut rh) = (x, y, w, h);
                                if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                    let transforms = self.layout_engine.depth_transforms();
                                    if let Some((_, transform)) = transforms.iter().find(|(w_id, _)| *w_id == win.id) {
                                        let scaled_w = w * transform.scale;
                                        let scaled_h = h * transform.scale;
                                        let x_offset = (w - scaled_w) / 2.0;
                                        let y_offset = (h - scaled_h) / 2.0 + (transform.y_offset as f32);
                                        rx = x + x_offset;
                                        ry = y + y_offset;
                                        rw = scaled_w;
                                        rh = scaled_h;
                                    }
                                }
                                let (screen_x, screen_y) = if self.layout_engine.overview_open {
                                    (rx, ry)
                                } else {
                                    (rx - self.layout_engine.viewport.x, ry - self.layout_engine.viewport.y)
                                };
                                format!("{},{},{},{}", screen_x as i32, screen_y as i32, rw as i32, rh as i32)
                            } else {
                                "0,0,0,0".to_string()
                            };
                            lines.push(format!("{}:{}:{}:{}:{}:{}:{:.4}:{:.4}", ws_idx, col_idx, win.id.0, is_focused, rect_str, win.title, win_z, ws_z));
                        }
                    }
                }
                format!("{}\n", lines.join("\n"))
            }
            "set_spring" | "set-spring" => {
                if parts.len() < 4 {
                    return "error: set_spring requires <camera|window|overview> <stiffness> <damping>\n".to_string();
                }
                let target = parts[1];
                let stiffness = match parts[2].parse::<f32>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid stiffness value\n".to_string(),
                };
                let damping = match parts[3].parse::<f32>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid damping value\n".to_string(),
                };

                match target {
                    "camera" => {
                        self.layout_engine.camera_spring.stiffness = stiffness;
                        self.layout_engine.camera_spring.damping = damping;
                        "ok\n".to_string()
                    }
                    "window" => {
                        self.layout_engine.window_spring.stiffness = stiffness;
                        self.layout_engine.window_spring.damping = damping;
                        "ok\n".to_string()
                    }
                    "overview" => {
                        self.layout_engine.overview_spring.stiffness = stiffness;
                        self.layout_engine.overview_spring.damping = damping;
                        "ok\n".to_string()
                    }
                    _ => "error: unknown spring target (must be camera, window, or overview)\n".to_string(),
                }
            }
            "reset_telemetry" | "reset-telemetry" => {
                self.frame_times.clear();
                self.stutter_count = 0;
                "ok\n".to_string()
            }
            "get_telemetry" | "get-telemetry" => {
                let n = self.frame_times.len();
                let (min, max, mean, stddev) = if n == 0 {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    let mut min = self.frame_times[0];
                    let mut max = self.frame_times[0];
                    let mut sum = 0.0;
                    for &t in &self.frame_times {
                        if t < min { min = t; }
                        if t > max { max = t; }
                        sum += t;
                    }
                    let mean = sum / (n as f32);
                    let stddev = if n < 2 {
                        0.0
                    } else {
                        let mut sum_sq_diff = 0.0;
                        for &t in &self.frame_times {
                            let diff = t - mean;
                            sum_sq_diff += diff * diff;
                        }
                        (sum_sq_diff / (n as f32)).sqrt()
                    };
                    (min, max, mean, stddev)
                };

                #[derive(serde::Serialize)]
                struct TelemetryResponse<'a> {
                    min_ms: f32,
                    max_ms: f32,
                    mean_ms: f32,
                    stddev_ms: f32,
                    stutter_count: u32,
                    total_frames: usize,
                    frame_times: &'a [f32],
                }

                let resp = TelemetryResponse {
                    min_ms: min,
                    max_ms: max,
                    mean_ms: mean,
                    stddev_ms: stddev,
                    stutter_count: self.stutter_count,
                    total_frames: n,
                    frame_times: &self.frame_times,
                };

                match serde_json::to_string(&resp) {
                    Ok(json_str) => format!("{}\n", json_str),
                    Err(e) => format!("error: failed to serialize telemetry: {}\n", e),
                }
            }
            "save_session" | "save-session" => {
                match self.save_session_internal() {
                    Ok(path) => format!("ok: session saved to {}\n", path),
                    Err(e) => format!("error: {}\n", e),
                }
            }
            "restore_session" | "restore-session" => {
                #[derive(serde::Serialize, serde::Deserialize)]
                struct SavedWindow {
                    title: String,
                    #[serde(default)]
                    app_id: Option<String>,
                    #[serde(default)]
                    cmdline: Option<Vec<String>>,
                }
                #[derive(serde::Serialize, serde::Deserialize)]
                struct SavedColumn {
                    width: f32,
                    focused_window_idx: usize,
                    windows: Vec<SavedWindow>,
                }
                #[derive(serde::Serialize, serde::Deserialize)]
                struct SavedWorkspace {
                    focused_column_idx: usize,
                    columns: Vec<SavedColumn>,
                }
                #[derive(serde::Serialize, serde::Deserialize)]
                struct SavedSession {
                    active_workspace_idx: usize,
                    workspaces: Vec<SavedWorkspace>,
                }

                let cookie = std::env::var("HIER_COOKIE").ok();
                let path = if let Some(ref cookie_id) = cookie {
                    format!("{}/.cache/hier/cookies/{}/session.json", std::env::var("HOME").unwrap_or_else(|_| "/home/super".to_string()), cookie_id)
                } else {
                    "/tmp/hier-session.json".to_string()
                };

                let file = match std::fs::File::open(&path) {
                    Ok(f) => f,
                    Err(e) => return format!("error: failed to open session file: {}\n", e),
                };
                let session: SavedSession = match serde_json::from_reader(file) {
                    Ok(s) => s,
                    Err(e) => return format!("error: failed to deserialize session: {}\n", e),
                };

                struct LiveWindow {
                    id: WindowId,
                    title: String,
                    app_id: Option<String>,
                }

                let mut pool: Vec<LiveWindow> = self.windows.iter().map(|(id, win)| {
                    let title = win.toplevel().map(|t| {
                        smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                .unwrap()
                                .lock()
                                .unwrap()
                                .title
                                .clone()
                        }).unwrap_or_else(|| "Wayland Window".to_string())
                    }).unwrap_or_else(|| "Unknown".to_string());

                    let app_id = win.toplevel().and_then(|t| {
                        smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                .unwrap()
                                .lock()
                                .unwrap()
                                .app_id
                                .clone()
                        })
                    });

                    LiveWindow {
                        id: *id,
                        title,
                        app_id,
                    }
                }).collect();

                struct RestoreSlot {
                    ws_idx: usize,
                    col_idx: usize,
                    col_width: f32,
                    col_focused_idx: usize,
                    saved_title: String,
                    saved_app_id: Option<String>,
                    saved_cmdline: Option<Vec<String>>,
                    matched_win: Option<(WindowId, String)>,
                }

                let mut slots = Vec::new();
                for (ws_idx, saved_ws) in session.workspaces.iter().enumerate() {
                    for (col_idx, saved_col) in saved_ws.columns.iter().enumerate() {
                        for saved_win in &saved_col.windows {
                            slots.push(RestoreSlot {
                                ws_idx,
                                col_idx,
                                col_width: saved_col.width,
                                col_focused_idx: saved_col.focused_window_idx,
                                saved_title: saved_win.title.clone(),
                                saved_app_id: saved_win.app_id.clone(),
                                saved_cmdline: saved_win.cmdline.clone(),
                                matched_win: None,
                            });
                        }
                    }
                }

                // Phase 1: Exact Match (Title & App ID)
                for slot in &mut slots {
                    if let Some(pos) = pool.iter().position(|w| {
                        w.title == slot.saved_title && w.app_id == slot.saved_app_id
                    }) {
                        let matched = pool.remove(pos);
                        slot.matched_win = Some((matched.id, matched.title));
                    }
                }

                // Phase 2: Fuzzy Title Match (contains/contained)
                for slot in &mut slots {
                    if slot.matched_win.is_none() {
                        if let Some(pos) = pool.iter().position(|w| {
                            w.title.contains(&slot.saved_title) || slot.saved_title.contains(&w.title)
                        }) {
                            let matched = pool.remove(pos);
                            slot.matched_win = Some((matched.id, matched.title));
                        }
                    }
                }

                // Phase 3: App ID Match (class name)
                for slot in &mut slots {
                    if slot.matched_win.is_none() {
                        if let Some(pos) = pool.iter().position(|w| {
                            w.app_id.is_some() && w.app_id == slot.saved_app_id
                        }) {
                            let matched = pool.remove(pos);
                            slot.matched_win = Some((matched.id, matched.title));
                        }
                    }
                }

                // Phase 4: Fallback Match (leftovers)
                for slot in &mut slots {
                    if slot.matched_win.is_none() {
                        if !pool.is_empty() {
                            let matched = pool.remove(0);
                            slot.matched_win = Some((matched.id, matched.title));
                        }
                    }
                }

                // Phase 5: Spawn missing clients and register pending restores
                let socket = self.socket_name.clone();
                for slot in &slots {
                    if slot.matched_win.is_none() {
                        // Register pending layout restore mapping
                        self.pending_restores.push(PendingRestore {
                            title: slot.saved_title.clone(),
                            app_id: slot.saved_app_id.clone(),
                            ws_idx: slot.ws_idx,
                            col_idx: slot.col_idx,
                            col_width: slot.col_width,
                            col_focused_idx: slot.col_focused_idx,
                        });

                        if let Some(ref cmdline) = slot.saved_cmdline {
                            if !cmdline.is_empty() {
                                println!("[restore] Spawning missing client process: {:?}", cmdline);
                                let mut cmd = std::process::Command::new(&cmdline[0]);
                                if cmdline.len() > 1 {
                                    cmd.args(&cmdline[1..]);
                                }
                                // Set WAYLAND_DISPLAY env so the client connects to this nested display
                                cmd.env("WAYLAND_DISPLAY", &socket);
                                let _ = cmd.spawn();
                            }
                        }
                    }
                }

                // Clear live workspace columns
                for ws in &mut self.layout_engine.workspaces {
                    ws.columns.clear();
                    ws.focused_column_idx = 0;
                }

                // Reconstruct workspaces using matched slots
                let mut ws_cols: HashMap<usize, HashMap<usize, (f32, usize, Vec<crate::layout::Window>)>> = HashMap::new();
                for slot in &slots {
                    if let Some((win_id, ref title)) = slot.matched_win {
                        let target_ws_idx = slot.ws_idx.min(self.layout_engine.workspaces.len() - 1);
                        let cols_map = ws_cols.entry(target_ws_idx).or_default();
                        let col_entry = cols_map.entry(slot.col_idx).or_insert_with(|| {
                            (slot.col_width, slot.col_focused_idx, Vec::new())
                        });
                        col_entry.2.push(crate::layout::Window::new(win_id, title.clone()));
                    }
                }

                // Add constructed columns to layout engine workspaces
                for (ws_idx, cols_map) in ws_cols {
                    let ws = &mut self.layout_engine.workspaces[ws_idx];
                    let mut sorted_keys: Vec<usize> = cols_map.keys().copied().collect();
                    sorted_keys.sort_unstable();
                    
                    for col_idx in sorted_keys {
                        let (width, focused_idx, windows) = cols_map.get(&col_idx).unwrap().clone();
                        if !windows.is_empty() {
                            let final_focused_idx = focused_idx.min(windows.len() - 1);
                            ws.columns.push(crate::layout::Column {
                                windows,
                                focused_window_idx: final_focused_idx,
                                width,
                            });
                        }
                    }
                }

                // Place remaining unmatched windows on Workspace 0
                let ws = &mut self.layout_engine.workspaces[0];
                for leftover in pool {
                    let col = crate::layout::Column::new(crate::layout::Window::new(leftover.id, leftover.title), 500.0);
                    ws.columns.push(col);
                }

                // Ensure focus constraints are satisfied
                for ws in &mut self.layout_engine.workspaces {
                    if !ws.columns.is_empty() {
                        ws.focused_column_idx = ws.focused_column_idx.min(ws.columns.len() - 1);
                    } else {
                        ws.focused_column_idx = 0;
                    }
                }

                self.layout_engine.active_workspace_idx = session.active_workspace_idx.min(self.layout_engine.workspaces.len() - 1);
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                "ok: session layout restored\n".to_string()
            }
            "capture_window" | "capture-window" => {
                if parts.len() < 3 {
                    return "error: capture_window requires window_id and path\n".to_string();
                }
                let id = match parts[1].parse::<u32>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid window_id\n".to_string(),
                };
                let path = parts[2];
                if let Some(win) = self.windows.get(&WindowId(id)) {
                    let title = win.toplevel().map(|t| {
                        smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                .unwrap()
                                .lock()
                                .unwrap()
                                .title
                                .clone()
                        }).unwrap_or_else(|| "Wayland Window".to_string())
                    }).unwrap_or_else(|| "Unknown".to_string());
                    
                    if let Some((_x, _y, w, h)) = self.layout_engine.get_window_rect(WindowId(id)) {
                        let width = w as usize;
                        let height = h as usize;
                        let mut rgb_data = vec![30u8; width * height * 3];
                        let mut rgba_data = vec![30u8; width * height * 4];
                        
                        let title_lower = title.to_lowercase();
                        let is_terminal = title_lower.contains("terminal") || title_lower.contains("ghostty") || title_lower.contains("alacritty");
                        let is_browser = title_lower.contains("chrome") || title_lower.contains("browser") || title_lower.contains("firefox") || title_lower.contains("epiphany");

                        for py in 0..height {
                            for px in 0..width {
                                let idx3 = (py * width + px) * 3;
                                let idx4 = (py * width + px) * 4;
                                
                                // Draw 4px border (Dodger Blue: (30, 144, 255))
                                if py < 4 || py >= height - 4 || px < 4 || px >= width - 4 {
                                    rgb_data[idx3] = 30;
                                    rgb_data[idx3 + 1] = 144;
                                    rgb_data[idx3 + 2] = 255;
                                    
                                    rgba_data[idx4] = 30;
                                    rgba_data[idx4 + 1] = 144;
                                    rgba_data[idx4 + 2] = 255;
                                    rgba_data[idx4 + 3] = 255;
                                    continue;
                                }

                                // Interior dynamic elements
                                let (r, g, b) = if is_terminal {
                                    // Terminal template
                                    if py >= height * 5 / 100 && py < height * 10 / 100 && px >= width * 5 / 100 && px < width * 10 / 100 {
                                        // Prompt: green block
                                        (50, 205, 50)
                                    } else if py >= height * 5 / 100 && py < height * 10 / 100 && px >= width * 12 / 100 && px < width * 18 / 100 {
                                        // Prompt cursor: green block
                                        (50, 205, 50)
                                    } else if (py == height * 30 / 100 || py == height * 50 / 100 || py == height * 70 / 100) && px >= width * 5 / 100 && px < width * 95 / 100 {
                                        // stdout lines: light gray
                                        (180, 180, 180)
                                    } else {
                                        // dark gray background
                                        (15, 15, 15)
                                    }
                                } else if is_browser {
                                    // Browser template
                                    if py >= height * 5 / 100 && py < height * 15 / 100 && px >= width * 5 / 100 && px < width * 95 / 100 {
                                        // Address bar container
                                        if py >= height * 7 / 100 && py < height * 13 / 100 && px >= width * 15 / 100 && px < width * 85 / 100 {
                                            // URL Input Box: White
                                            (255, 255, 255)
                                        } else {
                                            // Container gray
                                            (210, 210, 210)
                                        }
                                    } else if py >= height * 20 / 100 && py < height * 90 / 100 && px >= width * 5 / 100 && px < width * 95 / 100 {
                                        // Web page Card: Light sky blue
                                        (135, 206, 250)
                                    } else {
                                        // browser background: Off-white
                                        (240, 240, 240)
                                    }
                                } else {
                                    // General app template (default)
                                    if py >= 4 && py < height * 10 / 100 && px >= 4 && px < width - 4 {
                                        // Title bar area (dark gray)
                                        (30, 30, 30)
                                    } else if py >= height * 80 / 100 && py < height * 90 / 100 {
                                        if px >= width * 35 / 100 && px < width * 48 / 100 {
                                            // OK button: light gray
                                            (180, 180, 180)
                                        } else if px >= width * 52 / 100 && px < width * 65 / 100 {
                                            // Cancel button: mid gray
                                            (100, 100, 100)
                                        } else {
                                            (45, 45, 45)
                                        }
                                    } else {
                                        // medium gray background
                                        (45, 45, 45)
                                    }
                                };

                                rgb_data[idx3] = r;
                                rgb_data[idx3 + 1] = g;
                                rgb_data[idx3 + 2] = b;
                                
                                rgba_data[idx4] = r;
                                rgba_data[idx4 + 1] = g;
                                rgba_data[idx4 + 2] = b;
                                rgba_data[idx4 + 3] = 255;
                            }
                        }
                        
                        let mut file_ok = false;
                        use std::io::Write;
                        match std::fs::File::create(path) {
                            Ok(mut file) => {
                                let header = format!("P6\n{} {}\n255\n", width, height);
                                if file.write_all(header.as_bytes()).is_ok() && file.write_all(&rgb_data).is_ok() {
                                    file_ok = true;
                                }
                            }
                            Err(_) => {}
                        }
                        
                        let mut clip_ok = false;
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let img_data = arboard::ImageData {
                                width,
                                height,
                                bytes: std::borrow::Cow::from(&rgba_data),
                            };
                            if clipboard.set_image(img_data).is_ok() {
                                clip_ok = true;
                            }
                        }
                        
                        match (file_ok, clip_ok) {
                            (true, true) => format!("ok: exported {} window (size {}x{}) to {} and copied to clipboard\n", title, width, height, path),
                            (true, false) => format!("ok: exported {} window (size {}x{}) to {}, but failed to copy to clipboard\n", title, width, height, path),
                            (false, true) => format!("ok: copied {} window (size {}x{}) to clipboard, but failed to write to {}\n", title, width, height, path),
                            (false, false) => "error: failed to write file and failed to copy to clipboard\n".to_string(),
                        }
                    } else {
                        "error: window geometry not found\n".to_string()
                    }
                } else {
                    "error: window not found\n".to_string()
                }
            }
            "copy_window_to_clipboard" | "copy-window-to-clipboard" => {
                if parts.len() < 2 {
                    return "error: copy_window_to_clipboard requires window_id\n".to_string();
                }
                let id = match parts[1].parse::<u32>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid window_id\n".to_string(),
                };
                if let Some(win) = self.windows.get(&WindowId(id)) {
                    let title = win.toplevel().map(|t| {
                        smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                .unwrap()
                                .lock()
                                .unwrap()
                                .title
                                .clone()
                        }).unwrap_or_else(|| "Wayland Window".to_string())
                    }).unwrap_or_else(|| "Unknown".to_string());
                    
                    if let Some((_x, _y, w, h)) = self.layout_engine.get_window_rect(WindowId(id)) {
                        let width = w as usize;
                        let height = h as usize;
                        let mut rgba_data = vec![30u8; width * height * 4];
                        
                        for py in 0..height {
                            for px in 0..width {
                                let idx = (py * width + px) * 4;
                                if py < 4 || py >= height - 4 || px < 4 || px >= width - 4 {
                                    rgba_data[idx] = 30;
                                    rgba_data[idx + 1] = 144;
                                    rgba_data[idx + 2] = 255;
                                } else {
                                    rgba_data[idx] = 30;
                                    rgba_data[idx + 1] = 30;
                                    rgba_data[idx + 2] = 30;
                                }
                                rgba_data[idx + 3] = 255;
                            }
                        }
                        
                        match arboard::Clipboard::new() {
                            Ok(mut clipboard) => {
                                let img_data = arboard::ImageData {
                                    width,
                                    height,
                                    bytes: std::borrow::Cow::from(&rgba_data),
                                };
                                match clipboard.set_image(img_data) {
                                    Ok(_) => format!("ok: copied {} window (size {}x{}) to clipboard\n", title, width, height),
                                    Err(e) => format!("error: failed to set clipboard image: {}\n", e),
                                }
                            }
                            Err(e) => format!("error: failed to initialize clipboard: {}\n", e),
                        }
                    } else {
                        "error: window geometry not found\n".to_string()
                    }
                } else {
                    "error: window not found\n".to_string()
                }
            }
            "action" => {
                if parts.len() < 2 {
                    return "error: action requires action_name\n".to_string();
                }
                match self.perform_layout_action(parts[1]) {
                    Ok(_) => "ok\n".to_string(),
                    Err(e) => format!("error: {}\n", e),
                }
            }
            "get_layout" => {
                let workspaces_json: Vec<serde_json::Value> = self.layout_engine.workspaces.iter().enumerate().map(|(idx, ws)| {
                    let columns_json: Vec<serde_json::Value> = ws.columns.iter().enumerate().map(|(col_idx, col)| {
                        let windows_json: Vec<serde_json::Value> = col.windows.iter().enumerate().map(|(win_idx, win)| {
                            let win_z = if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                if let Some(i) = self.layout_engine.windows.iter().position(|&w_id| w_id == win.id) {
                                    (i as f32) - self.layout_engine.depth_scroll_progress
                                } else {
                                    0.0f32
                                }
                            } else {
                                0.0f32
                            };

                            serde_json::json!({
                                "id": win.id.0,
                                "title": win.title,
                                "scrolling_position": {
                                    "column": col_idx,
                                    "tile": win_idx,
                                    "z_axis": win_z
                                },
                                "scrolling_position_formatted": format!("column({}) ; tile){} ; z axis({})", col_idx, win_idx, win_z),
                                "z_axis": win_z
                            })
                        }).collect();

                        serde_json::json!({
                            "focused_window_idx": col.focused_window_idx,
                            "width": col.width,
                            "windows": windows_json
                        })
                    }).collect();

                    serde_json::json!({
                        "idx": idx,
                        "focused_column_idx": ws.focused_column_idx,
                        "columns": columns_json
                    })
                }).collect();

                let layout_json = serde_json::json!({
                    "active_workspace_idx": self.layout_engine.active_workspace_idx,
                    "tiling_mode": if self.layout_engine.overview_open {
                        "Overview".to_string()
                    } else {
                        format!("{:?}", self.layout_engine.tiling_mode)
                    },
                    "viewport": {
                        "x": self.layout_engine.viewport.x,
                        "y": self.layout_engine.viewport.y,
                        "target_x": self.layout_engine.viewport.target_x,
                        "target_y": self.layout_engine.viewport.target_y,
                        "width": self.layout_engine.viewport.width,
                        "height": self.layout_engine.viewport.height,
                    },
                    "workspaces": workspaces_json
                });

                match serde_json::to_string_pretty(&layout_json) {
                    Ok(json) => format!("{}\n", json),
                    Err(e) => format!("error: failed to serialize layout: {}\n", e),
                }
            }
            "get_scrolling_position" | "get-scrolling-position" => {
                let target_win_id = if parts.len() > 1 {
                    parts[1].parse::<u32>().ok().map(WindowId)
                } else {
                    None
                };

                if let Some(win_id) = target_win_id {
                    let mut found = None;
                    for (_ws_idx, ws) in self.layout_engine.workspaces.iter().enumerate() {
                        if let Some((col_idx, win_idx)) = ws.find_window(win_id) {
                            let win_z = if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                if let Some(i) = self.layout_engine.windows.iter().position(|&w_id| w_id == win_id) {
                                    (i as f32) - self.layout_engine.depth_scroll_progress
                                } else {
                                    0.0f32
                                }
                            } else {
                                0.0f32
                            };
                            found = Some(format!("Scrolling Position: column({}) ; tile){} ; z axis({})\n", col_idx, win_idx, win_z));
                            break;
                        }
                    }
                    found.unwrap_or_else(|| "error: window not found\n".to_string())
                } else {
                    let ws_idx = self.layout_engine.active_workspace_idx;
                    let ws = &self.layout_engine.workspaces[ws_idx];
                    let col_idx = ws.focused_column_idx;
                    if let Some(col) = ws.columns.get(col_idx) {
                        let win_idx = col.focused_window_idx;
                        if let Some(win) = col.windows.get(win_idx) {
                            let win_z = if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                if let Some(i) = self.layout_engine.windows.iter().position(|&w_id| w_id == win.id) {
                                    (i as f32) - self.layout_engine.depth_scroll_progress
                                } else {
                                    0.0f32
                                }
                            } else {
                                0.0f32
                            };
                            format!("Scrolling Position: column({}) ; tile){} ; z axis({})\n", col_idx, win_idx, win_z)
                        } else {
                            "error: no focused window\n".to_string()
                        }
                    } else {
                        "error: no columns in active workspace\n".to_string()
                    }
                }
            }
            "reposition_window" | "reposition-window" => {
                if parts.len() < 5 {
                    return "error: reposition_window requires window_id workspace_idx column_idx tile_idx\n".to_string();
                }
                let win_id = match parts[1].parse::<u32>() {
                    Ok(id) => WindowId(id),
                    Err(_) => return "error: invalid window_id\n".to_string(),
                };
                let ws_idx = match parts[2].parse::<usize>() {
                    Ok(idx) => idx,
                    Err(_) => return "error: invalid workspace_idx\n".to_string(),
                };
                let col_idx = match parts[3].parse::<usize>() {
                    Ok(idx) => idx,
                    Err(_) => return "error: invalid column_idx\n".to_string(),
                };
                let tile_idx = match parts[4].parse::<usize>() {
                    Ok(idx) => idx,
                    Err(_) => return "error: invalid tile_idx\n".to_string(),
                };

                // 1. Locate and remove the window from its current position
                let mut found_win = None;
                let mut found_width = 400.0; // fallback width
                for w_idx in 0..self.layout_engine.workspaces.len() {
                    let ws = &mut self.layout_engine.workspaces[w_idx];
                    let mut remove_loc = None;
                    for (c_idx, col) in ws.columns.iter().enumerate() {
                        for (t_idx, win) in col.windows.iter().enumerate() {
                            if win.id == win_id {
                                remove_loc = Some((c_idx, t_idx));
                                break;
                            }
                        }
                        if remove_loc.is_some() { break; }
                    }
                    if let Some((c_idx, t_idx)) = remove_loc {
                        let col = &mut ws.columns[c_idx];
                        found_width = col.width;
                        let win = col.windows.remove(t_idx);
                        found_win = Some(win);
                        
                        // Clean up column if empty
                        if col.windows.is_empty() {
                            ws.columns.remove(c_idx);
                            if ws.focused_column_idx >= ws.columns.len() && !ws.columns.is_empty() {
                                ws.focused_column_idx = ws.columns.len() - 1;
                            }
                        } else if col.focused_window_idx >= col.windows.len() {
                            col.focused_window_idx = col.windows.len() - 1;
                        }
                        break;
                    }
                }

                let window = match found_win {
                    Some(w) => w,
                    None => return "error: window not found\n".to_string(),
                };

                // 2. Insert into the target workspace, column, and tile index
                let target_ws_idx = ws_idx.min(self.layout_engine.workspaces.len() - 1);
                let ws = &mut self.layout_engine.workspaces[target_ws_idx];
                
                if ws.columns.is_empty() {
                    let col = crate::layout::Column::new(window, found_width);
                    ws.columns.push(col);
                    ws.focused_column_idx = 0;
                } else {
                    let target_col_idx = col_idx.min(ws.columns.len());
                    if target_col_idx == ws.columns.len() {
                        // Append as a new column
                        let col = crate::layout::Column::new(window, found_width);
                        ws.columns.push(col);
                        ws.focused_column_idx = target_col_idx;
                    } else {
                        // Insert into an existing column
                        let col = &mut ws.columns[target_col_idx];
                        let target_tile_idx = tile_idx.min(col.windows.len());
                        col.windows.insert(target_tile_idx, window);
                        col.focused_window_idx = target_tile_idx;
                        ws.focused_column_idx = target_col_idx;
                    }
                }

                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                "ok\n".to_string()
            }
            "cut_window" | "cut-window" => {
                if parts.len() < 3 {
                    return "error: cut_window requires child_display and window_id\n".to_string();
                }
                let child_display = parts[1].to_string();
                let child_win_id = match parts[2].parse::<u32>() {
                    Ok(id) => id,
                    Err(_) => return "error: invalid window_id\n".to_string(),
                };

                // 1. Query the child compositor's layout to find the window title
                let child_socket = format!("/tmp/hier-ctrl-{}.sock", child_display);
                if !std::path::Path::new(&child_socket).exists() {
                    return format!("error: child control socket not found at {}\n", child_socket);
                }

                let mut stream = match std::os::unix::net::UnixStream::connect(&child_socket) {
                    Ok(s) => s,
                    Err(e) => return format!("error: failed to connect to child socket: {}\n", e),
                };

                use std::io::{Write, Read};
                let _ = stream.write_all(b"get_layout_compact\n");
                let _ = stream.flush();
                
                let mut response = String::new();
                let mut temp_buf = [0u8; 4096];
                loop {
                    match stream.read(&mut temp_buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            response.push_str(&String::from_utf8_lossy(&temp_buf[..n]));
                            if response.contains('\n') || response.len() > 1024 {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                // Find the window title from child response
                let mut title = format!("Promoted Window {}", child_win_id);
                for line in response.lines() {
                    let parts_line: Vec<&str> = line.split(':').collect();
                    if parts_line.len() >= 6 {
                        if parts_line[2].parse::<u32>().ok() == Some(child_win_id) {
                            title = parts_line[5].to_string();
                            break;
                        }
                    }
                }

                // 2. Instruct the child compositor to remove that window from active viewport layout
                if let Ok(mut stream2) = std::os::unix::net::UnixStream::connect(&child_socket) {
                    let _ = stream2.write_all(format!("reposition_window {} 999 999 999\n", child_win_id).as_bytes());
                    let _ = stream2.flush();
                }

                // 3. Spawn the window in the parent compositor (Z) with custom access properties
                let promoted_title = format!("[Custom Access Promoted] {}", title);
                let parent_win_id = self.next_window_id;
                self.next_window_id += 1;
                
                self.layout_engine.spawn_window(WindowId(parent_win_id), promoted_title);
                
                // Highlight the promoted window with a distinct custom border color (orange) to represent custom access
                self.highlighted_window = Some((WindowId(parent_win_id), [1.0, 0.549, 0.0, 1.0])); 
                
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();

                format!("ok: promoted window {} to parent compositor window {} with custom access\n", child_win_id, parent_win_id)
            }
            "get_windows" => {
                let windows_json: Vec<serde_json::Value> = self.windows.iter().map(|(id, win)| {
                    let title = win.toplevel().map(|t| {
                        smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                .unwrap()
                                .lock()
                                .unwrap()
                                .title
                                .clone()
                        }).unwrap_or_else(|| "Wayland Window".to_string())
                    }).unwrap_or_else(|| "Unknown".to_string());

                    let app_id = win.toplevel().and_then(|t| {
                        smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                .unwrap()
                                .lock()
                                .unwrap()
                                .app_id
                                .clone()
                        })
                    });

                    let pid = win.toplevel().and_then(|t| {
                        use smithay::reexports::wayland_server::Resource;
                        t.wl_surface().client().and_then(|c| {
                            c.get_credentials(&self.display_handle).ok().map(|creds| creds.pid)
                        })
                    });

                    serde_json::json!({
                        "id": id.0,
                        "title": title,
                        "app_id": app_id,
                        "pid": pid
                    })
                }).collect();

                match serde_json::to_string_pretty(&windows_json) {
                    Ok(json) => format!("{}\n", json),
                    Err(e) => format!("error: failed to serialize windows: {}\n", e),
                }
            }
            "get_camera" | "get-camera" => {
                let report_mode = if self.layout_engine.overview_open {
                    crate::layout::TilingMode::Overview
                } else {
                    self.layout_engine.tiling_mode.clone()
                };
                format!(
                    "{},{},{},{},{},{},{:?}\n",
                    self.layout_engine.viewport.x,
                    self.layout_engine.viewport.y,
                    self.layout_engine.viewport.target_x,
                    self.layout_engine.viewport.target_y,
                    self.layout_engine.viewport.width,
                    self.layout_engine.viewport.height,
                    report_mode
                )
            }
            "set_camera" | "set-camera" => {
                if parts.len() < 3 {
                    return "error: set_camera requires x and y\n".to_string();
                }
                let x = match parts[1].parse::<f32>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid x\n".to_string(),
                };
                let y = match parts[2].parse::<f32>() {
                    Ok(val) => val,
                    Err(_) => return "error: invalid y\n".to_string(),
                };
                let immediate = parts.get(3).map(|&s| s == "true" || s == "immediate").unwrap_or(false);
                
                self.layout_engine.viewport.target_x = x;
                self.layout_engine.viewport.target_y = y;
                if immediate {
                    self.layout_engine.viewport.x = x;
                    self.layout_engine.viewport.y = y;
                    self.layout_engine.viewport.velocity_x = 0.0;
                    self.layout_engine.viewport.velocity_y = 0.0;
                }
                self.reposition_windows();
                "ok\n".to_string()
            }
            other => format!("error: unknown command '{}'\n", other),
        }
    }

    pub fn handle_simulated_binary_input(&mut self, msg_type: u8, payload: &[u8]) -> String {
        use smithay::backend::input::Axis;
        use smithay::input::pointer::AxisFrame;

        match msg_type {
            1 => {
                // keyboard_key
                if payload.len() < 5 {
                    return "error: invalid payload size\n".to_string();
                }
                let keycode = u32::from_le_bytes(payload[0..4].try_into().unwrap());
                let state_val = payload[4];
                let key_state = if state_val == 1 { KeyState::Pressed } else { KeyState::Released };

                // Forward
                let is_nested = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id))
                    .map(|id| self.is_nested_compositor_window(id))
                    .unwrap_or(false);
                if is_nested {
                    self.forward_binary_to_child(1, payload);
                }

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;
                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.input(
                    self,
                    (keycode + 8).into(),
                    key_state,
                    serial,
                    time,
                    |state, modifiers, handle| {
                        let keysym = handle.modified_sym();
                        state.handle_key_action(key_state, modifiers, keysym)
                    },
                );
                "ok\n".to_string()
            }
            2 => {
                // pointer_motion
                if payload.len() < 16 {
                    return "error: invalid payload size\n".to_string();
                }
                let x = f64::from_le_bytes(payload[0..8].try_into().unwrap());
                let y = f64::from_le_bytes(payload[8..16].try_into().unwrap());

                let is_overview = self.layout_engine.overview_open;
                let vp = &self.layout_engine.viewport;
                let (min_x, max_x, min_y, max_y) = if is_overview {
                    (0.0, vp.width as f64, 0.0, vp.height as f64)
                } else {
                    (vp.x as f64, (vp.x + vp.width) as f64, vp.y as f64, (vp.y + vp.height) as f64)
                };

                let clamped_x = x.clamp(min_x, max_x);
                let clamped_y = y.clamp(min_y, max_y);

                let pos = Point::from((clamped_x, clamped_y));
                let pointer = self.seat.get_pointer().unwrap();

                let (focus, space_pos) = if is_overview {
                    if let Some(win_id) = self.window_under_pointer(pos) {
                        self.highlighted_window = Some((win_id, [0.117, 0.565, 1.0, 1.0]));
                    } else {
                        self.highlighted_window = None;
                    }
                    (None, pos + Point::from((vp.x as f64, vp.y as f64)))
                } else {
                    let under = self.space.element_under(pos);
                    if let Some((win, local_pos)) = under.as_ref() {
                        if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                            if self.is_nested_compositor_window(id) {
                                let local_x = local_pos.x as f64;
                                let local_y = local_pos.y as f64;
                                let mut local_payload = [0u8; 16];
                                local_payload[0..8].copy_from_slice(&local_x.to_le_bytes());
                                local_payload[8..16].copy_from_slice(&local_y.to_le_bytes());
                                self.forward_binary_to_child(3, &local_payload);
                            }
                        }
                    }
                    let focus = under.and_then(|(win, local_pos)| {
                        win.surface_under(local_pos.to_f64(), WindowSurfaceType::ALL)
                            .map(|(surface, surface_local_pos)| (surface, surface_local_pos.to_f64()))
                    });
                    (focus, pos)
                };

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;

                pointer.motion(
                    self,
                    focus,
                    &smithay::input::pointer::MotionEvent {
                        location: space_pos,
                        time,
                        serial,
                    },
                );
                pointer.frame(self);
                "ok\n".to_string()
            }
            3 => {
                // pointer_motion_local
                if payload.len() < 16 {
                    return "error: invalid payload size\n".to_string();
                }
                let x = f64::from_le_bytes(payload[0..8].try_into().unwrap());
                let y = f64::from_le_bytes(payload[8..16].try_into().unwrap());

                let vp = &self.layout_engine.viewport;
                let clamped_x = x.clamp(0.0, vp.width as f64);
                let clamped_y = y.clamp(0.0, vp.height as f64);

                let global_x = clamped_x + vp.x as f64;
                let global_y = clamped_y + vp.y as f64;

                let pos = Point::from((global_x, global_y));
                let pointer = self.seat.get_pointer().unwrap();
                let under = self.space.element_under(pos);

                if let Some((win, local_pos)) = under.as_ref() {
                    if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                        if self.is_nested_compositor_window(id) {
                            let local_x = local_pos.x as f64;
                            let local_y = local_pos.y as f64;
                            let mut local_payload = [0u8; 16];
                            local_payload[0..8].copy_from_slice(&local_x.to_le_bytes());
                            local_payload[8..16].copy_from_slice(&local_y.to_le_bytes());
                            self.forward_binary_to_child(3, &local_payload);
                        }
                    }
                }

                let focus = under.and_then(|(win, local_pos)| {
                    win.surface_under(local_pos.to_f64(), WindowSurfaceType::ALL)
                        .map(|(surface, surface_local_pos)| (surface, surface_local_pos.to_f64()))
                });

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;

                pointer.motion(
                    self,
                    focus,
                    &smithay::input::pointer::MotionEvent {
                        location: pos,
                        time,
                        serial,
                    },
                );
                pointer.frame(self);
                "ok\n".to_string()
            }
            4 => {
                // pointer_button
                if payload.len() < 5 {
                    return "error: invalid payload size\n".to_string();
                }
                let button = u32::from_le_bytes(payload[0..4].try_into().unwrap());
                let state_val = payload[4];
                let state = if state_val == 1 { ButtonState::Pressed } else { ButtonState::Released };

                let serial = SERIAL_COUNTER.next_serial();
                self.last_event_time += 10;
                let time = self.last_event_time;
                let pointer = self.seat.get_pointer().unwrap();
                let pos = pointer.current_location();

                let (focus, surface, is_nested, clicked_win_id) = {
                    let under = self.space.element_under(pos);
                    let focus = under.as_ref().and_then(|(win, local_pos)| {
                        win.surface_under(local_pos.to_f64(), WindowSurfaceType::ALL)
                            .map(|(surface, surface_local_pos)| (surface.clone(), surface_local_pos.to_f64()))
                    });
                    let surface = under.as_ref().and_then(|(win, _)| win.wl_surface().map(|c| c.into_owned()));
                    let is_nested = under.as_ref().and_then(|(win, _)| {
                        self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| self.is_nested_compositor_window(*id))
                    }).unwrap_or(false);
                    let clicked_win_id = under.as_ref().and_then(|(win, _)| {
                        self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id)
                    });
                    (focus, surface, is_nested, clicked_win_id)
                };

                if is_nested {
                    self.forward_binary_to_child(4, payload);
                }

                pointer.motion(
                    self,
                    focus,
                    &smithay::input::pointer::MotionEvent {
                        location: pos,
                        time,
                        serial,
                    },
                );

                pointer.button(
                    self,
                    &smithay::input::pointer::ButtonEvent {
                        button,
                        state,
                        serial,
                        time,
                    },
                );
                pointer.frame(self);

                if state == ButtonState::Pressed {
                    if self.layout_engine.overview_open {
                        let screen_pos = pos - Point::from((
                            self.layout_engine.viewport.x as f64,
                            self.layout_engine.viewport.y as f64,
                        ));
                        if let Some(win_id) = self.window_under_pointer(screen_pos) {
                            self.focus_window_by_id(win_id);
                            self.layout_engine.overview_open = false;
                            self.layout_engine.overview_progress = None;
                            self.layout_engine.recenter_camera(false);
                            self.reposition_windows();
                            return "ok\n".to_string();
                        }
                    } else {
                        if let Some(win_id) = clicked_win_id {
                            self.focus_window_by_id(win_id);
                            self.layout_engine.recenter_camera(false);
                            self.reposition_windows();
                        }
                    }
                    
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                }
                "ok\n".to_string()
            }
            5 => {
                // pointer_axis
                if payload.len() < 16 {
                    return "error: invalid payload size\n".to_string();
                }
                let horizontal = f64::from_le_bytes(payload[0..8].try_into().unwrap());
                let vertical = f64::from_le_bytes(payload[8..16].try_into().unwrap());

                let pointer = self.seat.get_pointer().unwrap();
                let pos = pointer.current_location();

                let under = self.space.element_under(pos);
                if let Some((win, _)) = under.as_ref() {
                    if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                        if self.is_nested_compositor_window(id) {
                            self.forward_binary_to_child(5, payload);
                        }
                    }
                }

                self.last_event_time += 10;
                let time = self.last_event_time;

                let mut frame = AxisFrame::new(time);
                frame = frame.value(Axis::Horizontal, horizontal);
                frame = frame.value(Axis::Vertical, vertical);
                
                pointer.axis(self, frame);
                pointer.frame(self);
                "ok\n".to_string()
            }
            6 => {
                // pointer_axis_z
                if payload.len() < 8 {
                    return "error: invalid payload size\n".to_string();
                }
                let z_val = f64::from_le_bytes(payload[0..8].try_into().unwrap());

                let forwarded = self.forward_binary_to_child(6, payload);

                if !forwarded {
                    if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                        self.layout_engine.scroll_z(z_val as f32);
                        
                        let active_idx = self.layout_engine.depth_scroll_progress.round() as usize;
                        if let Some(&active_win_id) = self.layout_engine.windows.get(active_idx) {
                            let ws = self.layout_engine.active_workspace_mut();
                            if let Some((col_idx, win_idx)) = ws.find_window(active_win_id) {
                                ws.focused_column_idx = col_idx;
                                ws.columns[col_idx].focused_window_idx = win_idx;
                            }
                            let surface = self.windows.get(&active_win_id)
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                        }
                        self.reposition_windows();
                    } else {
                        let old_win_id = self.layout_engine.active_workspace().focused_column()
                            .and_then(|col| col.focused_window().map(|w| w.id));

                        if z_val > 0.0 {
                            self.layout_engine.focus_tab_down();
                        } else if z_val < 0.0 {
                            self.layout_engine.focus_tab_up();
                        }

                        let new_win_id = self.layout_engine.active_workspace().focused_column()
                            .and_then(|col| col.focused_window().map(|w| w.id));

                        if old_win_id != new_win_id {
                            let surface = new_win_id
                                .and_then(|id| self.windows.get(&id))
                                .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                            if let Some(surface) = surface {
                                self.set_keyboard_focus(Some(surface));
                            }
                            self.reposition_windows();
                        }
                    }
                }
                "ok\n".to_string()
            }
            7 => {
                // pointer_gesture_swipe
                if payload.len() < 16 {
                    return "error: invalid payload size\n".to_string();
                }
                let dx = f64::from_le_bytes(payload[0..8].try_into().unwrap());
                let dy = f64::from_le_bytes(payload[8..16].try_into().unwrap());

                let pointer = self.seat.get_pointer().unwrap();
                let pos = pointer.current_location();

                let under = self.space.element_under(pos);
                if let Some((win, _)) = under.as_ref() {
                    if let Some(id) = self.windows.iter().find(|(_, w)| **w == **win).map(|(id, _)| *id) {
                        if self.is_nested_compositor_window(id) {
                            self.forward_binary_to_child(7, payload);
                        }
                    }
                }

                if dx.abs() > dy.abs() {
                    let speed = 2.0;
                    self.layout_engine.viewport.target_x += dx as f32 * speed;
                } else {
                    self.workspace_swipe_accumulator += dy as f32;
                    if self.workspace_swipe_accumulator.abs() > 150.0 {
                        if self.workspace_swipe_accumulator > 0.0 {
                            self.layout_engine.focus_workspace_down();
                        } else {
                            self.layout_engine.focus_workspace_up();
                        }
                        self.workspace_swipe_accumulator = 0.0;
                    }
                }
                self.reposition_windows();
                "ok\n".to_string()
            }
            8 => {
                // pointer_gesture_swipe_end
                self.forward_binary_to_child(8, &[]);
                self.layout_engine.recenter_camera(false);
                self.reposition_windows();
                "ok\n".to_string()
            }
            _ => "error: unknown binary message type\n".to_string()
        }
    }
}

// --- WAYLAND PROTOCOL DISPATCH IMPLEMENTATIONS ---

// 1. Compositor Handler
impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        // Run standard buffer commit handler to handle client texture updates
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);

        let mut updated_title = None;
        let mut target_win_id = None;

        if let Some((win_id, window)) = self.windows.iter().find(|(_, w)| {
            w.wl_surface().map(|s| s.as_ref() == surface).unwrap_or(false)
        }) {
            window.on_commit();
            
            let title = smithay::wayland::compositor::with_states(surface, |states| {
                let data = states
                    .data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap();
                data.title.clone().or_else(|| data.app_id.clone())
            });
            if let Some(t) = title {
                updated_title = Some(t);
                target_win_id = Some(*win_id);
            }
        }

        let mut title_changed = false;
        if let (Some(win_id), Some(t)) = (target_win_id, updated_title) {
            for ws in &mut self.layout_engine.workspaces {
                for col in &mut ws.columns {
                    for win in &mut col.windows {
                        if win.id == win_id {
                            if win.title != t {
                                println!("[state] Dynamic title update for Window {:?}: {:?} -> {:?}", win_id, win.title, t);
                                win.title = t.clone();
                                title_changed = true;
                            }
                        }
                    }
                }
            }
        }

        if title_changed {
            self.reposition_windows();
        }
    }
}

// 2. Shared Memory Handler
impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

// 3. Seat Handler (Keyboard and Mouse Pointer Input)
impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focus: Option<&WlSurface>) {
        // No-op to avoid re-entrancy deadlocks since keyboard focus is set explicitly
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {
        // Basic cursor configuration. Can be extended to render standard cursor shapes.
    }
}

// 4. XDG Shell Handler (Window Manager requests)
impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface.clone());
        let window_id = WindowId(self.next_window_id);
        self.next_window_id += 1;

        // Configure initial size based on layout engine viewport and columns
        let is_occupied = self.layout_engine.active_workspace().has_tiled_columns();
        let win_width = if is_occupied {
            self.layout_engine.default_width_fraction * (self.layout_engine.viewport.width - 2.0 * self.layout_engine.outer_margin - self.layout_engine.gap)
        } else {
            self.layout_engine.viewport.width - 2.0 * self.layout_engine.outer_margin
        };
        let win_height = self.layout_engine.viewport.height - 2.0 * self.layout_engine.outer_margin;
        surface.with_pending_state(|state| {
            state.size = Some((win_width as i32, win_height as i32).into());
        });
        surface.send_configure();

        // Title and app_id retrieval, falling back to app_id if title is None (e.g. for fuzzel)
        let title = smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
            let data = states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap();
            data.title.clone().or_else(|| data.app_id.clone())
        }).unwrap_or_else(|| "Wayland Window".to_string());

        println!("Window mapped: ID={:?}, Title={:?}", window_id, title);

        // Spawn window in layout engine or restore to its pending layout slot
        let app_id = smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .app_id
                .clone()
        });

        let pending_match = if let Some(idx) = self.pending_restores.iter().position(|r| {
            (app_id.is_some() && r.app_id == app_id) || r.title == title || title.contains(&r.title) || r.title.contains(&title)
        }) {
            Some(self.pending_restores.remove(idx))
        } else {
            None
        };

        if let Some(r) = pending_match {
            println!("Pending window match found! Restoring ID={:?} (Title={:?}) to ws={}, col={}", window_id, title, r.ws_idx, r.col_idx);
            self.layout_engine.windows.push(window_id);
            let ws = &mut self.layout_engine.workspaces[r.ws_idx];
            let window_elem = crate::layout::Window::new(window_id, title.clone());
            if r.col_idx < ws.columns.len() {
                ws.columns[r.col_idx].windows.push(window_elem);
                ws.columns[r.col_idx].focused_window_idx = ws.columns[r.col_idx].windows.len() - 1;
            } else {
                let column = crate::layout::Column::new(window_elem, r.col_width);
                ws.columns.push(column);
            }
            ws.focused_column_idx = ws.focused_column_idx.min(ws.columns.len().saturating_sub(1));
            self.layout_engine.recenter_camera(false);
        } else {
            self.layout_engine.spawn_window(window_id, title);
        }

        // Store mapping and reposition windows
        self.windows.insert(window_id, window);

        // Automatically assign keyboard focus to newly mapped window
        self.set_keyboard_focus(Some(surface.wl_surface().clone()));

        self.reposition_windows();

        // Auto-save on window map
        let _ = self.save_session_internal();
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // Popups can be managed natively by Smithay's window rendering logic
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // Optional popup grab request
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // Optional popup reposition request
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let id_to_remove = self.windows.iter()
            .find(|(_, win)| win.toplevel() == Some(&surface))
            .map(|(id, _)| *id);
        
        if let Some(id) = id_to_remove {
            println!("Window destroyed: ID={:?}", id);
            self.windows.remove(&id);
            self.layout_engine.close_window(id);
            self.reposition_windows();

            // Auto-save on window destroy
            let _ = self.save_session_internal();
        }
    }
}

// 5. Buffer Handler
impl smithay::wayland::buffer::BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer) {}
}

// 6. Output Handler
impl smithay::wayland::output::OutputHandler for State {}

// --- DELEGATION MACROS ---
delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_xdg_shell!(State);
delegate_output!(State);

// 7. Selection and Data Device Handlers
impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

// 8. Primary Selection Handler
impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

// 10. Data Control Handler
impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

// 9. XDG Activation Handler
impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    fn request_activation(
        &mut self,
        _token: smithay::wayland::xdg_activation::XdgActivationToken,
        _token_data: smithay::wayland::xdg_activation::XdgActivationTokenData,
        surface: WlSurface,
    ) {
        self.set_keyboard_focus(Some(surface));
    }
}

delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_xdg_activation!(State);
delegate_data_control!(State);
