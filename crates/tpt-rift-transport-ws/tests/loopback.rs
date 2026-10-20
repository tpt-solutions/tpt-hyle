use std::time::Duration;

use tokio::net::TcpListener;
use tpt_hyle::VecStorage;
use tpt_hyle_test_utils::mem_store;
use tpt_rift::{sync_round, PeerLink, Replica, Transport};
use tpt_rift_transport_ws::{accept, connect};

type R = Replica<VecStorage>;

fn replica(node: u128) -> R {
    Replica::new(mem_store(node))
}

fn has(r: &mut R, key: &[u8]) -> bool {
    r.store().contains_key(key).unwrap()
}

async fn pair() -> (
    tpt_rift_transport_ws::WebSocketTransport<tokio::net::TcpStream>,
    tpt_rift_transport_ws::WebSocketTransport<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { accept(&listener).await.unwrap() });
    let client = connect(&format!("ws://{addr}")).await.unwrap();
    (server.await.unwrap(), client)
}

#[tokio::test]
async fn replicas_converge_over_a_real_socket() {
    let (server_end, client_end) = pair().await;
    let mut a = replica(1);
    let mut b = replica(2);
    a.store().put(b"from-a", b"hello").unwrap();
    b.store().put(b"from-b", b"world").unwrap();

    let mut la = PeerLink::new(2, &a);
    let mut lb = PeerLink::new(1, &b);
    // Frames take time to cross the socket, so poll until both sides have
    // both keys, or give up after a generous deadline.
    let done = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            sync_round(&mut a, &mut la, &client_end, 1000)
                .await
                .unwrap();
            sync_round(&mut b, &mut lb, &server_end, 1000)
                .await
                .unwrap();
            if has(&mut a, b"from-b") && has(&mut b, b"from-a") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        done.is_ok(),
        "replicas did not converge within the deadline"
    );
    assert!(has(&mut a, b"from-b") && has(&mut b, b"from-a"));
}

#[tokio::test]
async fn malformed_frame_closes_the_link_without_reaching_the_store() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { accept(&listener).await.unwrap() });

    // A raw client that sends bytes which are not a delta.
    let (mut raw, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .unwrap();
    let server_end = server.await.unwrap();
    use futures_util::SinkExt;
    raw.send(tokio_tungstenite::tungstenite::Message::Binary(
        b"junk".to_vec(),
    ))
    .await
    .unwrap();

    // The server end must report the closed link, not hand over a delta.
    let outcome = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match server_end.receive().await {
                Ok(None) => tokio::time::sleep(Duration::from_millis(5)).await,
                other => return other,
            }
        }
    })
    .await
    .expect("link should close promptly");
    assert!(
        outcome.is_err(),
        "expected a closed-link error, got {outcome:?}"
    );
}
