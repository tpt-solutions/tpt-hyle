use tpt_hyle_rift::{connect, open, Database};
use tpt_rift::{MockTransport, Replica};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tpt-hyle-rift-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(db: &mut Database, key: &[u8]) -> Option<Vec<u8>> {
    let mut buf = [0u8; 128];
    db.store()
        .get(key, &mut buf)
        .unwrap()
        .map(|n| buf[..n].to_vec())
}

#[tokio::test]
async fn resume_recovers_after_the_other_side_compacts() {
    let dir = scratch("compact");
    let mut a = open(dir.join("a"), 1, b"test passphrase").unwrap();
    let mut b = open(dir.join("b"), 2, b"test passphrase").unwrap();
    a.store().put(b"k", b"v1").unwrap();
    a.store().put(b"k", b"v2").unwrap();

    let (ta, tb) = MockTransport::pair();
    {
        let mut ca = connect(&mut a, 2, ta);
        let mut cb = connect(&mut b, 1, tb);
        for _ in 0..2 {
            ca.resume().await.unwrap();
            cb.resume().await.unwrap();
        }
    }
    assert_eq!(read(&mut b, b"k").as_deref(), Some(&b"v2"[..]));

    // Compact A. B's position in A's log is now stale.
    let compacted = a.into_store().compact_file(dir.join("a")).unwrap();
    let mut a: Database = Replica::new(compacted);
    a.store().put(b"after", b"compaction").unwrap();

    // Fresh transport: the stale position is detected, reset, and retried.
    let (ta, tb) = MockTransport::pair();
    let mut ca = connect(&mut a, 2, ta);
    let mut cb = connect(&mut b, 1, tb);
    for _ in 0..2 {
        ca.resume().await.unwrap();
        cb.resume().await.unwrap();
    }
    assert_eq!(read(&mut b, b"after").as_deref(), Some(&b"compaction"[..]));
    assert_eq!(read(&mut b, b"k").as_deref(), Some(&b"v2"[..]));
    std::fs::remove_dir_all(&dir).unwrap();
}
