#![forbid(unsafe_code)]
//! Unified API: tpt-hyle storage plus tpt-rift sync.
//!
//! ```no_run
//! use tpt_hyle_rift::{connect, open};
//! use tpt_rift::MockTransport;
//!
//! # async fn demo() -> Result<(), tpt_rift::RiftError> {
//! let mut db = open("app_data", 1, b"a passphrase")?;
//! let (transport, _peer_end) = MockTransport::pair();
//! let mut sync = connect(&mut db, 2, transport);
//! sync.resume().await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tpt_hyle::{FileStorage, Hyle};
use tpt_rift::{sync_round, Backoff, PeerLink, Replica, RiftError, RoundStats, Transport};

/// Maximum events read per round. Larger logs are sent over several rounds.
pub const DEFAULT_MAX_EVENTS: usize = 4096;

/// A local, file-backed replica. Use it for reads and writes, as with
/// [`tpt_hyle::Hyle`].
pub type Database = Replica<FileStorage>;

/// Wakes a running sync loop so it syncs now instead of at the next tick.
///
/// Call [`Trigger::notify`] after a local write. Several notifications that
/// arrive before the loop wakes count as one round.
#[derive(Clone, Default)]
pub struct Trigger(Arc<Notify>);

impl Trigger {
    /// A new trigger, not yet connected to any loop.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests a round. Never blocks.
    pub fn notify(&self) {
        self.0.notify_one();
    }

    async fn notified(&self) {
        self.0.notified().await;
    }
}

/// Opens (or creates) the database at `path` for `node_id`.
///
/// Files are created beside `path`. See [`tpt_hyle::Hyle::open_file`]. Each
/// node id must be used by one writer only.
pub fn open(
    path: impl AsRef<Path>,
    node_id: u128,
    passphrase: &[u8],
) -> Result<Database, RiftError> {
    Ok(Replica::new(Hyle::open_file(path, node_id, passphrase)?))
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
    /// This loop never reconnects. For a transport that can drop, use
    /// [`run_reconnecting`].
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

/// How often and how hard a reconnecting session syncs.
#[derive(Debug, Clone, Copy)]
pub struct SyncPolicy {
    /// Time between rounds when nothing triggers one.
    pub interval: Duration,
    /// Delays between attempts after a failure.
    pub backoff: Backoff,
    /// Most events exported per round.
    pub max_events: usize,
}

impl Default for SyncPolicy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            backoff: Backoff {
                base: Duration::from_millis(250),
                max: Duration::from_secs(60),
            },
            max_events: DEFAULT_MAX_EVENTS,
        }
    }
}

/// Keeps a session with one peer alive until `stop` completes.
///
/// `connect` opens a fresh transport. It runs at the start and after every
/// failure, with backoff between attempts. Within a connection, a round runs
/// on the policy's interval, or as soon as `trigger` is notified. Each new
/// connection resends the whole log. That is safe because import is
/// idempotent, and it covers anything the dead connection may have dropped
/// after `send` returned.
///
/// A store error stops the loop and is returned.
pub async fn run_reconnecting<T, C, Fut, S>(
    db: &mut Database,
    peer_node: u128,
    mut connect: C,
    policy: SyncPolicy,
    trigger: &Trigger,
    stop: S,
) -> Result<(), RiftError>
where
    T: Transport,
    C: FnMut() -> Fut,
    Fut: Future<Output = Result<T, RiftError>>,
    S: Future<Output = ()>,
{
    tokio::pin!(stop);
    let mut link = PeerLink::new(peer_node, db);
    let mut failures = 0u32;
    loop {
        let session_failure = match connect().await {
            Ok(transport) => {
                link.reset(db);
                loop {
                    match sync_round(db, &mut link, &transport, policy.max_events).await {
                        Ok(_) => failures = 0,
                        Err(RiftError::Store(e)) => return Err(RiftError::Store(e)),
                        Err(RiftError::StaleGeneration { .. }) => link.reset(db),
                        Err(e) => break e,
                    }
                    tokio::select! {
                        _ = &mut stop => return Ok(()),
                        _ = tokio::time::sleep(policy.interval) => {}
                        _ = trigger.notified() => {}
                    }
                }
            }
            Err(RiftError::Store(e)) => return Err(RiftError::Store(e)),
            Err(e) => e,
        };
        failures = failures.saturating_add(1);
        let _ = session_failure;
        tokio::select! {
            _ = &mut stop => return Ok(()),
            _ = tokio::time::sleep(policy.backoff.delay(failures)) => {}
        }
    }
}
