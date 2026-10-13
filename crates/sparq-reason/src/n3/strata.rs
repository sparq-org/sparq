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
//! [`stratify`] builds a predicate dependency graph over the forward AND backward rules
//! and assigns each forward rule the least stratum that puts it after every rule it
//! negates over. The fixpoint drivers ([`super::run_closure`] and the compiled engine's
//! `BoundRuleSet::eval`) then run their unchanged semi-naive loop once per stratum.
//!
//! The analysis is fail-closed by construction: a premise or conclusion it cannot pin to
//! one stored predicate is UNKNOWN, which depends on (or produces) every predicate.
//!
//! * a plain premise atom with an IRI predicate the evaluators do not recognise as a
//!   builtin reads that predicate (whatever its namespace); a variable or other non-IRI
//!   predicate is UNKNOWN;
//! * a store-scoped `log:includes` / `log:supports` / `log:notIncludes` reads the atoms of
//!   its `{ … }` object (the evaluator reads only a literal formula object); inside
//!   `notIncludes` they are NEGATIVE. A scope that is not a formula literal counts as the
//!   store, even when a variable may turn out to hold a formula;
//! * a store-scoped `log:collectAllIn` / `log:forAllIn` reads its literal clause formulas
//!   NEGATIVELY; a clause or subject given through a variable is UNKNOWN and negative;
//! * every other premise relation reads what its registry entry declares
//!   ([`super::StoreRead`]): a stored predicate and a virtual list relation read their
//!   predicate; a list generator or functional builtin may walk a stored
//!   `rdf:first`/`rdf:rest` list, so it reads both, with the surrounding polarity; a
//!   builtin over its operands' values reads nothing;
//! * `log:conclusion` and `log:supports` close a formula under its own rules (a nested
//!   run): UNKNOWN, with the surrounding polarity. A nested run that is not stratifiable
//!   makes the whole run fail (see [`super::run_closure`]);
//! * a conclusion produces its IRI predicate; a variable predicate produces UNKNOWN.
//!
//! A negative edge that lies on a dependency cycle (UNKNOWN edges included) makes the
//! rules on that cycle, and every rule that depends on them, unstratifiable;
//! [`NegationCycles`] says what happens to them (an error by default).

use super::model::{Rule, Term};
use super::{collect_op, relation, scope_op, CollectOp, Relation, ScopeOp, StoreRead};
use rustc_hash::{FxHashMap, FxHashSet};

const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// What the N3 engines do with rules that negate through a dependency cycle: store-scoped
/// `log:notIncludes` / `log:collectAllIn` / `log:forAllIn` over a predicate that depends on
/// the negating rule's own conclusions, which no stratification can order soundly.
///
/// Only the rules on such a cycle, and the rules that depend on them, are affected; every
/// other rule of the document is stratified normally. A cycle inside a NESTED closure
/// (`log:conclusion`, `log:supports`) is always an error unless the run is
/// [`SinglePass`](Self::SinglePass): an incomplete nested closure would look like
/// evidence of absence to its consumer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NegationCycles {
    /// Refuse the document with an error naming the cycle. The default of every N3 entry
    /// point.
    #[default]
    Reject,
    /// Do not evaluate the affected rules, so nothing that depends on an unorderable
    /// negation is derived, and report a diagnostic.
    FailClosed,
    /// Legacy opt-in: run the affected rules together, single-pass, after every
    /// stratifiable rule, and report a diagnostic. Their negation may see an incomplete
    /// store (fail open).
    SinglePass,
}

/// Stratum value of a rule [`NegationCycles::FailClosed`] does not evaluate.
pub(crate) const DROPPED: usize = usize::MAX;

