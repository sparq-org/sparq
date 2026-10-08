//! Automatic stratification of N3 rule documents for store-scoped negation-as-failure
//! (GH #6201, #5756).
//!
//! Store-scoped `log:notIncludes`, `log:collectAllIn` and `log:forAllIn` are
//! NON-MONOTONIC: their answer over the current store can flip as the closure grows, and
//! the chainer never retracts a derivation. A single fixpoint over the whole document
//! therefore fails OPEN whenever a rule negates (or aggregates over) a predicate that
//! another rule of the same document derives: the negating rule fires in round 0, before
//! the derivation it should have waited for.
//!
//! [`stratify`] builds the predicate dependency graph over the forward AND backward rules
//! (a premise predicate inside a store-scoped negation / aggregation clause is a NEGATIVE
//! edge, every other premise predicate a positive one; a `{ … }` formula-literal scope is
//! local and adds no edge; a variable predicate on either side stands for every
//! predicate) and assigns each forward rule the least stratum that puts it after every
//! rule it negates over. The fixpoint drivers ([`super::run_closure`] and the compiled
//! engine's `BoundRuleSet::eval`) then run their unchanged semi-naive loop once per
//! stratum.
//!
//! * No store-scoped negation, or negation over BASE predicates only (nothing derives
//!   them): a single stratum — the drivers take exactly their pre-stratification path.
//! * A cycle through negation (not stratifiable): a single stratum (today's single-pass
//!   behaviour) plus a diagnostic naming the predicate on the cycle.
//!
//! The analysis is conservative: anything it cannot resolve statically (a variable
//! predicate, a variable scope that may be bound to a formula at run time) is treated as
//! store-wide. Running a monotone rule in a later stratum never changes the closure, so
//! over-approximation can only add strata or a spurious cycle diagnostic, never change a
//! stratifiable program's answer.

use super::model::{Rule, Term};
use super::{collect_op, scope_op, ScopeOp};
use rustc_hash::{FxHashMap, FxHashSet};

/// The builtin vocabularies (`log:`, `math:`, `string:`, `list:`, `time:` …): evaluated,
/// never matched against stored facts, so they carry no dependency.
const SWAP_NS: &str = "http://www.w3.org/2000/10/swap/";

/// A forward-rule stratum assignment ([`stratify`]).
#[derive(Debug, Default)]
pub(crate) struct Strata {
    /// Forward-rule index → stratum, `0..n_strata`. `None` ⇔ a single stratum (every
    /// rule in stratum 0), the drivers' unchanged fast path.
    pub(crate) rule_stratum: Option<Vec<usize>>,
    /// Number of strata (≥ 1).
    pub(crate) n_strata: usize,
    /// Set when the document negates through a dependency cycle: it is evaluated as one
    /// stratum (single-pass, the pre-stratification behaviour) and its store-scoped
    /// negation may fail open.
    pub(crate) warning: Option<String>,
}

impl Strata {
    fn single(warning: Option<String>) -> Self {
        Strata {
            rule_stratum: None,
            n_strata: 1,
            warning,
        }
    }
}

/// A predicate a rule consumes or produces: a ground IRI, or any predicate.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Pred<'a> {
    Iri(&'a str),
    Any,
}

/// A predicate position's dependency key; `None` for builtins (no stored facts).
fn pred_of(t: &Term) -> Option<Pred<'_>> {
    match t {
        Term::Iri(p) if p.starts_with(SWAP_NS) => None,
        Term::Iri(p) => Some(Pred::Iri(p)),
        _ => Some(Pred::Any),
    }
}

/// A scope term that is syntactically LOCAL: a `{ … }` formula literal, or `{}` (which
/// parses as the literal `true`). Anything else may denote the current store.
fn local_scope(t: &Term) -> bool {
    match t {
        Term::Formula(_) => true,
        Term::Lit(v, _, _) => v == "true",
        _ => false,
    }
}

/// Collect `(predicate, negative)` premise dependencies of `atoms`, descending into
/// store-scoped `log:includes`/`supports` (positive context kept) and `log:notIncludes`
/// / `log:collectAllIn` / `log:forAllIn` clauses (negative context).
fn premise_deps<'a>(atoms: &'a [[Term; 3]], neg: bool, out: &mut Vec<(Pred<'a>, bool)>) {
    for atom in atoms {
        if let Some(op) = scope_op(&atom[1]) {
            if !local_scope(&atom[0]) {
                if let Term::Formula(inner) = &atom[2] {
                    premise_deps(inner, neg || matches!(op, ScopeOp::NotIncludes), out);
                }
            }
            continue;
        }
        if collect_op(&atom[1]).is_some() {
            if !local_scope(&atom[2]) {
                if let Term::List(members) = &atom[0] {
                    for m in members {
                        if let Term::Formula(clause) = m {
                            premise_deps(clause, true, out);
                        }
                    }
                }
            }
            continue;
        }
        if let Some(p) = pred_of(&atom[1]) {
            out.push((p, neg));
        }
    }
}

