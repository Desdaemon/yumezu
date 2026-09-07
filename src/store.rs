//! The few choices that outlive the run that made them: one string per key, a localStorage entry
//! on the page and a file of that name everywhere else.
//!
//! Every failure here is dropped rather than reported -- nothing but a setting's next reading
//! depends on it -- so a page with no storage simply has no memory.

/// The empty string is a value like any other: a key whose presence is the whole answer is
/// written with one.
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

/// `None` takes the value back.
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

/// `None` where the page has no store: served from a file, or storage off.
#[cfg(target_family = "wasm")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(not(target_family = "wasm"))]
fn file(key: &str) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    let directory = super::ANDROID.get()?.internal_data_path()?;
    #[cfg(not(target_os = "android"))]
    // Read off the environment rather than through a crate: two variables and one fallback.
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

/// Deliberately not [`file`]'s directory, which holds what a person chose and must survive.
/// Android may empty this one when the device is short of room and a desktop `cache` may be swept
/// by a cleaner, which is why [`super::fetch`] is allowed to grow here.
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

/// Asked of Java because the activity glue publishes only `internalDataPath`, the app's *files*
/// directory, which nothing ever reclaims -- a cache left there would grow until uninstall.
#[cfg(target_os = "android")]
fn android_cache_directory() -> Option<std::path::PathBuf> {
    use jni::objects::{JObject, JString};
    use jni::{jni_sig, jni_str};

    let context = ndk_context::android_context();
    // SAFETY: the glue publishes the VM and the context before it calls `android_main`, and both
    // outlive the app.
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
