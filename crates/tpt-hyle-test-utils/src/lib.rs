//! In-memory store, mock transports and proptest generators for the tpt-hyle
//! workspace.
//!
//! Mock transports for `tpt-rift` arrive with Phase 2. What is here now:
//! an in-memory store, a model-checking [`Op`] type, and proptest strategies.

#![forbid(unsafe_code)]

use tpt_hyle::{Hyle, VecStorage};

/// A store backed entirely by memory. Nothing touches the disk.
pub type MemStore = Hyle<VecStorage, VecStorage>;

/// Opens an empty in-memory store for `node_id`.
pub fn mem_store(node_id: u128) -> MemStore {
    Hyle::open(VecStorage::new(), VecStorage::new(), node_id)
        .expect("opening an empty in-memory store cannot fail")
}

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
