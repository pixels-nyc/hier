#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use std::process::Command;
use serde_json::Value;
use smithay::{
    backend::winit::{self, WinitEvent},
    reexports::calloop::{EventLoop, Interest, Mode},
    reexports::calloop::generic::Generic,
    reexports::wayland_server::Display,
    output::{Output, PhysicalProperties, Mode as OutputMode, Scale, Subpixel},
    utils::Transform,
};
use crate::state::{State, ClientState};

smithay::backend::renderer::element::render_elements! {
    pub MyRenderElement<=smithay::backend::renderer::gles::GlesRenderer>;
    Space = smithay::desktop::space::SpaceRenderElements<smithay::backend::renderer::gles::GlesRenderer, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<smithay::backend::renderer::gles::GlesRenderer>>,
    Solid = smithay::backend::renderer::element::solid::SolidColorRenderElement,
    Surface = smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<smithay::backend::renderer::gles::GlesRenderer>,
}

fn create_clipboard(display: &str) -> Option<arboard::Clipboard> {
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_MUTEX.lock().unwrap();

    let old_display = std::env::var("WAYLAND_DISPLAY").ok();
    unsafe { std::env::set_var("WAYLAND_DISPLAY", display); }
    let clipboard = arboard::Clipboard::new();
    if let Some(old) = old_display {
        unsafe { std::env::set_var("WAYLAND_DISPLAY", old); }
    } else {
        unsafe { std::env::remove_var("WAYLAND_DISPLAY"); }
    }

    match clipboard {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("[Clipboard Sync] Failed to connect to display {}: {}", display, e);
            None
        }
    }
}

