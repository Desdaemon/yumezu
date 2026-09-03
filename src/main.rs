#[cfg(target_family = "wasm")]
use winit::platform::web::EventLoopExtWebSys;

#[path = "app.rs"]
pub(crate) mod app;

/// The native entry point. The other two platforms build the event loop differently and call
/// [`run`] themselves: the page from `lib.rs`'s `start`, the phone from its `android_main`.
#[allow(unused)]
pub fn main() {
    run(winit::event_loop::EventLoop::new().unwrap());
}

/// Runs the app on an event loop the caller has already built, which is the whole of what the
/// platforms differ in here.
#[allow(unused)]
pub(crate) fn run(event_loop: winit::event_loop::EventLoop<()>) {
    let mut app = app::App::new();

    #[cfg(not(target_family = "wasm"))]
    event_loop.run_app(&mut app).unwrap();

    #[cfg(target_family = "wasm")]
    event_loop.spawn_app(app);
}
