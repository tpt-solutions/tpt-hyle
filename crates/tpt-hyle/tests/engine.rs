use std::cell::Cell;
use std::rc::Rc;

use tpt_hyle::{Error, FixedStorage, Hyle, Key, Storage, VecStorage, HEADER_LEN, MAX_KEY_LEN};

const NODE: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
const KEY: [u8; 32] = [0x42; 32];

/// A fresh session for every open, as the real store requires.
fn session(n: u8) -> [u8; 16] {
    [n; 16]
}

type Mem = Hyle<VecStorage>;

fn fresh() -> Mem {
    Hyle::open_with_session(VecStorage::new(), NODE, Key::from_bytes(KEY), session(1)).unwrap()
}

fn reopen(storage: VecStorage, n: u8) -> Result<Mem, Error> {
    Hyle::open_with_session(storage, NODE, Key::from_bytes(KEY), session(n))
}

fn get_string<S: Storage>(db: &mut Hyle<S>, key: &[u8]) -> Option<String> {
    let mut buf = [0u8; 256];
    db.get(key, &mut buf)
        .unwrap()
        .map(|n| String::from_utf8(buf[..n].to_vec()).unwrap())
}

/// The frames in a raw store file, as (offset, nonce) pairs.
fn frames(bytes: &[u8]) -> Vec<(usize, [u8; 24])> {
    let mut out = Vec::new();
    let mut at = HEADER_LEN as usize;
    while at + 29 <= bytes.len() {
        let sealed = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&bytes[at + 5..at + 29]);
        out.push((at, nonce));
        at += 29 + sealed;
    }
    out
}

#[test]
fn put_then_get_round_trips() {
    let mut db = fresh();
    db.put(b"user:101", b"alice").unwrap();
    assert_eq!(get_string(&mut db, b"user:101").as_deref(), Some("alice"));
    assert_eq!(get_string(&mut db, b"user:102"), None);
}

#[test]
fn latest_write_wins_and_empty_value_is_valid() {
    let mut db = fresh();
    db.put(b"k", b"one").unwrap();
    db.put(b"k", b"two").unwrap();
    assert_eq!(get_string(&mut db, b"k").as_deref(), Some("two"));
    db.put(b"k", b"").unwrap();
    assert_eq!(get_string(&mut db, b"k").as_deref(), Some(""));
    assert!(db.contains_key(b"k").unwrap());
}

#[test]
fn delete_hides_key_until_rewritten() {
    let mut db = fresh();
    db.put(b"k", b"v").unwrap();
    assert!(db.delete(b"k").unwrap());
    assert_eq!(get_string(&mut db, b"k"), None);
    assert!(!db.delete(b"k").unwrap(), "second delete is a no-op");
    assert!(!db.delete(b"never-existed").unwrap());
    db.put(b"k", b"back").unwrap();
    assert_eq!(get_string(&mut db, b"k").as_deref(), Some("back"));
}

#[test]
fn rejects_empty_and_oversized_keys() {
    let mut db = fresh();
    assert!(matches!(db.put(b"", b"v"), Err(Error::EmptyKey)));
    let long = vec![b'x'; MAX_KEY_LEN + 1];
    assert!(matches!(db.put(&long, b"v"), Err(Error::KeyTooLong)));
    let max = vec![b'x'; MAX_KEY_LEN];
    db.put(&max, b"ok").unwrap();
    assert_eq!(get_string(&mut db, &max).as_deref(), Some("ok"));
}

#[test]
fn small_output_buffer_reports_needed_size() {
    let mut db = fresh();
    db.put(b"k", b"0123456789").unwrap();
    let mut small = [0u8; 4];
    assert!(matches!(
        db.get(b"k", &mut small),
        Err(Error::BufferTooSmall { needed: 10 })
    ));
}

#[test]
fn clock_advances_per_write_and_observes_remote() {
    let mut db = fresh();
    assert_eq!(db.lamport(), 0);
    let a = db.put(b"a", b"1").unwrap();
    let b = db.put(b"b", b"2").unwrap();
    assert_eq!((a.lamport, b.lamport), (1, 2));
    assert_eq!(a.node_id, NODE);
    db.observe_lamport(100);
    db.observe_lamport(50);
    assert_eq!(db.lamport(), 100);
    assert_eq!(db.put(b"c", b"3").unwrap().lamport, 101);
}

