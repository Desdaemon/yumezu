//! Where the dump lives between requests, and where it survives a restart.
//!
//! No database: the dump is small enough to hold whole, it is rebuilt rather than edited, and the
//! file it is written to is the very thing clients are served. So it is one JSON document, kept
//! parsed for the sync that reads the last one and serialized for the requests that hand it out. A
//! server coming up with the wiki unreachable still serves the last dump it wrote.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::model::Dump;

pub struct Snapshot {
    /// For the next sync, which reads the previous dump to keep the published world order and to
    /// carry over what an operator marked on them.
    pub dump: Dump,
    /// Byte for byte what a client is sent and what the file holds. Kept rather than produced per
    /// request, being a couple of megabytes and identical every time.
    pub json: Arc<str>,
}

pub struct Store {
    path: PathBuf,
    current: RwLock<Arc<Snapshot>>,
}

impl Store {
    /// A missing or unreadable file is the state before the first sync. One that is there but
    /// malformed means a previous run wrote something a later one cannot read.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let dump = match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<Dump>(&json) {
                Ok(dump) => {
                    tracing::info!("{} worlds read from {}", dump.worlds.len(), path.display());
                    Some(dump)
                }
                Err(error) => {
                    tracing::error!("{} is not a dump this can read: {error}", path.display());
                    None
                }
            },
            Err(error) => {
                tracing::info!("starting with no dump: {}: {error}", path.display());
                None
            }
        };
        Store {
            path,
            current: RwLock::new(Arc::new(snapshot(dump.unwrap_or_default()))),
        }
    }

    /// Safe to hold across an await: a sync finishing meanwhile replaces the store's snapshot
    /// without disturbing this one.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current.read().unwrap().clone()
    }

    /// Written into a neighbouring file and renamed over the real one, so a run dying mid-write
    /// leaves the previous dump intact rather than half a document.
    ///
    /// A failed write is reported and nothing more: the new dump is still better than the old one
    /// for everyone being served now, and the next sync tries the file again.
    pub fn publish(&self, dump: Dump) -> Arc<Snapshot> {
        let snapshot = Arc::new(snapshot(dump));
        if let Err(error) = write(&self.path, &snapshot.json) {
            tracing::error!("cannot write {}: {error}", self.path.display());
        }
        *self.current.write().unwrap() = snapshot.clone();
        snapshot
    }
}

/// Serialized once, so every reader afterwards is handed the same bytes.
fn snapshot(dump: Dump) -> Snapshot {
    let json = serde_json::to_string(&dump).expect("a dump is always serializable");
    Snapshot {
        dump,
        json: json.into(),
    }
}

/// Atomically as far as the filesystem allows.
fn write(path: &Path, json: &str) -> std::io::Result<()> {
    if let Some(directory) = path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
    {
        std::fs::create_dir_all(directory)?;
    }
    let staging = path.with_extension("json.new");
    std::fs::write(&staging, json)?;
    std::fs::rename(&staging, path)
}
