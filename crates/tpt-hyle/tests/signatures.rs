use tpt_hyle::{verify_event, ChangeEvent, Hyle, Key, VecStorage, HEADER_LEN};

const KEY: [u8; 32] = [0x33; 32];

fn store(node: u128, session: u8) -> Hyle<VecStorage> {
    Hyle::open_with_session(VecStorage::new(), node, Key::from_bytes(KEY), [session; 16]).unwrap()
}

/// An event with its key, value, and signature.
type Stored = (ChangeEvent, Vec<u8>, Vec<u8>, [u8; 64]);

/// Every event in the log with its key and value, for checking signatures.
fn events(db: &mut Hyle<VecStorage>) -> Vec<Stored> {
    let evs: Vec<ChangeEvent> = db
        .events_since(HEADER_LEN)
        .unwrap()
        .map(|r| r.unwrap().1)
        .collect();
    evs.into_iter()
        .map(|ev| {
            let mut key = vec![0u8; ev.key_len as usize];
            let mut value = vec![0u8; ev.payload_len as usize];
            db.read_record(&ev, &mut key, &mut value).unwrap();
            let sig = db.event_signature(&ev).unwrap();
            (ev, key, value, sig)
        })
        .collect()
}

fn check(public: &[u8; 32], ev: &ChangeEvent, key: &[u8], value: &[u8], sig: &[u8; 64]) -> bool {
    verify_event(
        public,
        ev.node_id,
        ev.lamport,
        ev.tombstone,
        key,
        value,
        sig,
    )
}

#[test]
fn every_event_verifies_under_its_node_key() {
    let mut db = store(7, 1);
    db.put(b"a", b"one").unwrap();
    db.put(b"a", b"two").unwrap();
    db.delete(b"a").unwrap();
    let public = db.public_key();
    for (ev, key, value, sig) in events(&mut db) {
        assert!(check(&public, &ev, &key, &value, &sig));
    }
}

#[test]
fn a_changed_field_breaks_the_signature() {
    let mut db = store(7, 1);
    db.put(b"k", b"v").unwrap();
    let public = db.public_key();
    let (ev, key, value, sig) = events(&mut db).remove(0);

    assert!(!check(&public, &ev, &key, b"w", &sig), "value");
    assert!(!check(&public, &ev, b"j", &value, &sig), "key");
    let mut later = ev;
    later.lamport += 1;
    assert!(!check(&public, &later, &key, &value, &sig), "lamport");
    let mut other = ev;
    other.node_id = 8;
    assert!(!check(&public, &other, &key, &value, &sig), "node");
    let mut tomb = ev;
    tomb.tombstone = true;
    assert!(!check(&public, &tomb, &key, &value, &sig), "tombstone flag");
}

#[test]
fn another_store_key_does_not_verify() {
    let mut db = store(7, 1);
    db.put(b"k", b"v").unwrap();
    let (ev, key, value, sig) = events(&mut db).remove(0);
    let stranger =
        Hyle::open_with_session(VecStorage::new(), 7, Key::from_bytes([0x44; 32]), [2; 16])
            .unwrap();
    assert!(!check(&stranger.public_key(), &ev, &key, &value, &sig));
}

#[test]
fn keys_are_per_node_and_stable_across_opens() {
    let a = store(7, 1).public_key();
    let b = store(8, 1).public_key();
    assert_ne!(a, b, "each node has its own key");
    assert_eq!(
        a,
        store(7, 9).public_key(),
        "the same node and key give the same key"
    );
}

#[test]
fn compaction_keeps_each_origin_signature() {
    let mut db = store(7, 1);
    db.put(b"a", b"1").unwrap();
    db.put(b"a", b"2").unwrap();
    let before = events(&mut db);

    let mut compacted = db.compact_into(VecStorage::new(), [5; 16]).unwrap();
    let public = compacted.public_key();
    let after = events(&mut compacted);
    assert_eq!(after.len(), 1);
    let (ev, key, value, sig) = &after[0];
    assert_eq!(
        sig, &before[1].3,
        "the signature is the origin's, not re-made"
    );
    assert!(check(&public, ev, key, value, sig));
}

#[test]
fn a_remote_event_keeps_the_signature_it_arrived_with() {
    let mut origin = store(7, 1);
    origin.put(b"k", b"v").unwrap();
    let (ev, key, value, sig) = events(&mut origin).remove(0);

    let mut relay = store(8, 2);
    relay
        .put_remote(&key, &value, ev.lamport, ev.node_id, false, sig)
        .unwrap();
    let (_, _, _, got) = events(&mut relay).remove(0);
    assert_eq!(got, sig);
}
