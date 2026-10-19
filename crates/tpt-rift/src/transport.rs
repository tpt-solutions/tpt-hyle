//! The transport trait. Moves deltas between replicas. It does not know how
//! they are encoded or applied.

use async_trait::async_trait;

use crate::wire::Delta;
use crate::RiftError;

/// Carries deltas to and from one peer.
///
/// Implementations choose how bytes move: a WebSocket, WebRTC, a file, or an
/// in-process queue. Deltas are self-contained and idempotent to import, so
/// a transport may deliver them late, twice, or out of order.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Opens the connection. Default is a no-op for transports that need none.
    async fn connect(&self) -> Result<(), RiftError> {
        Ok(())
    }

    /// Closes the connection. Default is a no-op.
    async fn disconnect(&self) -> Result<(), RiftError> {
        Ok(())
    }

    /// Sends one delta to the peer.
    async fn send(&self, delta: Delta) -> Result<(), RiftError>;

    /// Returns the next delta the peer has sent that is waiting to be read,
    /// or `None` if there is none right now.
    async fn receive(&self) -> Result<Option<Delta>, RiftError>;
}
