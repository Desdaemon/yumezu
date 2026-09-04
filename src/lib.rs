#![allow(special_module_name)]
#![cfg_attr(target_os = "android", deny(unsafe_code))]
#![cfg_attr(not(target_os = "android"), forbid(unsafe_code))]

mod main;

// Entry point for wasm
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_log::init_with_level(log::Level::Debug).unwrap();

    use log::info;
    info!("Logging works!");

    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    main::main();
    Ok(())
}

/// Entry point for Android, called by the NativeActivity glue on the thread it starts for it.
///
/// The apk carries no Java of its own, so this library *is* the app: see `android/`. The handle
/// the glue passes in is the only way to reach anything the framework owns, which is why it is
/// both given to winit and kept for everything under `app` that needs the framework later.
#[allow(unsafe_code)]
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    main::app::use_android_app(app.clone());
    main::run(
        winit::event_loop::EventLoop::builder()
            .with_android_app(app)
            .build()
            .unwrap(),
    );
}
