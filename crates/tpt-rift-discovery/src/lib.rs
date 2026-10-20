#![forbid(unsafe_code)]
//! LAN discovery for tpt-rift peers, by UDP.
//!
//! A node periodically announces its node id and the port it serves on. A
//! listener collects announcements from the LAN.
//!
//! Announcements are not authenticated. They say only "a node with this id
//! listens on this port". Anyone on the network can send a false one. That is
//! harmless, because connecting still requires the bearer token from the
//! server's configuration. Discovery finds candidates. It does not trust them.
//!
//! The format is 23 bytes: magic `TPTD` (4), version `1` (1), node id (16,
//! little-endian), port (2, little-endian). See `docs/discovery.md`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;

/// Default UDP port for announcements.
pub const DEFAULT_PORT: u16 = 47813;
/// Magic bytes at the start of every announcement.
pub const MAGIC: [u8; 4] = *b"TPTD";
/// Version of this encoding.
pub const VERSION: u8 = 1;
/// Encoded size of an announcement.
pub const ANNOUNCE_LEN: usize = 23;

/// One node's announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Announce {
    /// The node's id.
    pub node_id: u128,
    /// The TCP port the node serves on. The address comes from the packet source.
    pub port: u16,
}

/// Errors from decoding an announcement.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The packet is not exactly [`ANNOUNCE_LEN`] bytes.
    BadLength,
    /// The magic bytes do not match.
    BadMagic,
    /// The version is not one this build understands.
    BadVersion,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::BadLength => write!(f, "announcement has the wrong length"),
            DecodeError::BadMagic => write!(f, "announcement magic does not match"),
            DecodeError::BadVersion => write!(f, "unsupported announcement version"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl Announce {
    /// Serializes the announcement.
    pub fn encode(&self) -> [u8; ANNOUNCE_LEN] {
        let mut out = [0u8; ANNOUNCE_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5..21].copy_from_slice(&self.node_id.to_le_bytes());
        out[21..23].copy_from_slice(&self.port.to_le_bytes());
        out
    }

    /// Parses an announcement. Rejects anything that is not exactly one.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != ANNOUNCE_LEN {
            return Err(DecodeError::BadLength);
        }
        if bytes[..4] != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if bytes[4] != VERSION {
            return Err(DecodeError::BadVersion);
        }
        let mut id = [0u8; 16];
        id.copy_from_slice(&bytes[5..21]);
        let mut port = [0u8; 2];
        port.copy_from_slice(&bytes[21..23]);
        Ok(Self {
            node_id: u128::from_le_bytes(id),
            port: u16::from_le_bytes(port),
        })
    }
}

/// A peer seen on the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    /// Address the announcement came from, with the announced port.
    pub addr: SocketAddr,
    /// The node's id.
    pub node_id: u128,
    /// When the most recent announcement arrived.
    pub last_seen: Instant,
}

/// Sends `announce` to `target` every `interval` until the task is dropped.
///
/// For a LAN, `target` is the broadcast address, for example
/// `255.255.255.255:47813`. Set broadcast on the socket first, which this does.
pub async fn announce_forever(
    announce: Announce,
    target: SocketAddr,
    interval: Duration,
) -> std::io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;
    let packet = announce.encode();
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        socket.send_to(&packet, target).await?;
    }
}

/// Collects announcements from the network. Keeps the latest per node id.
pub struct Listener {
    socket: UdpSocket,
    peers: BTreeMap<u128, Peer>,
}

impl Listener {
    /// Binds to `addr`, usually `0.0.0.0:47813`.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        Ok(Self {
            socket: UdpSocket::bind(addr).await?,
            peers: BTreeMap::new(),
        })
    }

    /// The local address the listener is bound to.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Waits for one valid announcement, records it, and returns it.
    /// Malformed packets are skipped.
    pub async fn next_peer(&mut self) -> std::io::Result<Peer> {
        let mut buf = [0u8; 64];
        loop {
            let (n, from) = self.socket.recv_from(&mut buf).await?;
            let Ok(announce) = Announce::decode(&buf[..n]) else {
                continue;
            };
            let peer = Peer {
                addr: SocketAddr::new(from.ip(), announce.port),
                node_id: announce.node_id,
                last_seen: Instant::now(),
            };
            self.peers.insert(announce.node_id, peer);
            return Ok(peer);
        }
    }

    /// Peers seen so far, by node id.
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.values()
    }
}
