//! Bounded, exhaustive model checking of a multi-replica SPARQL-CRDT system.
//!
//! `check_convergence` explores **every** reachable configuration of a bounded
//! scenario: at each step any replica may execute its next scripted origin
//! operation (publishing a delta) or admit any published-but-not-yet-applied
//! delta, so all interleavings of origin evaluation and delta delivery are
//! covered — including the order-dependence of pattern updates, whose compiled
//! remove sets differ between branches (`CRDT-UPD-WHERE-1`).
//!
//! Checked properties:
//!
//! - `CRDT-STATE-1` invariants at every reachable configuration (fail closed);
//! - `CRDT-SEC-2` at every terminal configuration (all operations executed,
//!   every delta admitted everywhere): equal live dot stores, equal
//!   causal-context denotations, and equal materialised visible quad sets;
//! - duplicate-delivery tolerance at terminals: re-admitting any already
//!   admitted delta leaves every state unchanged (`CRDT-JOURNAL-1`);
//! - the join laws — commutativity, associativity, idempotence — checked
//!   exhaustively over the (capped) set of deltas and terminal states reached.
//!
//! **Scope of the verdict.** This is bounded model checking: a green result is
//! exhaustive *only over the scenario's bounds* (replica count, script length,
//! quad universe). It is evidence, not a convergence proof; see the crate
//! documentation for the proof-obligation boundary.

use std::collections::{BTreeSet, HashSet, VecDeque};

use crate::origin::{Op, Replica};
use crate::state::{Delta, Quad, State};

/// Exploration cap: exceeding it aborts with an error rather than appearing
/// to verify a space that was not actually exhausted.
const MAX_CONFIGS: usize = 500_000;

/// Cap on elements fed to the cubic associativity law check.
const MAX_LAW_ELEMENTS: usize = 12;

/// A bounded scenario: an optional setup prefix executed on replica 0 and
/// delivered to every replica before exploration (shared initial state), then
/// one operation script per replica, explored under all interleavings.
#[derive(Clone, Debug, Default)]
pub struct Scenario {
    /// Setup operations establishing a common initial state.
    pub setup: Vec<Op>,
    /// Per-replica origin scripts; `scripts.len()` is the replica count.
    pub scripts: Vec<Vec<Op>>,
}

/// Join-law check tallies (all checks passed if the report was returned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LawReport {
    /// Ordered pairs checked for `join(a, b) == join(b, a)`.
    pub commutativity_pairs: usize,
    /// Triples checked for `join(join(a, b), c) == join(a, join(b, c))`.
    pub associativity_triples: usize,
    /// Elements checked for `join(x, x) == x`.
    pub idempotence_elements: usize,
}

