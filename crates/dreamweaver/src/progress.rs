//! What a sync is doing, for whoever is waiting on it.
//!
//! A sync takes a minute or so, and `GET /data` answers `needs update` for the whole of it rather
//! than serving a dump about to be replaced. The app on the other end has nothing to draw and
//! nothing to say about why, so `GET /pollUpdate` reads this.
//!
//! The stage names are the reference implementation's own, so a reader written against its
//! `/pollUpdate` reads these unchanged. They are a coarser account than its worker gives: this
//! program fetches the authors, the releases and the passages as one concurrent question, and
//! there is no honest way to call that three stages.

use std::sync::RwLock;

/// Asking the wiki what it has changed, where a soft sync starts and often ends.
pub const CHANGES: &str = "init";
pub const WORLDS: &str = "fetchWorldData";
/// Reading the passages, and with them the authors and the release history.
pub const PASSAGES: &str = "fetchConnData";

/// The stage a sync has reached, or `None` for none running -- which is `done` to a client.
#[derive(Default)]
pub struct Progress(RwLock<Option<&'static str>>);

impl Progress {
    pub fn at(&self, task: &'static str) {
        *self.0.write().unwrap() = Some(task);
    }

    /// Whatever came of it.
    pub fn done(&self) {
        *self.0.write().unwrap() = None;
    }

    pub fn task(&self) -> Option<&'static str> {
        *self.0.read().unwrap()
    }
}