/// Stratify a document's forward `rules` (with its `backward` rules as proof-time
/// dependencies). See the module docs for the contract.
pub(crate) fn stratify(rules: &[Rule], backward: &[Rule]) -> Strata {
    let nf = rules.len();
    let all: Vec<&Rule> = rules.iter().chain(backward).collect();
    let deps: Vec<Vec<(Pred<'_>, bool)>> = all
        .iter()
        .map(|r| {
            let mut d = Vec::new();
            premise_deps(&r.premise, false, &mut d);
            d
        })
        .collect();
    // Fast exit 1: no store-scoped negation at all.
    if !deps.iter().flatten().any(|&(_, neg)| neg) {
        return Strata::single(None);
    }
    let prods: Vec<Vec<Pred<'_>>> = all
        .iter()
        .map(|r| r.conclusion.iter().filter_map(|c| pred_of(&c[1])).collect())
        .collect();
    // Fast exit 2: every negated predicate is a base predicate (no rule derives it).
    let produced: FxHashSet<Pred<'_>> = prods.iter().flatten().copied().collect();
    let any_produced = produced.contains(&Pred::Any);
    let negates_derived = deps.iter().flatten().any(|&(p, neg)| {
        neg && match p {
            Pred::Any => !produced.is_empty(),
            Pred::Iri(_) => any_produced || produced.contains(&p),
        }
    });
    if !negates_derived {
        return Strata::single(None);
    }

    // Bipartite dependency graph: rule nodes `0..all.len()`, then one node per predicate
    // (`Pred::Any` included, so a variable producer reaches a variable consumer).
    // producer rule → predicate (weight 0) → consumer rule (weight 1 when negative).
    let mut pred_ix: FxHashMap<Pred<'_>, usize> = FxHashMap::default();
    pred_ix.insert(Pred::Any, all.len());
    for p in prods
        .iter()
        .flatten()
        .chain(deps.iter().flatten().map(|(p, _)| p))
    {
        let next = all.len() + pred_ix.len();
        pred_ix.entry(*p).or_insert(next);
    }
    let n = all.len() + pred_ix.len();
    let mut edges: FxHashMap<usize, FxHashSet<usize>> = FxHashMap::default();
    let mut neg_edges: FxHashSet<(usize, usize)> = FxHashSet::default();
    let pred_nodes: Vec<usize> = pred_ix.values().copied().collect();
    for (r, ps) in prods.iter().enumerate() {
        for p in ps {
            match p {
                Pred::Any => {
                    let e = edges.entry(r).or_default();
                    e.extend(pred_nodes.iter().copied());
                }
                Pred::Iri(_) => {
                    edges.entry(r).or_default().insert(pred_ix[p]);
                }
            }
        }
    }
    for (r, ds) in deps.iter().enumerate() {
        for &(p, neg) in ds {
            let sources: &[usize] = match p {
                Pred::Any => &pred_nodes,
                Pred::Iri(_) => std::slice::from_ref(&pred_ix[&p]),
            };
            for &s in sources {
                edges.entry(s).or_default().insert(r);
                if neg {
                    neg_edges.insert((s, r));
                }
            }
        }
    }

    let sccs = crate::incremental::n3_sccs(n, &edges);
    let mut scc_of = vec![0usize; n];
    for (k, scc) in sccs.iter().enumerate() {
        for &v in scc {
            scc_of[v] = k;
        }
    }
    // A negative edge inside one strongly-connected component is a cycle through
    // negation: not stratifiable.
    if let Some(&(s, r)) = neg_edges.iter().find(|&&(s, r)| scc_of[s] == scc_of[r]) {
        let pred = pred_ix
            .iter()
            .find(|&(_, &ix)| ix == s)
            .map(|(p, _)| match p {
                Pred::Iri(i) => format!("<{i}>"),
                Pred::Any => "a variable predicate".to_string(),
            })
            .unwrap_or_default();
        // Name a variable-conclusion producer on the cycle: the usual reason a negated
        // base predicate still looks derived.
        let via_any = prods
            .iter()
            .enumerate()
            .any(|(u, ps)| scc_of[u] == scc_of[s] && ps.contains(&Pred::Any));
        let via = if via_any {
            " (through a rule whose conclusion has a variable predicate, which may derive any \
             predicate)"
        } else {
            ""
        };
        let rule = if r < nf {
            format!("forward rule {r}")
        } else {
            format!("backward rule {}", r - nf)
        };
        return Strata::single(Some(format!(
            "n3 stratification: {rule} negates or aggregates (store-scoped log:notIncludes / \
             log:collectAllIn / log:forAllIn) over {pred}, which depends on that rule's own \
             conclusions{via} (a cycle through negation). The document is not stratifiable and is \
             evaluated single-pass: that negation may see an incomplete store (fail open)."
        )));
    }
    // Longest path (counting negative edges) over the condensation; `n3_sccs` returns the
    // components in topological (dependencies-first) order.
    let mut level = vec![0usize; n];
    for scc in &sccs {
        let lvl = scc.iter().map(|&v| level[v]).max().unwrap_or(0);
        for &u in scc {
            level[u] = lvl;
        }
        for &u in scc {
            let Some(succ) = edges.get(&u) else { continue };
            for &v in succ {
                if scc_of[v] != scc_of[u] {
                    let w = usize::from(neg_edges.contains(&(u, v)));
                    level[v] = level[v].max(lvl + w);
                }
            }
        }
    }
    // Compact the forward rules' levels to `0..n_strata`.
    let mut distinct: Vec<usize> = level[..nf].to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() <= 1 {
        return Strata::single(None);
    }
    let rank: FxHashMap<usize, usize> = distinct.iter().enumerate().map(|(i, &l)| (l, i)).collect();
    Strata {
        rule_stratum: Some(level[..nf].iter().map(|l| rank[l]).collect()),
        n_strata: distinct.len(),
        warning: None,
    }
}