#[test]
fn reopen_restores_data_and_clock() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"a", b"2").unwrap();
    db.delete(b"gone").unwrap();
    db.put(b"b", b"3").unwrap();
    let mut db = reopen(db.into_storage(), 2).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("2"));
    assert_eq!(get_string(&mut db, b"b").as_deref(), Some("3"));
    assert_eq!(db.lamport(), 3);
    assert_eq!(db.put(b"c", b"4").unwrap().lamport, 4);
}

#[test]
fn reopen_with_the_wrong_key_is_refused_and_the_file_is_untouched() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let storage = db.into_storage();
    let before = storage.bytes().to_vec();
    let wrong = Hyle::open_with_session(
        storage.clone(),
        NODE,
        Key::from_bytes([0x99; 32]),
        session(3),
    );
    assert!(matches!(wrong, Err(Error::WrongKey)));
    // A wrong key must never truncate what it cannot read.
    assert_eq!(storage.bytes(), &before[..]);
}

#[test]
fn reopen_with_other_node_id_is_refused() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    let storage = db.into_storage();
    match Hyle::open_with_session(storage, NODE + 1, Key::from_bytes(KEY), session(2)) {
        Err(Error::NodeIdMismatch { stored, expected }) => {
            assert_eq!(stored, NODE);
            assert_eq!(expected, NODE + 1);
        }
        other => panic!("expected NodeIdMismatch, got {:?}", other.err()),
    }
}

#[test]
fn plaintext_never_reaches_the_file() {
    let mut db = fresh();
    db.put(b"secret-key-marker", b"SECRET-VALUE-MARKER-0123456789")
        .unwrap();
    let bytes = db.storage().bytes().to_vec();
    assert!(!bytes.windows(12).any(|w| w == b"SECRET-VALUE"));
    assert!(!bytes.windows(10).any(|w| w == b"secret-key"));
}

#[test]
fn nonces_are_never_reused_even_after_a_rollback() {
    let mut db = fresh();
    db.put(b"keep", b"yes").unwrap();
    {
        let mut batch = db.batch().unwrap();
        batch.put(b"temp", b"will be rolled back").unwrap();
        // Dropped without commit: the frame is removed from the file.
    }
    db.put(b"after", b"rollback").unwrap();
    let raw = db.storage().bytes().to_vec();
    let nonces: Vec<[u8; 24]> = frames(&raw).into_iter().map(|(_, n)| n).collect();
    let unique: std::collections::BTreeSet<_> = nonces.iter().collect();
    assert_eq!(unique.len(), nonces.len(), "every frame has its own nonce");
}

#[test]
fn events_since_yields_feed_in_order_and_resumes() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    db.delete(b"a").unwrap();

    let all: Vec<_> = db
        .events_since(HEADER_LEN)
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let lamports: Vec<u64> = all.iter().map(|(_, ev)| ev.lamport).collect();
    assert_eq!(lamports, vec![1, 2, 3]);
    assert!(all[2].1.tombstone);
    // The last write is followed by its batch's commit frame, 105 bytes.
    assert_eq!(
        all.last().unwrap().0 + 105,
        db.sync_end(),
        "the feed ends at the log end"
    );

    let resume = all[0].0;
    let rest: Vec<_> = db
        .events_since(resume)
        .unwrap()
        .map(|r| r.unwrap().1.lamport)
        .collect();
    assert_eq!(rest, vec![2, 3]);
    assert_eq!(db.events_since(db.sync_end()).unwrap().count(), 0);
}

#[test]
fn events_since_rejects_bad_cursors() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    assert!(matches!(db.events_since(0), Err(Error::BadCursor(0))));
    assert!(matches!(
        db.events_since(HEADER_LEN + 1),
        Err(Error::BadCursor(_))
    ));
    let past_end = db.sync_end() + 1;
    assert!(matches!(
        db.events_since(past_end),
        Err(Error::BadCursor(_))
    ));
}

#[test]
fn read_record_returns_key_and_value_for_sync_consumers() {
    let mut db = fresh();
    let ev = db.put(b"user:7", b"payload").unwrap();
    let mut k = [0u8; 16];
    let mut v = [0u8; 16];
    let (kl, vl) = db.read_record(&ev, &mut k, &mut v).unwrap();
    assert_eq!(&k[..kl], b"user:7");
    assert_eq!(&v[..vl], b"payload");
}

#[test]
fn uncommitted_batch_is_dropped_on_open() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    let committed_len = db.storage().len();
    db.put(b"b", b"2").unwrap();
    // Remove the second batch's commit frame, as a crash before it would.
    let mut storage = db.into_storage();
    let cut = storage.len() - 105;
    storage.truncate(cut).unwrap();

    let mut db = reopen(storage, 2).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("1"));
    assert_eq!(get_string(&mut db, b"b"), None);
    assert_eq!(db.storage().len(), committed_len);
}

