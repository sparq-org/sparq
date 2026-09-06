//! Delivery-schedule execution for generated (property-test) verification.
//!
//! A schedule is an explicit sequence of transport events over a fixed set of
//! published envelopes: single deliveries in any permutation, duplicated
//! deliveries (replay), batched deliveries (several envelope deltas joined
//! into one fragment before admission — `CRDT-UPD-BATCH-1`'s joined-fragment
//! form), full state/snapshot joins (`CRDT-JOURNAL-2`), and context
//! compaction round-trips (`CRDT-CTX-1`). Property tests generate arbitrary
//! schedules, append a completion pass so every envelope eventually reaches
//! every replica (assumption (d) of `CRDT-SEC-1`), and then assert the
//! `CRDT-SEC-2` convergence equalities via `assert_converged`.

use crate::origin::{Envelope, Replica};
use crate::state::Delta;

/// One transport event in a generated schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Admit one envelope's delta at one replica (also used for duplicated
    /// or replayed delivery — admission is idempotent).
    Deliver {
        /// Index of the receiving replica.
        to: usize,
        /// Index of the envelope being delivered.
        envelope: usize,
    },
    /// Join several envelopes' deltas into one batch fragment, then admit the
    /// batch atomically.
    DeliverBatch {
        /// Index of the receiving replica.
        to: usize,
        /// Indices of the batched envelopes.
        envelopes: Vec<usize>,
    },
    /// Full state/snapshot synchronisation: join replica `from`'s complete
    /// state — *and its durable provenance journal* — into replica `to`
    /// (`CRDT-JOURNAL-2` snapshot catch-up). Carrying provenance keeps the
    /// `CRDT-WIRE-4` reuse check enforceable for dots the snapshot has already
    /// retired from its live store.
    SnapshotSync {
        /// Index of the replica providing the snapshot.
        from: usize,
        /// Index of the receiving replica.
        to: usize,
    },
    /// Re-encode and re-normalise replica `at`'s causal context (compaction
    /// round-trip), which must preserve its denotation (`CRDT-CTX-1`).
    Compact {
        /// Index of the replica whose context is recompacted.
        at: usize,
    },
}

/// Execute a schedule, checking `CRDT-STATE-1` invariants after every step.
///
/// # Errors
///
/// Returns an error on an out-of-range replica/envelope index, a rejected
/// admission, a compaction denotation change, or an invariant violation.
pub fn run_schedule(
    replicas: &mut [Replica],
    envelopes: &[Envelope],
    steps: &[Step],
) -> Result<(), String> {
    let get_env = |i: usize| -> Result<&Envelope, String> {
        envelopes
            .get(i)
            .ok_or_else(|| format!("schedule references unknown envelope {}", i))
    };
    for step in steps {
        match step {
            Step::Deliver { to, envelope } => {
                let delta = get_env(*envelope)?.to_delta();
                replica_mut(replicas, *to)?.apply(&delta)?;
            }
            Step::DeliverBatch { to, envelopes: batch } => {
                let mut joined = Delta::bottom();
                for &e in batch {
                    joined = joined.join(&get_env(e)?.to_delta());
                }
                replica_mut(replicas, *to)?.apply(&joined)?;
            }
            Step::SnapshotSync { from, to } => {
                let snapshot = replica_mut(replicas, *from)?.snapshot();
                replica_mut(replicas, *to)?.apply_snapshot(&snapshot)?;
            }
            Step::Compact { at } => {
                let replica = replica_mut(replicas, *at)?;
                let mut state = replica.state().clone();
                state.recompact_context()?;
                if &state != replica.state() {
                    return Err("CRDT-CTX-1: compaction changed replica state".to_owned());
                }
            }
        }
        for replica in replicas.iter() {
            replica.state().check_invariants()?;
        }
    }
    Ok(())
}

/// Completion pass: deliver every envelope to every replica, each replica in
/// the caller-supplied envelope `order` (a permutation of all envelope
/// indices). Establishes assumption (d) of `CRDT-SEC-1` — every valid delta
/// is eventually incorporated — after an arbitrary schedule prefix.
///
/// # Errors
///
/// Returns an error if `order` is not a permutation of every envelope index,
/// or if any admission fails.
pub fn deliver_all(
    replicas: &mut [Replica],
    envelopes: &[Envelope],
    order: &[usize],
) -> Result<(), String> {
    let mut seen: Vec<usize> = order.to_vec();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != envelopes.len() || seen.iter().enumerate().any(|(i, &e)| i != e) {
        return Err("deliver_all order must be a permutation of all envelope indices".to_owned());
    }
    for replica in replicas.iter_mut() {
        for &e in order {
            let delta = envelopes[e].to_delta();
            replica.apply(&delta)?;
        }
    }
    Ok(())
}

