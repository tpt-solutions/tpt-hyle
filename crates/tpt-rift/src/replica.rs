//! A replica: a local store plus the logic to export and import deltas.

use tpt_hyle::{Hyle, Storage, HEADER_LEN};

use crate::directory::Directory;

use crate::wire::{Change, Delta};
use crate::RiftError;

/// Where a peer has read up to in *our* log.
///
/// The cursor is valid only for the generation it was taken from. After a
/// compaction, the peer starts again from the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCursor {
    /// Generation of the log the offset belongs to.
    pub generation: u32,
    /// Byte offset of the next unread sync record.
    pub offset: u64,
}

/// What an export produced, and where the cursor moves to once it is sent.
#[derive(Debug, Clone)]
pub struct Export {
    /// Changes to send. May be empty when every event in range came from the
    /// peer itself.
    pub delta: Delta,
    /// The cursor to store after the delta has been delivered.
    pub next: PeerCursor,
}

/// Counts from one import.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats {
    /// Changes that won and were appended.
    pub applied: usize,
    /// Changes that lost to a newer local write, or were already applied.
    pub skipped: usize,
}

/// A store taking part in replication.
pub struct Replica<S: Storage> {
    store: Hyle<S>,
    directory: Option<Directory>,
}

impl<S: Storage> Replica<S> {
    /// Wraps a store.
    pub fn new(store: Hyle<S>) -> Self {
        Self {
            store,
            directory: None,
        }
    }

    /// Requires every incoming event to come from a node in `directory`, with a
    /// valid signature. Checked before anything in the delta is applied.
    pub fn set_directory(&mut self, directory: Directory) {
        self.directory = Some(directory);
    }

    /// Lists `node_id` with `public_key` in this replica's directory, and turns
    /// checking on. The first call also lists this replica's own node, so events
    /// it wrote still pass when a peer sends them back.
    pub fn trust(&mut self, node_id: u128, public_key: [u8; 32]) -> Result<(), RiftError> {
        let own = self.node_id();
        let own_key = self.public_key();
        let directory = self.directory.get_or_insert_with(Directory::new);
        if directory.get(own).is_none() {
            directory.insert(own, own_key)?;
        }
        directory.insert(node_id, public_key)
    }

    /// This replica's Ed25519 public key. Peers list it to accept its events.
    pub fn public_key(&self) -> [u8; 32] {
        self.store.public_key()
    }

    /// The underlying store, for local reads and writes.
    pub fn store_ref(&self) -> &Hyle<S> {
        &self.store
    }

    /// The underlying store, for local reads and writes.
    pub fn store(&mut self) -> &mut Hyle<S> {
        &mut self.store
    }

    /// Unwraps the store.
    pub fn into_store(self) -> Hyle<S> {
        self.store
    }

    /// The node id this replica writes under.
    pub fn node_id(&self) -> u128 {
        self.store.node_id()
    }

    /// A cursor at the start of the current generation, used for a peer that
    /// has never synced or must resynchronize in full.
    pub fn start_cursor(&self) -> PeerCursor {
        PeerCursor {
            generation: self.store.generation(),
            offset: HEADER_LEN,
        }
    }

    /// Collects up to `max_events` sync events after `cursor`, skipping any
    /// that originated at `peer_node`. Skipping avoids echoing a peer's own
    /// writes back to it. The returned cursor moves past skipped events too.
    pub fn export(
        &mut self,
        peer_node: u128,
        cursor: PeerCursor,
        max_events: usize,
    ) -> Result<Export, RiftError> {
        let current = self.store.generation();
        if cursor.generation != current {
            return Err(RiftError::StaleGeneration {
                cursor: cursor.generation,
                current,
            });
        }

        let events: Vec<_> = self
            .store
            .events_since(cursor.offset)?
            .take(max_events)
            .collect::<Result<_, _>>()?;

        // Fewer events than asked for means the log is exhausted. The cursor then
        // moves to the true end, past any trailing commit frames, which carry no
        // events. A caught-up check compares against that end.
        let exhausted = events.len() < max_events;
        let mut next = cursor.offset;
        let mut changes = Vec::new();
        for (end, ev) in events {
            next = end;
            if ev.node_id == peer_node {
                continue;
            }
            let mut key = vec![0u8; ev.key_len as usize];
            let mut value = vec![0u8; ev.payload_len as usize];
            self.store.read_record(&ev, &mut key, &mut value)?;
            let signature = self.store.event_signature(&ev)?;
            changes.push(Change {
                key,
                value,
                lamport: ev.lamport,
                node_id: ev.node_id,
                tombstone: ev.tombstone,
                signature,
            });
        }
        if exhausted {
            next = self.store.sync_end();
        }
        debug_assert!(next >= cursor.offset);

        Ok(Export {
            delta: Delta {
                generation: current,
                changes,
                caught_up: None,
                ack: None,
            },
            next: PeerCursor {
                generation: current,
                offset: next,
            },
        })
    }

    /// Applies a delta with last-writer-wins by `(lamport, node_id)`.
    ///
    /// A change is applied only if it is strictly greater than the current
    /// latest event for its key. Anything equal or smaller is skipped. That
    /// makes import idempotent, so duplicates and reordering are harmless.
    /// Once every replica has seen the same set of changes, they agree.
    pub fn import(&mut self, delta: &Delta) -> Result<ImportStats, RiftError> {
        let mut stats = ImportStats::default();
        if delta.changes.is_empty() {
            return Ok(stats);
        }
        if let Some(directory) = &self.directory {
            for ch in &delta.changes {
                let key = directory.get(ch.node_id).ok_or(RiftError::Protocol(
                    "change from a node not in the directory",
                ))?;
                if !tpt_hyle::verify_event(
                    key,
                    ch.node_id,
                    ch.lamport,
                    ch.tombstone,
                    &ch.key,
                    &ch.value,
                    &ch.signature,
                ) {
                    return Err(RiftError::Protocol("event signature does not verify"));
                }
            }
        }
        // One batch per delta: it lands whole or not at all. The batch sees
        // its own earlier writes, so two changes to one key compare correctly.
        let mut batch = self.store.batch()?;
        for ch in &delta.changes {
            let wins = match batch.latest_event(&ch.key)? {
                Some(cur) => (ch.lamport, ch.node_id) > (cur.lamport, cur.node_id),
                None => true,
            };
            if wins {
                batch.put_remote(
                    &ch.key,
                    &ch.value,
                    ch.lamport,
                    ch.node_id,
                    ch.tombstone,
                    ch.signature,
                )?;
                stats.applied += 1;
            } else {
                stats.skipped += 1;
            }
        }
        // Forced, so the clock is persisted even when every change lost.
        batch.commit_forced()?;
        Ok(stats)
    }
}
