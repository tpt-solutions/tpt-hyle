#![forbid(unsafe_code)]
//! Reference sync server for tpt-rift.
//!
//! The server holds one hub replica, with its own node id, and gives each
//! client connection a per-peer cursor into the hub's log. Clients
//! authenticate with a bearer token. Each token is bound to one node id, and
//! carries key-prefix lists for reading and writing.
//!
//! Rules applied to every delta from a client:
//!
//! - A change whose node id is not the token's node id is skipped. It is a
//!   forwarded write or a spoofed id, and the client is not entitled to
//!   vouch for either.
//! - A change from the client's own node whose key is outside its write
//!   prefixes closes the connection.
//! - Export to a client includes only keys it may read.
//!
//! The server does not terminate TLS. Run it behind a TLS-terminating reverse
//! proxy whenever it is reachable from outside a trusted network.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tpt_hyle::{FileStorage, Hyle};
use tpt_rift::{Delta, PeerLink, Replica, RiftError, Transport};
use tpt_rift_transport_ws::{accept_with, WebSocketTransport};

/// Largest accepted message from a client, unless the config says otherwise.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 1 << 20;
/// How often each connection checks for new changes, unless configured.
pub const DEFAULT_POLL_MS: u64 = 25;
/// Events read per export. A longer backlog goes out over several polls.
const EXPORT_BATCH: usize = 1024;

pub type Hub = Arc<Mutex<Replica<FileStorage, FileStorage>>>;

/// Server configuration, as read from JSON.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Base path of the hub store, as passed to `Hyle::open_file`.
    pub store: PathBuf,
    /// Address to listen on, for example `127.0.0.1:8080`.
    pub listen: String,
    /// Hub node id, in decimal. Must differ from every token's node id.
    pub node_id: String,
    /// Largest accepted message from a client.
    #[serde(default = "default_max_message_bytes")]
    pub max_message_bytes: usize,
    /// Poll interval for each connection, in milliseconds.
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
    /// One entry per client.
    pub tokens: Vec<TokenConfig>,
}

fn default_max_message_bytes() -> usize {
    DEFAULT_MAX_MESSAGE_BYTES
}

fn default_poll_ms() -> u64 {
    DEFAULT_POLL_MS
}

/// One client's credentials and permissions.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenConfig {
    /// SHA-256 of the bearer token, as 64 hex digits. The token itself is
    /// never stored.
    pub sha256: String,
    /// The client's node id, in decimal.
    pub node_id: String,
    /// Key prefixes the client may read. Empty means nothing.
    #[serde(default)]
    pub read: Vec<String>,
    /// Key prefixes the client may write. Empty means nothing.
    #[serde(default)]
    pub write: Vec<String>,
}

/// What one authenticated client may do.
#[derive(Debug, Clone)]
pub struct Auth {
    /// The node id this client writes under.
    pub node_id: u128,
    read: Vec<Vec<u8>>,
    write: Vec<Vec<u8>>,
}

impl Auth {
    /// `true` if the client may read `key`.
    pub fn can_read(&self, key: &[u8]) -> bool {
        self.read.iter().any(|p| key.starts_with(p))
    }

    /// `true` if the client may write `key`.
    pub fn can_write(&self, key: &[u8]) -> bool {
        self.write.iter().any(|p| key.starts_with(p))
    }
}

/// Errors from reading or checking a configuration.
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config error: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// The token table: hashes mapped to permissions.
#[derive(Debug, Clone)]
pub struct Access {
    entries: Vec<([u8; 32], Auth)>,
}

impl Access {
    /// Builds the table. Checks that node ids are unique and distinct from the
    /// hub's. Returns the hub node id along with the table.
    pub fn from_config(config: &Config) -> Result<(u128, Self), ConfigError> {
        let hub = parse_u128(&config.node_id, "node_id")?;
        let mut seen = vec![hub];
        let mut entries = Vec::with_capacity(config.tokens.len());
        for t in &config.tokens {
            let hash = parse_hash(&t.sha256)?;
            let node_id = parse_u128(&t.node_id, "tokens[].node_id")?;
            if seen.contains(&node_id) {
                return Err(ConfigError(format!(
                    "node id {node_id} is used more than once"
                )));
            }
            seen.push(node_id);
            let auth = Auth {
                node_id,
                read: t.read.iter().map(|p| p.as_bytes().to_vec()).collect(),
                write: t.write.iter().map(|p| p.as_bytes().to_vec()).collect(),
            };
            entries.push((hash, auth));
        }
        Ok((hub, Self { entries }))
    }

    /// Returns the permissions for `token`, or `None`. Every entry is compared,
    /// so timing does not reveal which one matched.
    pub fn authenticate(&self, token: &str) -> Option<Auth> {
        let digest = sha256(token.as_bytes());
        let mut found = None;
        for (hash, auth) in &self.entries {
            if constant_time_eq(hash, &digest) {
                found = Some(auth.clone());
            }
        }
        found
    }
}