/// Assert the `CRDT-SEC-2` equalities across all replicas: equal live dot
/// stores, equal causal-context denotations, equal visible quad sets.
///
/// # Errors
///
/// Returns a description of the first divergence found.
pub fn assert_converged(replicas: &[Replica]) -> Result<(), String> {
    let Some(first) = replicas.first() else {
        return Ok(());
    };
    for replica in &replicas[1..] {
        let a = first.state();
        let b = replica.state();
        if a.store() != b.store() {
            return Err(format!(
                "CRDT-SEC-2: dot stores diverged between replicas {} and {}",
                first.id(),
                replica.id()
            ));
        }
        if !a.context().denotation_eq(b.context()) || a.context() != b.context() {
            return Err(format!(
                "CRDT-SEC-2: context denotations diverged between replicas {} and {}",
                first.id(),
                replica.id()
            ));
        }
        if a.visible() != b.visible() {
            return Err(format!(
                "CRDT-SEC-2: visible quad sets diverged between replicas {} and {}",
                first.id(),
                replica.id()
            ));
        }
    }
    Ok(())
}

fn replica_mut(replicas: &mut [Replica], i: usize) -> Result<&mut Replica, String> {
    let len = replicas.len();
    replicas
        .get_mut(i)
        .ok_or_else(|| format!("schedule references unknown replica {} (of {})", i, len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::CausalContext;
    use crate::origin::Op;
    use crate::state::{GraphKey, Quad};

    fn q(s: u8) -> Quad {
        Quad::new(s, 0, 0, GraphKey::Default)
    }

    fn two_origins() -> (Vec<Replica>, Vec<Envelope>) {
        let mut a = Replica::new(0);
        let mut b = Replica::new(1);
        let mut envelopes = a.execute_request(&[Op::InsertData(vec![q(1)])]);
        envelopes.extend(b.execute_request(&[Op::InsertData(vec![q(2)]), Op::DeleteData(vec![q(2)])]));
        (vec![a, b], envelopes)
    }

    #[test]
    fn run_schedule_covers_every_step_kind_and_converges() {
        let (mut replicas, envelopes) = two_origins();
        run_schedule(
            &mut replicas,
            &envelopes,
            &[
                Step::Deliver { to: 0, envelope: 1 },
                Step::Deliver { to: 0, envelope: 1 }, // replayed duplicate
                Step::DeliverBatch { to: 0, envelopes: vec![1, 2] },
                Step::Compact { at: 0 },
                Step::SnapshotSync { from: 0, to: 1 },
            ],
        )
        .unwrap();
        deliver_all(&mut replicas, &envelopes, &[2, 0, 1]).unwrap();
        assert_converged(&replicas).unwrap();
        assert_eq!(replicas[0].state().visible(), std::collections::BTreeSet::from([q(1)]));
    }

    #[test]
    fn snapshot_sync_step_preserves_wire4_provenance() {
        // CRDT-WIRE-4 through a generated SnapshotSync (CRDT-JOURNAL-2): a dot
        // added then removed at replica 0, snapshot-synced to a fresh replica 1,
        // must leave replica 1 still able to reject a forged reuse of the retired
        // dot — the catch-up cannot silently lose the validation journal.
        let mut src = Replica::new(0);
        let envs = src.execute_request(&[Op::InsertData(vec![q(1)]), Op::DeleteData(vec![q(1)])]);
        let d = envs[0].adds[0].1;
        let mut replicas = vec![src, Replica::new(1)];
        run_schedule(&mut replicas, &[], &[Step::SnapshotSync { from: 0, to: 1 }]).unwrap();
        assert!(!replicas[1].state().is_visible(&q(1)));

        let forged = Delta::from_parts(
            std::collections::BTreeMap::from([(q(7), std::collections::BTreeSet::from([d]))]),
            CausalContext::from_dots([d]),
        );
        let before = replicas[1].state().clone();
        assert!(replicas[1].apply(&forged).is_err()); // snapshot-delivered provenance rejects reuse
        assert_eq!(*replicas[1].state(), before); // fail closed
    }

    #[test]
    fn run_schedule_rejects_unknown_indices() {
        let (mut replicas, envelopes) = two_origins();
        assert!(run_schedule(&mut replicas, &envelopes, &[Step::Deliver { to: 9, envelope: 0 }])
            .is_err());
        assert!(run_schedule(&mut replicas, &envelopes, &[Step::Deliver { to: 0, envelope: 9 }])
            .is_err());
    }

    #[test]
    fn deliver_all_requires_a_full_permutation() {
        let (mut replicas, envelopes) = two_origins();
        assert!(deliver_all(&mut replicas, &envelopes, &[0, 0, 1]).is_err());
        assert!(deliver_all(&mut replicas, &envelopes, &[0, 1]).is_err());
        deliver_all(&mut replicas, &envelopes, &[1, 2, 0]).unwrap();
    }

    #[test]
    fn assert_converged_reports_divergence_and_accepts_empty() {
        assert_converged(&[]).unwrap();
        let (mut replicas, envelopes) = two_origins();
        assert!(assert_converged(&replicas).is_err()); // not yet delivered
        deliver_all(&mut replicas, &envelopes, &[0, 1, 2]).unwrap();
        assert_converged(&replicas).unwrap();
    }
}
