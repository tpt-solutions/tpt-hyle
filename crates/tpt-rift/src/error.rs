//! Errors for replication.

use core::fmt;

/// Everything that can go wrong while exchanging deltas.
#[derive(Debug)]
#[non_exhaustive]
pub enum RiftError {
    /// The local store reported an error.
    Store(tpt_hyle::Error),
    /// A cursor belongs to an older generation. The log was compacted since,
    /// so the peer has to resynchronize from the start. See
    /// [`PeerLink::reset`](crate::PeerLink::reset).
    StaleGeneration {
        /// Generation the cursor was taken from.
        cursor: u32,
        /// Generation of the local log now.
        current: u32,
    },
    /// The transport failed to move a message.
    Transport(String),
    /// A received message is malformed or breaks the wire rules.
    Protocol(&'static str),
    /// A document could not be encoded or decoded as JSON.
    Json(String),
}

impl fmt::Display for RiftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RiftError::Store(e) => write!(f, "store error: {e}"),
            RiftError::StaleGeneration { cursor, current } => write!(
                f,
                "peer cursor is from generation {cursor}, the log is now generation {current}"
            ),
            RiftError::Transport(msg) => write!(f, "transport error: {msg}"),
            RiftError::Protocol(what) => write!(f, "protocol error: {what}"),
            RiftError::Json(msg) => write!(f, "JSON error: {msg}"),
        }
    }
}

impl std::error::Error for RiftError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RiftError::Store(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tpt_hyle::Error> for RiftError {
    fn from(e: tpt_hyle::Error) -> Self {
        RiftError::Store(e)
    }
}
