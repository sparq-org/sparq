//! (sq-kskgn) Seeded differential coverage for persisted vector deltas.
#![cfg(feature = "delta")]

use sparq_vectors::{nearest_exact, VectorStore};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const DIM: usize = 4;
const BASE_IDS: u32 = 8;
const ALL_IDS: u32 = 16;
const WORKLOADS: u64 = 32;
const STEPS: usize = 48;

static SEQ: AtomicU64 = AtomicU64::new(0);

type Membership = Vec<Option<Vec<f32>>>;
type Rankings = Vec<Vec<(u32, f32)>>;

fn tmp(seed: u64) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sparq_delta_fixpoint_{}_{}_{}.spqv",
        std::process::id(),
        seed,
        n
    ))
}

/// Tiny local PRNG: reproducibility matters here, not statistical quality.
fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn vector(seed: &mut u64) -> [f32; DIM] {
    let mut out = [0.0; DIM];
    for value in &mut out {
        *value = ((next(seed) % 1000) + 1) as f32 / 1000.0;
    }
    out
}

fn snapshot(store: &VectorStore, queries: &[[f32; DIM]]) -> (Membership, Rankings) {
    let membership = (0..ALL_IDS)
        .map(|id| store.get(id).map(<[f32]>::to_vec))
        .collect();
    let nearest = queries
        .iter()
        .map(|query| nearest_exact(store, query, ALL_IDS as usize))
        .collect();
    (membership, nearest)
}

#[test]
fn delta_save_load_behavioral_fixpoint() {
    for workload in 0..WORKLOADS {
        let mut seed = 0x9e37_79b9_7f4a_7c15 ^ workload.wrapping_mul(0x1000_0000_01b3);
        let path = tmp(workload);
        let mut store = VectorStore::create(&path, DIM).unwrap();
        let mut live = [false; ALL_IDS as usize];

        for id in 0..BASE_IDS {
            store.put(id, &vector(&mut seed)).unwrap();
            live[id as usize] = true;
        }
        store.finalize().unwrap();

        // Force both persisted record kinds in every workload before adding generated churn.
        assert!(store.remove((next(&mut seed) % BASE_IDS as u64) as u32));
        let removed = (0..BASE_IDS).find(|&id| store.get(id).is_none()).unwrap();
        live[removed as usize] = false;
        let appended = BASE_IDS + (next(&mut seed) % (ALL_IDS - BASE_IDS) as u64) as u32;
        store.add(appended, &vector(&mut seed)).unwrap();
        live[appended as usize] = true;

        for _ in 0..STEPS {
            let id = (next(&mut seed) % ALL_IDS as u64) as u32;
            match (next(&mut seed) % 3, live[id as usize]) {
                (0, true) => {
                    assert!(store.remove(id));
                    live[id as usize] = false;
                }
                (1, true) => store.update(id, &vector(&mut seed)).unwrap(),
                (_, false) => {
                    store.add(id, &vector(&mut seed)).unwrap();
                    live[id as usize] = true;
                }
                _ => {}
            }
        }

        let mut queries = vec![
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.25, 0.5, 0.75, 1.0],
        ];
        queries.extend((0..5).map(|_| vector(&mut seed)));
        let before = snapshot(&store, &queries);

        let delta_path = store.save_delta().unwrap();
        drop(store);
        let reopened = VectorStore::open_with_delta(&path).unwrap();
        let after = snapshot(&reopened, &queries);

        assert_eq!(
            after.0, before.0,
            "membership mismatch in workload {workload}"
        );
        assert_eq!(
            after.1, before.1,
            "exact-nearest mismatch in workload {workload}"
        );

        drop(reopened);
        std::fs::remove_file(path).ok();
        std::fs::remove_file(delta_path).ok();
    }
}
