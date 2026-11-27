//! The running sync's stage for `GET /pollUpdate`, under the reference implementation's names.

use std::sync::RwLock;

/// Querying the wiki for what has changed, where a soft sync starts and often ends.
pub const CHANGES: &str = "init";
pub const WORLDS: &str = "fetchWorldData";
/// Reading the connections, and with them the authors and the release history.
pub const CONNECTIONS: &str = "fetchConnData";

/// The stage a sync has reached, or `None` when no sync is running: `done` to a client.
#[derive(Default)]
pub struct Progress(RwLock<Option<&'static str>>);

impl Progress {
    pub fn at(&self, task: &'static str) {
        *self.0.write().unwrap() = Some(task);
    }

    /// Clears the stage, whether the sync succeeded or failed.
    pub fn done(&self) {
        *self.0.write().unwrap() = None;
    }

    pub fn task(&self) -> Option<&'static str> {
        *self.0.read().unwrap()
    }
}
