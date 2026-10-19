use tpt_hyle::{Error, Hyle, Storage, VecStorage, HEADER_LEN};

const NODE: u128 = 7;

fn fresh() -> Hyle<VecStorage, VecStorage> {
    Hyle::open(VecStorage::new(), VecStorage::new(), NODE).unwrap()
}

fn read<St: Storage, Sy: Storage>(db: &mut Hyle<St, Sy>, key: &[u8]) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    db.get(key, &mut buf).unwrap().map(|n| buf[..n].to_vec())
}

#[test]
fn compaction_keeps_latest_event_per_key_and_tombstones() {
    let mut db = fresh();
    db.put(b"a", b"one").unwrap(); // lamport 1, superseded
    db.put(b"b", b"bee").unwrap(); // lamport 2, deleted below
    db.put(b"a", b"two").unwrap(); // lamport 3, live
    db.delete(b"b").unwrap(); // lamport 4, tombstone
    let old_gen = db.generation();
    let old_state = db.state_len();

    let (state, sync) = (VecStorage::new(), VecStorage::new());
    let mut compacted = db.compact_into(state, sync).unwrap();

    assert_eq!(compacted.generation(), old_gen + 1);
    assert!(
        compacted.state_len() < old_state,
        "superseded values dropped"
    );
    assert_eq!(read(&mut compacted, b"a").as_deref(), Some(&b"two"[..]));
    assert_eq!(read(&mut compacted, b"b"), None);

    // Two events survive: the live "a" and the tombstone for "b".
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

    let mut compacted = db
        .compact_into(VecStorage::new(), VecStorage::new())
        .unwrap();
    // Everything was a tombstone at the end, but the clock must not rewind.
    assert_eq!(compacted.lamport(), before);
    assert_eq!(compacted.put(b"new", b"x").unwrap().lamport, before + 1);
}

#[test]
fn compaction_keeps_original_node_ids() {
    let mut db = fresh();
    db.put(b"k", b"v").unwrap();
    let mut compacted = db
        .compact_into(VecStorage::new(), VecStorage::new())
        .unwrap();
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
fn compaction_refuses_non_empty_target() {
    let mut db = fresh();
    db.put(b"k", b"v").unwrap();
    let mut dirty = VecStorage::new();
    dirty.append(b"junk").unwrap();
    assert!(matches!(
        db.compact_into(dirty, VecStorage::new()),
        Err(Error::TargetNotEmpty)
    ));
}

#[test]
fn compacted_store_reopens_and_old_cursors_are_detectably_stale() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let cursor_gen = db.generation();
    let compacted = db
        .compact_into(VecStorage::new(), VecStorage::new())
        .unwrap();
    assert_ne!(compacted.generation(), cursor_gen);

    let (state, sync) = compacted.into_parts();
    let mut reopened = Hyle::open(state, sync, NODE).unwrap();
    assert_eq!(reopened.generation(), cursor_gen + 1);
    assert_eq!(read(&mut reopened, b"a").as_deref(), Some(&b"1"[..]));
    assert_eq!(read(&mut reopened, b"b").as_deref(), Some(&b"2"[..]));
}

#[test]
fn mismatched_generations_are_refused_on_open() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    let (old_state, old_sync) = db.into_parts();
    let mut db = Hyle::open(old_state.clone(), old_sync, NODE).unwrap();
    let compacted = db
        .compact_into(VecStorage::new(), VecStorage::new())
        .unwrap();
    let (_new_state, new_sync) = compacted.into_parts();

    // Old state with new sync: what a crash between two renames would leave.
    assert!(matches!(
        Hyle::open(old_state, new_sync, NODE),
        Err(Error::Corrupt(_))
    ));
}

#[cfg(feature = "std")]
mod file {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tpt-hyle-compact-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn compact_file_swaps_generation_and_removes_old_state() {
        let dir = scratch("swap");
        let base = dir.join("db");
        let mut db = Hyle::open_file(&base, NODE).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"a", b"2").unwrap();
        db.put(b"gone", b"x").unwrap();
        db.delete(b"gone").unwrap();

        let db = db.compact_file(&base).unwrap();
        assert_eq!(db.generation(), 1);
        drop(db);

        assert!(dir.join("db.state.1").exists());
        assert!(!dir.join("db.state.0").exists(), "old state removed");
        assert!(!dir.join("db.sync.tmp").exists());

        let mut db = Hyle::open_file(&base, NODE).unwrap();
        assert_eq!(read(&mut db, b"a").as_deref(), Some(&b"2"[..]));
        assert_eq!(read(&mut db, b"gone"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn leftovers_from_an_interrupted_compaction_are_cleaned_up() {
        let dir = scratch("crash");
        let base = dir.join("db");
        {
            let mut db = Hyle::open_file(&base, NODE).unwrap();
            db.put(b"a", b"1").unwrap();
        }
        // Simulate a crash before the commit rename: a half-built new state
        // file and a temp sync file exist beside the live generation 0.
        std::fs::write(dir.join("db.state.1"), b"partial").unwrap();
        std::fs::write(dir.join("db.sync.tmp"), b"partial").unwrap();

        let mut db = Hyle::open_file(&base, NODE).unwrap();
        assert_eq!(read(&mut db, b"a").as_deref(), Some(&b"1"[..]));
        assert_eq!(db.generation(), 0);
        assert!(!dir.join("db.state.1").exists());
        assert!(!dir.join("db.sync.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
