use tpt_hyle::{Error, FixedStorage, Hyle, Key, Storage, VecStorage};

const NODE: u128 = 3;
const KEY: [u8; 32] = [0x33; 32];

fn fresh() -> Hyle<VecStorage> {
    Hyle::open_with_session(VecStorage::new(), NODE, Key::from_bytes(KEY), [5; 16]).unwrap()
}

fn get<S: Storage>(db: &mut Hyle<S>, key: &[u8]) -> Option<Vec<u8>> {
    let mut buf = [0u8; 64];
    db.get(key, &mut buf).unwrap().map(|n| buf[..n].to_vec())
}

#[test]
fn committed_batch_lands_whole() {
    let mut db = fresh();
    let mut batch = db.batch().unwrap();
    batch.put(b"a", b"1").unwrap();
    batch.put(b"b", b"2").unwrap();
    assert_eq!(batch.commit().unwrap(), 2);
    assert_eq!(get(&mut db, b"a").as_deref(), Some(&b"1"[..]));
    assert_eq!(get(&mut db, b"b").as_deref(), Some(&b"2"[..]));
}

#[test]
fn dropped_batch_rolls_back_every_frame() {
    let mut db = fresh();
    db.put(b"keep", b"yes").unwrap();
    let (len_before, lamport_before) = (db.sync_end(), db.lamport());
    {
        let mut batch = db.batch().unwrap();
        batch.put(b"a", b"1").unwrap();
        batch.delete(b"keep").unwrap();
    }
    assert_eq!(db.sync_end(), len_before);
    assert_eq!(db.lamport(), lamport_before);
    assert_eq!(get(&mut db, b"a"), None);
    assert_eq!(get(&mut db, b"keep").as_deref(), Some(&b"yes"[..]));
}

#[test]
fn batch_sees_its_own_writes() {
    let mut db = fresh();
    let mut batch = db.batch().unwrap();
    batch.put(b"k", b"first").unwrap();
    batch.put(b"k", b"second").unwrap();
    assert_eq!(batch.latest_event(b"k").unwrap().unwrap().lamport, 2);
    assert!(batch.delete(b"k").unwrap());
    assert!(!batch.delete(b"missing").unwrap());
    batch.commit().unwrap();
    assert_eq!(get(&mut db, b"k"), None);
}

#[test]
fn empty_batch_writes_nothing() {
    let mut db = fresh();
    let before = db.sync_end();
    assert_eq!(db.batch().unwrap().commit().unwrap(), 0);
    assert_eq!(db.sync_end(), before);
}

#[test]
fn forced_commit_persists_the_clock_without_events() {
    let mut db = fresh();
    db.put(b"k", b"v").unwrap();
    db.observe_lamport(500);
    db.batch().unwrap().commit_forced().unwrap();
    let storage = db.into_storage();
    let reopened = Hyle::open_with_session(storage, NODE, Key::from_bytes(KEY), [6; 16]).unwrap();
    assert_eq!(reopened.lamport(), 500);
}

#[test]
fn a_full_store_poisons_and_refuses_new_batches_and_reads() {
    let mut db = Hyle::open_with_session(
        FixedStorage::<300>::new(),
        NODE,
        Key::from_bytes(KEY),
        [7; 16],
    )
    .unwrap();
    assert!(matches!(db.put(b"k", &[0u8; 200]), Err(Error::Full)));
    assert!(matches!(db.batch(), Err(Error::Poisoned)));
    let mut buf = [0u8; 8];
    assert!(matches!(db.get(b"k", &mut buf), Err(Error::Poisoned)));
}

#[test]
fn index_is_rebuilt_on_reopen_and_serves_many_updates() {
    let mut db = fresh();
    for round in 0..50u32 {
        db.put(b"hot", &round.to_le_bytes()).unwrap();
        db.put(format!("cold-{round}").as_bytes(), b"x").unwrap();
    }
    let storage = db.into_storage();
    let mut db = Hyle::open_with_session(storage, NODE, Key::from_bytes(KEY), [8; 16]).unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(db.get(b"hot", &mut buf).unwrap(), Some(4));
    assert_eq!(u32::from_le_bytes(buf[..4].try_into().unwrap()), 49);
    for round in 0..50u32 {
        assert!(db.contains_key(format!("cold-{round}").as_bytes()).unwrap());
    }
}

#[test]
fn live_entries_returns_latest_live_values_under_a_prefix() {
    let mut db = fresh();
    db.put(b"doc/a", b"1").unwrap();
    db.put(b"doc/b", b"2").unwrap();
    db.put(b"doc/a", b"3").unwrap();
    db.delete(b"doc/b").unwrap();
    db.put(b"other", b"x").unwrap();
    let entries = db.live_entries(b"doc/").unwrap();
    assert_eq!(entries, vec![(b"doc/a".to_vec(), b"3".to_vec())]);
    assert_eq!(db.live_entries(b"").unwrap().len(), 2);
}
