//! The dump in memory and as one JSON file, the file clients are served, so a restart with the
//! wiki unreachable still serves the last dump.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::model::Dump;

pub struct Snapshot {
    /// For the next sync, which keeps the previous dump's world order and operator marks.
    pub dump: Dump,
    /// Byte for byte what a client is sent and what the file holds; serialized once per dump.
    pub json: Arc<str>,
    /// The connections `/getNextLocations` traverses, built once with the dump.
    pub routing: Routing,
}

/// The dump as connections to traverse, and a lookup for callers that name a world by title.
pub struct Routing {
    pub connections: Vec<Vec<yumezu_routing::Step>>,
    /// By the wiki's English title, which is what YNOproject's location list calls a world.
    named: std::collections::HashMap<String, usize>,
}

impl Routing {
    pub fn of(dump: &Dump) -> Self {
        Routing {
            connections: yumezu_routing::connections(&dump.worlds),
            named: dump
                .worlds
                .iter()
                .enumerate()
                .map(|(at, world)| (world.title.clone(), at))
                .collect(),
        }
    }

    pub fn world_named(&self, title: &str) -> Option<usize> {
        self.named.get(title).copied()
    }
}

pub struct Store {
    path: PathBuf,
    current: RwLock<Arc<Snapshot>>,
}

impl Store {
    /// A missing file is the state before the first sync; a malformed one came from an
    /// incompatible build.
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

    /// A failed write is reported and nothing more: the new dump is still the one served from
    /// memory, and the next sync tries the file again.
    pub fn publish(&self, dump: Dump) -> Arc<Snapshot> {
        let snapshot = Arc::new(snapshot(dump));
        if let Err(error) = write_atomically(&self.path, snapshot.json.as_bytes()) {
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
        routing: Routing::of(&dump),
        dump,
        json: json.into(),
    }
}

/// Written beside `path`, then renamed over the original: a crash mid-write keeps the old file.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(directory) = path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
    {
        std::fs::create_dir_all(directory)?;
    }
    let mut staging = path.as_os_str().to_owned();
    staging.push(".new");
    std::fs::write(&staging, bytes)?;
    std::fs::rename(&staging, path)
}
