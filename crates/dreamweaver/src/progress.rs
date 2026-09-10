//! What a sync is doing, for whoever is waiting on it.
//!
//! A sync takes a minute or so, and until the first one lands `GET /data` answers `needs update`.
//! The app has nothing to draw and nothing to say about why, so `GET /pollUpdate` reads this.
//!
//! The stage names are the reference implementation's own, so a reader written against its
//! `/pollUpdate` reads these unchanged. They are coarser than its worker gives: this program
//! fetches the authors, the releases and the connections as one concurrent question.

use std::sync::RwLock;

/// Asking the wiki what it has changed, where a soft sync starts and often ends.
pub const CHANGES: &str = "init";
pub const WORLDS: &str = "fetchWorldData";
/// Reading the connections, and with them the authors and the release history.
pub const CONNECTIONS: &str = "fetchConnData";

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
