//! The Ed25519 public keys of the nodes a replica accepts events from.

use std::collections::BTreeMap;

use crate::RiftError;

/// Public keys by node id. A replica with a directory accepts an event only if
/// its node is listed and its signature verifies. See
/// [`Replica::set_directory`](crate::Replica::set_directory).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Directory {
    keys: BTreeMap<u128, [u8; 32]>,
}

impl Directory {
    /// An empty directory. It accepts nothing until nodes are added.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lists `node_id` with `public_key`. Listing the same node twice with the
    /// same key is fine. A different key for a listed node is refused, since
    /// that would let one of two keys sign for the node.
    pub fn insert(&mut self, node_id: u128, public_key: [u8; 32]) -> Result<(), RiftError> {
        match self.keys.get(&node_id) {
            Some(existing) if *existing != public_key => {
                Err(RiftError::Protocol("node listed twice with different keys"))
            }
            _ => {
                self.keys.insert(node_id, public_key);
                Ok(())
            }
        }
    }

    /// The public key for `node_id`, if listed.
    pub fn get(&self, node_id: u128) -> Option<&[u8; 32]> {
        self.keys.get(&node_id)
    }

    /// Number of nodes listed.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// `true` if no node is listed.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}
