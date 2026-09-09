#[path = "main.rs"]
mod entry;

// `t!` expands to `$crate::i18n`, and this crate has two roots, so each names the module.
pub(crate) use entry::i18n;

#[cfg(all(target_arch = "wasm32", not(test)))]
use wasm_bindgen::prelude::*;

// Not under test: this runs on module load, and the harness loads the module into a page that has
// no canvas for the event loop to attach to.
#[cfg(all(target_arch = "wasm32", not(test)))]
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

// Every test in this crate is a browser test: the harness would otherwise run them under Node,
// which has no DOM and is not what ships.
#[cfg(all(test, target_family = "wasm"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
