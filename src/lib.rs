#[path = "main.rs"]
mod entry;

// `t!` expands to `$crate::i18n`, and this crate has two roots, so each names the module.
pub(crate) use entry::i18n;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_log::init_with_level(log::Level::Debug).unwrap();

    log::info!("Logging initialized.");

    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    entry::main();
    Ok(())
}

#[allow(unsafe_code)]
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    // The only route to anything the framework owns, and needed after startup too.
    entry::app::use_android_app(app.clone());
    entry::run(
        winit::event_loop::EventLoop::builder()
            .with_android_app(app)
            .build()
            .unwrap(),
    );
}
