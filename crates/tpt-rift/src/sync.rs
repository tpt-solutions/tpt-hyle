//! One round of sync with one peer.

use tpt_hyle::Storage;

use crate::replica::{PeerCursor, Replica};
use crate::transport::Transport;
use crate::wire::{Ack, Delta};
use crate::RiftError;

/// Tracks one peer: its node id, how far it has read our log, and what it
/// has acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerLink {
    /// The peer's node id. Events from this node are not sent back to it.
    pub peer_node: u128,
    /// How far the peer has read our log.
    pub cursor: PeerCursor,
    /// The latest `caught_up` marker the peer sent us, as `(generation, offset)`.
    caught_up: Option<(u32, u64)>,
    /// The last acknowledgement we sent, with the generation it refers to.
    last_ack: Option<(u32, Ack)>,
}

impl PeerLink {
    /// A link that has never synced. It starts at the beginning of the
    /// replica's current generation.
    pub fn new<S: Storage>(peer_node: u128, replica: &Replica<S>) -> Self {
        Self {
            peer_node,
            cursor: replica.start_cursor(),
            caught_up: None,
            last_ack: None,
        }
    }

    /// Restarts from the beginning of the current generation. Call this after
    /// [`RiftError::StaleGeneration`]. The peer then receives the whole
    /// current log again, which is safe because import is idempotent.
    /// Markers and acks from the old generation no longer apply.
    pub fn reset<S: Storage>(&mut self, replica: &Replica<S>) {
        self.cursor = replica.start_cursor();
        self.caught_up = None;
        self.last_ack = None;
    }
}

/// What one round did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RoundStats {
    /// Changes sent to the peer.
    pub sent: usize,
    /// Incoming changes that won and were applied.
    pub applied: usize,
    /// Incoming changes that were skipped.
    pub skipped: usize,
}

/// Runs one round: export to the peer, then drain whatever the peer has sent.
///
/// Export runs up to `max_events` events. The link's cursor advances only
/// after the delta is sent, so a failed send is retried from the same place.
/// If the peer's cursor is stale, the error is returned, and the caller
/// should call [`PeerLink::reset`] and try again.
///
/// When this round's export reaches the end of our log, and the peer has
/// sent a `caught_up` marker, the round ends with an [`Ack`] for that marker.
/// The ack is sent once per marker, after the export it covers.
pub async fn sync_round<S: Storage, T: Transport>(
    replica: &mut Replica<S>,
    link: &mut PeerLink,
    transport: &T,
    max_events: usize,
) -> Result<RoundStats, RiftError> {
    let mut stats = RoundStats::default();
    // Read before exporting. Any write with a lamport at or below this was
    // made before the export, so a complete export has sent it.
    let lamport = replica.store_ref().lamport();

    let export = replica.export(link.peer_node, link.cursor, max_events)?;
    let complete = export.next.offset == replica.store_ref().sync_end();
    if !export.delta.changes.is_empty() {
        stats.sent = export.delta.changes.len();
        transport.send(export.delta).await?;
    }
    link.cursor = export.next;

    while let Some(delta) = transport.receive().await? {
        let s = replica.import(&delta)?;
        stats.applied += s.applied;
        stats.skipped += s.skipped;
        if let Some(offset) = delta.caught_up {
            link.caught_up = Some((delta.generation, offset));
        }
    }

    if let Some((generation, offset)) = link.caught_up {
        let ack = Ack { offset, lamport };
        if complete && link.last_ack != Some((generation, ack)) {
            transport
                .send(Delta {
                    generation,
                    changes: Vec::new(),
                    caught_up: None,
                    ack: Some(ack),
                })
                .await?;
            link.last_ack = Some((generation, ack));
        }
    }
    Ok(stats)
}

/// Exponential backoff for retries, capped at `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Delay after the first failure.
    pub base: std::time::Duration,
    /// Upper bound on any delay.
    pub max: std::time::Duration,
}

impl Backoff {
    /// Delay before retry number `failures` (1 for the first retry).
    /// Doubles each time, and never exceeds `max`.
    pub fn delay(&self, failures: u32) -> std::time::Duration {
        let shift = failures.saturating_sub(1).min(30);
        let factor = 1u32 << shift;
        self.base.saturating_mul(factor).min(self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::Backoff;
    use std::time::Duration;

    #[test]
    fn backoff_doubles_then_caps() {
        let b = Backoff {
            base: Duration::from_millis(100),
            max: Duration::from_secs(5),
        };
        assert_eq!(b.delay(1), Duration::from_millis(100));
        assert_eq!(b.delay(2), Duration::from_millis(200));
        assert_eq!(b.delay(3), Duration::from_millis(400));
        assert_eq!(b.delay(10), Duration::from_secs(5));
        assert_eq!(b.delay(u32::MAX), Duration::from_secs(5));
        assert_eq!(b.delay(0), Duration::from_millis(100));
    }
}