#[test]
fn torn_commit_frame_is_dropped_on_open() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let mut storage = db.into_storage();
    let len = storage.len();
    storage.truncate(len - 20).unwrap();
    let mut db = reopen(storage, 2).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("1"));
    assert_eq!(get_string(&mut db, b"b"), None);
    db.put(b"b", b"3").unwrap();
    assert_eq!(get_string(&mut db, b"b").as_deref(), Some("3"));
}

#[test]
fn damage_inside_committed_data_is_refused_on_open() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    db.put(b"c", b"3").unwrap();
    let mut storage = db.into_storage();
    // A byte inside the first frame's ciphertext. Later batches still verify.
    storage.bytes_mut()[HEADER_LEN as usize + 40] ^= 0x01;
    assert!(matches!(reopen(storage, 2), Err(Error::Corrupt(_))));
}

#[test]
fn a_damaged_final_commit_is_treated_as_an_uncommitted_batch() {
    // Indistinguishable from a torn write: the last batch is dropped whole.
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let mut storage = db.into_storage();
    let last = storage.len() as usize - 1;
    storage.bytes_mut()[last] ^= 0x40;
    let mut db = reopen(storage, 2).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("1"));
    assert_eq!(get_string(&mut db, b"b"), None);
}

/// Wraps a `VecStorage` and XORs one byte on read once armed. Lets a test
/// damage the file after `open` has already verified it.
struct Armed {
    inner: VecStorage,
    flip: Rc<Cell<Option<u64>>>,
}

impl Storage for Armed {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        self.inner.read_at(offset, buf)?;
        if let Some(at) = self.flip.get() {
            if at >= offset && at < offset + buf.len() as u64 {
                buf[(at - offset) as usize] ^= 0x40;
            }
        }
        Ok(())
    }
    fn append(&mut self, data: &[u8]) -> Result<u64, Error> {
        self.inner.append(data)
    }
    fn truncate(&mut self, len: u64) -> Result<(), Error> {
        self.inner.truncate(len)
    }
    fn flush(&mut self) -> Result<(), Error> {
        self.inner.flush()
    }
}

#[test]
fn value_damaged_after_open_is_reported_on_read() {
    let flip = Rc::new(Cell::new(None));
    let mut db = Hyle::open_with_session(
        Armed {
            inner: VecStorage::new(),
            flip: flip.clone(),
        },
        NODE,
        Key::from_bytes(KEY),
        session(1),
    )
    .unwrap();
    db.put(b"k", b"value").unwrap();
    // Inside the sealed value of the only frame.
    flip.set(Some(HEADER_LEN + 29 + 40));
    let mut buf = [0u8; 16];
    assert!(db.get(b"k", &mut buf).is_err());
}

/// Flips every bit of every byte in a small store, one at a time. Opening and
/// reading must return a result, never panic.
#[test]
fn single_bit_flips_never_panic() {
    let mut db = fresh();
    db.put(b"alpha", b"one").unwrap();
    db.put(b"beta", b"two-two").unwrap();
    db.delete(b"alpha").unwrap();
    db.put(b"gamma", b"").unwrap();
    let storage = db.into_storage();
    let len = storage.len() as usize;

    for pos in 0..len {
        for bit in 0..8 {
            let mut s = storage.clone();
            s.bytes_mut()[pos] ^= 1 << bit;
            if let Ok(mut db) = reopen(s, 9) {
                let mut buf = [0u8; 16];
                let _ = db.get(b"alpha", &mut buf);
                let _ = db.get(b"beta", &mut buf);
                let _ = db.get(b"gamma", &mut buf);
                let _ = db.events_since(HEADER_LEN).map(|it| it.count());
            }
        }
    }
}

#[test]
fn passphrase_store_reopens_with_the_same_passphrase_only() {
    let storage = VecStorage::new();
    let mut db = Hyle::open_with_passphrase(storage, NODE, b"correct horse").unwrap();
    db.put(b"k", b"v").unwrap();
    let storage = db.into_storage();

    let mut db = Hyle::open_with_passphrase(storage.clone(), NODE, b"correct horse").unwrap();
    assert_eq!(get_string(&mut db, b"k").as_deref(), Some("v"));
    assert!(matches!(
        Hyle::open_with_passphrase(storage, NODE, b"battery staple"),
        Err(Error::WrongKey)
    ));
}