/// Keeps the changes a client may send, applying the rules in the crate docs.
pub fn screen_incoming(delta: Delta, auth: &Auth) -> Result<Delta, RiftError> {
    let mut kept = Vec::with_capacity(delta.changes.len());
    for change in delta.changes {
        if change.node_id != auth.node_id {
            continue;
        }
        if !auth.can_write(&change.key) {
            return Err(RiftError::Protocol(
                "write outside the permitted key prefixes",
            ));
        }
        kept.push(change);
    }
    Ok(Delta {
        generation: delta.generation,
        changes: kept,
    })
}

/// Serves one authenticated connection until it closes or breaks a rule.
pub async fn serve_connection(
    conn: WebSocketTransport<TcpStream>,
    auth: Auth,
    hub: Hub,
    poll: Duration,
) -> Result<(), RiftError> {
    let mut link = {
        let hub = hub.lock().await;
        PeerLink::new(auth.node_id, &hub)
    };
    loop {
        while let Some(delta) = conn.receive().await? {
            let accepted = screen_incoming(delta, &auth)?;
            if !accepted.changes.is_empty() {
                hub.lock().await.import(&accepted)?;
            }
        }

        let export = {
            let mut hub = hub.lock().await;
            match hub.export(auth.node_id, link.cursor, EXPORT_BATCH) {
                Err(RiftError::StaleGeneration { .. }) => {
                    link.reset(&hub);
                    None
                }
                other => Some(other?),
            }
        };
        if let Some(export) = export {
            link.cursor = export.next;
            let changes: Vec<_> = export
                .delta
                .changes
                .into_iter()
                .filter(|c| auth.can_read(&c.key))
                .collect();
            if !changes.is_empty() {
                conn.send(Delta {
                    generation: export.delta.generation,
                    changes,
                })
                .await?;
            }
        }

        tokio::time::sleep(poll).await;
    }
}

/// Runs the server until the listener fails.
pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let (hub_node, access) = Access::from_config(&config)?;
    let store = Hyle::open_file(&config.store, hub_node)?;
    let hub: Hub = Arc::new(Mutex::new(Replica::new(store)));
    let listener = TcpListener::bind(&config.listen).await?;
    let poll = Duration::from_millis(config.poll_ms);
    accept_loop(listener, access, hub, poll, config.max_message_bytes).await;
    Ok(())
}

/// Accepts connections on `listener` and serves each one on its own task.
/// Runs until the listener fails.
pub async fn accept_loop(
    listener: TcpListener,
    access: Access,
    hub: Hub,
    poll: Duration,
    max_bytes: usize,
) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                eprintln!("listener failed: {e}");
                return;
            }
        };
        let access = access.clone();
        let hub = hub.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, access, hub, poll, max_bytes).await {
                eprintln!("connection from {peer} ended: {e}");
            }
        });
    }
}

// The callback error type is fixed by tungstenite, and its size cannot be
// reduced here.
#[allow(clippy::result_large_err)]
async fn handle(
    stream: TcpStream,
    access: Access,
    hub: Hub,
    poll: Duration,
    max_bytes: usize,
) -> Result<(), RiftError> {
    let mut granted: Option<Auth> = None;
    let conn = accept_with(stream, max_bytes, |request, response| {
        let token = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        match token.and_then(|t| access.authenticate(t)) {
            Some(auth) => {
                granted = Some(auth);
                Ok(response)
            }
            None => Err(unauthorized()),
        }
    })
    .await?;
    let auth = granted.ok_or(RiftError::Protocol(
        "handshake accepted without credentials",
    ))?;
    serve_connection(conn, auth, hub, poll).await
}

fn unauthorized() -> tokio_tungstenite::tungstenite::handshake::server::ErrorResponse {
    tokio_tungstenite::tungstenite::http::Response::builder()
        .status(401)
        .body(Some("unauthorized".to_string()))
        .expect("a static response always builds")
}

/// Reads and parses a configuration file.
pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("reading {}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| ConfigError(format!("parsing {}: {e}", path.display())))
}

/// Generates a fresh bearer token. Returns the token, to hand to the client
/// once, and its SHA-256 hash, to put in the config.
pub fn new_token() -> (String, String) {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hash = hash_token(&token);
    (token, hash)
}

/// SHA-256 of a token, as lowercase hex.
pub fn hash_token(token: &str) -> String {
    sha256(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn parse_u128(text: &str, field: &str) -> Result<u128, ConfigError> {
    text.parse()
        .map_err(|_| ConfigError(format!("{field} must be a decimal u128, got {text:?}")))
}

fn parse_hash(text: &str) -> Result<[u8; 32], ConfigError> {
    let bad = || ConfigError("sha256 must be 64 hex digits".into());
    if text.len() != 64 || !text.is_ascii() {
        return Err(bad());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
    }
    Ok(out)
}
