//! What the address the page was opened at carries:
//!
//! ```text
//! https://explorer.yume.wiki/?location=Nexus&lang=en
//! ```
//!
//! Only the page has an address. Everywhere else both are `None` and a run opens on whatever it
//! left behind.

/// The world to open on, by either of its names and in whatever case. See
/// [`super::AppEntities::world_named`].
pub(super) fn location() -> Option<String> {
    param("location")
}

/// A BCP 47 tag, matched the way the device's own is. Never written to the store: a link is read
/// in the language it names without changing the language the reader chose.
pub(super) fn language() -> Option<String> {
    param("lang")
}

/// Which of `profile`'s switches a run opens with, comma separated, for a page with nobody to
/// click them -- what `YUMEZU_FRAMES` names everywhere there is an environment to name it in.
#[cfg(all(target_family = "wasm", feature = "profile"))]
pub(super) fn frames() -> Option<String> {
    param("frames")
}

/// The pose a `profile` run opens on, as `eye` then `target`: six numbers, comma separated, which
/// is what the `camera in ... dimensions` log line prints. See `profile::pan_aside`.
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