#[cfg(feature = "std")]
#[test]
fn file_backed_store_survives_reopen() {
    let dir = std::env::temp_dir().join(format!("tpt-hyle-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db");
    let _ = std::fs::remove_file(&path);

    {
        let mut db = Hyle::open_file(&path, NODE, b"pw").unwrap();
        db.put(b"user:1", b"persisted").unwrap();
        db.delete(b"tmp").unwrap();
    }
    {
        let mut db = Hyle::open_file(&path, NODE, b"pw").unwrap();
        assert_eq!(get_string(&mut db, b"user:1").as_deref(), Some("persisted"));
        assert_eq!(db.lamport(), 1);
    }
    assert_eq!(tpt_hyle::file_node_id(&path).unwrap(), NODE);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn fixed_storage_is_a_storage_and_reports_full() {
    let mut db = Hyle::open_with_session(
        FixedStorage::<600>::new(),
        NODE,
        Key::from_bytes(KEY),
        session(1),
    )
    .unwrap();
    let mut wrote = 0;
    loop {
        match db.put(b"k", &[0u8; 32]) {
            Ok(_) => wrote += 1,
            Err(Error::Full) => break,
            Err(e) => panic!("unexpected error {e}"),
        }
        assert!(wrote < 100, "should have run out of space");
    }
    assert!(wrote >= 1);
}

#[test]
fn storage_trait_is_used_through_vec_backend() {
    let mut s = VecStorage::new();
    assert!(s.is_empty());
    assert_eq!(s.append(b"abc").unwrap(), 0);
    assert_eq!(s.append(b"def").unwrap(), 3);
    let mut buf = [0u8; 4];
    s.read_at(2, &mut buf).unwrap();
    assert_eq!(&buf, b"cdef");
    assert!(matches!(
        s.read_at(5, &mut buf),
        Err(Error::OutOfBounds { .. })
    ));
    s.truncate(2).unwrap();
    assert_eq!(s.len(), 2);
    assert!(matches!(s.truncate(9), Err(Error::OutOfBounds { .. })));
}

/// The signature `node` puts on a write. A scratch store with the same key
/// and node id makes the write at `lamport`, so the signature is the one that
/// node would produce.
fn signature_for(node: u128, lamport: u64, key: &[u8], value: &[u8], tombstone: bool) -> [u8; 64] {
    let mut scratch =
        Hyle::open_with_session(VecStorage::new(), node, Key::from_bytes(KEY), session(9)).unwrap();
    if tombstone {
        scratch.observe_lamport(lamport - 2);
        scratch.put(key, b"x").unwrap();
        scratch.delete(key).unwrap();
    } else {
        scratch.observe_lamport(lamport - 1);
        scratch.put(key, value).unwrap();
    }
    let ev = scratch
        .events_since(HEADER_LEN)
        .unwrap()
        .map(|r| r.unwrap().1)
        .last()
        .unwrap();
    scratch.event_signature(&ev).unwrap()
}

#[test]
fn remote_write_keeps_its_clock_and_origin() {
    let mut db = fresh();
    let sig = signature_for(9, 50, b"k", b"from-peer", false);
    db.put_remote(b"k", b"from-peer", 50, 9, false, sig)
        .unwrap();
    assert_eq!(db.lamport(), 50, "clock covers the remote timestamp");
    let latest = db.latest_event(b"k").unwrap().unwrap();
    assert_eq!((latest.lamport, latest.node_id), (50, 9));
    assert_eq!(db.put(b"k", b"local").unwrap().lamport, 51);
    assert_eq!(get_string(&mut db, b"k").as_deref(), Some("local"));
}

#[test]
fn remote_tombstone_must_not_carry_a_value() {
    let mut db = fresh();
    assert!(matches!(
        db.put_remote(b"k", b"x", 1, 2, true, [0; 64]),
        Err(Error::TombstoneWithValue)
    ));
    let sig = signature_for(2, 2, b"k", b"", true);
    db.put_remote(b"k", b"", 2, 2, true, sig).unwrap();
    assert!(!db.contains_key(b"k").unwrap());
}

#[test]
fn latest_event_reports_tombstones() {
    let mut db = fresh();
    assert!(db.latest_event(b"k").unwrap().is_none());
    db.put(b"k", b"v").unwrap();
    db.delete(b"k").unwrap();
    assert!(db.latest_event(b"k").unwrap().unwrap().tombstone);
}

#[test]
fn header_is_the_declared_length() {
    let db = fresh();
    assert_eq!(db.sync_end(), HEADER_LEN);
}
