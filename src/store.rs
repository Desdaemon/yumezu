//! Settings that outlive the session, one string per key: localStorage on the web build, a file per
//! key elsewhere. Failures are dropped.

/// The empty string is a value: a flag key is written with one.
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

/// `None` for a key never written, or written with neither `on` nor `off`.
pub(super) fn read_switch(key: &str) -> Option<bool> {
    match read(key)?.as_str() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

pub(super) fn write_switch(key: &str, on: bool) {
    write(key, Some(if on { "on" } else { "off" }));
}

/// Whether the flag key is present.
pub(super) fn read_flag(key: &str) -> bool {
    read(key).is_some()
}

pub(super) fn write_flag(key: &str, set: bool) {
    write(key, set.then_some(""));
}

/// Clamps a value written by a version with a different range.
pub(super) fn read_clamped(key: &str, range: std::ops::RangeInclusive<f32>) -> Option<f32> {
    let value: f32 = read(key)?.parse().ok()?;
    Some(value.clamp(*range.start(), *range.end()))
}

/// `None` removes the key.
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

/// `None` where the web build has no store: served from a file, or storage off.
#[cfg(target_family = "wasm")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(not(target_family = "wasm"))]
fn file(key: &str) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    let directory = super::ANDROID.get()?.internal_data_path()?;
    #[cfg(not(target_os = "android"))]
    let directory = xdg_directory("XDG_CONFIG_HOME", "/.config")?;
    Some(directory.join(key))
}

/// Separate from [`file`]'s directory, which holds the user's settings. The OS may clear this
/// cache when short of room, so [`super::fetch`] may grow here.
#[cfg(not(target_family = "wasm"))]
pub(super) fn cache_directory() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    let directory = android_cache_directory()?;
    #[cfg(not(target_os = "android"))]
    let directory = xdg_directory("XDG_CACHE_HOME", "/.cache")?;
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory)
}

/// This app's directory under `variable`, or under `home_suffix` in `HOME` without `variable`.
#[cfg(not(any(target_family = "wasm", target_os = "android")))]
fn xdg_directory(variable: &str, home_suffix: &str) -> Option<std::path::PathBuf> {
    let base = match std::env::var_os(variable) {
        Some(base) => base,
        None => {
            let mut home = std::env::var_os("HOME")?;
            home.push(home_suffix);
            home
        }
    };
    Some(std::path::PathBuf::from(base).join(env!("CARGO_PKG_NAME")))
}

/// Read from Java: the activity glue publishes only `internalDataPath`, the app's *files*
/// directory, which is never reclaimed.
#[cfg(target_os = "android")]
fn android_cache_directory() -> Option<std::path::PathBuf> {
    use jni::objects::{JObject, JString};
    use jni::{jni_sig, jni_str};

    let context = ndk_context::android_context();
    // SAFETY: the glue publishes both before `android_main`, and both outlive the app.
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
