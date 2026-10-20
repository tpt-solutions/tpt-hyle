//! In-memory store, mock transports and proptest generators for the tpt-hyle
//! workspace.
//!
//! Mock transports for `tpt-rift` arrive with Phase 2. What is here now:
//! an in-memory store, a model-checking [`Op`] type, and proptest strategies.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use tpt_hyle::{Hyle, Key, VecStorage};

/// A store backed entirely by memory. Nothing touches the disk.
/// The test key. Real stores take a key from a passphrase or a key store.
pub const TEST_KEY: [u8; 32] = [0x5a; 32];

/// The in-memory store used by the tests.
pub type MemStore = Hyle<VecStorage>;

/// One mutation, for driving a store and a reference model in lockstep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Write `value` under `key`.
    Put(Vec<u8>, Vec<u8>),
    /// Delete `key`.
    Delete(Vec<u8>),
}

/// Proptest strategies. Keys come from a small alphabet so that overwrites
/// and deletes are frequent.
pub mod strategies {
    use proptest::prelude::*;

    use super::Op;

    /// Keys of 1 or 2 bytes, each byte in `0..4`.
    pub fn key() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(0u8..4, 1..=2)
    }

    /// Values of up to 16 arbitrary bytes. Empty values are included.
    pub fn value() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(any::<u8>(), 0..=16)
    }

    /// A put (three in four) or a delete.
    pub fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => (key(), value()).prop_map(|(k, v)| Op::Put(k, v)),
            1 => key().prop_map(Op::Delete),
        ]
    }
}

static SESSIONS: AtomicU64 = AtomicU64::new(1);

/// A session prefix that is different on every call, as the store requires for
/// every open. The counter is process-wide, so two opens never share one.
pub fn fresh_session() -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&SESSIONS.fetch_add(1, Ordering::SeqCst).to_le_bytes());
    out
}

/// Opens an empty in-memory store for `node_id`, with the test key and a fresh
/// session.
pub fn mem_store(node_id: u128) -> MemStore {
    reopen_mem(VecStorage::new(), node_id)
}

/// Opens an in-memory store over existing bytes, with a fresh session. Used to
/// reopen a store in a test.
pub fn reopen_mem(storage: VecStorage, node_id: u128) -> MemStore {
    Hyle::open_with_session(storage, node_id, Key::from_bytes(TEST_KEY), fresh_session())
        .expect("reopening a store with the test key cannot fail")
}
