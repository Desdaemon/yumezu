//! What the address the page was opened at asks for:
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
    asked_for("location")
}

/// A BCP 47 tag, matched the way the device's own is. Never written to the store: a link is read
/// in the language it asks for without changing the language the reader chose.
pub(super) fn language() -> Option<String> {
    asked_for("lang")
}

/// An empty value counts as unasked: `?location=` names no world.
fn asked_for(key: &str) -> Option<String> {
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
