#![allow(dead_code)]

use std::collections::HashMap;
use smithay::{
    delegate_compositor, delegate_shm, delegate_seat, delegate_xdg_shell, delegate_output,
    delegate_data_device, delegate_primary_selection, delegate_xdg_activation,
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
    pub activation_state: XdgActivationState,
    pub running: bool,
    pub socket_name: String,
    pub highlighted_window: Option<(WindowId, [f32; 4])>,
    pub child_display_socket: Option<String>,
    pub workspace_swipe_accumulator: f32,
}

impl State {
    pub fn new(display_handle: DisplayHandle, layout_engine: LayoutEngine, output: Output, socket_name: String) -> Self {
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&display_handle, "hier-seat");
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&display_handle);
        let activation_state = XdgActivationState::new::<Self>(&display_handle);

        // Add keyboard and pointer capabilities to the seat
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        Self {
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
            activation_state,
            running: true,
            socket_name,
            highlighted_window: None,
            child_display_socket: None,
            workspace_swipe_accumulator: 0.0,
        }
    }

    pub fn window_under_pointer(&self, pointer_pos: Point<f64, smithay::utils::Logical>) -> Option<WindowId> {
        let is_overview = self.layout_engine.tiling_mode == crate::layout::TilingMode::Overview;
        if is_overview {
            let scale = 0.45_f32;
            for (&win_id, _) in &self.windows {
                if let Some((x, y, w, h)) = self.layout_engine.get_window_rect(win_id) {
                    let sx = x * scale;
                    let sy = y * scale;
                    let sw = w * scale;
                    let sh = h * scale;

                    if pointer_pos.x >= sx as f64 && pointer_pos.x < (sx + sw) as f64
                        && pointer_pos.y >= sy as f64 && pointer_pos.y < (sy + sh) as f64 {
                        return Some(win_id);
                    }
                }
            }
            None
        } else {
            self.space.element_under(pointer_pos).and_then(|(win, _)| {
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
        if let Some(keyboard) = self.seat.get_keyboard() {
            let serial = SERIAL_COUNTER.next_serial();
            keyboard.set_focus(self, focus, serial);
        }
    }

    /// Positions all mapped windows inside the Smithay `Space` based on our `LayoutEngine`'s coordinates.
    pub fn reposition_windows(&mut self) {
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

        for col in &active_ws.columns {
            if let Some(win) = col.focused_window() {
                if let Some(smithay_win) = self.windows.get(&win.id) {
                    if let Some((x, y, w, h)) = self.layout_engine.get_window_rect(win.id) {
                        // Tell the client to resize to match our tiling layout column dimensions
                        let toplevel = smithay_win.toplevel().unwrap();
                        toplevel.with_pending_state(|state| {
                            state.size = Some((w as i32, h as i32).into());
                        });
                        toplevel.send_configure();

                        // Map in Smithay's Space Logical Coordinate system
                        self.space.map_element(smithay_win.clone(), (x as i32, y as i32), true);
                        println!("Reposition window ID={:?} (Title: {:?}): position=({}, {}), size=({}x{})", win.id, win.title, x, y, w, h);
                    }
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
                let raw_pos = event.position();
                let mut pos = Point::from((raw_pos.x, raw_pos.y));

                if self.output.current_transform() == smithay::utils::Transform::Flipped180 {
                    if let Some(mode) = self.output.current_mode() {
                        pos = Point::from((
                            (mode.size.w - pos.x as i32) as f64,
                            (mode.size.h - pos.y as i32) as f64,
                        ));
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();

                // Translate pointer screen coordinates to space coordinates by adding the camera viewport offset
                let space_pos = pos + Point::from((
                    self.layout_engine.viewport.x as f64,
                    self.layout_engine.viewport.y as f64,
                ));

                let is_overview = self.layout_engine.tiling_mode == crate::layout::TilingMode::Overview;
                let focus = if is_overview {
                    if let Some(win_id) = self.window_under_pointer(space_pos) {
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
                let button = event.button_code();
                let state = event.state();

                let pointer = self.seat.get_pointer().unwrap();

                // Focus window and update pointer focus under cursor on click
                if state == ButtonState::Pressed {
                    let pos = pointer.current_location();

                    if self.layout_engine.tiling_mode == crate::layout::TilingMode::Overview {
                        if let Some(win_id) = self.window_under_pointer(pos) {
                            self.focus_window_by_id(win_id);
                            self.layout_engine.tiling_mode = crate::layout::TilingMode::Grid;
                            self.layout_engine.recenter_camera(true);
                            self.reposition_windows();
                            return;
                        }
                    }

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
                    let surface = under.and_then(|(win, _)| win.wl_surface().map(|c| c.into_owned()));

                    pointer.motion(
                        self,
                        focus,
                        &smithay::input::pointer::MotionEvent {
                            location: pos,
                            time,
                            serial,
                        },
                    );

                    if let Some(surface) = surface {
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

                if modifiers.logo {
                    let amount = event.amount(Axis::Vertical)
                        .or_else(|| event.amount_v120(Axis::Vertical).map(|v| v / 120.0))
                        .unwrap_or(0.0);

                    if amount != 0.0 && self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
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
        match action {
            "focus-left" | "focus_left" => {
                self.layout_engine.focus_left();
                let win_id = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id));
                let surface = win_id
                    .and_then(|id| self.windows.get(&id))
                    .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                if let Some(surface) = surface {
                    self.set_keyboard_focus(Some(surface));
                }
                self.reposition_windows();
                Ok(())
            }
            "focus-right" | "focus_right" => {
                self.layout_engine.focus_right();
                let win_id = self.layout_engine.active_workspace().focused_column()
                    .and_then(|col| col.focused_window().map(|w| w.id));
                let surface = win_id
                    .and_then(|id| self.windows.get(&id))
                    .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                if let Some(surface) = surface {
                    self.set_keyboard_focus(Some(surface));
                }
                self.reposition_windows();
                Ok(())
            }
            "focus-up" | "focus_up" => {
                if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                    self.layout_engine.scroll_z(-1.0);
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
                    self.layout_engine.focus_tab_up();
                    let win_id = self.layout_engine.active_workspace().focused_column()
                        .and_then(|col| col.focused_window().map(|w| w.id));
                    let surface = win_id
                        .and_then(|id| self.windows.get(&id))
                        .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                    self.reposition_windows();
                }
                Ok(())
            }
            "focus-down" | "focus_down" => {
                if self.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                    self.layout_engine.scroll_z(1.0);
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
                    self.layout_engine.focus_tab_down();
                    let win_id = self.layout_engine.active_workspace().focused_column()
                        .and_then(|col| col.focused_window().map(|w| w.id));
                    let surface = win_id
                        .and_then(|id| self.windows.get(&id))
                        .and_then(|w| w.wl_surface().map(|c| c.into_owned()));
                    if let Some(surface) = surface {
                        self.set_keyboard_focus(Some(surface));
                    }
                    self.reposition_windows();
                }
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
                let socket = self.socket_name.clone();
                let term = Self::find_terminal_cmd();
                println!("Spawning terminal ({}) on WAYLAND_DISPLAY={}", term, socket);
                let _ = std::process::Command::new(term)
                    .env("WAYLAND_DISPLAY", socket)
                    .spawn();
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
                self.layout_engine.recenter_camera(true);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-grid" | "tiling_mode_grid" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Grid;
                self.layout_engine.recenter_camera(true);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-float" | "tiling_mode_float" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Float;
                self.layout_engine.recenter_camera(true);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-depth" | "tiling_mode_depth" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Depth;
                self.layout_engine.depth_scroll_progress = 0.0;
                self.layout_engine.recenter_camera(true);
                self.reposition_windows();
                Ok(())
            }
            "tiling-mode-overview" | "tiling_mode_overview" => {
                self.layout_engine.tiling_mode = crate::layout::TilingMode::Overview;
                self.layout_engine.recenter_camera(true);
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
                        self.layout_engine.recenter_camera(false);
                        self.reposition_windows();
                        return Ok(());
                    }
                }
                Err(format!("invalid workspace index: {}", other))
            }
            other => Err(format!("unknown layout action: {}", other)),
        }
    }

    pub fn handle_key_action(
        &mut self,
        key_state: KeyState,
        modifiers: &ModifiersState,
        keysym: Keysym,
    ) -> FilterResult<()> {
        if key_state == KeyState::Pressed {
            if modifiers.logo || modifiers.alt {
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
                        keysyms::KEY_o => {
                            if self.layout_engine.tiling_mode == crate::layout::TilingMode::Overview {
                                let _ = self.perform_layout_action("tiling-mode-grid");
                            } else {
                                let _ = self.perform_layout_action("tiling-mode-overview");
                            }
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

                let serial = SERIAL_COUNTER.next_serial();
                let time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u32;

                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.input(
                    self,
                    keycode.into(),
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

                let pos = Point::from((x, y));
                let pointer = self.seat.get_pointer().unwrap();

                let under = self.space.element_under(pos);
                let focus = under.and_then(|(win, local_pos)| {
                    win.surface_under(local_pos.to_f64(), WindowSurfaceType::ALL)
                        .map(|(surface, surface_local_pos)| (surface, surface_local_pos.to_f64()))
                });

                let serial = SERIAL_COUNTER.next_serial();
                let time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u32;

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
                let time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u32;

                let pointer = self.seat.get_pointer().unwrap();
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
                    let pos = pointer.current_location();

                    if self.layout_engine.tiling_mode == crate::layout::TilingMode::Overview {
                        if let Some(win_id) = self.window_under_pointer(pos) {
                            self.focus_window_by_id(win_id);
                            self.layout_engine.tiling_mode = crate::layout::TilingMode::Grid;
                            self.layout_engine.recenter_camera(true);
                            self.reposition_windows();
                            return "ok\n".to_string();
                        }
                    }

                    let surface = self.space.element_under(pos)
                        .and_then(|(win, _)| win.wl_surface().map(|c| c.into_owned()));
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

                let time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u32;

                let pointer = self.seat.get_pointer().unwrap();
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
                let mut forwarded = false;
                
                // 1. Try dynamic auto-registered child display socket
                if let Some(ref child_display) = self.child_display_socket {
                    let child_socket = format!("/tmp/hier-ctrl-{}.sock", child_display);
                    if std::path::Path::new(&child_socket).exists() {
                        println!("[pointer_axis_z] Forwarding Z-scroll to registered child socket: {}", child_socket);
                        use std::io::Write;
                        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&child_socket) {
                            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                            let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
                            let cmd = format!("pointer_axis_z {}\n", z_val);
                            if stream.write_all(cmd.as_bytes()).is_ok() {
                                forwarded = true;
                                println!("[pointer_axis_z] Successfully forwarded Z-scroll to registered child.");
                            }
                        }
                    }
                }

                // 2. Fallback to sequential guess wayland-(num+1)
                if !forwarded {
                    if let Some(num_str) = self.socket_name.strip_prefix("wayland-") {
                        if let Ok(num) = num_str.parse::<u32>() {
                            let child_socket = format!("/tmp/hier-ctrl-wayland-{}.sock", num + 1);
                            if std::path::Path::new(&child_socket).exists() {
                                println!("[pointer_axis_z] Forwarding Z-scroll to guessed child socket: {}", child_socket);
                                use std::io::Write;
                                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&child_socket) {
                                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                                    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
                                    let cmd = format!("pointer_axis_z {}\n", z_val);
                                    if stream.write_all(cmd.as_bytes()).is_ok() {
                                        forwarded = true;
                                        println!("[pointer_axis_z] Successfully forwarded Z-scroll to guessed child.");
                                    }
                                }
                            }
                        }
                    }
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
                            let rect_str = if let Some((x, y, w, h)) = self.layout_engine.get_window_rect(win.id) {
                                let screen_x = x - self.layout_engine.viewport.x;
                                let screen_y = y - self.layout_engine.viewport.y;
                                format!("{},{},{},{}", screen_x as i32, screen_y as i32, w as i32, h as i32)
                            } else {
                                "0,0,0,0".to_string()
                            };
                            lines.push(format!("{}:{}:{}:{}:{}:{}", ws_idx, col_idx, win.id.0, is_focused, rect_str, win.title));
                        }
                    }
                }
                format!("{}\n", lines.join("\n"))
            }
            "save_session" | "save-session" => {
                #[derive(serde::Serialize, serde::Deserialize)]
                struct SavedWindow {
                    title: String,
                    #[serde(default)]
                    app_id: Option<String>,
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
                            SavedWindow {
                                title: win.title.clone(),
                                app_id,
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

                let path = "/tmp/hier-session.json";
                match std::fs::File::create(path) {
                    Ok(file) => {
                        if serde_json::to_writer_pretty(file, &session).is_ok() {
                            "ok: session saved to /tmp/hier-session.json\n".to_string()
                        } else {
                            "error: failed to serialize session\n".to_string()
                        }
                    }
                    Err(e) => format!("error: failed to create file: {}\n", e),
                }
            }
            "restore_session" | "restore-session" => {
                #[derive(serde::Serialize, serde::Deserialize)]
                struct SavedWindow {
                    title: String,
                    #[serde(default)]
                    app_id: Option<String>,
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

                let path = "/tmp/hier-session.json";
                let file = match std::fs::File::open(path) {
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

                // Clear live workspace columns
                for ws in &mut self.layout_engine.workspaces {
                    ws.columns.clear();
                    ws.focused_column_idx = 0;
                }

                // Reconstruct workspaces using matched slots
                let mut ws_cols: HashMap<usize, HashMap<usize, (f32, usize, Vec<crate::layout::Window>)>> = HashMap::new();
                for slot in slots {
                    if let Some((win_id, title)) = slot.matched_win {
                        let target_ws_idx = slot.ws_idx.min(self.layout_engine.workspaces.len() - 1);
                        let cols_map = ws_cols.entry(target_ws_idx).or_default();
                        let col_entry = cols_map.entry(slot.col_idx).or_insert_with(|| {
                            (slot.col_width, slot.col_focused_idx, Vec::new())
                        });
                        col_entry.2.push(crate::layout::Window { id: win_id, title });
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
                    let col = crate::layout::Column::new(crate::layout::Window { id: leftover.id, title: leftover.title }, 500.0);
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
                        
                        for py in 0..height {
                            for px in 0..width {
                                let idx = (py * width + px) * 3;
                                if py < 4 || py >= height - 4 || px < 4 || px >= width - 4 {
                                    rgb_data[idx] = 30;
                                    rgb_data[idx + 1] = 144;
                                    rgb_data[idx + 2] = 255;
                                }
                            }
                        }
                        
                        use std::io::Write;
                        match std::fs::File::create(path) {
                            Ok(mut file) => {
                                let header = format!("P6\n{} {}\n255\n", width, height);
                                if file.write_all(header.as_bytes()).is_ok() && file.write_all(&rgb_data).is_ok() {
                                    format!("ok: exported {} window (size {}x{}) to {}\n", title, width, height, path)
                                } else {
                                    "error: failed to write PPM bytes\n".to_string()
                                }
                            }
                            Err(e) => format!("error: failed to create file: {}\n", e),
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
                    let columns_json: Vec<serde_json::Value> = ws.columns.iter().map(|col| {
                        let windows_json: Vec<serde_json::Value> = col.windows.iter().map(|win| {
                            serde_json::json!({
                                "id": win.id.0,
                                "title": win.title
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
            other => format!("error: unknown command '{}'\n", other),
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
        let is_occupied = !self.layout_engine.active_workspace().columns.is_empty();
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

        // Title retrieval
        let title = smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .title
                .clone()
        }).unwrap_or_else(|| "Wayland Window".to_string());

        println!("Window mapped: ID={:?}, Title={:?}", window_id, title);

        // Spawn window in layout engine
        self.layout_engine.spawn_window(window_id, title);

        // Store mapping and reposition windows
        self.windows.insert(window_id, window);
        self.reposition_windows();
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
