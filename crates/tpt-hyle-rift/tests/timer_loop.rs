use std::time::Duration;

use tpt_hyle_rift::{connect, open};
use tpt_rift::{Backoff, MockTransport};

#[tokio::test]
async fn timer_loop_converges_two_replicas_then_stops_on_signal() {
    let dir = std::env::temp_dir().join(format!("tpt-hyle-rift-timer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = open(dir.join("a"), 1).unwrap();
    let mut b = open(dir.join("b"), 2).unwrap();
    a.store().put(b"from-a", b"1").unwrap();
    b.store().put(b"from-b", b"2").unwrap();

    let (ta, tb) = MockTransport::pair();
    let backoff = Backoff {
        base: Duration::from_millis(5),
        max: Duration::from_millis(50),
    };
    {
        let mut ca = connect(&mut a, 2, ta);
        let mut cb = connect(&mut b, 1, tb);
        // A runs on its timer for a while. B is driven by hand in the same
        // task, so no spawn is needed.
        let run_a = ca.run_until(
            Duration::from_millis(5),
            backoff,
            tokio::time::sleep(Duration::from_millis(200)),
        );
        let drive_b = async {
            for _ in 0..40 {
                cb.resume().await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        let (ra, ()) = tokio::join!(run_a, drive_b);
        ra.expect("timer loop should stop cleanly on the stop signal");
    }

    assert!(a.store().contains_key(b"from-b").unwrap());
    assert!(b.store().contains_key(b"from-a").unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
}
