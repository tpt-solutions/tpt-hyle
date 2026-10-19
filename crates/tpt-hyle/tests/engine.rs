use std::cell::Cell;
use std::rc::Rc;

use tpt_hyle::{Error, FixedStorage, Hyle, Storage, VecStorage, HEADER_LEN, MAX_KEY_LEN};

const NODE: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;

type Mem = Hyle<VecStorage, VecStorage>;

fn fresh() -> Mem {
    Hyle::open(VecStorage::new(), VecStorage::new(), NODE).unwrap()
}

fn get_string<St: Storage, Sy: Storage>(db: &mut Hyle<St, Sy>, key: &[u8]) -> Option<String> {
    let mut buf = [0u8; 256];
    db.get(key, &mut buf)
        .unwrap()
        .map(|n| String::from_utf8(buf[..n].to_vec()).unwrap())
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
    assert!(!db.contains_key(b"k").unwrap());
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
    let (state, sync) = db.into_parts();

    let mut db = Hyle::open(state, sync, NODE).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("2"));
    assert_eq!(get_string(&mut db, b"b").as_deref(), Some("3"));
    // Three committed writes. The delete of a missing key writes nothing.
    assert_eq!(db.lamport(), 3);
    assert_eq!(db.put(b"c", b"4").unwrap().lamport, 4);
}

#[test]
fn reopen_with_other_node_id_is_refused() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    let (state, sync) = db.into_parts();
    match Hyle::open(state, sync, NODE + 1) {
        Err(Error::NodeIdMismatch { stored, expected }) => {
            assert_eq!(stored, NODE);
            assert_eq!(expected, NODE + 1);
        }
        other => panic!("expected NodeIdMismatch, got {:?}", other.err()),
    }
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
    // The feed skips commit records, so the last cursor sits just before the
    // commit record that closes the last batch.
    assert_eq!(all.last().unwrap().0 + 56, db.sync_end());

    // Resume from the cursor after the first event: should yield the last two.
    let resume = all[0].0;
    let rest: Vec<_> = db
        .events_since(resume)
        .unwrap()
        .map(|r| r.unwrap().1.lamport)
        .collect();
    assert_eq!(rest, vec![2, 3]);

    // Cursor at the end yields nothing.
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
    let past_end = db.sync_end() + 56;
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
fn fixed_storage_store_works_without_heap() {
    let mut db = Hyle::open(FixedStorage::<512>::new(), FixedStorage::<512>::new(), NODE).unwrap();
    db.put(b"k", b"v").unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(db.get(b"k", &mut buf).unwrap(), Some(1));
    assert_eq!(buf[0], b'v');
}

#[test]
fn fixed_storage_reports_full() {
    let mut db = Hyle::open(
        FixedStorage::<128>::new(),
        FixedStorage::<1024>::new(),
        NODE,
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
fn torn_sync_record_is_truncated_on_reopen() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let (state, mut sync) = db.into_parts();
    // Simulate a crash that wrote only part of the last sync record.
    let len = sync.len();
    sync.truncate(len - 20).unwrap();

    let mut db = Hyle::open(state, sync, NODE).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("1"));
    assert_eq!(
        get_string(&mut db, b"b"),
        None,
        "uncommitted put is dropped"
    );
    assert_eq!((db.sync_end() - HEADER_LEN) % 56, 0);

    // The store is usable again and new writes land after the repaired tail.
    db.put(b"b", b"3").unwrap();
    assert_eq!(get_string(&mut db, b"b").as_deref(), Some("3"));
}

#[test]
fn orphaned_state_record_is_reclaimed_on_reopen() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    let (mut state, sync) = db.into_parts();
    let committed_state_len = state.len();
    // A state record with no matching sync event, as a crash between the
    // two appends would leave.
    state.append(&[0xFF; 20]).unwrap();

    let mut db = Hyle::open(state, sync, NODE).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("1"));
    let (state, _) = db.into_parts();
    assert_eq!(state.len(), committed_state_len);
}

