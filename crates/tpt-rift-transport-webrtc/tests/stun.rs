use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tpt_rift_transport_webrtc::{create_offer_with, IceConfig};

const MAGIC_COOKIE: u32 = 0x2112_A442;

/// A minimal STUN server: answers each Binding Request with the sender's
/// address in XOR-MAPPED-ADDRESS, as RFC 5389 requires.
async fn stun_server() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 576];
        loop {
            let Ok((n, from)) = socket.recv_from(&mut buf).await else {
                return;
            };
            // Header: type(2) length(2) cookie(4) transaction id(12).
            if n < 20 || buf[0..2] != [0x00, 0x01] || buf[4..8] != MAGIC_COOKIE.to_be_bytes() {
                continue;
            }
            let SocketAddr::V4(v4) = from else {
                continue;
            };
            let port = v4.port() ^ (MAGIC_COOKIE >> 16) as u16;
            let ip = u32::from(*v4.ip()) ^ MAGIC_COOKIE;

            let mut response = Vec::with_capacity(32);
            response.extend_from_slice(&[0x01, 0x01]); // Binding Success
            response.extend_from_slice(&12u16.to_be_bytes()); // one attribute
            response.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
            response.extend_from_slice(&buf[8..20]); // transaction id
            response.extend_from_slice(&0x0020u16.to_be_bytes()); // XOR-MAPPED-ADDRESS
            response.extend_from_slice(&8u16.to_be_bytes());
            response.extend_from_slice(&[0x00, 0x01]); // reserved, IPv4
            response.extend_from_slice(&port.to_be_bytes());
            response.extend_from_slice(&ip.to_be_bytes());
            let _ = socket.send_to(&response, from).await;
        }
    });
    addr
}

#[tokio::test]
async fn a_stun_server_gives_the_offer_a_server_reflexive_candidate() {
    let stun = stun_server().await;
    let ice = IceConfig {
        stun_urls: vec![format!("stun:{stun}")],
        turn: vec![],
    };
    let offer = tokio::time::timeout(Duration::from_secs(30), create_offer_with(&ice))
        .await
        .expect("gathering should finish once the STUN server answers")
        .unwrap();
    assert!(
        offer.sdp().contains("typ srflx"),
        "the offer should include a server-reflexive candidate:\n{}",
        offer.sdp()
    );
    // Host candidates are still gathered alongside it.
    assert!(offer.sdp().contains("typ host"));
}

#[tokio::test]
async fn the_default_config_gathers_host_candidates_only() {
    let offer = create_offer_with(&IceConfig::default()).await.unwrap();
    assert!(offer.sdp().contains("typ host"));
    assert!(!offer.sdp().contains("typ srflx"));
}
