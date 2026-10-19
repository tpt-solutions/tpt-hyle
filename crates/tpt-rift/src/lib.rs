#![forbid(unsafe_code)]
//! CRDT sync engine that replicates directly from the tpt-hyle Sync Segment.
//!
//! Each replica exports the events it has written since a peer's cursor. The
//! receiving side applies them with last-writer-wins by `(lamport, node_id)`.
//! Applying is idempotent and order-independent, so deltas may arrive late,
//! twice, or out of order. The wire format is in `docs/wire.md`.
//!
//! Transports implement [`Transport`]. [`mock::MockTransport`] is an
//! in-process one for tests.

mod error;
mod mock;
mod replica;
mod sync;
mod transport;
mod wire;

pub use error::RiftError;
pub use mock::MockTransport;
pub use replica::{Export, ImportStats, PeerCursor, Replica};
pub use sync::{sync_round, Backoff, PeerLink, RoundStats};
pub use transport::Transport;
pub use wire::{Change, Delta, MAGIC, MAX_CHANGES, WIRE_VERSION};
