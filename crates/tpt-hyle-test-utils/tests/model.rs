//! Property tests: the store must agree with a `BTreeMap` model, during
//! operation, after reopening, and after compaction. A torn last batch must
//! roll back to the last committed state.

use std::collections::BTreeMap;

use proptest::prelude::*;
use tpt_hyle::{Storage, HEADER_LEN};
use tpt_hyle_test_utils::strategies::op;
use tpt_hyle_test_utils::{fresh_session, mem_store, reopen_mem, MemStore, Op};

const NODE: u128 = 42;
type Model = BTreeMap<Vec<u8>, Vec<u8>>;

/// Length of a commit frame: header 29, sealed payload 60 + 16 tag.
const COMMIT_FRAME_LEN: u64 = 105;

fn fail<E: std::fmt::Display>(e: E) -> TestCaseError {
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
/// each time the store commits a batch. `snapshots[i]` is the state after `i`
/// commits.
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
                model.remove(k);
                snapshots.push(model.clone());
            } else {
                model.remove(k);
            }
        }
    }
    Ok(())
}

fn check(store: &mut MemStore, model: &Model) -> Result<(), TestCaseError> {
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

        let mut reopened = reopen_mem(store.into_storage(), NODE);
        check(&mut reopened, &model)?;
    }

    #[test]
    fn torn_last_batch_rolls_back_to_the_last_committed_state(
        ops in prop::collection::vec(op(), 1..60),
        cut in 1u64..COMMIT_FRAME_LEN,
    ) {
        let mut store = mem_store(NODE);
        let mut model = Model::new();
        let mut snapshots = vec![Model::new()];
        for op in &ops {
            apply(&mut store, &mut model, &mut snapshots, op)?;
        }
        let commits = snapshots.len() - 1;
        prop_assume!(commits >= 1);

        // Cut into the last commit frame, as a crash mid-flush would.
        let mut storage = store.into_storage();
        let len = storage.len();
        prop_assert!(len > HEADER_LEN);
        storage.truncate(len - cut).map_err(fail)?;

        let mut recovered = reopen_mem(storage, NODE);
        check(&mut recovered, &snapshots[commits - 1])?;

        // The repaired store accepts new writes.
        recovered.put(&[0], b"after").map_err(fail)?;
        let mut expected = snapshots[commits - 1].clone();
        expected.insert(vec![0], b"after".to_vec());
        check(&mut recovered, &expected)?;
    }

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
        let len_before = store.sync_end();

        let mut compacted = store
            .compact_into(tpt_hyle::VecStorage::new(), fresh_session())
            .map_err(fail)?;
        check(&mut compacted, &model)?;
        prop_assert_eq!(compacted.lamport(), lamport);
        prop_assert_eq!(compacted.generation(), generation + 1);
        prop_assert!(compacted.sync_end() <= len_before, "compaction never grows the log");

        let next = compacted.put(&[0], b"next").map_err(fail)?;
        prop_assert_eq!(next.lamport, lamport + 1);
    }
}
