#[cfg(target_family = "wasm")]
use winit::platform::web::EventLoopExtWebSys;

#[path = "app.rs"]
pub(crate) mod app;

// `t!` expands to `$crate::i18n`, and this crate has two roots -- this file for the binary,
// `lib.rs` for the library -- so each names the module for the macro to find.
pub(crate) use app::i18n;

#[allow(unused)]
pub fn main() {
    // The page and the phone install their own logger instead; see `lib.rs`.
    #[cfg(all(not(target_family = "wasm"), not(target_os = "android")))]
    env_logger::init();
    run(winit::event_loop::EventLoop::new().unwrap());
}

#[allow(unused)]
pub(crate) fn run(event_loop: winit::event_loop::EventLoop<()>) {
    let mut app = app::App::new();

    #[cfg(not(target_family = "wasm"))]
    event_loop.run_app(&mut app).unwrap();

    #[cfg(target_family = "wasm")]
    event_loop.spawn_app(app);
}
