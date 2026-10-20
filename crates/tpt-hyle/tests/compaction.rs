use tpt_hyle::{Error, Hyle, Key, Storage, VecStorage, HEADER_LEN};

const NODE: u128 = 7;
const KEY: [u8; 32] = [0x11; 32];

fn fresh() -> Hyle<VecStorage> {
    Hyle::open_with_session(VecStorage::new(), NODE, Key::from_bytes(KEY), [1; 16]).unwrap()
}

fn read<S: Storage>(db: &mut Hyle<S>, key: &[u8]) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    db.get(key, &mut buf).unwrap().map(|n| buf[..n].to_vec())
}

#[test]
fn compaction_keeps_latest_event_per_key_and_tombstones() {
    let mut db = fresh();
    db.put(b"a", b"one").unwrap();
    db.put(b"b", b"bee").unwrap();
    db.put(b"a", b"two").unwrap();
    db.delete(b"b").unwrap();
    let old_gen = db.generation();
    let old_len = db.sync_end();

    let mut compacted = db.compact_into(VecStorage::new(), [2; 16]).unwrap();
    assert_eq!(compacted.generation(), old_gen + 1);
    assert!(compacted.sync_end() < old_len, "superseded writes dropped");
    assert_eq!(read(&mut compacted, b"a").as_deref(), Some(&b"two"[..]));
    assert_eq!(read(&mut compacted, b"b"), None);

    let feed: Vec<_> = compacted
        .events_since(HEADER_LEN)
        .unwrap()
        .map(|r| r.unwrap().1)
        .collect();
    let lamports: Vec<u64> = feed.iter().map(|e| e.lamport).collect();
    assert_eq!(lamports, vec![3, 4]);
    assert!(feed[1].tombstone);
}

#[test]
fn compaction_preserves_clock_and_keeps_writing() {
    let mut db = fresh();
    db.put(b"k", b"v").unwrap();
    db.put(b"k", b"w").unwrap();
    db.delete(b"k").unwrap();
    let before = db.lamport();
    let mut compacted = db.compact_into(VecStorage::new(), [2; 16]).unwrap();
    assert_eq!(compacted.lamport(), before);
    assert_eq!(compacted.put(b"new", b"x").unwrap().lamport, before + 1);
}

#[test]
fn compaction_keeps_original_node_ids() {
    let mut db = fresh();
    db.put(b"k", b"v").unwrap();
    let mut compacted = db.compact_into(VecStorage::new(), [2; 16]).unwrap();
    let ev = compacted
        .events_since(HEADER_LEN)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .1;
    assert_eq!(ev.node_id, NODE);
    assert_eq!(compacted.node_id(), NODE);
}

#[test]
fn collectable_tombstones_are_dropped_and_others_kept() {
    let mut db = fresh();
    db.put(b"a", b"one").unwrap();
    db.delete(b"a").unwrap();
    // Everything up to here is before b's tombstone, so a's tombstone ends
    // at or below this offset and b's ends above it.
    let cut = db.sync_end();
    db.put(b"b", b"two").unwrap();
    db.delete(b"b").unwrap();

    let mut compacted = db
        .compact_into_collecting(VecStorage::new(), [2; 16], |end, _| end <= cut)
        .unwrap();

    assert_eq!(read(&mut compacted, b"a"), None);
    assert_eq!(read(&mut compacted, b"b"), None);
    let feed: Vec<_> = compacted
        .events_since(HEADER_LEN)
        .unwrap()
        .map(|r| r.unwrap().1)
        .collect();
    assert_eq!(feed.len(), 1, "only b's tombstone is kept");
    assert!(feed[0].tombstone);
    assert_eq!(feed[0].lamport, 4, "b's tombstone is the last write");
}

#[test]
fn collecting_nothing_matches_plain_compaction() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.delete(b"a").unwrap();
    let mut compacted = db
        .compact_into_collecting(VecStorage::new(), [2; 16], |_, _| false)
        .unwrap();
    let feed: Vec<_> = compacted
        .events_since(HEADER_LEN)
        .unwrap()
        .map(|r| r.unwrap().1)
        .collect();
    assert_eq!(
        feed.len(),
        1,
        "a's tombstone is kept when nothing is collectable"
    );
}

#[test]
fn compaction_refuses_non_empty_target() {
    let mut db = fresh();
    db.put(b"k", b"v").unwrap();
    let mut dirty = VecStorage::new();
    dirty.append(b"junk").unwrap();
    assert!(matches!(
        db.compact_into(dirty, [2; 16]),
        Err(Error::TargetNotEmpty)
    ));
}

#[test]
fn compacted_store_reopens_and_old_cursors_are_detectably_stale() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let cursor_gen = db.generation();
    let compacted = db.compact_into(VecStorage::new(), [2; 16]).unwrap();
    assert_ne!(compacted.generation(), cursor_gen);

    let storage = compacted.into_storage();
    let mut reopened =
        Hyle::open_with_session(storage, NODE, Key::from_bytes(KEY), [3; 16]).unwrap();
    assert_eq!(reopened.generation(), cursor_gen + 1);
    assert_eq!(read(&mut reopened, b"a").as_deref(), Some(&b"1"[..]));
    assert_eq!(read(&mut reopened, b"b").as_deref(), Some(&b"2"[..]));
}

#[cfg(feature = "std")]
mod file {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tpt-hyle-compact-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn compact_file_swaps_generation_and_leaves_one_file() {
        let dir = scratch("swap");
        let path = dir.join("db");
        let mut db = Hyle::open_file_with_key(&path, NODE, Key::from_bytes(KEY)).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"a", b"2").unwrap();
        db.put(b"gone", b"x").unwrap();
        db.delete(b"gone").unwrap();

        let mut db = db.compact_file(&path).unwrap();
        assert_eq!(db.generation(), 1);
        assert_eq!(read(&mut db, b"a").as_deref(), Some(&b"2"[..]));
        assert!(
            !dir.join("db.tmp").exists(),
            "the temp file is renamed away"
        );
        drop(db);

        let mut db = Hyle::open_file_with_key(&path, NODE, Key::from_bytes(KEY)).unwrap();
        assert_eq!(read(&mut db, b"a").as_deref(), Some(&b"2"[..]));
        assert_eq!(read(&mut db, b"gone"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_leftover_temp_file_does_not_affect_the_store() {
        let dir = scratch("crash");
        let path = dir.join("db");
        {
            let mut db = Hyle::open_file_with_key(&path, NODE, Key::from_bytes(KEY)).unwrap();
            db.put(b"a", b"1").unwrap();
        }
        // A compaction that died before its rename leaves a partial temp file.
        std::fs::write(dir.join("db.tmp"), b"partial").unwrap();
        let mut db = Hyle::open_file_with_key(&path, NODE, Key::from_bytes(KEY)).unwrap();
        assert_eq!(read(&mut db, b"a").as_deref(), Some(&b"1"[..]));
        // The next compaction replaces the leftover rather than failing on it.
        let db = db.compact_file(&path).unwrap();
        assert_eq!(db.generation(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
