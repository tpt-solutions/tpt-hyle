//! UniFFI bindings for tpt-rift. One object, `SyncStore`, is a file-backed
//! replica. Kotlin and Swift code is generated from these definitions.
//!
//! The surface matches the wasm bindings. It is smaller on purpose: open, read,
//! write, export a delta for a peer, and import one. Transport is left to the
//! app, as it is on the web.
//!
//! Each store must have its own file set. Only one process may open a path at
//! a time.

#![forbid(unsafe_code)]

use std::fmt;
use std::sync::{Arc, Mutex};

use tpt_hyle::Hyle;
use tpt_rift::{Delta, Replica};

uniffi::setup_scaffolding!();

type Store = Replica<tpt_hyle::FileStorage, tpt_hyle::FileStorage>;

/// Errors from the store. Each carries a readable message for the app to show
/// or log.
#[derive(Debug, uniffi::Error)]
pub enum SyncError {
    /// The store or the file system failed.
    Store { message: String },
    /// A node id, peer id, or delta was malformed.
    Invalid { message: String },
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Store { message } | SyncError::Invalid { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for SyncError {}

fn store_err(e: impl fmt::Display) -> SyncError {
    SyncError::Store {
        message: e.to_string(),
    }
}

fn parse_node(text: &str) -> Result<u128, SyncError> {
    text.parse().map_err(|_| SyncError::Invalid {
        message: format!("node id must be a decimal u128, got {text:?}"),
    })
}

/// A file-backed replica, for use from Kotlin or Swift.
#[derive(uniffi::Object)]
pub struct SyncStore {
    inner: Mutex<Store>,
}

#[uniffi::export]
impl SyncStore {
    /// Opens or creates the store at `path`. `node_id` is a decimal `u128`, and
    /// each device needs its own.
    #[uniffi::constructor]
    pub fn open(path: String, node_id: String) -> Result<Arc<Self>, SyncError> {
        let node = parse_node(&node_id)?;
        let db = Hyle::open_file(path, node).map_err(store_err)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Replica::new(db)),
        }))
    }

    /// Writes `value` under `key`.
    pub fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), SyncError> {
        self.with(|r| r.store().put(&key, &value).map(|_| ()).map_err(store_err))
    }

    /// The value under `key`, or `None` if absent or deleted.
    pub fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, SyncError> {
        self.with(|r| {
            let db = r.store();
            let ev = match db.latest_event(&key).map_err(store_err)? {
                Some(ev) if !ev.tombstone => ev,
                _ => return Ok(None),
            };
            let mut buf = vec![0u8; ev.payload_len as usize];
            db.get(&key, &mut buf).map_err(store_err)?;
            Ok(Some(buf))
        })
    }

    /// Deletes `key`. Returns whether it was present.
    pub fn delete(&self, key: Vec<u8>) -> Result<bool, SyncError> {
        self.with(|r| r.store().delete(&key).map_err(store_err))
    }

    /// Encodes the changes this store holds that `peer_node` does not
    /// originate. Send the bytes to that peer.
    pub fn export_for(&self, peer_node: String) -> Result<Vec<u8>, SyncError> {
        let peer = parse_node(&peer_node)?;
        self.with(|r| {
            let start = r.start_cursor();
            let export = r.export(peer, start, usize::MAX).map_err(store_err)?;
            Ok(export.delta.encode())
        })
    }

    /// Applies bytes from a peer. Returns how many changes won.
    pub fn import_from(&self, bytes: Vec<u8>) -> Result<u32, SyncError> {
        let delta = Delta::decode(&bytes).map_err(|e| SyncError::Invalid {
            message: e.to_string(),
        })?;
        self.with(|r| {
            let stats = r.import(&delta).map_err(store_err)?;
            Ok(stats.applied as u32)
        })
    }

    /// The local Lamport clock.
    pub fn lamport(&self) -> Result<u64, SyncError> {
        self.with(|r| Ok(r.store().lamport()))
    }
}

impl SyncStore {
    fn with<T>(&self, f: impl FnOnce(&mut Store) -> Result<T, SyncError>) -> Result<T, SyncError> {
        let mut guard = self.inner.lock().map_err(|_| SyncError::Store {
            message: "store lock poisoned by an earlier panic".into(),
        })?;
        f(&mut guard)
    }
}
