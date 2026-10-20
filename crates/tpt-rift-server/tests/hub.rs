use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::MaybeTlsStream;
use tpt_hyle::{Hyle, VecStorage};
use tpt_rift::{Change, Delta, PeerLink, Replica, Transport};
use tpt_rift_server::{
    accept_loop, decode_signal, encode_signal, hash_token, Access, Config, Hub, Registry,
    TokenConfig,
};
use tpt_rift_transport_ws::{connect_authorized, WebSocketTransport};

const HUB_NODE: u128 = 1;

static SESSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// An in-memory store with a fresh session, so no two opens share nonces.
fn mem_hyle(node: u128) -> Hyle<VecStorage> {
    let mut session = [0u8; 16];
    let n = SESSIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    session[..8].copy_from_slice(&n.to_le_bytes());
    Hyle::open_with_session(
        VecStorage::new(),
        node,
        tpt_hyle::Key::from_bytes([0x5a; 32]),
        session,
    )
    .unwrap()
}

struct Client {
    replica: Replica<VecStorage>,
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
    start_hub_with_rate(tokens, 200).await
}

async fn start_hub_with_rate(tokens: Vec<TokenConfig>, rate: u32) -> std::net::SocketAddr {
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
        messages_per_second: rate,
        tokens,
    };
    let (hub_node, access) = Access::from_config(&config).unwrap();
    let store = Hyle::open_file(&config.store, hub_node, b"test hub passphrase").unwrap();
    let hub: Hub = Arc::new(Mutex::new(Replica::new(store)));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(accept_loop(
        listener,
        access,
        hub,
        Registry::default(),
        Duration::from_millis(5),
        1 << 20,
        rate,
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
    let public_key = mem_hyle(node)
        .public_key()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    TokenConfig {
        sha256: hash_token(token),
        node_id: node.to_string(),
        public_key,
        read: read.iter().map(|s| s.to_string()).collect(),
        write: write.iter().map(|s| s.to_string()).collect(),
    }
}

async fn client(addr: std::net::SocketAddr, token: &str, node: u128) -> Client {
    let transport = connect_authorized(&format!("ws://{addr}"), token)
        .await
        .unwrap();
    let replica = Replica::new(mem_hyle(node));
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
                signature: [0; 64],
            }],
            ..Default::default()
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
                signature: [0; 64],
            }],
            ..Default::default()
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
                signature: [0; 64],
            }],
            ..Default::default()
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

