//! What a sync is doing, for whoever is waiting on it.
//!
//! A sync takes a minute or so, and `GET /data` answers `needs update` for the whole of it rather
//! than serving a dump that is about to be replaced -- see [`crate::data`]. The app on the other
//! end has nothing to draw and nothing to say about why, so the sync says where it has got to as
//! it goes and `POST /pollUpdate` is how a client reads that.
//!
//! The stage names are the reference implementation's own, so a reader written against its
//! `/pollUpdate` reads these unchanged -- see [`crate`]. They are a coarser account than its
//! worker gives: this program fetches the authors, the releases and the passages as one
//! concurrent question, and there is no honest way to call that three stages.

use std::sync::RwLock;

/// Asking the wiki what it has changed, which is where a soft sync starts and often ends.
pub const CHANGES: &str = "init";
/// Reading the worlds.
pub const WORLDS: &str = "fetchWorldData";
/// Reading the passages, and with them the authors and the release history.
pub const PASSAGES: &str = "fetchConnData";

/// Where the running sync has got to.
///
/// The stage, or `None` for no sync running -- which is what `done` means to a client.
#[derive(Default)]
pub struct Progress(RwLock<Option<&'static str>>);

impl Progress {
    /// Says a sync has reached `task`.
    pub fn at(&self, task: &'static str) {
        *self.0.write().unwrap() = Some(task);
    }

    /// Says the sync has stopped, whatever came of it.
    pub fn done(&self) {
        *self.0.write().unwrap() = None;
    }

    /// The stage a client is told about. `None` is `done`.
    pub fn task(&self) -> Option<&'static str> {
        *self.0.read().unwrap()
    }
}