#[test]
fn damage_before_the_tail_is_reported_not_silently_dropped() {
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    db.put(b"c", b"3").unwrap();
    let (state, mut sync) = db.into_parts();

    // Flip a bit in the middle sync record (the checksum field).
    let mid = HEADER_LEN as usize + 56;
    sync.bytes_mut()[mid] ^= 0x01;

    assert!(matches!(
        Hyle::open(state, sync, NODE),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn damaged_committed_record_is_reported_on_open() {
    // The last put is committed: its commit record is intact. Damage to its
    // data is therefore not a torn write, and open must refuse it.
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let (mut state, sync) = db.into_parts();
    let last = state.len() as usize - 1;
    state.bytes_mut()[last] ^= 0x40;

    assert!(matches!(
        Hyle::open(state, sync, NODE),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn uncommitted_tail_is_dropped_on_open() {
    // Simulate a batch that wrote its records but never reached its commit
    // record: strip the last commit record off the sync log.
    let mut db = fresh();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let (state, mut sync) = db.into_parts();
    let len = sync.len();
    sync.truncate(len - 56).unwrap();

    let mut db = Hyle::open(state, sync, NODE).unwrap();
    assert_eq!(get_string(&mut db, b"a").as_deref(), Some("1"));
    assert_eq!(get_string(&mut db, b"b"), None);
    assert_eq!(
        db.sync_end(),
        HEADER_LEN + 2 * 56,
        "data and commit of 'a' only"
    );
}

/// Wraps a `VecStorage` and XORs one byte on read once armed. Lets a test
/// damage a segment after `open` has already verified it.
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
    let mut db = Hyle::open(
        Armed {
            inner: VecStorage::new(),
            flip: flip.clone(),
        },
        Armed {
            inner: VecStorage::new(),
            flip: Rc::new(Cell::new(None)),
        },
        NODE,
    )
    .unwrap();
    db.put(b"k", b"value").unwrap();

    // Value starts after the 32-byte header, 10-byte record header and key.
    flip.set(Some(HEADER_LEN + 10 + 1 + 2));
    let mut buf = [0u8; 16];
    assert!(matches!(db.get(b"k", &mut buf), Err(Error::Corrupt(_))));
}

/// Flips every bit of every byte in a small store, one at a time. Open and
/// read must return a result (Ok or Err), never panic.
#[test]
fn single_bit_flips_never_panic() {
    let mut db = fresh();
    db.put(b"alpha", b"one").unwrap();
    db.put(b"beta", b"two-two").unwrap();
    db.delete(b"alpha").unwrap();
    db.put(b"gamma", b"").unwrap();
    let (state, sync) = db.into_parts();
    let state_len = state.len() as usize;
    let sync_len = sync.len() as usize;

    for which in 0..2 {
        let (seg_len, is_state) = if which == 0 {
            (state_len, true)
        } else {
            (sync_len, false)
        };
        for pos in 0..seg_len {
            for bit in 0..8 {
                let mut s = state.clone();
                let mut y = sync.clone();
                if is_state {
                    s.bytes_mut()[pos] ^= 1 << bit;
                } else {
                    y.bytes_mut()[pos] ^= 1 << bit;
                }
                if let Ok(mut db) = Hyle::open(s, y, NODE) {
                    let mut buf = [0u8; 16];
                    let _ = db.get(b"alpha", &mut buf);
                    let _ = db.get(b"beta", &mut buf);
                    let _ = db.get(b"gamma", &mut buf);
                    let _ = db.events_since(HEADER_LEN).map(|it| it.count());
                }
            }
        }
    }
}

#[cfg(feature = "std")]
#[test]
fn file_backed_store_survives_reopen() {
    let dir = std::env::temp_dir().join(format!("tpt-hyle-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("db");

    {
        let mut db = Hyle::open_file(&base, NODE).unwrap();
        db.put(b"user:1", b"persisted").unwrap();
        db.delete(b"tmp").unwrap();
    }
    {
        let mut db = Hyle::open_file(&base, NODE).unwrap();
        assert_eq!(get_string(&mut db, b"user:1").as_deref(), Some("persisted"));
        assert_eq!(db.lamport(), 1);
    }

    let _ = std::fs::remove_dir_all(&dir);
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

#[test]
fn remote_write_keeps_its_clock_and_origin() {
    let mut db = fresh();
    db.put_remote(b"k", b"from-peer", 50, 9, false).unwrap();
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
        db.put_remote(b"k", b"x", 1, 2, true),
        Err(Error::TombstoneWithValue)
    ));
    db.put_remote(b"k", b"", 1, 2, true).unwrap();
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
