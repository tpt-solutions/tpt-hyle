use tpt_hyle::{Hyle, Key, VecStorage};
use tpt_rift::{sync_round, MockTransport, PeerLink, Replica, RiftError, RoundStats};

type R = Replica<VecStorage>;

static SESSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A store with a fresh session, so no two opens share nonces.
fn fresh_hyle(node: u128) -> Hyle<VecStorage> {
    let mut session = [0u8; 16];
    let n = SESSIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    session[..8].copy_from_slice(&n.to_le_bytes());
    Hyle::open_with_session(
        VecStorage::new(),
        node,
        Key::from_bytes([0x5a; 32]),
        session,
    )
    .unwrap()
}

fn replica(node: u128) -> R {
    Replica::new(fresh_hyle(node))
}

fn read(r: &mut R, key: &[u8]) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    r.store()
        .get(key, &mut buf)
        .unwrap()
        .map(|n| buf[..n].to_vec())
}

#[tokio::test]
async fn two_replicas_converge_after_offline_writes() {
    let mut a = replica(1);
    let mut b = replica(2);
    // Both write while "offline", before any sync.
    a.store().put(b"user:1", b"alice-a").unwrap();
    a.store().put(b"shared", b"from-a").unwrap();
    b.store().put(b"user:2", b"bob-b").unwrap();
    b.store().put(b"shared", b"from-b").unwrap(); // both lamport 1: node 2 wins

    let (ta, tb) = MockTransport::pair();
    let mut la = PeerLink::new(2, &a);
    let mut lb = PeerLink::new(1, &b);

    for _ in 0..3 {
        sync_round(&mut a, &mut la, &ta, 1000).await.unwrap();
        sync_round(&mut b, &mut lb, &tb, 1000).await.unwrap();
    }

    for r in [&mut a, &mut b] {
        assert_eq!(read(r, b"user:1").as_deref(), Some(&b"alice-a"[..]));
        assert_eq!(read(r, b"user:2").as_deref(), Some(&b"bob-b"[..]));
        // Both wrote lamport 1; node 2 wins the tie-break.
        assert_eq!(read(r, b"shared").as_deref(), Some(&b"from-b"[..]));
    }

    // Quiescent: a further round moves nothing.
    let quiet_a = sync_round(&mut a, &mut la, &ta, 1000).await.unwrap();
    let quiet_b = sync_round(&mut b, &mut lb, &tb, 1000).await.unwrap();
    assert_eq!(quiet_a, RoundStats::default());
    assert_eq!(quiet_b, RoundStats::default());
}

#[tokio::test]
async fn a_newer_local_write_is_not_overwritten_by_an_older_remote_one() {
    let mut a = replica(1);
    let mut b = replica(2);
    b.store().put(b"k", b"old").unwrap(); // b lamport 1
    a.store().put(b"k", b"v1").unwrap();
    a.store().put(b"k", b"v2").unwrap();
    a.store().put(b"k", b"v3").unwrap(); // a lamport 3

    let (ta, tb) = MockTransport::pair();
    let mut la = PeerLink::new(2, &a);
    let mut lb = PeerLink::new(1, &b);
    for _ in 0..3 {
        sync_round(&mut a, &mut la, &ta, 1000).await.unwrap();
        sync_round(&mut b, &mut lb, &tb, 1000).await.unwrap();
    }
    assert_eq!(read(&mut a, b"k").as_deref(), Some(&b"v3"[..]));
    assert_eq!(read(&mut b, b"k").as_deref(), Some(&b"v3"[..]));
}

#[tokio::test]
async fn deletes_propagate_and_win_over_older_values() {
    let mut a = replica(1);
    let mut b = replica(2);
    let (ta, tb) = MockTransport::pair();
    let mut la = PeerLink::new(2, &a);
    let mut lb = PeerLink::new(1, &b);

    a.store().put(b"gone", b"x").unwrap();
    for _ in 0..2 {
        sync_round(&mut a, &mut la, &ta, 1000).await.unwrap();
        sync_round(&mut b, &mut lb, &tb, 1000).await.unwrap();
    }
    assert_eq!(read(&mut b, b"gone").as_deref(), Some(&b"x"[..]));

    b.store().delete(b"gone").unwrap();
    for _ in 0..2 {
        sync_round(&mut a, &mut la, &ta, 1000).await.unwrap();
        sync_round(&mut b, &mut lb, &tb, 1000).await.unwrap();
    }
    assert_eq!(read(&mut a, b"gone"), None);
    assert_eq!(read(&mut b, b"gone"), None);
}

#[tokio::test]
async fn changes_are_forwarded_through_a_middle_node_without_echo() {
    // A <-> B <-> C, with no direct A-C link.
    let mut a = replica(1);
    let mut b = replica(2);
    let mut c = replica(3);
    let (ab_a, ab_b) = MockTransport::pair();
    let (bc_b, bc_c) = MockTransport::pair();
    let mut a_to_b = PeerLink::new(2, &a);
    let mut b_to_a = PeerLink::new(1, &b);
    let mut b_to_c = PeerLink::new(3, &b);
    let mut c_to_b = PeerLink::new(2, &c);

    a.store().put(b"from-a", b"hello").unwrap();
    for _ in 0..4 {
        sync_round(&mut a, &mut a_to_b, &ab_a, 1000).await.unwrap();
        sync_round(&mut b, &mut b_to_a, &ab_b, 1000).await.unwrap();
        sync_round(&mut b, &mut b_to_c, &bc_b, 1000).await.unwrap();
        sync_round(&mut c, &mut c_to_b, &bc_c, 1000).await.unwrap();
    }
    assert_eq!(read(&mut c, b"from-a").as_deref(), Some(&b"hello"[..]));

    // Echo check: A's own write must not come back to A as a new change.
    let echo = sync_round(&mut b, &mut b_to_a, &ab_b, 1000).await.unwrap();
    assert_eq!(echo.applied, 0);
    let back = sync_round(&mut a, &mut a_to_b, &ab_a, 1000).await.unwrap();
    assert_eq!(back.applied, 0);
}

#[tokio::test]
async fn stale_generation_is_reported_and_recovers_after_reset() {
    let mut a = replica(1);
    let mut b = replica(2);
    let (ta, tb) = MockTransport::pair();
    let mut la = PeerLink::new(2, &a);
    let mut lb = PeerLink::new(1, &b);

    a.store().put(b"k", b"v").unwrap();
    a.store().put(b"k", b"w").unwrap();
    for _ in 0..2 {
        sync_round(&mut a, &mut la, &ta, 1000).await.unwrap();
        sync_round(&mut b, &mut lb, &tb, 1000).await.unwrap();
    }

    // Compact A. Every cursor that peers hold on A's log is now stale.
    let old = std::mem::replace(&mut a, replica(1));
    let compacted = old
        .into_store()
        .compact_into(VecStorage::new(), [0xc0; 16])
        .unwrap();
    let mut a = Replica::new(compacted);

    match sync_round(&mut a, &mut la, &ta, 1000).await {
        Err(RiftError::StaleGeneration { cursor, current }) => {
            assert_eq!(current, cursor + 1);
        }
        other => panic!("expected StaleGeneration, got {other:?}"),
    }

    la.reset(&a);
    sync_round(&mut a, &mut la, &ta, 1000).await.unwrap();
    sync_round(&mut b, &mut lb, &tb, 1000).await.unwrap();
    assert_eq!(read(&mut b, b"k").as_deref(), Some(&b"w"[..]));
}
