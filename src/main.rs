mod layout;
mod spring;
mod state;
mod winit_backend;

fn main() {
    println!("=== Hier Wayland Compositor ===");
    let sandbox = std::env::args().any(|arg| arg == "--sandbox")
        || std::env::var("HIER_SANDBOX").is_ok();
    
    if sandbox {
        println!("[Sandbox Mode Enabled] Bypassing Wayland display loops.");
    }
    
    if let Err(e) = winit_backend::run_winit_compositor(sandbox) {
        eprintln!("Fatal compositor error: {}", e);
        std::process::exit(1);
    }
}