/// A forward-rule stratum assignment ([`stratify`]).
#[derive(Debug, Default, Clone)]
pub(crate) struct Strata {
    /// Forward-rule index → stratum in `0..n_strata`, or [`DROPPED`]. `None` ⇔ a single
    /// stratum holding every rule, the drivers' unchanged fast path.
    pub(crate) rule_stratum: Option<Vec<usize>>,
    /// Number of strata (≥ 1).
    pub(crate) n_strata: usize,
    /// Diagnostic for a negation cycle evaluated under [`NegationCycles::FailClosed`] or
    /// [`NegationCycles::SinglePass`].
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

/// A predicate a rule consumes or produces: a ground IRI, one class of `rdf:type`, or
/// UNKNOWN (every predicate).
///
/// `rdf:type` with a constant IRI object is keyed by its class, so negating one class from
/// a rule that concludes another is no cycle. Any other `rdf:type` atom (a variable, blank
/// or non-IRI object) is `Iri(rdf:type)`, which overlaps EVERY class ([`overlaps`]): an
/// atom with a constant IRI object can only match or derive triples with that object.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Pred<'a> {
    Iri(&'a str),
    Class(&'a str),
    Unknown,
}

/// Whether a triple one atom matches or derives can be a triple of the other.
fn overlaps(a: Pred<'_>, b: Pred<'_>) -> bool {
    match (a, b) {
        (Pred::Unknown, _) | (_, Pred::Unknown) => true,
        (Pred::Iri(x), Pred::Iri(y)) | (Pred::Class(x), Pred::Class(y)) => x == y,
        (Pred::Class(_), Pred::Iri(t)) | (Pred::Iri(t), Pred::Class(_)) => t == RDF_TYPE,
    }
}

/// The [`Pred`] of an atom.
fn atom_pred(atom: &[Term; 3]) -> Pred<'_> {
    match (&atom[1], &atom[2]) {
        (Term::Iri(p), Term::Iri(c)) if p == RDF_TYPE => Pred::Class(c),
        (Term::Iri(p), _) => Pred::Iri(p),
        _ => Pred::Unknown,
    }
}

/// A predicate dependency: `(predicate, negative)`.
type Dep<'a> = (Pred<'a>, bool);

/// A scope term that is syntactically LOCAL: a `{ … }` formula literal, or `{}` (which
/// parses as the literal `true`). Anything else may denote the current store.
fn local_scope(t: &Term) -> bool {
    match t {
        Term::Formula(_) => true,
        Term::Lit(v, _, _) => v == "true",
        _ => false,
    }
}

/// Cheap syntactic pre-check: does a premise hold a store-scoped non-monotonic operator?
/// (A store-scoped `log:includes` is followed into its formula, which may nest one.)
fn has_store_negation(atoms: &[[Term; 3]]) -> bool {
    atoms.iter().any(|a| {
        if let Some(op) = scope_op(&a[1]) {
            if local_scope(&a[0]) {
                return false;
            }
            return matches!(op, ScopeOp::NotIncludes)
                || matches!(&a[2], Term::Formula(f) if has_store_negation(f));
        }
        collect_op(&a[1]).is_some() && !local_scope(&a[2])
    })
}

/// The `(predicate, negative)` dependencies of premise `atoms`.
fn premise_deps<'a>(atoms: &'a [[Term; 3]], neg: bool, out: &mut Vec<Dep<'a>>) {
    for atom in atoms {
        let rel = relation(&atom[1]);
        if let Relation::Scope(op) = rel {
            if matches!(op, ScopeOp::Supports) {
                out.push((Pred::Unknown, neg));
            }
            if !local_scope(&atom[0]) {
                if let Term::Formula(inner) = &atom[2] {
                    premise_deps(inner, neg || matches!(op, ScopeOp::NotIncludes), out);
                }
            }
            continue;
        }
        if let Relation::Collect(op) = rel {
            if local_scope(&atom[2]) {
                continue;
            }
            let clauses: &[Term] = match (&atom[0], op) {
                (Term::List(ms), CollectOp::CollectAll) if ms.len() == 3 => &ms[1..2],
                (Term::List(ms), CollectOp::ForAll) if ms.len() == 2 => &ms[..],
                // A variable subject is bound at run time: unknown clauses.
                (Term::Var(_) | Term::Blank(_), _) => {
                    out.push((Pred::Unknown, true));
                    &[]
                }
                // A malformed literal list, or a subject that is no list, fails the premise.
                _ => &[],
            };
            for c in clauses {
                match c {
                    Term::Formula(f) => premise_deps(f, true, out),
                    // Bound at run time: an unknown clause.
                    Term::Var(_) | Term::Blank(_) => out.push((Pred::Unknown, true)),
                    // `{}` has no atoms; any other value fails the premise.
                    _ => {}
                }
            }
            continue;
        }
        // Everything else: what the relation's registry entry declares it reads.
        match rel.store_read() {
            StoreRead::Nothing => {}
            StoreRead::Joins | StoreRead::VirtualList => out.push((atom_pred(atom), neg)),
            StoreRead::ListCells(_) | StoreRead::MemberListCells => {
                out.push((Pred::Iri(RDF_FIRST), neg));
                out.push((Pred::Iri(RDF_REST), neg));
            }
            StoreRead::Nested => {
                out.push((Pred::Iri(RDF_FIRST), neg));
                out.push((Pred::Iri(RDF_REST), neg));
                out.push((Pred::Unknown, neg));
            }
            // Scope and aggregation relations are handled above; a future one that is
            // not reads everything, negatively.
            StoreRead::Scoped => out.push((Pred::Unknown, true)),
        }
    }
}

