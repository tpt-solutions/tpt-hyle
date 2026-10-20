#![forbid(unsafe_code)]
//! WebAssembly bindings for tpt-rift.
//!
//! A [`Store`] is an in-memory replica. It holds its data in memory and does
//! no I/O. JavaScript moves the bytes. It calls [`Store::export`] to get a
//! delta for a peer, sends those bytes over whatever link it has, and calls
//! [`Store::import`] on the bytes it receives. Import is idempotent, so the
//! link may duplicate or reorder messages.
//!
//! Persistence is not built. IndexedDB would need an asynchronous storage
//! backend, and the storage trait is synchronous. Until that exists, data
//! lasts as long as the page.

use tpt_hyle::{Hyle, VecStorage};
use tpt_rift::{Delta, Replica};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// An in-memory replica, usable from JavaScript.
#[wasm_bindgen]
pub struct Store {
    replica: Replica<VecStorage, VecStorage>,
}

#[wasm_bindgen]
impl Store {
    /// Creates an empty store for `node_id`, given as a decimal string. Each
    /// running copy needs its own node id.
    #[wasm_bindgen(constructor)]
    pub fn new(node_id: &str) -> Result<Store, JsError> {
        let node: u128 = node_id
            .parse()
            .map_err(|_| JsError::new("node_id must be a decimal u128"))?;
        let db = Hyle::open(VecStorage::new(), VecStorage::new(), node).map_err(js_err)?;
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

    /// The local Lamport clock. Returned as a JavaScript number, which is exact
    /// below 2^53.
    pub fn lamport(&mut self) -> f64 {
        self.replica.store().lamport() as f64
    }

    /// Encodes every change this store holds that `peer_node` does not
    /// originate. Send the result to that peer.
    pub fn export(&mut self, peer_node: &str) -> Result<Vec<u8>, JsError> {
        let peer: u128 = peer_node
            .parse()
            .map_err(|_| JsError::new("peer_node must be a decimal u128"))?;
        let start = self.replica.start_cursor();
        let export = self
            .replica
            .export(peer, start, usize::MAX)
            .map_err(js_err)?;
        Ok(export.delta.encode())
    }

    /// Applies bytes from a peer. Returns how many changes won and were applied.
    pub fn import(&mut self, bytes: &[u8]) -> Result<u32, JsError> {
        let delta = Delta::decode(bytes).map_err(js_err)?;
        let stats = self.replica.import(&delta).map_err(js_err)?;
        Ok(stats.applied as u32)
    }
}
