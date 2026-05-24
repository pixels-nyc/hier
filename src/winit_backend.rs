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
}

pub fn detect_host_transform() -> Transform {
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

pub fn run_winit_compositor() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = Display::<State>::new()?;
    let display_handle = display.handle();

    let mut event_loop = EventLoop::<State>::try_new()?;
    let loop_handle = event_loop.handle();

    let (backend, winit_event_loop) = winit::init::<smithay::backend::renderer::gles::GlesRenderer>()?;
    let backend = Rc::new(RefCell::new(backend));

    let size = backend.borrow().window().inner_size();
    if std::env::var("HIER_FULLSCREEN").is_ok() {
        backend.borrow().window().set_fullscreen(Some(::winit::window::Fullscreen::Borderless(None)));
    }
    let layout_engine = crate::layout::LayoutEngine::new(
        size.width as f32,
        size.height as f32,
        10.0, // gap
        20.0, // outer margin
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
    use smithay::wayland::socket::ListeningSocketSource;
    let socket = ListeningSocketSource::new_auto()?;
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    println!("--------------------------------------------------");
    println!("Compositor started!");
    println!("WAYLAND_DISPLAY={}", socket_name);
    println!("To launch clients in nested window, run:");
    println!("  export WAYLAND_DISPLAY={}", socket_name);
    println!("  alacritty # or any Wayland client");
    println!("--------------------------------------------------");

    let mut state = State::new(display_handle, layout_engine, output.clone(), socket_name.clone());
    state.space.map_output(&output, (0, 0));

    // Create Control Unix Domain Socket Listener
    let ctrl_socket_path = std::env::var("HIER_CTRL_SOCKET")
        .unwrap_or_else(|_| format!("/tmp/hier-ctrl-{}.sock", socket_name));
    let _ = std::fs::remove_file(&ctrl_socket_path);
    let ctrl_listener = std::os::unix::net::UnixListener::bind(&ctrl_socket_path)?;
    ctrl_listener.set_nonblocking(true)?;

    println!("--------------------------------------------------");
    println!("Control socket listening at: {}", ctrl_socket_path);
    println!("--------------------------------------------------");

    // Register this nested compositor with its parent compositor if running inside one
    if let Ok(parent_display) = std::env::var("WAYLAND_DISPLAY") {
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
                                    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                                        let line_bytes: Vec<u8> = buffer.drain(..=pos).collect();
                                        if let Ok(line_str) = String::from_utf8(line_bytes) {
                                            let line_trimmed = line_str.trim();
                                            if !line_trimmed.is_empty() {
                                                let response = state.handle_simulated_input(line_trimmed);
                                                let _ = (&**stream).write_all(response.as_bytes());
                                                let _ = (&**stream).flush();
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

    loop_handle.insert_source(socket, move |client_stream, _metadata, state| {
        state.display_handle.insert_client(client_stream, std::sync::Arc::new(ClientState {
            compositor_state: smithay::wayland::compositor::CompositorClientState::default(),
        })).unwrap();
    })?;

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

    let mut damage_tracker = smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);

    // Register Winit Event Loop
    let backend_clone = backend.clone();
    loop_handle.insert_source(winit_event_loop, move |event, _, state| {
        match event {
            WinitEvent::Resized { size, .. } => {
                state.layout_engine.resize_viewport(size.w as f32, size.h as f32);
                let current_transform = detect_host_transform();
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
                    if let Some((win_id, color_arr)) = state.highlighted_window {
                        if let Some((x, y, w, h)) = state.layout_engine.get_window_rect(win_id) {
                            let color = smithay::backend::renderer::Color32F::from(color_arr);
                            let border_thickness = 4;
                            let scale_factor = state.output.current_scale().fractional_scale();
                            
                            let px = (x as f64 * scale_factor) as i32;
                            let py = (y as f64 * scale_factor) as i32;
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
                    }

                    // 2. Add space elements (windows)
                    for elem in space_elements {
                        render_elements.push(MyRenderElement::Space(elem));
                    }

                    damage_tracker.render_output(
                        renderer,
                        &mut framebuffer,
                        age,
                        &render_elements,
                        smithay::backend::renderer::Color32F::from([0.08f32, 0.08f32, 0.08f32, 1.0f32]),
                    ).expect("failed to render output")
                };

                backend.submit(damage.damage.map(|v| v.as_slice())).unwrap();
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

        state.layout_engine.tick(dt);

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;

        // Request winit window redraw on every tick to draw client updates and animations
        backend.borrow().window().request_redraw();
    }

    Ok(())
}
