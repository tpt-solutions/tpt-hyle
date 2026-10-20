#![forbid(unsafe_code)]
//! WebAssembly bindings for tpt-rift.
//!
//! A [`Store`] is an in-memory encrypted replica. It does no I/O of its own.
//! JavaScript moves the bytes:
//!
//! - [`Store::export`] returns the delta a peer lacks. JavaScript sends it.
//! - [`Store::import`] applies a delta JavaScript received. Import is
//!   idempotent, so the link may duplicate or reorder messages.
//! - [`Store::save`] returns the whole store as bytes. JavaScript persists them,
//!   for example in IndexedDB, and [`Store::load`] restores them. Recovery runs on
//!   load, so a save cut short leaves a tail that is dropped.
//!
//! The store is encrypted with a key derived from a passphrase, so the saved
//! bytes are not readable without it.

use tpt_hyle::{Hyle, VecStorage};
use tpt_rift::{Delta, Replica};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn parse_node(node_id: &str) -> Result<u128, JsError> {
    node_id
        .parse()
        .map_err(|_| JsError::new("node_id must be a decimal u128"))
}

/// An in-memory encrypted replica, usable from JavaScript.
#[wasm_bindgen]
pub struct Store {
    replica: Replica<VecStorage>,
}

#[wasm_bindgen]
impl Store {
    /// Creates an empty store for `node_id`, a decimal string. Each running
    /// copy needs its own node id. The passphrase derives the encryption key.
    #[wasm_bindgen(constructor)]
    pub fn new(node_id: &str, passphrase: &[u8]) -> Result<Store, JsError> {
        let node = parse_node(node_id)?;
        let db = Hyle::open_with_passphrase(VecStorage::new(), node, passphrase).map_err(js_err)?;
        Ok(Store {
            replica: Replica::new(db),
        })
    }

    /// Restores a store from bytes returned by [`Store::save`]. Fails with a
    /// wrong-key error if the passphrase does not match.
    pub fn load(node_id: &str, passphrase: &[u8], bytes: &[u8]) -> Result<Store, JsError> {
        let node = parse_node(node_id)?;
        let storage = VecStorage::from_bytes(bytes.to_vec());
        let db = Hyle::open_with_passphrase(storage, node, passphrase).map_err(js_err)?;
        Ok(Store {
            replica: Replica::new(db),
        })
    }

    /// Writes `value` under `key`.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), JsError> {
        self.replica
            .store()
            .put(key, value)
            .map(|_| ())
            .map_err(js_err)
    }

    /// The value under `key`, or `undefined` if absent or deleted.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, JsError> {
        let db = self.replica.store();
        let ev = match db.latest_event(key).map_err(js_err)? {
            Some(ev) if !ev.tombstone => ev,
            _ => return Ok(None),
        };
        let mut buf = vec![0u8; ev.payload_len as usize];
        db.get(key, &mut buf).map_err(js_err)?;
        Ok(Some(buf))
    }

    /// Deletes `key`. Returns whether it was present.
    pub fn delete(&mut self, key: &[u8]) -> Result<bool, JsError> {
        self.replica.store().delete(key).map_err(js_err)
    }

    /// The local Lamport clock, as a JavaScript number. Exact below 2^53.
    pub fn lamport(&mut self) -> f64 {
        self.replica.store().lamport() as f64
    }

    /// Encodes every change this store holds that `peer_node` does not originate.
    pub fn export(&mut self, peer_node: &str) -> Result<Vec<u8>, JsError> {
        let peer = parse_node(peer_node)?;
        let start = self.replica.start_cursor();
        let export = self
            .replica
            .export(peer, start, usize::MAX)
            .map_err(js_err)?;
        Ok(export.delta.encode())
    }

    /// Applies bytes from a peer. Returns how many changes won.
    pub fn import(&mut self, bytes: &[u8]) -> Result<u32, JsError> {
        let delta = Delta::decode(bytes).map_err(js_err)?;
        let stats = self.replica.import(&delta).map_err(js_err)?;
        Ok(stats.applied as u32)
    }

    /// The whole store as bytes, for persisting. Encrypted.
    pub fn save(&self) -> Vec<u8> {
        self.replica.store_ref().storage().bytes().to_vec()
    }
}
