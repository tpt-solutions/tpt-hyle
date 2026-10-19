//! Convergence: replicas that wrote independently end up identical, no matter
//! the order in which they receive each other's changes, or how often.

use std::collections::BTreeMap;

use proptest::prelude::*;
use tpt_hyle::{Hyle, VecStorage};
use tpt_hyle_test_utils::strategies::op;
use tpt_hyle_test_utils::Op;
use tpt_rift::{Delta, Replica};

type R = Replica<VecStorage, VecStorage>;

/// Sentinel peer id that matches no real event, so an export includes
/// everything.
const NO_PEER: u128 = u128::MAX;

fn replica(node: u128) -> R {
    Replica::new(Hyle::open(VecStorage::new(), VecStorage::new(), node).unwrap())
}

fn read_all(r: &mut R) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut buf = [0u8; 64];
    for a in 0u8..4 {
        for keys in [vec![a], vec![a, 0], vec![a, 1], vec![a, 2], vec![a, 3]] {
            if let Some(n) = r.store().get(&keys, &mut buf).unwrap() {
                out.insert(keys, buf[..n].to_vec());
            }
        }
    }
    out
}

fn fail<E: std::fmt::Display>(e: E) -> TestCaseError {
    TestCaseError::fail(e.to_string())
}

proptest! {
    #[test]
    fn replicas_converge_under_reordering_and_duplication(
        writes in prop::collection::vec((0usize..3, op()), 0..80),
        schedule in prop::collection::vec(0usize..1000, 0..60),
    ) {
        // Phase 1: each replica takes its share of writes while offline.
        let mut replicas = [replica(1), replica(2), replica(3)];
        for (who, op) in &writes {
            let r = &mut replicas[*who];
            match op {
                Op::Put(k, v) => { r.store().put(k, v).map_err(fail)?; }
                Op::Delete(k) => { r.store().delete(k).map_err(fail)?; }
            }
        }

        // Phase 2: every replica exports its full log, as it would on
        // reconnect.
        let deltas: Vec<Delta> = replicas
            .iter_mut()
            .map(|r| {
                let start = r.start_cursor();
                r.export(NO_PEER, start, usize::MAX).map_err(fail).map(|e| e.delta)
            })
            .collect::<Result<_, _>>()?;

        // Reference: per key, the change with the greatest (lamport, node_id)
        // decides the result. A tombstone there means the key is absent.
        let mut winners: BTreeMap<Vec<u8>, (u64, u128, bool, Vec<u8>)> = BTreeMap::new();
        for d in &deltas {
            for ch in &d.changes {
                let rank = (ch.lamport, ch.node_id);
                let better = match winners.get(&ch.key) {
                    Some(&(l, n, _, _)) => rank > (l, n),
                    None => true,
                };
                if better {
                    winners.insert(ch.key.clone(), (ch.lamport, ch.node_id, ch.tombstone, ch.value.clone()));
                }
            }
        }
        let expected: BTreeMap<Vec<u8>, Vec<u8>> = winners
            .into_iter()
            .filter(|(_, (_, _, tomb, _))| !tomb)
            .map(|(k, (_, _, _, v))| (k, v))
            .collect();

        // Phase 3: each replica receives all deltas, in a scrambled order,
        // with extra duplicates, and then once more in full order so that
        // every change is guaranteed to arrive at least once.
        for (i, r) in replicas.iter_mut().enumerate() {
            let n = deltas.len();
            for &s in &schedule {
                r.import(&deltas[(s + i) % n]).map_err(fail)?;
            }
            for d in &deltas {
                r.import(d).map_err(fail)?;
            }
        }

        // Phase 4: every replica matches the reference, and so each other.
        for r in replicas.iter_mut() {
            prop_assert_eq!(read_all(r), expected.clone());
        }
    }
}
