use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::MaybeTlsStream;
use tpt_hyle::{Hyle, VecStorage};
use tpt_rift::{Change, Delta, PeerLink, Replica, Transport};
use tpt_rift_server::{accept_loop, hash_token, Access, Config, Hub, TokenConfig};
use tpt_rift_transport_ws::{connect_authorized, WebSocketTransport};

const HUB_NODE: u128 = 1;

struct Client {
    replica: Replica<VecStorage, VecStorage>,
    link: PeerLink,
    transport: WebSocketTransport<MaybeTlsStream<TcpStream>>,
}

impl Client {
    fn has(&mut self, key: &[u8]) -> bool {
        self.replica.store().contains_key(key).unwrap()
    }

    fn value(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let mut buf = [0u8; 64];
        self.replica
            .store()
            .get(key, &mut buf)
            .unwrap()
            .map(|n| buf[..n].to_vec())
    }

    async fn round(&mut self) -> Result<(), tpt_rift::RiftError> {
        tpt_rift::sync_round(&mut self.replica, &mut self.link, &self.transport, 1000)
            .await
            .map(|_| ())
    }
}

/// Starts a hub on an ephemeral port, with the given token entries.
async fn start_hub(tokens: Vec<TokenConfig>) -> std::net::SocketAddr {
    let dir = std::env::temp_dir().join(format!(
        "tpt-rift-server-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let config = Config {
        store: dir.join("hub"),
        listen: "127.0.0.1:0".into(),
        node_id: HUB_NODE.to_string(),
        max_message_bytes: 1 << 20,
        poll_ms: 5,
        tokens,
    };
    let (hub_node, access) = Access::from_config(&config).unwrap();
    let store = Hyle::open_file(&config.store, hub_node).unwrap();
    let hub: Hub = Arc::new(Mutex::new(Replica::new(store)));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(accept_loop(
        listener,
        access,
        hub,
        Duration::from_millis(5),
        1 << 20,
    ));
    addr
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

fn entry(token: &str, node: u128, read: &[&str], write: &[&str]) -> TokenConfig {
    TokenConfig {
        sha256: hash_token(token),
        node_id: node.to_string(),
        read: read.iter().map(|s| s.to_string()).collect(),
        write: write.iter().map(|s| s.to_string()).collect(),
    }
}

async fn client(addr: std::net::SocketAddr, token: &str, node: u128) -> Client {
    let transport = connect_authorized(&format!("ws://{addr}"), token)
        .await
        .unwrap();
    let replica = Replica::new(Hyle::open(VecStorage::new(), VecStorage::new(), node).unwrap());
    let link = PeerLink::new(HUB_NODE, &replica);
    Client {
        replica,
        link,
        transport,
    }
}

/// Runs rounds on both clients until `done` holds, or fails after a deadline.
async fn converge(
    a: &mut Client,
    b: &mut Client,
    mut done: impl FnMut(&mut Client, &mut Client) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let _ = a.round().await;
            let _ = b.round().await;
            if done(a, b) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("clients did not converge in time");
}

#[tokio::test]
async fn a_wrong_token_is_refused() {
    let addr = start_hub(vec![entry("right", 10, &[""], &[""])]).await;
    assert!(connect_authorized(&format!("ws://{addr}"), "wrong")
        .await
        .is_err());
    assert!(connect_authorized(&format!("ws://{addr}"), "right")
        .await
        .is_ok());
}

#[tokio::test]
async fn writes_flow_between_authorized_clients_through_the_hub() {
    let addr = start_hub(vec![
        entry("ta", 10, &["users/"], &["users/"]),
        entry("tb", 11, &["users/"], &["users/"]),
    ])
    .await;
    let mut a = client(addr, "ta", 10).await;
    let mut b = client(addr, "tb", 11).await;

    a.replica.store().put(b"users/a", b"from-a").unwrap();
    converge(&mut a, &mut b, |_, b| b.has(b"users/a")).await;
    assert_eq!(b.value(b"users/a").as_deref(), Some(&b"from-a"[..]));

    b.replica.store().put(b"users/b", b"from-b").unwrap();
    converge(&mut a, &mut b, |a, _| a.has(b"users/b")).await;
    assert_eq!(a.value(b"users/b").as_deref(), Some(&b"from-b"[..]));
}

#[tokio::test]
async fn reads_are_filtered_by_key_prefix() {
    let addr = start_hub(vec![
        entry("writer", 10, &[""], &[""]),
        entry("reader", 12, &["public/"], &[]),
    ])
    .await;
    let mut w = client(addr, "writer", 10).await;
    let mut r = client(addr, "reader", 12).await;

    w.replica.store().put(b"private/x", b"secret").unwrap();
    w.replica.store().put(b"public/y", b"open").unwrap();
    converge(&mut w, &mut r, |_, r| r.has(b"public/y")).await;
    // Give the hub more rounds to misbehave, then check the filter held.
    for _ in 0..5 {
        let _ = r.round().await;
        let _ = w.round().await;
    }
    assert_eq!(r.value(b"public/y").as_deref(), Some(&b"open"[..]));
    assert!(!r.has(b"private/x"));
}

#[tokio::test]
async fn a_write_outside_permitted_prefixes_closes_the_link() {
    let addr = start_hub(vec![entry("rogue", 13, &[], &["users/"])]).await;
    let rogue = client(addr, "rogue", 13).await;

    rogue
        .transport
        .send(Delta {
            generation: 0,
            changes: vec![Change {
                key: b"secret/key".to_vec(),
                value: b"x".to_vec(),
                lamport: 1,
                node_id: 13,
                tombstone: false,
            }],
        })
        .await
        .unwrap();

    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rogue.transport.receive().await {
                Err(_) => return true,
                Ok(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("the hub should close a link that broke the write rules");
    assert!(closed);
}

#[tokio::test]
async fn a_spoofed_node_id_is_skipped_not_applied() {
    let addr = start_hub(vec![
        entry("victim", 10, &["users/"], &["users/"]),
        entry("spoofer", 13, &[], &["users/"]),
        entry("reader", 14, &["users/"], &[]),
    ])
    .await;
    let mut spoofer = client(addr, "spoofer", 13).await;
    let mut reader = client(addr, "reader", 14).await;

    // The spoofer claims to be node 10 and writes a key it may write.
    spoofer
        .transport
        .send(Delta {
            generation: 0,
            changes: vec![Change {
                key: b"users/evil".to_vec(),
                value: b"forged".to_vec(),
                lamport: 99,
                node_id: 10,
                tombstone: false,
            }],
        })
        .await
        .unwrap();

    // A genuine write from the reader's side, to show the hub is alive and
    // syncing. Then check the forgery never reached the reader.
    let mut honest = client(addr, "victim", 10).await;
    honest.replica.store().put(b"users/real", b"ok").unwrap();
    converge(&mut honest, &mut reader, |_, r| r.has(b"users/real")).await;
    for _ in 0..5 {
        let _ = reader.round().await;
        let _ = spoofer.round().await;
    }
    assert!(!reader.has(b"users/evil"));
}

#[tokio::test]
async fn an_oversized_message_closes_the_link() {
    let addr = start_hub(vec![entry("big", 10, &[""], &[""])]).await;
    let client = client(addr, "big", 10).await;
    // A single change larger than the 1 MiB limit the hub was started with.
    let huge = vec![7u8; 2 << 20];
    let sent = client
        .transport
        .send(Delta {
            generation: 0,
            changes: vec![Change {
                key: b"k".to_vec(),
                value: huge,
                lamport: 1,
                node_id: 10,
                tombstone: false,
            }],
        })
        .await;
    // Either the send itself fails, or the hub drops the link and the
    // client sees it on its next receive.
    let transport = client.transport;
    let closed = match sent {
        Err(_) => true,
        Ok(()) => tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match transport.receive().await {
                    Err(_) => return true,
                    Ok(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .unwrap_or(false),
    };
    assert!(closed, "an oversized message must not be accepted");
}
