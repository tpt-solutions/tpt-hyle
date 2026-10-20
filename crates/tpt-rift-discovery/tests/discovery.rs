use std::net::SocketAddr;
use std::time::Duration;

use tpt_rift_discovery::{announce_forever, Announce, DecodeError, Listener, ANNOUNCE_LEN};

#[test]
fn an_announcement_round_trips() {
    let a = Announce {
        node_id: 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
        port: 8080,
    };
    assert_eq!(Announce::decode(&a.encode()).unwrap(), a);
    assert_eq!(a.encode().len(), ANNOUNCE_LEN);
}

#[test]
fn malformed_announcements_are_rejected() {
    let good = Announce {
        node_id: 7,
        port: 1,
    }
    .encode();
    for cut in 0..good.len() {
        assert_eq!(Announce::decode(&good[..cut]), Err(DecodeError::BadLength));
    }
    let mut long = good.to_vec();
    long.push(0);
    assert_eq!(Announce::decode(&long), Err(DecodeError::BadLength));

    let mut bad_magic = good;
    bad_magic[0] ^= 0xFF;
    assert_eq!(Announce::decode(&bad_magic), Err(DecodeError::BadMagic));

    let mut bad_version = good;
    bad_version[4] = 99;
    assert_eq!(Announce::decode(&bad_version), Err(DecodeError::BadVersion));
}

#[tokio::test]
async fn a_listener_learns_an_announcing_node() {
    let mut listener = Listener::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let target = listener.local_addr().unwrap();
    let announce = Announce {
        node_id: 42,
        port: 9000,
    };
    // Unicast to the listener, rather than broadcast, so the test does not
    // depend on the loopback interface supporting broadcast.
    let sender = tokio::spawn(announce_forever(
        announce,
        target,
        Duration::from_millis(20),
    ));

    let peer = tokio::time::timeout(Duration::from_secs(5), listener.next_peer())
        .await
        .expect("an announcement should arrive")
        .unwrap();
    sender.abort();

    assert_eq!(peer.node_id, 42);
    assert_eq!(peer.addr.port(), 9000);
    assert_eq!(listener.peers().count(), 1);
}

#[tokio::test]
async fn garbage_on_the_port_is_skipped_not_believed() {
    let mut listener = Listener::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let target = listener.local_addr().unwrap();
    let junk = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    junk.send_to(b"hello, not an announcement", target)
        .await
        .unwrap();

    let sender = tokio::spawn(announce_forever(
        Announce {
            node_id: 5,
            port: 1,
        },
        target,
        Duration::from_millis(20),
    ));
    let peer = tokio::time::timeout(Duration::from_secs(5), listener.next_peer())
        .await
        .expect("the real announcement should still arrive")
        .unwrap();
    sender.abort();
    assert_eq!(peer.node_id, 5, "the junk packet must not be recorded");
}
