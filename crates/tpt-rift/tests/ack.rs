//! The client's side of the caught-up handshake. A hub marker is answered with
//! an ack only once the client has pushed everything it wrote.

use tpt_hyle::{Hyle, Key, VecStorage};
use tpt_rift::{sync_round, Ack, Delta, MockTransport, PeerLink, Replica, Transport};

static SESSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn replica(node: u128) -> Replica<VecStorage> {
    let mut session = [0u8; 16];
    let n = SESSIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    session[..8].copy_from_slice(&n.to_le_bytes());
    Replica::new(
        Hyle::open_with_session(
            VecStorage::new(),
            node,
            Key::from_bytes([0x5a; 32]),
            session,
        )
        .unwrap(),
    )
}

/// Everything the hub end has received from the client, in order.
async fn drain(hub: &MockTransport) -> Vec<Delta> {
    let mut out = Vec::new();
    while let Ok(Some(d)) = hub.receive().await {
        out.push(d);
    }
    out
}

fn acks(deltas: &[Delta]) -> Vec<Ack> {
    deltas.iter().filter_map(|d| d.ack).collect()
}

fn marker(generation: u32, offset: u64) -> Delta {
    Delta {
        generation,
        caught_up: Some(offset),
        ..Default::default()
    }
}

#[tokio::test]
async fn an_ack_follows_a_marker_once_the_export_is_complete() {
    let mut r = replica(10);
    let (client, hub) = MockTransport::pair();
    let mut link = PeerLink::new(1, &r);
    let end = r.store_ref().sync_end();

    hub.send(marker(0, end)).await.unwrap();
    sync_round(&mut r, &mut link, &client, 16).await.unwrap();

    let got = acks(&drain(&hub).await);
    assert_eq!(
        got,
        vec![Ack {
            offset: end,
            lamport: 0
        }]
    );
}

#[tokio::test]
async fn no_ack_while_the_export_is_incomplete() {
    let mut r = replica(10);
    for i in 0..3u8 {
        r.store().put(&[b'k', i], b"v").unwrap();
    }
    let (client, hub) = MockTransport::pair();
    let mut link = PeerLink::new(1, &r);
    let end = r.store_ref().sync_end();
    hub.send(marker(0, end)).await.unwrap();

    // Two of three events fit in this round, so the export is not complete.
    sync_round(&mut r, &mut link, &client, 2).await.unwrap();
    assert!(
        acks(&drain(&hub).await).is_empty(),
        "no ack before the push is complete"
    );

    // The last event goes out now, and the ack follows it. Its lamport is the
    // clock read before the export, which is the last write's lamport.
    sync_round(&mut r, &mut link, &client, 2).await.unwrap();
    let got = acks(&drain(&hub).await);
    assert_eq!(
        got,
        vec![Ack {
            offset: end,
            lamport: 3
        }]
    );
}

#[tokio::test]
async fn an_ack_is_sent_once_per_marker() {
    let mut r = replica(10);
    let (client, hub) = MockTransport::pair();
    let mut link = PeerLink::new(1, &r);
    let end = r.store_ref().sync_end();
    hub.send(marker(0, end)).await.unwrap();

    sync_round(&mut r, &mut link, &client, 16).await.unwrap();
    sync_round(&mut r, &mut link, &client, 16).await.unwrap();
    assert_eq!(acks(&drain(&hub).await).len(), 1);
}

#[tokio::test]
async fn a_reset_forgets_the_marker() {
    let mut r = replica(10);
    let (client, hub) = MockTransport::pair();
    let mut link = PeerLink::new(1, &r);
    let end = r.store_ref().sync_end();
    hub.send(marker(0, end)).await.unwrap();
    sync_round(&mut r, &mut link, &client, 16).await.unwrap();
    drain(&hub).await;

    link.reset(&r);
    sync_round(&mut r, &mut link, &client, 16).await.unwrap();
    assert!(
        acks(&drain(&hub).await).is_empty(),
        "no marker in the new generation yet"
    );
}
