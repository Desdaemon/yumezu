//! Query parameters of the web build's URL (`?location=Nexus&lang=en`); `None` on other builds.

/// The world to open on, by either name, case-insensitive. See
/// [`super::AppEntities::world_named`].
pub(super) fn location() -> Option<String> {
    param("location")
}

/// A BCP 47 tag, matched as the device's is. Not stored, so a link leaves the user's chosen
/// language unchanged.
pub(super) fn language() -> Option<String> {
    param("lang")
}

/// The `profile` switches a session starts with, comma separated: `YUMEZU_FRAMES` for the web
/// build.
#[cfg(all(target_family = "wasm", feature = "profile"))]
pub(super) fn frames() -> Option<String> {
    param("frames")
}

/// The pose a `profile` session opens on, as the six numbers the `camera in ... dimensions` log
/// line prints: `eye` then `target`. See `profile::pan_aside`.
#[cfg(all(target_family = "wasm", feature = "profile"))]
pub(super) fn camera() -> Option<String> {
    param("camera")
}

/// An empty value counts as absent: `?location=` names no world.
fn param(key: &str) -> Option<String> {
    #[cfg(target_family = "wasm")]
    {
        let search = web_sys::window()?.location().search().ok()?;
        let value = web_sys::UrlSearchParams::new_with_str(&search)
            .ok()?
            .get(key)?;
        (!value.trim().is_empty()).then_some(value)
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let _ = key;
        None
    }
}
