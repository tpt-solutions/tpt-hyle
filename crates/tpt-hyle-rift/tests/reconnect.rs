use std::time::Duration;

use tpt_hyle_rift::{connect, open, run_reconnecting, Database, SyncPolicy, Trigger};
use tpt_rift::{Backoff, MockTransport, RiftError};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tpt-hyle-rift-reconnect-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(db: &mut Database, key: &[u8]) -> bool {
    db.store().contains_key(key).unwrap()
}

fn fast_policy(interval: Duration) -> SyncPolicy {
    SyncPolicy {
        interval,
        backoff: Backoff {
            base: Duration::from_millis(20),
            max: Duration::from_millis(100),
        },
        max_events: 1024,
    }
}

#[tokio::test]
async fn a_failed_connection_is_retried_and_converges() {
    let dir = scratch("retry");
    let mut a = open(dir.join("a"), 1, b"test passphrase").unwrap();
    let mut b = open(dir.join("b"), 2, b"test passphrase").unwrap();
    b.store().put(b"from-b", b"1").unwrap();
    a.store().put(b"from-a", b"1").unwrap();

    let (a_end, b_end) = MockTransport::pair();
    // The first attempt fails as if the network were down. The second works.
    let mut attempts = 0u32;
    let mut a_end = Some(a_end);
    let connector = || {
        attempts += 1;
        let result = if attempts == 1 {
            Err(RiftError::Transport("network down".into()))
        } else {
            Ok(a_end.take().expect("connected once only"))
        };
        std::future::ready(result)
    };

    let trigger = Trigger::new();
    let stop = tokio::time::sleep(Duration::from_millis(1500));
    let (run, ()) = tokio::join!(
        run_reconnecting(
            &mut a,
            2,
            connector,
            fast_policy(Duration::from_secs(60)),
            &trigger,
            stop
        ),
        async {
            let mut cb = connect(&mut b, 1, b_end);
            for _ in 0..100 {
                let _ = cb.resume().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    );
    run.unwrap();
    assert!(read(&mut a, b"from-b"));
    assert!(read(&mut b, b"from-a"));
}

#[tokio::test]
async fn a_trigger_wakes_the_loop_long_before_its_timer() {
    let dir = scratch("trigger");
    let mut a = open(dir.join("a"), 1, b"test passphrase").unwrap();
    let mut b = open(dir.join("b"), 2, b"test passphrase").unwrap();

    let (a_end, b_end) = MockTransport::pair();
    let a_end = std::cell::Cell::new(Some(a_end));
    let connector = || std::future::ready(Ok::<_, RiftError>(a_end.take().unwrap()));

    let trigger = Trigger::new();
    let trigger_for_writer = trigger.clone();
    let stop = tokio::time::sleep(Duration::from_millis(1500));
    // The interval is one minute, so only the trigger can deliver in time.
    let (run, ()) = tokio::join!(
        run_reconnecting(
            &mut a,
            2,
            connector,
            fast_policy(Duration::from_secs(60)),
            &trigger,
            stop
        ),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // B writes, sends, then notifies. Sending first means the
            // notification finds the data already waiting for A.
            b.store().put(b"late", b"write").unwrap();
            let mut cb = connect(&mut b, 1, b_end);
            cb.resume().await.unwrap();
            trigger_for_writer.notify();
            for _ in 0..50 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    );
    run.unwrap();
    assert!(read(&mut a, b"late"), "the trigger should have run a round");
}
