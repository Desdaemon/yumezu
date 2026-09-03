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
