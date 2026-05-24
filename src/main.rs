mod layout;
mod spring;
mod state;
mod winit_backend;

fn main() {
    println!("=== Hier Wayland Compositor ===");
    if let Err(e) = winit_backend::run_winit_compositor() {
        eprintln!("Fatal compositor error: {}", e);
        std::process::exit(1);
    }
}
