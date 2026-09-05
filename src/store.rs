//! The few choices that outlive the run that made them.
//!
//! One string per key: an entry in localStorage on the page, a file of that name in the one
//! directory this app is allowed to keep anything in everywhere else. A page served from a file
//! or with storage turned off has neither, and so has no memory: every failure here is dropped
//! rather than reported, since there is nowhere on screen to report it to and nothing but a
//! setting's next reading depends on it.

/// What was written under `key` on some earlier run, if anything was. The empty string is a
/// value like any other: a key whose presence is the whole answer is written with one.
pub(super) fn read(key: &str) -> Option<String> {
    #[cfg(target_family = "wasm")]
    {
        storage()?.get_item(key).ok().flatten()
    }
    #[cfg(not(target_family = "wasm"))]
    {
        std::fs::read_to_string(file(key)?).ok()
    }
}

/// Writes a value down, or takes it back with `None`.
pub(super) fn write(key: &str, value: Option<&str>) {
    #[cfg(target_family = "wasm")]
    {
        let Some(storage) = storage() else { return };
        let _ = match value {
            Some(value) => storage.set_item(key, value),
            None => storage.remove_item(key),
        };
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let Some(file) = file(key) else { return };
        match value {
            Some(value) => {
                if let Some(directory) = file.parent() {
                    let _ = std::fs::create_dir_all(directory);
                }
                let _ = std::fs::write(&file, value);
            }
            None => {
                let _ = std::fs::remove_file(&file);
            }
        }
    }
}

/// The page's own store, which a page served from a file or with storage turned off has none of.
#[cfg(target_family = "wasm")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The file a key is kept in. `None` where there is no directory to be found to keep it in, which
/// leaves every choice lasting only as long as the run.
#[cfg(not(target_family = "wasm"))]
fn file(key: &str) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    // The app's own private directory, which the framework hands out and nothing else can read.
    let directory = super::ANDROID.get()?.internal_data_path()?;
    #[cfg(not(target_os = "android"))]
    // Where a desktop keeps what an app is configured with. Read off the environment rather than
    // through a crate: it is two variables and one fallback between them.
    let directory = std::path::PathBuf::from(match std::env::var_os("XDG_CONFIG_HOME") {
        Some(config) => config,
        None => {
            let mut home = std::env::var_os("HOME")?;
            home.push("/.config");
            home
        }
    })
    .join(env!("CARGO_PKG_NAME"));
    Some(directory.join(key))
}

/// Where something may be kept that the app would rather have than not, but can lose without
/// losing anything a person chose. Made if it is not already there.
///
/// Told apart from [`file`]'s directory on both platforms, because the two are kept under
/// different promises: what a person set is theirs until they change it, and what is here is a
/// copy of something the network can be asked for again. Android may empty this directory
/// whenever the device is short of room, and a desktop expects a `cache` a cleaner may sweep --
/// which is the whole reason the downloads in [`super::fetch`] are allowed to grow here at all.
///
/// `None` where there is nowhere to put it, which leaves the run with no cache and nothing worse.
#[cfg(not(target_family = "wasm"))]
pub(super) fn cache_directory() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    let directory = android_cache_directory()?;
    #[cfg(not(target_os = "android"))]
    let directory = std::path::PathBuf::from(match std::env::var_os("XDG_CACHE_HOME") {
        Some(cache) => cache,
        None => {
            let mut home = std::env::var_os("HOME")?;
            home.push("/.cache");
            home
        }
    })
    .join(env!("CARGO_PKG_NAME"));
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory)
}

/// What `Context.getCacheDir()` answers, which is the one directory here the system will empty by
/// itself when the device runs short of room.
///
/// Asked of Java rather than taken from the activity glue, which publishes only
/// `internalDataPath` -- the app's *files* directory, which nothing ever reclaims. A cache left
/// there would grow for as long as the app stays installed. The call is placed the way
/// `app::open_in_browser` places its own; see the reasoning there about the context and the VM.
#[cfg(target_os = "android")]
fn android_cache_directory() -> Option<std::path::PathBuf> {
    use jni::objects::{JObject, JString};
    use jni::{jni_sig, jni_str};

    let context = ndk_context::android_context();
    // Safe on the same grounds as `app::open_in_browser`: the glue publishes both before it
    // calls `android_main`, and both outlive the app.
    #[allow(unsafe_code)]
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) };
    let found = vm.attach_current_thread(|env| -> Result<String, jni::errors::Error> {
        #[allow(unsafe_code)]
        let application = unsafe { JObject::from_raw(env, context.context().cast()) };
        let directory = env
            .call_method(
                &application,
                jni_str!("getCacheDir"),
                jni_sig!("()Ljava/io/File;"),
                &[],
            )?
            .l()?;
        let path = env
            .call_method(
                &directory,
                jni_str!("getAbsolutePath"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        env.cast_local::<JString>(path)?.try_to_string(env)
    });
    match found {
        Ok(path) => Some(std::path::PathBuf::from(path)),
        Err(error) => {
            log::warn!("no cache directory to download into: {error}");
            None
        }
    }
}