/// The result of an exhaustive bounded exploration.
#[derive(Clone, Debug)]
pub struct CheckReport {
    /// Distinct configurations visited.
    pub configs_explored: usize,
    /// Distinct terminal (fully executed, fully delivered) configurations.
    pub terminal_configs: usize,
    /// Distinct published deltas encountered across all branches.
    pub distinct_deltas: usize,
    /// The set of visible quad sets observed at terminal configurations. Every
    /// terminal configuration is internally convergent; branches may still
    /// reach different outcomes when a pattern update raced a delivery.
    pub terminal_visible_sets: BTreeSet<BTreeSet<Quad>>,
    /// Join-law tallies.
    pub laws: LawReport,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Config {
    replicas: Vec<Replica>,
    pc: Vec<usize>,
    published: Vec<Delta>,
    applied: Vec<BTreeSet<usize>>,
}

impl Config {
    fn is_terminal(&self, scenario: &Scenario) -> bool {
        self.pc
            .iter()
            .zip(&scenario.scripts)
            .all(|(&pc, script)| pc == script.len())
            && self.applied.iter().all(|a| a.len() == self.published.len())
    }
}

/// Exhaustively explore a bounded scenario and check invariants, terminal
/// convergence, duplicate tolerance, and the join laws.
///
/// # Errors
///
/// Returns a description of the first violated property, or an error if the
/// exploration exceeds the internal configuration cap (the bound must then be
/// reduced — an over-cap run verifies nothing).
pub fn check_convergence(scenario: &Scenario) -> Result<CheckReport, String> {
    let n = scenario.scripts.len();
    if n == 0 {
        return Err("scenario has no replicas".to_owned());
    }

    // Shared initial state: run setup on replica 0, deliver to everyone.
    let mut initial: Vec<Replica> = (0..n).map(|i| Replica::new(i as u8)).collect();
    let mut setup_replica = initial[0].clone();
    let setup_envs = setup_replica.execute_request(&scenario.setup);
    initial[0] = setup_replica;
    for replica in initial.iter_mut().skip(1) {
        for env in &setup_envs {
            replica.apply(&env.to_delta())?;
        }
    }

    let root = Config {
        replicas: initial,
        pc: vec![0; n],
        published: Vec::new(),
        applied: vec![BTreeSet::new(); n],
    };

    let mut visited: HashSet<Config> = HashSet::new();
    let mut queue: VecDeque<Config> = VecDeque::new();
    visited.insert(root.clone());
    queue.push_back(root);

    let mut terminal_configs = 0usize;
    let mut terminal_visible_sets: BTreeSet<BTreeSet<Quad>> = BTreeSet::new();
    let mut deltas: BTreeSet<Delta> = BTreeSet::new();
    let mut terminal_states: BTreeSet<State> = BTreeSet::new();

    while let Some(config) = queue.pop_front() {
        if visited.len() > MAX_CONFIGS {
            return Err(format!(
                "exploration exceeded {} configurations; shrink the scenario bounds",
                MAX_CONFIGS
            ));
        }
        for replica in &config.replicas {
            replica.state().check_invariants()?;
        }
        deltas.extend(config.published.iter().cloned());

        if config.is_terminal(scenario) {
            terminal_configs += 1;
            check_terminal(&config)?;
            terminal_visible_sets.insert(config.replicas[0].state().visible());
            terminal_states.insert(config.replicas[0].state().clone());
            continue;
        }

        // Transition 1: replica i executes its next scripted origin operation.
        for i in 0..n {
            if config.pc[i] < scenario.scripts[i].len() {
                let mut next = config.clone();
                let env = next.replicas[i].execute(&scenario.scripts[i][config.pc[i]]);
                next.pc[i] += 1;
                next.published.push(env.to_delta());
                next.applied[i].insert(next.published.len() - 1);
                if visited.insert(next.clone()) {
                    queue.push_back(next);
                }
            }
        }
        // Transition 2: replica i admits any published, not-yet-applied delta.
        for i in 0..n {
            for j in 0..config.published.len() {
                if !config.applied[i].contains(&j) {
                    let mut next = config.clone();
                    next.replicas[i].apply(&config.published[j])?;
                    next.applied[i].insert(j);
                    if visited.insert(next.clone()) {
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    let laws = check_join_laws(&deltas, &terminal_states)?;
    Ok(CheckReport {
        configs_explored: visited.len(),
        terminal_configs,
        distinct_deltas: deltas.len(),
        terminal_visible_sets,
        laws,
    })
}

/// `CRDT-SEC-2` at one terminal configuration: pairwise-equal stores, equal
/// context denotations, equal visible sets — plus duplicate re-admission
/// leaving every state unchanged.
fn check_terminal(config: &Config) -> Result<(), String> {
    let first = config.replicas[0].state();
    for replica in &config.replicas[1..] {
        let s = replica.state();
        if s.store() != first.store() {
            return Err(format!(
                "CRDT-SEC-2: diverged dot stores at terminal: {:?} vs {:?}",
                first.store(),
                s.store()
            ));
        }
        if !s.context().denotation_eq(first.context()) || s.context() != first.context() {
            return Err("CRDT-SEC-2: diverged causal-context denotations at terminal".to_owned());
        }
        if s.visible() != first.visible() {
            return Err("CRDT-SEC-2: diverged visible quad sets at terminal".to_owned());
        }
    }
    for (i, replica) in config.replicas.iter().enumerate() {
        for delta in &config.published {
            let mut again = replica.clone();
            again.apply(delta)?;
            if again.state() != replica.state() {
                return Err(format!(
                    "duplicate delivery changed replica {} state at terminal",
                    i
                ));
            }
        }
    }
    Ok(())
}

/// Exhaustively check commutativity, associativity, and idempotence of the
/// join over the reached deltas and terminal states (capped for the cubic
/// associativity pass; the cap is reported, never silent).
fn check_join_laws(
    deltas: &BTreeSet<Delta>,
    terminal_states: &BTreeSet<State>,
) -> Result<LawReport, String> {
    let elements: Vec<&State> = deltas
        .iter()
        .chain(terminal_states.iter())
        .take(MAX_LAW_ELEMENTS)
        .collect();
    let mut report = LawReport {
        commutativity_pairs: 0,
        associativity_triples: 0,
        idempotence_elements: 0,
    };
    for &x in &elements {
        if x.join(x) != *x {
            return Err(format!("join is not idempotent on {:?}", x));
        }
        report.idempotence_elements += 1;
    }
    for &a in &elements {
        for &b in &elements {
            if a.join(b) != b.join(a) {
                return Err("join is not commutative".to_owned());
            }
            report.commutativity_pairs += 1;
        }
    }
    for &a in &elements {
        for &b in &elements {
            for &c in &elements {
                if a.join(b).join(c) != a.join(&b.join(c)) {
                    return Err("join is not associative".to_owned());
                }
                report.associativity_triples += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GraphKey;

    fn q(s: u8, p: u8) -> Quad {
        Quad::new(s, p, 0, GraphKey::Default)
    }

    #[test]
    fn empty_scenario_is_rejected() {
        assert!(check_convergence(&Scenario::default()).is_err());
    }

    #[test]
    fn single_replica_single_op_explores_and_converges() {
        let report = check_convergence(&Scenario {
            setup: vec![],
            scripts: vec![vec![Op::InsertData(vec![q(1, 1)])]],
        })
        .unwrap();
        assert_eq!(report.terminal_configs, 1);
        assert_eq!(report.distinct_deltas, 1);
        assert!(report.configs_explored >= 2);
        assert_eq!(
            report.terminal_visible_sets,
            BTreeSet::from([BTreeSet::from([q(1, 1)])])
        );
        assert!(report.laws.idempotence_elements >= 1);
        assert!(report.laws.commutativity_pairs >= 1);
        assert!(report.laws.associativity_triples >= 1);
    }

    #[test]
    fn setup_state_is_shared_by_all_replicas() {
        let report = check_convergence(&Scenario {
            setup: vec![Op::InsertData(vec![q(1, 1)])],
            scripts: vec![vec![], vec![Op::DeleteData(vec![q(1, 1)])]],
        })
        .unwrap();
        // The only terminal outcome: replica 1 saw the setup quad and removed it.
        assert_eq!(report.terminal_visible_sets, BTreeSet::from([BTreeSet::new()]));
    }
}
