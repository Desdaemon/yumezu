#[cfg(target_family = "wasm")]
use winit::platform::web::EventLoopExtWebSys;

#[path = "app.rs"]
pub(crate) mod app;

// The one module named from the crate root rather than through `app`. [`app::i18n::t`] writes
// `$crate::i18n`, and the two targets of this crate have two different roots -- the binary's is
// this file, the library's is `lib.rs` -- so each of them names it here for the macro to find.
pub(crate) use app::i18n;

/// The native entry point. The other two platforms build the event loop differently and call
/// [`run`] themselves: the page from `lib.rs`'s `start`, the phone from its `android_main`.
#[allow(unused)]
pub fn main() {
    // The other two platforms install their own on the way in -- see `lib.rs`. Reads `RUST_LOG`
    // and says nothing without it: a windowed app has no terminal to write to most of the time,
    // and the messages worth having are the ones somebody went looking for.
    #[cfg(all(not(target_family = "wasm"), not(target_os = "android")))]
    env_logger::init();
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
