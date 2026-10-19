#![forbid(unsafe_code)]
//! Unified API: tpt-hyle storage plus tpt-rift sync.
//!
//! ```no_run
//! use tpt_hyle_rift::{connect, open};
//! use tpt_rift::MockTransport;
//!
//! # async fn demo() -> Result<(), tpt_rift::RiftError> {
//! let mut db = open("app_data", 1)?;
//! let (transport, _peer_end) = MockTransport::pair();
//! let mut sync = connect(&mut db, 2, transport);
//! sync.resume().await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use tpt_hyle::{FileStorage, Hyle};
use tpt_rift::{sync_round, Backoff, PeerLink, Replica, RiftError, RoundStats, Transport};

/// Maximum events read per round. Larger logs are sent over several rounds.
pub const DEFAULT_MAX_EVENTS: usize = 4096;

/// A local, file-backed replica. Use it for reads and writes, as with
/// [`tpt_hyle::Hyle`].
pub type Database = Replica<FileStorage, FileStorage>;

/// Opens (or creates) the database at `path` for `node_id`.
///
/// Files are created beside `path`. See [`tpt_hyle::Hyle::open_file`]. Each
/// node id must be used by one writer only.
pub fn open(path: impl AsRef<Path>, node_id: u128) -> Result<Database, RiftError> {
    Ok(Replica::new(Hyle::open_file(path, node_id)?))
}

/// A sync session with one peer.
pub struct Connection<'a, T> {
    replica: &'a mut Database,
    link: PeerLink,
    transport: T,
    max_events: usize,
}

/// Starts syncing `db` with the peer identified by `peer_node` over
/// `transport`. Nothing is sent until [`Connection::resume`] runs.
pub fn connect<T: Transport>(
    db: &mut Database,
    peer_node: u128,
    transport: T,
) -> Connection<'_, T> {
    let link = PeerLink::new(peer_node, db);
    Connection {
        replica: db,
        link,
        transport,
        max_events: DEFAULT_MAX_EVENTS,
    }
}

impl<T: Transport> Connection<'_, T> {
    /// Sets the most events exported per round.
    pub fn max_events(mut self, max_events: usize) -> Self {
        self.max_events = max_events.max(1);
        self
    }

    /// Opens the transport. Call once before the first [`resume`](Self::resume).
    pub async fn open(&self) -> Result<(), RiftError> {
        self.transport.connect().await
    }

    /// Runs one sync round.
    ///
    /// If the peer's position is stale because the local log was compacted,
    /// the position is reset and the round is retried once. Everything in the
    /// log is then resent, which is safe because import is idempotent.
    pub async fn resume(&mut self) -> Result<RoundStats, RiftError> {
        match sync_round(
            self.replica,
            &mut self.link,
            &self.transport,
            self.max_events,
        )
        .await
        {
            Err(RiftError::StaleGeneration { .. }) => {
                self.link.reset(self.replica);
                sync_round(
                    self.replica,
                    &mut self.link,
                    &self.transport,
                    self.max_events,
                )
                .await
            }
            other => other,
        }
    }

    /// Closes the transport.
    pub async fn close(&self) -> Result<(), RiftError> {
        self.transport.disconnect().await
    }

    /// Runs sync rounds on a timer until `stop` completes.
    ///
    /// A round runs every `interval` after a success. After a transport or
    /// protocol failure the loop waits `backoff` instead, growing with each
    /// consecutive failure. A store error stops the loop and is returned,
    /// because retrying cannot fix a damaged or poisoned store.
    ///
    /// This is timer-driven only. Connectivity-triggered rounds are the
    /// caller's job for now.
    pub async fn run_until<F: Future<Output = ()>>(
        &mut self,
        interval: Duration,
        backoff: Backoff,
        stop: F,
    ) -> Result<(), RiftError> {
        tokio::pin!(stop);
        let mut failures = 0u32;
        loop {
            let wait = match self.resume().await {
                Ok(_) => {
                    failures = 0;
                    interval
                }
                Err(RiftError::Store(e)) => return Err(RiftError::Store(e)),
                Err(_) => {
                    failures = failures.saturating_add(1);
                    backoff.delay(failures)
                }
            };
            tokio::select! {
                _ = &mut stop => return Ok(()),
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }
}
