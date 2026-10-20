use std::time::Duration;

use tpt_hyle::{Hyle, VecStorage};
use tpt_rift::{sync_round, PeerLink, Replica};
use tpt_rift_transport_webrtc::{accept, create_offer};

type R = Replica<VecStorage, VecStorage>;

fn replica(node: u128) -> R {
    Replica::new(Hyle::open(VecStorage::new(), VecStorage::new(), node).unwrap())
}

#[tokio::test]
async fn two_peers_sync_over_a_data_channel() {
    // Signaling is done here by passing strings. A real app carries them
    // over any path it has.
    let offer = create_offer().await.unwrap();
    let (answerer, answer_sdp) = accept(offer.sdp().to_string()).await.unwrap();
    let offerer = offer.complete(answer_sdp).await.unwrap();
    offerer.wait_open(Duration::from_secs(20)).await.unwrap();
    answerer.wait_open(Duration::from_secs(20)).await.unwrap();

    let mut a = replica(1);
    let mut b = replica(2);
    a.store().put(b"from-a", b"hello").unwrap();
    b.store().put(b"from-b", b"world").unwrap();

    let mut la = PeerLink::new(2, &a);
    let mut lb = PeerLink::new(1, &b);
    let done = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let _ = sync_round(&mut a, &mut la, &offerer, 16).await;
            let _ = sync_round(&mut b, &mut lb, &answerer, 16).await;
            if a.store().contains_key(b"from-b").unwrap()
                && b.store().contains_key(b"from-a").unwrap()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(done.is_ok(), "peers did not converge over the data channel");
}
