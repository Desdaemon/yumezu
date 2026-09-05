//! Where the dump lives between requests, and where it survives a restart.
//!
//! There is no database. The dump is small enough to hold whole, it is rebuilt from the wiki
//! rather than edited, and the file it is written to is the very thing clients are served -- so a
//! store here is one JSON document, kept parsed for the sync that has to read the last one and
//! kept serialized for the requests that just hand it out. A server that comes up with the
//! wiki unreachable still serves the last dump it wrote, which is the only durability this
//! needs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::model::Dump;

/// The dump as it stands, in both the forms this program uses it in.
pub struct Snapshot {
    /// Parsed, for the next sync: it reads the previous dump to keep worlds in the order they
    /// were already published in, and to carry over what an operator marked on them.
    pub dump: Dump,
    /// Serialized, byte for byte what a client is sent and what the file holds. Kept rather than
    /// produced per request, since it is a couple of megabytes and identical every time.
    pub json: Arc<str>,
}

/// The published dump and the file behind it.
pub struct Store {
    path: PathBuf,
    current: RwLock<Arc<Snapshot>>,
}

impl Store {
    /// Opens the store at `path`, reading whatever was last written there.
    ///
    /// A missing or unreadable file is not an error: it is the state before the first sync, and
    /// the store comes up empty and says so. A file that is there but malformed is worth
    /// complaining about, since it means a previous run wrote something a later one cannot read.
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

    /// The dump as it stands. Cheap, and safe to hold across an await: a sync finishing meanwhile
    /// replaces the store's snapshot without disturbing this one.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current.read().unwrap().clone()
    }

    /// Publishes `dump`, and writes it to the file.
    ///
    /// The write comes first, and it goes to a neighbouring file that is then renamed over the
    /// real one, so a run that dies mid-write leaves the previous dump intact rather than half a
    /// document. A write that fails is reported and nothing more: the new dump is still better
    /// than the old one for everyone being served now, and the next sync will try the file again.
    pub fn publish(&self, dump: Dump) -> Arc<Snapshot> {
        let snapshot = Arc::new(snapshot(dump));
        if let Err(error) = write(&self.path, &snapshot.json) {
            tracing::error!("cannot write {}: {error}", self.path.display());
        }
        *self.current.write().unwrap() = snapshot.clone();
        snapshot
    }
}

/// Serializes a dump once, so every reader of it afterwards is handed the same bytes.
fn snapshot(dump: Dump) -> Snapshot {
    let json = serde_json::to_string(&dump).expect("a dump is always serializable");
    Snapshot {
        dump,
        json: json.into(),
    }
}

/// Writes `json` to `path`, atomically as far as the filesystem allows.
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