async fn next_signal(client: &mut Client) -> tpt_rift_server::Signal {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(text) = client.transport.take_text().unwrap() {
                if let Some(signal) = decode_signal(&text) {
                    return signal;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a signal should arrive")
}

#[tokio::test]
async fn signals_are_relayed_with_the_origin_stamped_by_the_hub() {
    let addr = start_hub(vec![entry("ta", 10, &[], &[]), entry("tb", 11, &[], &[])]).await;
    let a = client(addr, "ta", 10).await;
    let mut b = client(addr, "tb", 11).await;

    a.transport
        .send_text(&encode_signal(11, "offer", "v=0 sdp-from-a"))
        .await
        .unwrap();
    let got = next_signal(&mut b).await;
    assert_eq!(got.from, 10, "the hub stamps the origin, not the client");
    assert_eq!(got.kind, "offer");
    assert_eq!(got.sdp, "v=0 sdp-from-a");
}

#[tokio::test]
async fn a_signal_to_an_unconnected_node_is_dropped() {
    let addr = start_hub(vec![entry("ta", 10, &[], &[])]).await;
    let a = client(addr, "ta", 10).await;
    a.transport
        .send_text(&encode_signal(99, "offer", "lost"))
        .await
        .unwrap();
    // The sender stays connected, so the hub did not treat it as an error.
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.transport
        .send_text(&encode_signal(10, "answer", "still here"))
        .await
        .expect("the connection should survive an unroutable signal");
}

#[tokio::test]
async fn webrtc_peers_connect_through_the_hub_and_sync() {
    use tpt_rift_transport_webrtc::{accept, create_offer};

    let addr = start_hub(vec![
        entry("ta", 10, &[""], &[""]),
        entry("tb", 11, &[""], &[""]),
    ])
    .await;
    let mut a = client(addr, "ta", 10).await;
    let mut b = client(addr, "tb", 11).await;

    // A offers, through the hub.
    let offer = create_offer().await.unwrap();
    a.transport
        .send_text(&encode_signal(11, "offer", offer.sdp()))
        .await
        .unwrap();

    // B answers, through the hub.
    let got = next_signal(&mut b).await;
    assert_eq!(got.kind, "offer");
    let (b_channel, answer_sdp) = accept(got.sdp).await.unwrap();
    b.transport
        .send_text(&encode_signal(10, "answer", &answer_sdp))
        .await
        .unwrap();

    // A completes the connection with the answer.
    let got = next_signal(&mut a).await;
    assert_eq!(got.kind, "answer");
    let a_channel = offer.complete(got.sdp).await.unwrap();
    a_channel.wait_open(Duration::from_secs(20)).await.unwrap();
    b_channel.wait_open(Duration::from_secs(20)).await.unwrap();

    // The data channel now carries deltas peer to peer, outside the hub.
    let mut ra = tpt_rift::Replica::new(mem_hyle(10));
    let mut rb = tpt_rift::Replica::new(mem_hyle(11));
    ra.store().put(b"direct", b"over webrtc").unwrap();
    let mut la = PeerLink::new(11, &ra);
    let mut lb = PeerLink::new(10, &rb);
    let done = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let _ = tpt_rift::sync_round(&mut ra, &mut la, &a_channel, 16).await;
            let _ = tpt_rift::sync_round(&mut rb, &mut lb, &b_channel, 16).await;
            if rb.store().contains_key(b"direct").unwrap() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(done.is_ok(), "the data channel did not carry the delta");
}

#[tokio::test]
async fn a_client_over_its_message_rate_is_disconnected() {
    let addr = start_hub_with_rate(vec![entry("flood", 10, &[], &[])], 20).await;
    let client = client(addr, "flood", 10).await;
    // The burst is 20 messages. Send well past it, as fast as the socket allows.
    for _ in 0..300 {
        if client
            .transport
            .send_text(&encode_signal(99, "offer", "x"))
            .await
            .is_err()
        {
            break;
        }
    }
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match client.transport.receive().await {
                Err(_) => return true,
                Ok(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(closed, "the hub should disconnect a client over its rate");
}

const PASS: &[u8] = b"test hub passphrase";

/// A config whose store is a real file, so a test can compact it.
fn file_config(tokens: Vec<TokenConfig>) -> Config {
    let dir = std::env::temp_dir().join(format!(
        "tpt-rift-server-gc-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Config {
        store: dir.join("hub"),
        listen: "127.0.0.1:0".into(),
        node_id: HUB_NODE.to_string(),
        max_message_bytes: 1 << 20,
        poll_ms: 5,
        messages_per_second: 200,
        tokens,
    }
}

/// Serves `config` the way `run` does, including its saved acks. Returns the
/// address, and the accept loop so the test can stop it.
async fn serve_config(config: &Config) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let (hub_node, mut access) = Access::from_config(config).unwrap();
    access
        .load_acks(&tpt_rift_server::acks_path(&config.store))
        .unwrap();
    let store = Hyle::open_file(&config.store, hub_node, PASS).unwrap();
    let hub: Hub = Arc::new(Mutex::new(Replica::new(store)));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(accept_loop(
        listener,
        access,
        hub,
        Registry::default(),
        Duration::from_millis(5),
        1 << 20,
        200,
    ));
    (addr, task)
}

/// Lets closed connections release the store file before it is compacted.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// The events left in the hub's store file.
fn stored_events(config: &Config) -> Vec<tpt_hyle::ChangeEvent> {
    let mut store = Hyle::open_file(&config.store, HUB_NODE, PASS).unwrap();
    store
        .events_since(tpt_hyle::HEADER_LEN)
        .unwrap()
        .map(|r| r.unwrap().1)
        .collect()
}

async fn rounds(a: &mut Client, b: &mut Client, n: usize) {
    for _ in 0..n {
        let _ = a.round().await;
        let _ = b.round().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_tombstone_is_collected_once_every_client_has_acked() {
    let config = file_config(vec![
        entry("ta", 10, &[""], &[""]),
        entry("tb", 11, &[""], &[""]),
    ]);
    let (addr, server) = serve_config(&config).await;
    let mut a = client(addr, "ta", 10).await;
    let mut b = client(addr, "tb", 11).await;

    a.replica.store().put(b"k", b"v").unwrap();
    converge(&mut a, &mut b, |_, b| b.has(b"k")).await;
    a.replica.store().delete(b"k").unwrap();
    converge(&mut a, &mut b, |_, b| !b.has(b"k")).await;
    rounds(&mut a, &mut b, 20).await;

    drop(a);
    drop(b);
    server.abort();
    settle().await;
    let collection = tpt_rift_server::collect_tombstones(&config, PASS).unwrap();
    assert!(collection.bound.is_some(), "both clients acked");
    assert!(
        stored_events(&config).is_empty(),
        "the tombstone and the value it replaced are both gone"
    );
}

#[tokio::test]
async fn a_client_that_never_acks_blocks_collection() {
    let config = file_config(vec![
        entry("ta", 10, &[""], &[""]),
        entry("tb", 11, &[""], &[""]),
    ]);
    let (addr, server) = serve_config(&config).await;
    let mut a = client(addr, "ta", 10).await;
    a.replica.store().put(b"k", b"v").unwrap();
    a.replica.store().delete(b"k").unwrap();
    for _ in 0..20 {
        let _ = a.round().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    drop(a);
    server.abort();
    settle().await;
    // `tb` never connected, so it has no ack and nothing may be collected.
    let collection = tpt_rift_server::collect_tombstones(&config, PASS).unwrap();
    assert_eq!(collection.bound, None);
    let events = stored_events(&config);
    assert_eq!(events.len(), 1, "the tombstone stays");
    assert!(events[0].tombstone);
}

#[tokio::test]
async fn after_collection_a_replayed_old_write_is_refused_and_a_new_one_is_kept() {
    let config = file_config(vec![
        entry("ta", 10, &[""], &[""]),
        entry("tb", 11, &[""], &[""]),
    ]);

    // First run: write and delete a key, then collect.
    let (addr, server) = serve_config(&config).await;
    let mut a = client(addr, "ta", 10).await;
    let mut b = client(addr, "tb", 11).await;
    a.replica.store().put(b"k", b"v").unwrap();
    converge(&mut a, &mut b, |_, b| b.has(b"k")).await;
    a.replica.store().delete(b"k").unwrap();
    converge(&mut a, &mut b, |_, b| !b.has(b"k")).await;
    rounds(&mut a, &mut b, 20).await;
    drop(a);
    drop(b);
    server.abort();
    settle().await;
    assert!(tpt_rift_server::collect_tombstones(&config, PASS)
        .unwrap()
        .bound
        .is_some());

    // Second run, in the new generation. Both clients write two keys, so
    // their clocks reach 2 and their acks set the floor at 2.
    let (addr, server) = serve_config(&config).await;
    let mut a = client(addr, "ta", 10).await;
    let mut b = client(addr, "tb", 11).await;
    a.replica.store().put(b"pad1", b"x").unwrap();
    a.replica.store().put(b"pad2", b"x").unwrap();
    b.replica.store().put(b"pad3", b"x").unwrap();
    b.replica.store().put(b"pad4", b"x").unwrap();
    rounds(&mut a, &mut b, 30).await;

    // A write from before the collection, with lamport 1, is replayed. The
    // event is genuine: node 10 signed it when it wrote it, so only the floor
    // can refuse it. The hub must refuse it, because every client has already
    // acked past it.
    let mut old = Replica::new(mem_hyle(10));
    old.store().put(b"j", b"old").unwrap();
    let replay = old.export(11, old.start_cursor(), 16).unwrap().delta;
    assert_eq!(replay.changes.len(), 1);
    a.transport
        .send(Delta {
            generation: 1,
            ..replay
        })
        .await
        .unwrap();
    // A new write above the floor must still get through.
    b.replica.store().put(b"fresh", b"new").unwrap();
    rounds(&mut a, &mut b, 30).await;

    assert!(!b.has(b"j"), "the replayed write was refused");
    assert_eq!(a.value(b"fresh").as_deref(), Some(&b"new"[..]));
    server.abort();
}

#[tokio::test]
async fn a_forged_acknowledgement_closes_the_link() {
    let addr = start_hub(vec![entry("ta", 10, &[""], &[""])]).await;
    let client = client(addr, "ta", 10).await;
    client
        .transport
        .send(Delta {
            generation: 0,
            ack: Some(tpt_rift::Ack {
                offset: 128,
                lamport: 0,
                signature: [3; 64],
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match client.transport.receive().await {
                Err(_) => return true,
                Ok(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("the hub should close a link that sent a forged acknowledgement");
    assert!(closed);
}

#[tokio::test]
async fn a_forged_signature_closes_the_link() {
    let addr = start_hub(vec![entry("ta", 10, &[""], &[""])]).await;
    let client = client(addr, "ta", 10).await;
    client
        .transport
        .send(Delta {
            generation: 0,
            changes: vec![Change {
                key: b"k".to_vec(),
                value: b"forged".to_vec(),
                lamport: 1,
                node_id: 10,
                tombstone: false,
                signature: [7; 64],
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match client.transport.receive().await {
                Err(_) => return true,
                Ok(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("the hub should close a link that sent a forged event");
    assert!(closed);
}