pub fn detect_host_transform() -> Transform {
    if let Ok(val) = std::env::var("HIER_HOST_TRANSFORM") {
        return match val.as_str() {
            "180" | "Flipped180" => Transform::Normal,
            _ => Transform::Flipped180,
        };
    }
    let pid = std::process::id() as i64;
    
    // 1. Get workspace_id of our window
    let workspace_id = match Command::new("niri")
        .args(&["msg", "--json", "windows"])
        .output()
    {
        Ok(output) => {
            if let Ok(val) = serde_json::from_slice::<Value>(&output.stdout) {
                if let Some(arr) = val.as_array() {
                    arr.iter()
                        .find(|win| win["pid"].as_i64() == Some(pid) || win["title"].as_str() == Some("Smithay"))
                        .and_then(|win| win["workspace_id"].as_i64())
                } else {
                    None
                }
            } else {
                None
            }
        }
        Err(_) => None,
    };
    
    let workspace_id = match workspace_id {
        Some(id) => id,
        None => return Transform::Flipped180,
    };
    
    // 2. Get output name of our workspace
    let output_name = match Command::new("niri")
        .args(&["msg", "--json", "workspaces"])
        .output()
    {
        Ok(output) => {
            if let Ok(val) = serde_json::from_slice::<Value>(&output.stdout) {
                if let Some(arr) = val.as_array() {
                    arr.iter()
                        .find(|ws| ws["id"].as_i64() == Some(workspace_id))
                        .and_then(|ws| ws["output"].as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        }
        Err(_) => None,
    };
    
    let output_name = match output_name {
        Some(name) => name,
        None => return Transform::Flipped180,
    };
    
    // 3. Get transform of our output
    let transform_str = match Command::new("niri")
        .args(&["msg", "--json", "outputs"])
        .output()
    {
        Ok(output) => {
            if let Ok(val) = serde_json::from_slice::<Value>(&output.stdout) {
                val.get(&output_name)
                    .and_then(|out| out.get("logical"))
                    .and_then(|log| log.get("transform"))
                    .and_then(|trans| trans.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        }
        Err(_) => None,
    };
    
    match transform_str.as_deref() {
        Some("180") => Transform::Normal,
        _ => Transform::Flipped180,
    }
}

pub fn run_winit_compositor(sandbox: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = Display::<State>::new()?;
    let display_handle = display.handle();

    let mut event_loop = EventLoop::<State>::try_new()?;
    let loop_handle = event_loop.handle();

    let (backend, winit_event_loop) = winit::init::<smithay::backend::renderer::gles::GlesRenderer>()?;
    let backend = Rc::new(RefCell::new(backend));

    // Determine initial window size, applying fullscreen if requested
    if std::env::var("HIER_FULLSCREEN").is_ok() {
        backend.borrow().window().set_fullscreen(Some(::winit::window::Fullscreen::Borderless(None)));
    }
    // After any fullscreen change, fetch the (potentially updated) inner size
    let size = backend.borrow().window().inner_size();
    let layout_engine = crate::layout::LayoutEngine::new(
        size.width as f32,
        size.height as f32,
        10.0, // gap
        0.0,  // outer margin
        5,    // workspaces
    );

    // Create the Output
    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Winit".to_string(),
            model: "Winit".to_string(),
        },
    );
    output.create_global::<State>(&display_handle);
    
    let initial_transform = detect_host_transform();
    println!("Detected host transform: {:?}", initial_transform);

    output.change_current_state(
        Some(OutputMode {
            size: (size.width as i32, size.height as i32).into(),
            refresh: 60000,
        }),
        Some(initial_transform),
        Some(Scale::Integer(1)),
        Some((0, 0).into()),
    );

    // Register Listening Socket
    let (socket_name, socket_source_opt) = if sandbox {
        ("sandbox".to_string(), None)
    } else {
        use smithay::wayland::socket::ListeningSocketSource;
        let socket = ListeningSocketSource::new_auto()?;
        let socket_name = socket.socket_name().to_string_lossy().into_owned();
        (socket_name, Some(socket))
    };
    if !sandbox {
        println!("--------------------------------------------------");
        println!("Compositor started!");
        println!("WAYLAND_DISPLAY={}", socket_name);
        println!("To launch clients in nested window, run:");
        println!("  export WAYLAND_DISPLAY={}", socket_name);
        println!("  alacritty # or any Wayland client");
        println!("--------------------------------------------------");
    } else {
        println!("--------------------------------------------------");
        println!("Compositor started in Sandbox Mode!");
        println!("No Wayland socket will be initialized.");
        println!("--------------------------------------------------");
    }

    let mut state = State::new(display_handle, layout_engine, output.clone(), socket_name.clone(), sandbox);
    state.space.map_output(&output, (0, 0));

    let host_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    if !sandbox && !host_display.is_empty() {
        let child_display = socket_name.clone();
        std::thread::spawn(move || {
            // Sleep to let compositor startup finish and sockets bind properly
            std::thread::sleep(std::time::Duration::from_millis(1500));

            let mut host_clip = match create_clipboard(&host_display) {
                Some(c) => c,
                None => {
                    eprintln!("[Clipboard Sync] Host clipboard connection failed. Sync disabled.");
                    return;
                }
            };
            let mut child_clip = match create_clipboard(&child_display) {
                Some(c) => c,
                None => {
                    eprintln!("[Clipboard Sync] Child clipboard connection failed. Sync disabled.");
                    return;
                }
            };

            println!("[Clipboard Sync] Started: host ({}) <-> child ({})", host_display, child_display);

            let mut last_text = String::new();

            // Prime last_text from host clipboard if possible
            if let Ok(text) = host_clip.get_text() {
                last_text = text;
            }

            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));

                // Read from host
                if let Ok(host_text) = host_clip.get_text() {
                    if host_text != last_text {
                        println!("[Clipboard Sync] Host -> Child text update");
                        if child_clip.set_text(host_text.clone()).is_ok() {
                            last_text = host_text;
                            continue;
                        }
                    }
                }

                // Read from child
                if let Ok(child_text) = child_clip.get_text() {
                    if child_text != last_text {
                        println!("[Clipboard Sync] Child -> Host text update");
                        if host_clip.set_text(child_text.clone()).is_ok() {
                            last_text = child_text;
                        }
                    }
                }
            }
        });
    }

    // Create Control Unix Domain Socket Listener
    let ctrl_socket_path = std::env::var("HIER_CTRL_SOCKET")
        .unwrap_or_else(|_| format!("/tmp/hier-ctrl-{}.sock", socket_name));
    let _ = std::fs::remove_file(&ctrl_socket_path);
    let ctrl_listener = std::os::unix::net::UnixListener::bind(&ctrl_socket_path)?;
    
    // Restrict socket file permissions to owner-only read/write (0o600) to prevent unauthorized access by other local users
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(&ctrl_socket_path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(&ctrl_socket_path, perms);
    }
    
    ctrl_listener.set_nonblocking(true)?;

    println!("--------------------------------------------------");
    println!("Control socket listening at: {}", ctrl_socket_path);
    println!("--------------------------------------------------");

    // Register this nested compositor with its parent compositor if running inside one
    if !sandbox && std::env::var("WAYLAND_DISPLAY").is_ok() {
        let parent_display = std::env::var("WAYLAND_DISPLAY").unwrap();
        let parent_ctrl_socket = format!("/tmp/hier-ctrl-{}.sock", parent_display);
        println!("Checking parent control socket for auto-registration: {}", parent_ctrl_socket);
        if std::path::Path::new(&parent_ctrl_socket).exists() {
            println!("Parent control socket found. Attempting auto-registration...");
            match std::os::unix::net::UnixStream::connect(&parent_ctrl_socket) {
                Ok(mut stream) => {
                    use std::io::Write;
                    let msg = format!("register_child_display {}\n", socket_name);
                    if stream.write_all(msg.as_bytes()).is_ok() && stream.flush().is_ok() {
                        println!("✅ Auto-registered with parent display {}", parent_display);
                    } else {
                        eprintln!("⚠️ Failed to write registration message to parent");
                    }
                }
                Err(e) => {
                    eprintln!("⚠️ Failed to connect to parent control socket: {}", e);
                }
            }
        }
    }

    let ctrl_source = Generic::new(ctrl_listener, Interest::READ, Mode::Level);
    let loop_handle_clone = loop_handle.clone();
    loop_handle.insert_source(ctrl_source, move |readiness, listener, _state| {
        if readiness.readable {
            while let Ok((stream, _)) = listener.accept() {
                let _ = stream.set_nonblocking(true);
                let stream_source = Generic::new(stream, Interest::READ, Mode::Level);
                let mut buffer = Vec::new();
                let _ = loop_handle_clone.insert_source(
                    stream_source,
                    move |stream_readiness, stream, state| {
                        use std::io::{Read, Write};
                        if stream_readiness.readable {
                            let mut temp_buf = [0u8; 1024];
                            match (&**stream).read(&mut temp_buf) {
                                Ok(0) => {
                                    Ok::<calloop::PostAction, std::io::Error>(calloop::PostAction::Remove)
                                }
                                Ok(n) => {
                                    buffer.extend_from_slice(&temp_buf[..n]);
                                    loop {
                                        if buffer.is_empty() {
                                            break;
                                        }
                                        if buffer.starts_with(b"HIER") {
                                            if buffer.len() < 5 {
                                                break;
                                            }
                                            let msg_type = buffer[4];
                                            let payload_len = match msg_type {
                                                1 => 5,   // keyboard_key
                                                2 => 16,  // pointer_motion
                                                3 => 16,  // pointer_motion_local
                                                4 => 5,   // pointer_button
                                                5 => 16,  // pointer_axis
                                                6 => 8,   // pointer_axis_z
                                                7 => 16,  // pointer_gesture_swipe
                                                8 => 0,   // pointer_gesture_swipe_end
                                                _ => {
                                                    let _ = buffer.drain(..5);
                                                    let _ = (&**stream).write_all(b"error: unknown binary message type\n");
                                                    let _ = (&**stream).flush();
                                                    continue;
                                                }
                                            };
                                            let payload_end = 5 + payload_len;
                                            if buffer.len() < payload_end {
                                                break;
                                            }
                                            let response = state.handle_simulated_binary_input(msg_type, &buffer[5..payload_end]);
                                            let _ = (&**stream).write_all(response.as_bytes());
                                            let _ = (&**stream).flush();
                                            let _ = buffer.drain(..payload_end);
                                        } else {
                                            if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                                                let line_bytes: Vec<u8> = buffer.drain(..=pos).collect();
                                                if let Ok(line_str) = String::from_utf8(line_bytes) {
                                                    let line_trimmed = line_str.trim();
                                                    if !line_trimmed.is_empty() {
                                                        let response = state.handle_simulated_input(line_trimmed);
                                                        let _ = (&**stream).write_all(response.as_bytes());
                                                        let _ = (&**stream).flush();
                                                    }
                                                }
                                            } else {
                                                break;
                                            }
                                        }
                                    }
                                    Ok::<calloop::PostAction, std::io::Error>(calloop::PostAction::Continue)
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    Ok::<calloop::PostAction, std::io::Error>(calloop::PostAction::Continue)
                                }
                                Err(_) => {
                                    Ok::<calloop::PostAction, std::io::Error>(calloop::PostAction::Remove)
                                }
                            }
                        } else {
                            Ok::<calloop::PostAction, std::io::Error>(calloop::PostAction::Continue)
                        }
                    },
                );
            }
        }
        Ok::<calloop::PostAction, std::io::Error>(calloop::PostAction::Continue)
    })?;

    if let Some(socket_source) = socket_source_opt {
        loop_handle.insert_source(socket_source, move |client_stream, _metadata, state| {
            state.display_handle.insert_client(client_stream, std::sync::Arc::new(ClientState {
                compositor_state: smithay::wayland::compositor::CompositorClientState::default(),
            })).unwrap();
        })?;
    }

    if !sandbox {
        // Register display's poll FD to wake up calloop when client events arrive
        let display_fd = display.backend().poll_fd().try_clone_to_owned()?;
        loop_handle.insert_source(
            Generic::new(display_fd, Interest::READ, Mode::Level),
            |_, _, _| {
                // Callback does nothing; event loop returns immediately, and then
                // we dispatch client requests in the main loop tick.
                Ok(calloop::PostAction::Continue)
            },
        )?;
    }

    let mut damage_tracker = smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);

    // Register Winit Event Loop
    let backend_clone = backend.clone();
    let mut fullscreen_attempts = 0;
    loop_handle.insert_source(winit_event_loop, move |event, _, state| {
        match event {
            WinitEvent::Resized { size, .. } => {
                state.layout_engine.resize_viewport(size.w as f32, size.h as f32);
                let current_transform = state.output.current_transform();
                state.output.change_current_state(
                    Some(OutputMode {
                        size: (size.w as i32, size.h as i32).into(),
                        refresh: 60000,
                    }),
                    Some(current_transform),
                    None,
                    None,
                );
                state.reposition_windows();
            }
            WinitEvent::Input(event) => {
                state.process_input(event);
            }
            WinitEvent::Redraw => {
                if std::env::var("HIER_FULLSCREEN").is_ok() && fullscreen_attempts < 5 {
                    backend_clone.borrow().window().set_fullscreen(Some(::winit::window::Fullscreen::Borderless(None)));
                    fullscreen_attempts += 1;
                }
                let mut backend = backend_clone.borrow_mut();
                let age = backend.buffer_age().unwrap_or(0);
                
                // Position windows dynamically prior to rendering
                state.reposition_windows();

                let damage = {
                    let (renderer, mut framebuffer) = backend.bind().unwrap();
                    
                    let space_elements = smithay::desktop::space::space_render_elements(
                        renderer,
                        std::iter::once(&state.space),
                        &state.output,
                        1.0f32,
                    ).expect("failed to get space render elements");

                    println!("DEBUG RENDER: space_elements len = {}", space_elements.len());

                    let mut render_elements = Vec::new();
                    // 1. Add borders first to draw on top of windows (since front-to-back index 0 is front)
                    let current_scale = state.layout_engine.current_overview_scale;
                    let is_scaled = state.layout_engine.overview_open
                        || (current_scale - 1.0).abs() > 1e-3;

                    for (ws_idx, ws) in state.layout_engine.workspaces.iter().enumerate() {
                        for (col_idx, col) in ws.columns.iter().enumerate() {
                            for (win_idx, win) in col.windows.iter().enumerate() {
                                // Determine visibility
                                let is_visible = if is_scaled {
                                    true
                                } else if state.layout_engine.active_workspace_idx == ws_idx {
                                    if state.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                        true
                                    } else {
                                        col.focused_window_idx == win_idx
                                    }
                                } else {
                                    false
                                };

                                if !is_visible {
                                    continue;
                                }

                                // Determine color
                                let is_focused = state.layout_engine.active_workspace_idx == ws_idx
                                    && ws.focused_column_idx == col_idx
                                    && col.focused_window_idx == win_idx;

                                let color_arr = if let Some((h_id, h_color)) = state.highlighted_window {
                                    if h_id == win.id {
                                        h_color
                                    } else if is_focused {
                                        [0.23f32, 0.51f32, 0.96f32, 1.0f32] // Active focused
                                    } else {
                                        [0.2f32, 0.23f32, 0.27f32, 0.8f32] // Inactive border
                                    }
                                } else if is_focused {
                                    [0.23f32, 0.51f32, 0.96f32, 1.0f32] // Active focused
                                } else {
                                    [0.2f32, 0.23f32, 0.27f32, 0.8f32] // Inactive border
                                };

                                let rect = if is_scaled {
                                    if let Some((nx, ny, nw, nh)) = state.layout_engine.get_window_anim_or_target_for_mode(win.id, &state.layout_engine.underlying_tiling_mode) {
                                        let ws_y = ws_idx as f32 * state.layout_engine.viewport.height;
                                        let x_local = nx;
                                        let y_local = ny - ws_y;
                                        let is_overlay = col.is_overlay();
                                        Some(state.layout_engine.project_rect(x_local, y_local, nw, nh, ws_idx, current_scale, is_overlay))
                                    } else {
                                        None
                                    }
                                } else if state.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                    let transforms = state.layout_engine.depth_transforms();
                                    if let Some((_, transform)) = transforms.iter().find(|(w_id, _)| *w_id == win.id) {
                                        if let Some((x, y, w, h)) = state.layout_engine.get_window_anim_or_target(win.id) {
                                            let scaled_w = w * transform.scale;
                                            let scaled_h = h * transform.scale;
                                            let x_offset = (w - scaled_w) / 2.0;
                                            let y_offset = (h - scaled_h) / 2.0 + (transform.y_offset as f32);
                                            Some((x + x_offset, y + y_offset, scaled_w, scaled_h))
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    state.layout_engine.get_window_anim_or_target(win.id)
                                };

                                if let Some((x, y, w, h)) = rect {
                                    let mut color = smithay::backend::renderer::Color32F::from(color_arr);
                                    
                                    // Scale opacity down for Depth mode background cards
                                    if !is_scaled && state.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                                        let transforms = state.layout_engine.depth_transforms();
                                        if let Some((_, transform)) = transforms.iter().find(|(w_id, _)| *w_id == win.id) {
                                            color = smithay::backend::renderer::Color32F::from([
                                                color_arr[0] * transform.opacity,
                                                color_arr[1] * transform.opacity,
                                                color_arr[2] * transform.opacity,
                                                color_arr[3] * transform.opacity,
                                            ]);
                                        }
                                    }

                                    let border_thickness = 4;
                                    let scale_factor = state.output.current_scale().fractional_scale();
                                    
                                    let (px, py) = if is_scaled {
                                        ((x as f64 * scale_factor) as i32, (y as f64 * scale_factor) as i32)
                                    } else {
                                        (((x - state.layout_engine.viewport.x) as f64 * scale_factor) as i32,
                                         ((y - state.layout_engine.viewport.y) as f64 * scale_factor) as i32)
                                    };
                                    let pw = (w as f64 * scale_factor) as i32;
                                    let ph = (h as f64 * scale_factor) as i32;
                                    let pb = (border_thickness as f64 * scale_factor) as i32;
                                    
                                    use smithay::backend::renderer::element::solid::SolidColorRenderElement;
                                    use smithay::utils::{Rectangle, Point, Size};
                                    use smithay::backend::renderer::element::Kind;
                                    use smithay::backend::renderer::utils::CommitCounter;

                                    let id_top = smithay::backend::renderer::element::Id::new();
                                    let id_bottom = smithay::backend::renderer::element::Id::new();
                                    let id_left = smithay::backend::renderer::element::Id::new();
                                    let id_right = smithay::backend::renderer::element::Id::new();

                                    // Top border
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_top,
                                        Rectangle::new(Point::from((px - pb, py - pb)), Size::from((pw + 2 * pb, pb))),
                                        CommitCounter::default(),
                                        color,
                                        Kind::Unspecified,
                                    )));

                                    // Bottom border
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_bottom,
                                        Rectangle::new(Point::from((px - pb, py + ph)), Size::from((pw + 2 * pb, pb))),
                                        CommitCounter::default(),
                                        color,
                                        Kind::Unspecified,
                                    )));

                                    // Left border
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_left,
                                        Rectangle::new(Point::from((px - pb, py)), Size::from((pb, ph))),
                                        CommitCounter::default(),
                                        color,
                                        Kind::Unspecified,
                                    )));

                                    // Right border
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_right,
                                        Rectangle::new(Point::from((px + pw, py)), Size::from((pb, ph))),
                                        CommitCounter::default(),
                                        color,
                                        Kind::Unspecified,
                                    )));
                                }

                                // Draw Tab Indicators for Tabbed Columns (only on standard/overview mode, not depth mode)
                                if col.is_tabbed() && col.focused_window_idx == win_idx && state.layout_engine.tiling_mode != crate::layout::TilingMode::Depth {
                                    let col_geom = if is_scaled {
                                        if let Some((nx, ny, nw, nh)) = state.layout_engine.get_window_anim_or_target_for_mode(win.id, &state.layout_engine.underlying_tiling_mode) {
                                            let ws_y = ws_idx as f32 * state.layout_engine.viewport.height;
                                            let x_local = nx;
                                            let y_local = ny - ws_y;
                                            let is_overlay = col.is_overlay();
                                            Some(state.layout_engine.project_rect(x_local, y_local, nw, nh, ws_idx, current_scale, is_overlay))
                                        } else {
                                            None
                                        }
                                    } else {
                                        state.layout_engine.get_window_anim_or_target(win.id)
                                    };

                                    if let Some((col_x, col_y, col_w, _col_h)) = col_geom {
                                        let num_tabs = col.windows.len();
                                        let tab_gap = 4.0f32;
                                        let total_gap_w = (num_tabs - 1) as f32 * tab_gap;
                                        let single_tab_w = (col_w - total_gap_w) / num_tabs as f32;
                                        let tab_h = 4.0f32;
                                        
                                        // Draw tabs 8px above the window top boundary
                                        let scale_factor = state.output.current_scale().fractional_scale();
                                        let base_y = col_y - 8.0f32;

                                        for i in 0..num_tabs {
                                            let tab_x = col_x + (i as f32) * (single_tab_w + tab_gap);
                                            let is_active_tab = col.focused_window_idx == i;
                                            let tab_color_arr = if is_active_tab {
                                                [0.23f32, 0.51f32, 0.96f32, 1.0f32] // Active tab
                                            } else {
                                                [0.2f32, 0.23f32, 0.27f32, 0.5f32]  // Inactive tab
                                            };

                                            let t_color = smithay::backend::renderer::Color32F::from(tab_color_arr);

                                            let (px, py) = if is_scaled {
                                                ((tab_x as f64 * scale_factor) as i32, (base_y as f64 * scale_factor) as i32)
                                            } else {
                                                (((tab_x - state.layout_engine.viewport.x) as f64 * scale_factor) as i32,
                                                 ((base_y - state.layout_engine.viewport.y) as f64 * scale_factor) as i32)
                                            };
                                            let pw = (single_tab_w as f64 * scale_factor) as i32;
                                            let ph = (tab_h as f64 * scale_factor) as i32;

                                            use smithay::backend::renderer::element::solid::SolidColorRenderElement;
                                            use smithay::utils::{Rectangle, Point, Size};
                                            use smithay::backend::renderer::element::Kind;
                                            use smithay::backend::renderer::utils::CommitCounter;

                                            let id_tab = smithay::backend::renderer::element::Id::new();
                                            render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                                id_tab,
                                                Rectangle::new(Point::from((px, py)), Size::from((pw, ph))),
                                                CommitCounter::default(),
                                                t_color,
                                                Kind::Unspecified,
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if state.sandbox {
                        // Drawing mock windows in sandbox mode
                        let scale_factor = state.output.current_scale().fractional_scale();
                        
                        fn get_window_color(id: crate::layout::WindowId) -> [f32; 4] {
                            match id.0 % 4 {
                                0 => [0.15, 0.64, 0.41, 1.0], // Teal/Green
                                1 => [0.88, 0.11, 0.14, 1.0], // Red/Orange
                                2 => [0.12, 0.47, 0.81, 1.0], // Blue
                                _ => [0.55, 0.25, 0.70, 1.0], // Purple
                            }
                        }

                        use smithay::backend::renderer::element::solid::SolidColorRenderElement;
                        use smithay::utils::{Rectangle, Point, Size};
                        use smithay::backend::renderer::element::Kind;
                        use smithay::backend::renderer::utils::CommitCounter;

                        if state.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                            let transforms = state.layout_engine.depth_transforms();
                            for (win_id, transform) in transforms.into_iter().rev() {
                                if let Some((x, y, w, h)) = state.layout_engine.get_window_anim_or_target(win_id) {
                                    let scaled_w = w * transform.scale;
                                    let scaled_h = h * transform.scale;
                                    let x_offset = (w - scaled_w) / 2.0;
                                    let y_offset = (h - scaled_h) / 2.0 + (transform.y_offset as f32);
                                    
                                    let px = (((x + x_offset) - state.layout_engine.viewport.x) as f64 * scale_factor) as i32;
                                    let py = (((y + y_offset) - state.layout_engine.viewport.y) as f64 * scale_factor) as i32;
                                    let pw = (scaled_w as f64 * scale_factor) as i32;
                                    let ph = (scaled_h as f64 * scale_factor) as i32;

                                    let mut color_arr = get_window_color(win_id);
                                    let opacity = transform.opacity;
                                    color_arr[0] *= opacity;
                                    color_arr[1] *= opacity;
                                    color_arr[2] *= opacity;
                                    color_arr[3] *= opacity;

                                    let id_window = smithay::backend::renderer::element::Id::new();
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_window,
                                        Rectangle::new(Point::from((px, py)), Size::from((pw, ph))),
                                        CommitCounter::default(),
                                        smithay::backend::renderer::Color32F::from(color_arr),
                                        Kind::Unspecified,
                                    )));
                                }
                            }
                        } else if state.layout_engine.overview_open
                            || (state.layout_engine.current_overview_scale - 1.0).abs() > 1e-3
                        {
                            let current_scale = state.layout_engine.current_overview_scale;

                            // Skip workspace separator lines drawing to match Niri’s backdrop layout style

                            // Draw mock windows inside overview
                            if state.layout_engine.underlying_tiling_mode == crate::layout::TilingMode::Depth {
                                let transforms = state.layout_engine.depth_transforms();
                                for (win_id, transform) in transforms.into_iter().rev() {
                                    if let Some((nx, ny, nw, nh)) = state.layout_engine.get_window_anim_or_target_for_mode(win_id, &state.layout_engine.underlying_tiling_mode) {
                                        let ws_idx = state.layout_engine.workspaces.iter().position(|ws| ws.find_window(win_id).is_some()).unwrap();
                                        let ws_y = ws_idx as f32 * state.layout_engine.viewport.height;
                                        
                                        let scaled_w = nw * transform.scale;
                                        let scaled_h = nh * transform.scale;
                                        let x_offset = (nw - scaled_w) / 2.0;
                                        let y_offset = (nh - scaled_h) / 2.0 + (transform.y_offset as f32);

                                        let x_local = nx + x_offset;
                                        let y_local = ny - ws_y + y_offset;
                                        let col = &state.layout_engine.workspaces[ws_idx].columns[state.layout_engine.workspaces[ws_idx].find_window(win_id).unwrap().0];
                                        let is_overlay = col.is_overlay();

                                        let (sx, sy, sw, sh) = state.layout_engine.project_rect(x_local, y_local, scaled_w, scaled_h, ws_idx, current_scale, is_overlay);

                                        let px = (sx as f64 * scale_factor) as i32;
                                        let py = (sy as f64 * scale_factor) as i32;
                                        let pw = (sw as f64 * scale_factor) as i32;
                                        let ph = (sh as f64 * scale_factor) as i32;

                                        let mut color_arr = get_window_color(win_id);


                                        let opacity = transform.opacity;


                                        color_arr[0] *= opacity;


                                        color_arr[1] *= opacity;


                                        color_arr[2] *= opacity;


                                        color_arr[3] *= opacity;

                                        let id_window = smithay::backend::renderer::element::Id::new();
                                        render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                            id_window,
                                            Rectangle::new(Point::from((px, py)), Size::from((pw, ph))),
                                            CommitCounter::default(),
                                            smithay::backend::renderer::Color32F::from(color_arr),
                                            Kind::Unspecified,
                                        )));
                                    }
                                }
                            } else {
                                for &win_id in &state.layout_engine.windows {
                                    if let Some((nx, ny, nw, nh)) = state.layout_engine.get_window_anim_or_target_for_mode(win_id, &state.layout_engine.underlying_tiling_mode) {
                                        let ws_idx = state.layout_engine.workspaces.iter().position(|ws| ws.find_window(win_id).is_some()).unwrap();
                                        let ws_y = ws_idx as f32 * state.layout_engine.viewport.height;
                                        let x_local = nx;
                                        let y_local = ny - ws_y;
                                        let col = &state.layout_engine.workspaces[ws_idx].columns[state.layout_engine.workspaces[ws_idx].find_window(win_id).unwrap().0];
                                        let is_overlay = col.is_overlay();

                                        let (sx, sy, sw, sh) = state.layout_engine.project_rect(x_local, y_local, nw, nh, ws_idx, current_scale, is_overlay);

                                        let px = (sx as f64 * scale_factor) as i32;
                                        let py = (sy as f64 * scale_factor) as i32;
                                        let pw = (sw as f64 * scale_factor) as i32;
                                        let ph = (sh as f64 * scale_factor) as i32;

                                        let color_arr = get_window_color(win_id);

                                        let id_window = smithay::backend::renderer::element::Id::new();
                                        render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                            id_window,
                                            Rectangle::new(Point::from((px, py)), Size::from((pw, ph))),
                                            CommitCounter::default(),
                                            smithay::backend::renderer::Color32F::from(color_arr),
                                            Kind::Unspecified,
                                        )));
                                    }
                                }
                            }
                        } else {
                            // Standard layout mode (Grid, Diagonal, Float)
                            let geom_mode = &state.layout_engine.tiling_mode;
                            for ws in &state.layout_engine.workspaces {
                                for col in &ws.columns {
                                    if let Some(win) = col.focused_window() {
                                        if let Some((x, y, w, h)) = state.layout_engine.get_window_anim_or_target_for_mode(win.id, geom_mode) {
                                            let px = ((x - state.layout_engine.viewport.x) as f64 * scale_factor) as i32;
                                            let py = ((y - state.layout_engine.viewport.y) as f64 * scale_factor) as i32;
                                            let pw = (w as f64 * scale_factor) as i32;
                                            let ph = (h as f64 * scale_factor) as i32;

                                            let color_arr = get_window_color(win.id);
                                            let id_window = smithay::backend::renderer::element::Id::new();
                                            render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                                id_window,
                                                Rectangle::new(Point::from((px, py)), Size::from((pw, ph))),
                                                CommitCounter::default(),
                                                smithay::backend::renderer::Color32F::from(color_arr),
                                                Kind::Unspecified,
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        if state.layout_engine.tiling_mode == crate::layout::TilingMode::Depth {
                            let transforms = state.layout_engine.depth_transforms();
                            let scale_factor = state.output.current_scale().fractional_scale();
                            
                            for (win_id, transform) in transforms.into_iter().rev() {
                                if let Some(smithay_win) = state.windows.get(&win_id) {
                                    if let Some((x, y, w, h)) = state.layout_engine.get_window_anim_or_target(win_id) {
                                        let scaled_w = w * transform.scale;
                                        let scaled_h = h * transform.scale;
                                        let x_offset = (w - scaled_w) / 2.0;
                                        let y_offset = (h - scaled_h) / 2.0 + (transform.y_offset as f32);
                                        
                                        let px = (((x + x_offset) - state.layout_engine.viewport.x) as f64 * scale_factor) as i32;
                                        let py = (((y + y_offset) - state.layout_engine.viewport.y) as f64 * scale_factor) as i32;
                                        
                                        let location = smithay::utils::Point::from((px, py));
                                        let scale = smithay::utils::Scale::from(scale_factor * transform.scale as f64);
                                        
                                        use smithay::backend::renderer::element::AsRenderElements;
                                        let elems: Vec<MyRenderElement> = AsRenderElements::render_elements(
                                            smithay_win,
                                            renderer,
                                            location,
                                            scale,
                                            transform.opacity,
                                        );
                                        render_elements.extend(elems);
                                    }
                                }
                            }
                        } else if state.layout_engine.overview_open
                            || (state.layout_engine.current_overview_scale - 1.0).abs() > 1e-3
                        {
                            let current_scale = state.layout_engine.current_overview_scale;
                            let scale_factor = state.output.current_scale().fractional_scale();

                            // Skip workspace separator lines drawing in non-sandbox mode to match Niri's style

                            // 2. Draw scaled windows with projected bounds
                            if state.layout_engine.underlying_tiling_mode == crate::layout::TilingMode::Depth {
                                let transforms = state.layout_engine.depth_transforms();
                                for (win_id, transform) in transforms {
                                    if let Some(smithay_win) = state.windows.get(&win_id) {
                                        if let Some((nx, ny, nw, nh)) = state.layout_engine.get_window_anim_or_target_for_mode(win_id, &state.layout_engine.underlying_tiling_mode) {
                                            let ws_idx = state.layout_engine.workspaces.iter().position(|ws| ws.find_window(win_id).is_some()).unwrap();
                                            let ws_y = ws_idx as f32 * state.layout_engine.viewport.height;
                                            
                                            let scaled_w = nw * transform.scale;
                                            let scaled_h = nh * transform.scale;
                                            let x_offset = (nw - scaled_w) / 2.0;
                                            let y_offset = (nh - scaled_h) / 2.0 + (transform.y_offset as f32);

                                            let x_local = nx + x_offset;
                                            let y_local = ny - ws_y + y_offset;
                                            let col = &state.layout_engine.workspaces[ws_idx].columns[state.layout_engine.workspaces[ws_idx].find_window(win_id).unwrap().0];
                                            let is_overlay = col.is_overlay();

                                            let (sx, sy, _sw, _sh) = state.layout_engine.project_rect(x_local, y_local, scaled_w, scaled_h, ws_idx, current_scale, is_overlay);

                                            let px = (sx as f64 * scale_factor) as i32;
                                            let py = (sy as f64 * scale_factor) as i32;
                                            
                                            let location = smithay::utils::Point::from((px, py));
                                            let scale_val = smithay::utils::Scale::from(scale_factor * current_scale as f64 * transform.scale as f64);
                                            
                                            use smithay::backend::renderer::element::AsRenderElements;
                                            let elems: Vec<MyRenderElement> = AsRenderElements::render_elements(
                                                smithay_win,
                                                renderer,
                                                location,
                                                scale_val,
                                                transform.opacity,
                                            );
                                            render_elements.extend(elems);
                                        }
                                    }
                                }
                            } else {
                                for (&win_id, smithay_win) in &state.windows {
                                    if let Some((nx, ny, nw, nh)) = state.layout_engine.get_window_anim_or_target_for_mode(win_id, &state.layout_engine.underlying_tiling_mode) {
                                        let ws_idx = state.layout_engine.workspaces.iter().position(|ws| ws.find_window(win_id).is_some()).unwrap();
                                        let ws_y = ws_idx as f32 * state.layout_engine.viewport.height;
                                        let x_local = nx;
                                        let y_local = ny - ws_y;
                                        let col = &state.layout_engine.workspaces[ws_idx].columns[state.layout_engine.workspaces[ws_idx].find_window(win_id).unwrap().0];
                                        let is_overlay = col.is_overlay();

                                        let (sx, sy, _sw, _sh) = state.layout_engine.project_rect(x_local, y_local, nw, nh, ws_idx, current_scale, is_overlay);

                                        let px = (sx as f64 * scale_factor) as i32;
                                        let py = (sy as f64 * scale_factor) as i32;
                                        
                                        let location = smithay::utils::Point::from((px, py));
                                        let scale_val = smithay::utils::Scale::from(scale_factor * current_scale as f64);
                                        
                                        use smithay::backend::renderer::element::AsRenderElements;
                                        let elems: Vec<MyRenderElement> = AsRenderElements::render_elements(
                                            smithay_win,
                                            renderer,
                                            location,
                                            scale_val,
                                            1.0,
                                        );
                                        render_elements.extend(elems);
                                    }
                                }
                            }
                        } else {
                            // 2. Add space elements (windows)
                            for elem in space_elements {
                                render_elements.push(MyRenderElement::Space(elem));
                            }
                        }
                    }

                    if state.hud_opacity > 0.0 {
                        let opacity = state.hud_opacity.min(1.0);
                        let scale_factor = state.output.current_scale().fractional_scale();
                        let view_w = state.layout_engine.viewport.width;
                        
                        let card_w = 120.0f32;
                        let card_h = 48.0f32;
                        let card_x = (view_w - card_w) / 2.0;
                        let card_y = 24.0f32;

                        let px = (card_x as f64 * scale_factor) as i32;
                        let py = (card_y as f64 * scale_factor) as i32;
                        let pw = (card_w as f64 * scale_factor) as i32;
                        let ph = (card_h as f64 * scale_factor) as i32;

                        use smithay::backend::renderer::element::solid::SolidColorRenderElement;
                        use smithay::utils::{Rectangle, Point, Size};
                        use smithay::backend::renderer::element::Kind;
                        use smithay::backend::renderer::utils::CommitCounter;

                        // 1. Draw card background (sleek dark semi-transparent card)
                        let bg_color = smithay::backend::renderer::Color32F::from([0.1f32, 0.1f32, 0.12f32, 0.8f32 * opacity]);
                        let id_bg = smithay::backend::renderer::element::Id::new();
                        render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                            id_bg,
                            Rectangle::new(Point::from((px, py)), Size::from((pw, ph))),
                            CommitCounter::default(),
                            bg_color,
                            Kind::Unspecified,
                        )));

                        // 2. Accent color for the left indicator bar
                        let accent_color_arr = match state.hud_tiling_mode {
                            Some(crate::layout::TilingMode::Diagonal) => [0.55f32, 0.25f32, 0.70f32, 1.0f32 * opacity],
                            Some(crate::layout::TilingMode::Grid) => [0.15f32, 0.64f32, 0.41f32, 1.0f32 * opacity],
                            Some(crate::layout::TilingMode::Depth) => [0.12f32, 0.57f32, 1.0f32, 1.0f32 * opacity],
                            Some(crate::layout::TilingMode::Float) => [0.88f32, 0.11f32, 0.14f32, 1.0f32 * opacity],
                            Some(crate::layout::TilingMode::Overview) => [1.0f32, 0.60f32, 0.10f32, 1.0f32 * opacity],
                            None => [0.8f32, 0.8f32, 0.8f32, 1.0f32 * opacity],
                        };
                        let accent_color = smithay::backend::renderer::Color32F::from(accent_color_arr);

                        let bar_w = (4.0f64 * scale_factor) as i32;
                        let id_bar = smithay::backend::renderer::element::Id::new();
                        render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                            id_bar,
                            Rectangle::new(Point::from((px, py)), Size::from((bar_w, ph))),
                            CommitCounter::default(),
                            accent_color,
                            Kind::Unspecified,
                        )));

                        // 3. Draw visual glyphs inside the card
                        let glyph_color = smithay::backend::renderer::Color32F::from([0.9f32, 0.9f32, 0.95f32, 0.9f32 * opacity]);

                        match state.hud_tiling_mode {
                            Some(crate::layout::TilingMode::Grid) => {
                                let sq_w = (12.0f64 * scale_factor) as i32;
                                let offsets = [
                                    (44.0, 8.0),
                                    (60.0, 8.0),
                                    (44.0, 24.0),
                                    (60.0, 24.0),
                                ];
                                for (ox, oy) in offsets {
                                    let id_sq = smithay::backend::renderer::element::Id::new();
                                    let spx = ((card_x + ox) as f64 * scale_factor) as i32;
                                    let spy = ((card_y + oy) as f64 * scale_factor) as i32;
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_sq,
                                        Rectangle::new(Point::from((spx, spy)), Size::from((sq_w, sq_w))),
                                        CommitCounter::default(),
                                        glyph_color,
                                        Kind::Unspecified,
                                    )));
                                }
                            }
                            Some(crate::layout::TilingMode::Diagonal) => {
                                let sq_w = (10.0f64 * scale_factor) as i32;
                                let offsets = [
                                    (45.0, 9.0),
                                    (55.0, 19.0),
                                    (65.0, 29.0),
                                ];
                                for (ox, oy) in offsets {
                                    let id_sq = smithay::backend::renderer::element::Id::new();
                                    let spx = ((card_x + ox) as f64 * scale_factor) as i32;
                                    let spy = ((card_y + oy) as f64 * scale_factor) as i32;
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_sq,
                                        Rectangle::new(Point::from((spx, spy)), Size::from((sq_w, sq_w))),
                                        CommitCounter::default(),
                                        glyph_color,
                                        Kind::Unspecified,
                                    )));
                                }
                            }
                            Some(crate::layout::TilingMode::Depth) => {
                                let b_w = (16.0f64 * scale_factor) as i32;
                                let b_h = (20.0f64 * scale_factor) as i32;
                                let b_color = smithay::backend::renderer::Color32F::from([0.9f32, 0.9f32, 0.95f32, 0.4f32 * opacity]);
                                let b_px = ((card_x + 52.0) as f64 * scale_factor) as i32;
                                let b_py = ((card_y + 8.0) as f64 * scale_factor) as i32;
                                let id_b = smithay::backend::renderer::element::Id::new();
                                render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                    id_b,
                                    Rectangle::new(Point::from((b_px, b_py)), Size::from((b_w, b_h))),
                                    CommitCounter::default(),
                                    b_color,
                                    Kind::Unspecified,
                                )));

                                let m_w = (20.0f64 * scale_factor) as i32;
                                let m_h = (22.0f64 * scale_factor) as i32;
                                let m_color = smithay::backend::renderer::Color32F::from([0.9f32, 0.9f32, 0.95f32, 0.7f32 * opacity]);
                                let m_px = ((card_x + 50.0) as f64 * scale_factor) as i32;
                                let m_py = ((card_y + 13.0) as f64 * scale_factor) as i32;
                                let id_m = smithay::backend::renderer::element::Id::new();
                                render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                    id_m,
                                    Rectangle::new(Point::from((m_px, m_py)), Size::from((m_w, m_h))),
                                    CommitCounter::default(),
                                    m_color,
                                    Kind::Unspecified,
                                )));

                                let f_w = (24.0f64 * scale_factor) as i32;
                                let f_h = (24.0f64 * scale_factor) as i32;
                                let f_px = ((card_x + 48.0) as f64 * scale_factor) as i32;
                                let f_py = ((card_y + 18.0) as f64 * scale_factor) as i32;
                                let id_f = smithay::backend::renderer::element::Id::new();
                                render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                    id_f,
                                    Rectangle::new(Point::from((f_px, f_py)), Size::from((f_w, f_h))),
                                    CommitCounter::default(),
                                    glyph_color,
                                    Kind::Unspecified,
                                )));
                            }
                            Some(crate::layout::TilingMode::Float) => {
                                let rects = [
                                    (44.0, 10.0, 16.0, 10.0),
                                    (62.0, 14.0, 14.0, 20.0),
                                    (48.0, 24.0, 12.0, 14.0),
                                ];
                                for (ox, oy, ow, oh) in rects {
                                    let id_r = smithay::backend::renderer::element::Id::new();
                                    let rpx = ((card_x + ox) as f64 * scale_factor) as i32;
                                    let rpy = ((card_y + oy) as f64 * scale_factor) as i32;
                                    let rpw = (ow as f64 * scale_factor) as i32;
                                    let rph = (oh as f64 * scale_factor) as i32;
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_r,
                                        Rectangle::new(Point::from((rpx, rpy)), Size::from((rpw, rph))),
                                        CommitCounter::default(),
                                        glyph_color,
                                        Kind::Unspecified,
                                    )));
                                }
                            }
                            Some(crate::layout::TilingMode::Overview) => {
                                let box_w = (32.0f64 * scale_factor) as i32;
                                let line_t = (2.0f64 * scale_factor) as i32;
                                let bpx = ((card_x + 44.0) as f64 * scale_factor) as i32;
                                let bpy = ((card_y + 8.0) as f64 * scale_factor) as i32;

                                let id_t = smithay::backend::renderer::element::Id::new();
                                render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                    id_t,
                                    Rectangle::new(Point::from((bpx, bpy)), Size::from((box_w, line_t))),
                                    CommitCounter::default(),
                                    glyph_color,
                                    Kind::Unspecified,
                                )));
                                let id_bot = smithay::backend::renderer::element::Id::new();
                                render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                    id_bot,
                                    Rectangle::new(Point::from((bpx, bpy + box_w - line_t)), Size::from((box_w, line_t))),
                                    CommitCounter::default(),
                                    glyph_color,
                                    Kind::Unspecified,
                                )));
                                let id_l = smithay::backend::renderer::element::Id::new();
                                render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                    id_l,
                                    Rectangle::new(Point::from((bpx, bpy + line_t)), Size::from((line_t, box_w - 2 * line_t))),
                                    CommitCounter::default(),
                                    glyph_color,
                                    Kind::Unspecified,
                                )));
                                let id_r = smithay::backend::renderer::element::Id::new();
                                render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                    id_r,
                                    Rectangle::new(Point::from((bpx + box_w - line_t, bpy + line_t)), Size::from((line_t, box_w - 2 * line_t))),
                                    CommitCounter::default(),
                                    glyph_color,
                                    Kind::Unspecified,
                                )));

                                let th_w = (10.0f64 * scale_factor) as i32;
                                let thumbs = [
                                    (48.0, 12.0),
                                    (62.0, 12.0),
                                    (48.0, 26.0),
                                    (62.0, 26.0),
                                ];
                                let th_color = smithay::backend::renderer::Color32F::from([0.9f32, 0.9f32, 0.95f32, 0.5f32 * opacity]);
                                for (ox, oy) in thumbs {
                                    let id_th = smithay::backend::renderer::element::Id::new();
                                    let th_px = ((card_x + ox) as f64 * scale_factor) as i32;
                                    let th_py = ((card_y + oy) as f64 * scale_factor) as i32;
                                    render_elements.push(MyRenderElement::Solid(SolidColorRenderElement::new(
                                        id_th,
                                        Rectangle::new(Point::from((th_px, th_py)), Size::from((th_w, th_w))),
                                        CommitCounter::default(),
                                        th_color,
                                        Kind::Unspecified,
                                    )));
                                }
                            }
                            None => {}
                        }
                    }

                    let backdrop_color = if state.layout_engine.overview_open {
                        smithay::backend::renderer::Color32F::from([0.10f32, 0.10f32, 0.10f32, 1.0f32])
                    } else {
                        smithay::backend::renderer::Color32F::from([0.08f32, 0.08f32, 0.08f32, 1.0f32])
                    };
                    damage_tracker.render_output(
                        renderer,
                        &mut framebuffer,
                        age,
                        &render_elements,
                        backdrop_color,
                    ).expect("failed to render output")
                };
                backend.submit(damage.damage.map(|v| v.as_slice())).unwrap();

                let time = state.start_time.elapsed();


                state.space.elements().for_each(|window| {
                    window.send_frame(
                        &state.output,
                        time,
                        Some(Duration::ZERO),
                        |_, _| Some(state.output.clone()),
                    );
                });
            }
            WinitEvent::CloseRequested => {
                state.running = false;
            }
            _ => {}
        }
    })?;

    // Main loop: tick spring physics, dispatch Wayland clients, and flush
    let mut last_tick = std::time::Instant::now();
    while state.running {
        event_loop.dispatch(Some(Duration::from_millis(16)), &mut state)?;

        // Compute delta time for spring physics
        let now = std::time::Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f32();
        last_tick = now;

        state.record_frame_time(dt);

        state.layout_engine.tick(dt);

        // Update HUD state
        if state.hud_previous_mode.is_none() {
            state.hud_previous_mode = Some(state.layout_engine.tiling_mode.clone());
        }
        if Some(&state.layout_engine.tiling_mode) != state.hud_previous_mode.as_ref() {
            state.hud_tiling_mode = Some(state.layout_engine.tiling_mode.clone());
            state.hud_opacity = 2.0; // visible + hold
            state.hud_previous_mode = Some(state.layout_engine.tiling_mode.clone());
        }
        if state.hud_opacity > 0.0 {
            state.hud_opacity = (state.hud_opacity - dt * 1.5).max(0.0);
        }
        if !sandbox {
            display.dispatch_clients(&mut state)?;
            display.flush_clients()?;
        }

        // Request winit window redraw on every tick to draw client updates and animations
        backend.borrow().window().request_redraw();
    }

    Ok(())
}
