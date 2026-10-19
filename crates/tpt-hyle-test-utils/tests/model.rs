//! Property tests: the store must agree with a `BTreeMap` model, both during
//! operation and after reopening, and must recover a torn tail to the last
//! committed state.

use std::collections::BTreeMap;

use proptest::prelude::*;
use tpt_hyle::{Error, Hyle, Storage, HEADER_LEN, SYNC_RECORD_LEN};
use tpt_hyle_test_utils::strategies::op;
use tpt_hyle_test_utils::{mem_store, MemStore, Op};

const NODE: u128 = 42;
type Model = BTreeMap<Vec<u8>, Vec<u8>>;

fn fail(e: Error) -> TestCaseError {
    TestCaseError::fail(e.to_string())
}

/// Every key the strategies can produce: 1 or 2 bytes, each in `0..4`.
fn universe() -> Vec<Vec<u8>> {
    let mut keys = Vec::new();
    for a in 0u8..4 {
        keys.push(vec![a]);
        for b in 0u8..4 {
            keys.push(vec![a, b]);
        }
    }
    keys
}

/// Applies `op` to the store and the model. Records a snapshot of the model
/// each time the store commits a sync event. `snapshots[i]` is the state
/// after `i` events.
fn apply(
    store: &mut MemStore,
    model: &mut Model,
    snapshots: &mut Vec<Model>,
    op: &Op,
) -> Result<(), TestCaseError> {
    match op {
        Op::Put(k, v) => {
            store.put(k, v).map_err(fail)?;
            model.insert(k.clone(), v.clone());
            snapshots.push(model.clone());
        }
        Op::Delete(k) => {
            if store.delete(k).map_err(fail)? {
                snapshots.push({
                    model.remove(k);
                    model.clone()
                });
            } else {
                model.remove(k);
            }
        }
    }
    Ok(())
}

fn check(store: &mut Hyle<impl Storage, impl Storage>, model: &Model) -> Result<(), TestCaseError> {
    for key in universe() {
        let mut buf = [0u8; 64];
        let got = store
            .get(&key, &mut buf)
            .map_err(fail)?
            .map(|n| buf[..n].to_vec());
        prop_assert_eq!(got, model.get(&key).cloned(), "key {:?}", key);
    }
    Ok(())
}

proptest! {
    #[test]
    fn store_matches_model_during_and_after_reopen(ops in prop::collection::vec(op(), 0..60)) {
        let mut store = mem_store(NODE);
        let mut model = Model::new();
        let mut snapshots = vec![Model::new()];
        for op in &ops {
            apply(&mut store, &mut model, &mut snapshots, op)?;
        }
        check(&mut store, &model)?;

        let (state, sync) = store.into_parts();
        let mut reopened = Hyle::open(state, sync, NODE).map_err(fail)?;
        check(&mut reopened, &model)?;
    }

    #[test]
    fn torn_tail_recovers_to_last_committed_state(
        ops in prop::collection::vec(op(), 1..60),
        cut in 1u64..SYNC_RECORD_LEN as u64,
    ) {
        let mut store = mem_store(NODE);
        let mut model = Model::new();
        let mut snapshots = vec![Model::new()];
        for op in &ops {
            apply(&mut store, &mut model, &mut snapshots, op)?;
        }
        let (state, mut sync) = store.into_parts();
        let len = sync.len();
        // Each committed write is one data record and one commit record.
        let events = snapshots.len() - 1;
        prop_assume!(events >= 1);
        prop_assert_eq!(len, HEADER_LEN + 2 * SYNC_RECORD_LEN as u64 * events as u64);

        // Simulate a crash that wrote only part of the final commit record.
        // The last batch then has no valid commit, so it must disappear.
        sync.truncate(len - cut).map_err(fail)?;

        let mut recovered = Hyle::open(state, sync, NODE).map_err(fail)?;
        check(&mut recovered, &snapshots[events - 1])?;

        // The repaired store accepts new writes.
        recovered.put(&[0], b"after").map_err(fail)?;
        let mut expected = snapshots[events - 1].clone();
        expected.insert(vec![0], b"after".to_vec());
        check(&mut recovered, &expected)?;
    }
}

proptest! {
    #[test]
    fn compaction_preserves_model_and_clock(ops in prop::collection::vec(op(), 0..60)) {
        let mut store = mem_store(NODE);
        let mut model = Model::new();
        let mut snapshots = vec![Model::new()];
        for op in &ops {
            apply(&mut store, &mut model, &mut snapshots, op)?;
        }
        let lamport = store.lamport();
        let generation = store.generation();
        let sync_before = store.sync_end();

        let mut compacted = store
            .compact_into(tpt_hyle::VecStorage::new(), tpt_hyle::VecStorage::new())
            .map_err(fail)?;
        check(&mut compacted, &model)?;
        prop_assert_eq!(compacted.lamport(), lamport);
        prop_assert_eq!(compacted.generation(), generation + 1);
        prop_assert!(compacted.sync_end() <= sync_before, "compaction never grows the log");

        let next = compacted.put(&[0], b"next").map_err(fail)?;
        prop_assert_eq!(next.lamport, lamport + 1);
    }
}