fn produces(rule: &Rule) -> Vec<Pred<'_>> {
    rule.conclusion.iter().map(atom_pred).collect()
}

/// Stratify a document's forward `rules`, with its `backward` rules as proof-time
/// dependencies. See the module docs for the contract.
///
/// Errors only under [`NegationCycles::Reject`], when some rule negates through a
/// dependency cycle.
pub(crate) fn stratify(
    rules: &[Rule],
    backward: &[Rule],
    cycles: NegationCycles,
) -> Result<Strata, String> {
    // Fast exit: no store-scoped non-monotonic operator anywhere, so one stratum is exact.
    if !rules
        .iter()
        .chain(backward)
        .any(|r| has_store_negation(&r.premise))
    {
        return Ok(Strata::single(None));
    }
    let nf = rules.len();
    let all: Vec<&Rule> = rules.iter().chain(backward).collect();
    let n_rules = all.len();
    let deps: Vec<Vec<Dep<'_>>> = all
        .iter()
        .map(|r| {
            let mut d = Vec::new();
            premise_deps(&r.premise, false, &mut d);
            d
        })
        .collect();
    let prods: Vec<Vec<Pred<'_>>> = all.iter().map(|r| produces(r)).collect();

    // Fast exit: every negated predicate is a base predicate (no rule derives it).
    let produced: FxHashSet<Pred<'_>> = prods.iter().flatten().copied().collect();
    let negates_derived = deps
        .iter()
        .flatten()
        .any(|&(p, neg)| neg && produced.iter().any(|&q| overlaps(p, q)));
    if !negates_derived {
        return Ok(Strata::single(None));
    }

    // Bipartite dependency graph: rule nodes `0..n_rules`, then one node per predicate,
    // UNKNOWN included. producer rule → predicate (weight 0) → consumer rule (weight 1
    // when negative). An UNKNOWN producer feeds every predicate node; an UNKNOWN consumer
    // reads every predicate node.
    let mut pred_ix: FxHashMap<Pred<'_>, usize> = FxHashMap::default();
    pred_ix.insert(Pred::Unknown, n_rules);
    // The any-class `rdf:type` node, which every class node feeds.
    pred_ix.insert(Pred::Iri(RDF_TYPE), n_rules + 1);
    for p in prods
        .iter()
        .flatten()
        .chain(deps.iter().flatten().map(|(p, _)| p))
    {
        let next = n_rules + pred_ix.len();
        pred_ix.entry(*p).or_insert(next);
    }
    let n = n_rules + pred_ix.len();
    let mut edges: FxHashMap<usize, FxHashSet<usize>> = FxHashMap::default();
    let mut neg_edges: FxHashSet<(usize, usize)> = FxHashSet::default();
    let pred_nodes: Vec<usize> = pred_ix.values().copied().collect();
    let class_nodes: Vec<usize> = pred_ix
        .iter()
        .filter(|(p, _)| matches!(p, Pred::Class(_)))
        .map(|(_, &ix)| ix)
        .collect();
    // A producer feeds the node of what it derives, and every node that reads it: a class
    // also feeds the any-class `rdf:type` node; an any-class `rdf:type` also feeds every
    // class node. A consumer reads only its own node (UNKNOWN: every node).
    let any_type = pred_ix[&Pred::Iri(RDF_TYPE)];
    for (r, ps) in prods.iter().enumerate() {
        for p in ps {
            let e = edges.entry(r).or_default();
            match p {
                Pred::Unknown => e.extend(pred_nodes.iter().copied()),
                Pred::Iri(i) => {
                    e.insert(pred_ix[p]);
                    if *i == RDF_TYPE {
                        e.extend(class_nodes.iter().copied());
                    }
                }
                Pred::Class(_) => {
                    e.insert(pred_ix[p]);
                    e.insert(any_type);
                }
            }
        }
    }
    for (r, ds) in deps.iter().enumerate() {
        for &(p, neg) in ds {
            let sources: &[usize] = match p {
                Pred::Unknown => &pred_nodes,
                Pred::Iri(_) | Pred::Class(_) => std::slice::from_ref(&pred_ix[&p]),
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
    // negation. Every node reachable from such a component is TAINTED: no stratum orders
    // it soundly.
    let mut bad: Vec<(usize, usize)> = neg_edges
        .iter()
        .copied()
        .filter(|&(s, r)| scc_of[s] == scc_of[r])
        .collect();
    bad.sort_unstable();
    let mut tainted = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    for &(_, r) in &bad {
        for &v in &sccs[scc_of[r]] {
            if !tainted[v] {
                tainted[v] = true;
                stack.push(v);
            }
        }
    }
    while let Some(u) = stack.pop() {
        if let Some(succ) = edges.get(&u) {
            for &v in succ {
                if !tainted[v] {
                    tainted[v] = true;
                    stack.push(v);
                }
            }
        }
    }
    let warning = bad.first().map(|&(s, r)| {
        let pred = pred_ix
            .iter()
            .find(|&(_, &ix)| ix == s)
            .map(|(p, _)| match p {
                Pred::Iri(i) => format!("<{i}>"),
                Pred::Class(c) => format!("<{RDF_TYPE}> <{c}>"),
                Pred::Unknown => "an unknown predicate".to_string(),
            })
            .unwrap_or_default();
        let via_unknown = (0..n_rules).any(|u| {
            scc_of[u] == scc_of[r]
                && (prods[u].contains(&Pred::Unknown)
                    || deps[u].iter().any(|(p, _)| *p == Pred::Unknown))
        });
        let via = if via_unknown {
            " (the cycle runs through a dependency the analysis cannot pin to one predicate: \
             a variable predicate, a variable clause, or a nested closure, which counts as \
             every predicate)"
        } else {
            ""
        };
        let rule = if r < nf {
            format!("forward rule {r}")
        } else {
            format!("backward rule {}", r - nf)
        };
        let affected = (0..nf).filter(|&i| tainted[i]).count();
        format!(
            "n3 stratification: {rule} negates or aggregates (store-scoped log:notIncludes / \
             log:collectAllIn / log:forAllIn) over {pred}, which depends on that rule's own \
             conclusions{via}: a cycle through negation that no stratification can order; \
             {affected} forward rule(s) are on the cycle or depend on it."
        )
    });
    if let (Some(w), NegationCycles::Reject) = (&warning, cycles) {
        return Err(format!(
            "{w} The document is rejected: split it into explicit strata \
             (reason_n3_stratified), or opt in to NegationCycles::FailClosed or SinglePass \
             (reason_n3_terms_with_cycles)."
        ));
    }

    // Longest path (counting negative edges) over the untainted condensation; `n3_sccs`
    // returns the components in topological (dependencies-first) order.
    let mut level = vec![0usize; n];
    for scc in &sccs {
        if tainted[scc[0]] {
            continue;
        }
        let lvl = scc.iter().map(|&v| level[v]).max().unwrap_or(0);
        for &u in scc {
            level[u] = lvl;
        }
        for &u in scc {
            let Some(succ) = edges.get(&u) else { continue };
            for &v in succ {
                if scc_of[v] != scc_of[u] && !tainted[v] {
                    let w = usize::from(neg_edges.contains(&(u, v)));
                    level[v] = level[v].max(lvl + w);
                }
            }
        }
    }
    // Compact the untainted forward rules' levels to `0..k`; tainted forward rules go to
    // one extra final stratum (SinglePass) or are dropped (FailClosed).
    let mut distinct: Vec<usize> = (0..nf).filter(|&i| !tainted[i]).map(|i| level[i]).collect();
    distinct.sort_unstable();
    distinct.dedup();
    let any_tainted = (0..nf).any(|i| tainted[i]);
    if distinct.len() <= 1 && !any_tainted {
        return Ok(Strata::single(warning));
    }
    let rank: FxHashMap<usize, usize> = distinct.iter().enumerate().map(|(i, &l)| (l, i)).collect();
    let k = distinct.len();
    let rule_stratum: Vec<usize> = (0..nf)
        .map(|i| match (tainted[i], cycles) {
            (false, _) => rank[&level[i]],
            (true, NegationCycles::SinglePass) => k,
            (true, _) => DROPPED,
        })
        .collect();
    let n_strata = k + usize::from(any_tainted && cycles == NegationCycles::SinglePass);
    Ok(Strata {
        rule_stratum: Some(rule_stratum),
        n_strata: n_strata.max(1),
        warning,
    })
}
