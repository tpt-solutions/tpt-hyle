//! Long random histories with reopens and compactions.
//!
//! The central claim is that the sync log is the replication feed. So after
//! every cycle, replaying the feed from the header must rebuild exactly the
//! state that `get` reports.

use std::collections::BTreeMap;

use tpt_hyle::{Hyle, HEADER_LEN};
use tpt_hyle_test_utils::{mem_store, MemStore};

const NODE: u128 = 5;
const KEYS: u64 = 64;
type Model = BTreeMap<Vec<u8>, Vec<u8>>;

/// xorshift64*: small, deterministic, and enough for test input.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn key(i: u64) -> Vec<u8> {
    format!("key-{i:03}").into_bytes()
}

/// Applies `KEYS * 4` random operations to both the store and the model.
fn churn<St: tpt_hyle::Storage, Sy: tpt_hyle::Storage>(
    db: &mut Hyle<St, Sy>,
    model: &mut Model,
    rng: &mut Rng,
    ops: usize,
) {
    for _ in 0..ops {
        let k = key(rng.below(KEYS));
        if rng.below(4) == 0 {
            db.delete(&k).unwrap();
            model.remove(&k);
        } else {
            let len = rng.below(48) as usize;
            let v: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            db.put(&k, &v).unwrap();
            model.insert(k, v);
        }
    }
}

/// Rebuilds state by replaying the sync feed from the header.
fn replay_feed<St: tpt_hyle::Storage, Sy: tpt_hyle::Storage>(db: &mut Hyle<St, Sy>) -> Model {
    let events: Vec<_> = db
        .events_since(HEADER_LEN)
        .unwrap()
        .map(|item| item.unwrap().1)
        .collect();
    let mut model = Model::new();
    for ev in events {
        let mut k = vec![0u8; ev.key_len as usize];
        let mut v = vec![0u8; ev.payload_len as usize];
        db.read_record(&ev, &mut k, &mut v).unwrap();
        if ev.tombstone {
            model.remove(&k);
        } else {
            model.insert(k, v);
        }
    }
    model
}

fn check_gets<St: tpt_hyle::Storage, Sy: tpt_hyle::Storage>(db: &mut Hyle<St, Sy>, model: &Model) {
    let mut buf = [0u8; 64];
    for i in 0..KEYS {
        let k = key(i);
        let got = db.get(&k, &mut buf).unwrap().map(|n| buf[..n].to_vec());
        assert_eq!(got, model.get(&k).cloned(), "key {i}");
    }
}

#[test]
fn long_history_with_reopens_and_compactions() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut db: MemStore = mem_store(NODE);
    let mut model = Model::new();

    for cycle in 0..40 {
        churn(&mut db, &mut model, &mut rng, 250);
        assert_eq!(replay_feed(&mut db), model, "feed replay, cycle {cycle}");
        check_gets(&mut db, &model);

        // Reopen, as a restart would.
        let (state, sync) = db.into_parts();
        db = Hyle::open(state, sync, NODE).unwrap();
        check_gets(&mut db, &model);

        if cycle % 5 == 4 {
            let before = db.generation();
            db = db
                .compact_into(tpt_hyle::VecStorage::new(), tpt_hyle::VecStorage::new())
                .unwrap();
            assert_eq!(db.generation(), before + 1);
            assert_eq!(
                replay_feed(&mut db),
                model,
                "after compaction, cycle {cycle}"
            );
            check_gets(&mut db, &model);
        }
    }
}

#[test]
fn file_backed_history_with_file_compactions() {
    let dir = std::env::temp_dir().join(format!("tpt-hyle-stress-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("db");

    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut model = Model::new();
    let mut db = Hyle::open_file(&base, NODE).unwrap();

    for cycle in 0..8 {
        churn(&mut db, &mut model, &mut rng, 120);
        assert_eq!(replay_feed(&mut db), model, "feed replay, cycle {cycle}");

        drop(db);
        db = Hyle::open_file(&base, NODE).unwrap();
        check_gets(&mut db, &model);

        if cycle % 2 == 1 {
            db = db.compact_file(&base).unwrap();
            assert_eq!(
                replay_feed(&mut db),
                model,
                "after file compaction, cycle {cycle}"
            );
            check_gets(&mut db, &model);
        }
    }
    // Only the live generation's state file should remain.
    let states: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("db.state."))
        .collect();
    assert_eq!(states, vec![format!("db.state.{}", db.generation())]);

    drop(db);
    std::fs::remove_dir_all(&dir).unwrap();
}
