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
//! and assigns each forward rule the least stratum that puts it after every rule it
//! negates over. The fixpoint drivers ([`super::run_closure`] and the compiled engine's
//! `BoundRuleSet::eval`) then run their unchanged semi-naive loop once per stratum.
//!
//! What counts as a dependency follows the evaluator:
//!
//! * a plain premise atom reads its predicate (a variable predicate reads every predicate);
//! * a store-scoped `log:includes` / `log:supports` / `log:notIncludes` reads the
//!   predicates of its `{ … }` object (the evaluator only reads a literal formula object);
//!   inside `notIncludes` they are NEGATIVE;
//! * a store-scoped `log:collectAllIn` / `log:forAllIn` reads the predicates of its clause
//!   formulas NEGATIVELY. A clause given through a variable is resolved to the formulas
//!   that variable can take (the values its binding premise atom reads from the stored
//!   facts and from rule conclusions); an unresolvable clause reads every predicate;
//! * the list builtins (`list:member`, `list:in`, `list:iterate`) and every functional
//!   builtin may walk a stored `rdf:first`/`rdf:rest` list, so they read `rdf:first` and
//!   `rdf:rest` with the surrounding polarity;
//! * a `{ … }` formula-literal scope is local and reads nothing from the store.
//!
//! A conclusion with a variable predicate produces the predicates that variable can be
//! bound to, resolved the same way (an abstract least fixpoint over facts and
//! conclusions); when that is not resolvable it produces every predicate.
//!
//! Programs that negate nothing derived are ONE stratum, and the drivers take exactly
//! their pre-stratification path. A rule on a cycle through negation (and every rule that
//! depends on it) cannot be stratified; [`NegationCycles`] says what happens to it.

use super::model::{Rule, Term};
use super::{collect_op, functional_builtin, list_generator, scope_op, CollectOp, ScopeOp};
use rustc_hash::{FxHashMap, FxHashSet};

/// The builtin vocabularies (`log:`, `math:`, `string:`, `list:`, `time:` …): evaluated,
/// never matched against stored facts.
const SWAP_NS: &str = "http://www.w3.org/2000/10/swap/";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
/// Clause-resolution nesting bound (a data formula whose clause refers back to itself).
const MAX_DEPTH: usize = 8;

/// What the N3 engines do with rules that negate through a dependency cycle: store-scoped
/// `log:notIncludes` / `log:collectAllIn` / `log:forAllIn` over a predicate that depends on
/// the negating rule's own conclusions, which no stratification can order soundly.
///
/// Only the rules on such a cycle, and the rules that depend on them, are affected; every
/// other rule of the document is stratified normally.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NegationCycles {
    /// Refuse the document with an error naming the cycle. The default of every N3 entry
    /// point that can return an error.
    #[default]
    Reject,
    /// Do not evaluate the affected rules, so nothing that depends on an unorderable
    /// negation is derived, and report a diagnostic. Used where no error can be returned
    /// (compiled evaluation over input facts that create a cycle, incremental mutations,
    /// nested `log:conclusion` documents).
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
    /// Predicates whose stored values (in the facts passed in) the analysis read to
    /// resolve a variable predicate or clause. A caller that later adds facts on one of
    /// these must stratify again (the compiled engine, whose input facts arrive after
    /// compilation).
    #[cfg_attr(not(feature = "compiled-rules"), allow(dead_code))]
    pub(crate) consulted: Vec<String>,
}

impl Strata {
    fn single() -> Self {
        Strata {
            rule_stratum: None,
            n_strata: 1,
            warning: None,
            consulted: Vec::new(),
        }
    }
}

/// A predicate a rule consumes or produces: a ground IRI, or any predicate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Pred<'a> {
    Iri(&'a str),
    Any,
}

fn is_builtin(p: &str) -> bool {
    p.starts_with(SWAP_NS)
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

/// A predicate dependency: `(predicate, negative)`.
type Dep<'a> = (Pred<'a>, bool);

struct Analysis<'a> {
    all: Vec<&'a Rule>,
    facts: &'a [[Term; 3]],
    /// `(predicate, position)` → the values stored facts carry there (lazy cache).
    fact_values: FxHashMap<(&'a str, usize), Vec<&'a Term>>,
    consulted: FxHashSet<&'a str>,
    /// `(rule, conclusion index)` → predicates a variable-predicate conclusion can produce
    /// (`None` = any).
    var_preds: FxHashMap<(usize, usize), Option<FxHashSet<&'a str>>>,
}

impl<'a> Analysis<'a> {
    fn stored_values(&mut self, p: &'a str, pos: usize) -> Vec<&'a Term> {
        self.consulted.insert(p);
        let facts = self.facts;
        self.fact_values
            .entry((p, pos))
            .or_insert_with(|| {
                facts
                    .iter()
                    .filter(|f| matches!(&f[1], Term::Iri(q) if q == p))
                    .map(|f| &f[pos])
                    .collect()
            })
            .clone()
    }

    /// Can conclusion `ci` of rule `r` produce a fact with predicate `p`? `None` when the
    /// conclusion may produce any predicate.
    fn concludes(&self, r: usize, ci: usize, p: &str) -> Option<bool> {
        match &self.all[r].conclusion[ci][1] {
            Term::Iri(q) => Some(q == p),
            _ => match self.var_preds.get(&(r, ci)) {
                Some(Some(set)) => Some(set.contains(p)),
                _ => None,
            },
        }
    }

    /// The values `t` (a variable or blank of rule `r`'s premise) can be bound to: the
    /// terms its first binding premise atom `… P t` / `t P …` reads, from the stored facts
    /// and from every conclusion that can produce `P`. `None` when unresolvable.
    fn resolve(&mut self, r: usize, t: &Term) -> Option<Vec<&'a Term>> {
        let rule: &'a Rule = self.all[r];
        let (p, pos) = rule.premise.iter().find_map(|a| match &a[1] {
            Term::Iri(p) if !is_builtin(p) => {
                if a[2] == *t {
                    Some((p.as_str(), 2))
                } else if a[0] == *t {
                    Some((p.as_str(), 0))
                } else {
                    None
                }
            }
            _ => None,
        })?;
        let mut vals = self.stored_values(p, pos);
        for r2 in 0..self.all.len() {
            let producer: &'a Rule = self.all[r2];
            for (ci, c) in producer.conclusion.iter().enumerate() {
                if !self.concludes(r2, ci, p)? {
                    continue;
                }
                match &c[pos] {
                    Term::Var(_) => return None,
                    v => vals.push(v),
                }
            }
        }
        Some(vals)
    }

    /// Least fixpoint of the predicates each variable-predicate conclusion can produce.
    fn resolve_var_preds(&mut self) {
        let mut keys = Vec::new();
        for (r, rule) in self.all.iter().enumerate() {
            for (ci, c) in rule.conclusion.iter().enumerate() {
                if !matches!(&c[1], Term::Iri(_)) {
                    keys.push((r, ci));
                }
            }
        }
        for &k in &keys {
            self.var_preds.insert(k, Some(FxHashSet::default()));
        }
        loop {
            let mut changed = false;
            for &(r, ci) in &keys {
                let Some(cur) = self.var_preds[&(r, ci)].clone() else {
                    continue;
                };
                let rule: &'a Rule = self.all[r];
                let pred_term = &rule.conclusion[ci][1];
                let next: Option<FxHashSet<&'a str>> = match pred_term {
                    Term::Var(_) => self.resolve(r, pred_term).and_then(|vals| {
                        vals.into_iter()
                            .map(|v| match v {
                                Term::Iri(i) => Some(i.as_str()),
                                _ => None,
                            })
                            .collect()
                    }),
                    _ => None,
                };
                let merged = next.map(|mut n| {
                    n.extend(cur.iter().copied());
                    n
                });
                if merged.as_ref() != Some(&cur) {
                    self.var_preds.insert((r, ci), merged);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Dependencies of the formula a clause term of rule `r` denotes: a `{ … }` literal,
    /// or a variable resolved to the formulas it can take.
    fn clause_deps(
        &mut self,
        r: usize,
        t: &'a Term,
        neg: bool,
        depth: usize,
        out: &mut Vec<Dep<'a>>,
    ) {
        if depth > MAX_DEPTH {
            out.push((Pred::Any, neg));
            return;
        }
        match t {
            Term::Formula(f) => self.premise_deps(r, f, neg, depth + 1, out),
            Term::Var(_) | Term::Blank(_) => match self.resolve(r, t) {
                Some(vals) => {
                    for v in vals {
                        if let Term::Formula(f) = v {
                            self.premise_deps(r, f, neg, depth + 1, out);
                        }
                    }
                }
                None => out.push((Pred::Any, neg)),
            },
            // `{}` (the literal true) has no atoms; any other value fails the premise.
            _ => {}
        }
    }

    /// The dependencies of `atoms`: premise atoms of rule `r`, or a clause formula
    /// evaluated in that rule's binding context.
    fn premise_deps(
        &mut self,
        r: usize,
        atoms: &'a [[Term; 3]],
        neg: bool,
        depth: usize,
        out: &mut Vec<Dep<'a>>,
    ) {
        for atom in atoms {
            if let Some(op) = scope_op(&atom[1]) {
                // The evaluator reads only a literal `{ … }` object.
                if !local_scope(&atom[0]) {
                    if let Term::Formula(inner) = &atom[2] {
                        let inner_neg = neg || matches!(op, ScopeOp::NotIncludes);
                        self.premise_deps(r, inner, inner_neg, depth, out);
                    }
                }
                continue;
            }
            if let Some(op) = collect_op(&atom[1]) {
                if local_scope(&atom[2]) {
                    continue;
                }
                match (&atom[0], op) {
                    (Term::List(ms), CollectOp::CollectAll) if ms.len() == 3 => {
                        self.clause_deps(r, &ms[1], true, depth, out);
                    }
                    (Term::List(ms), CollectOp::ForAll) if ms.len() == 2 => {
                        self.clause_deps(r, &ms[0], true, depth, out);
                        self.clause_deps(r, &ms[1], true, depth, out);
                    }
                    // A variable subject is bound at run time: unknown clauses.
                    (Term::Var(_) | Term::Blank(_), _) => out.push((Pred::Any, true)),
                    // A malformed list or a non-list subject fails the premise.
                    _ => {}
                }
                continue;
            }
            match &atom[1] {
                Term::Iri(p) if is_builtin(p) => {
                    if list_generator(&atom[1]).is_some() || functional_builtin(&atom[1]).is_some()
                    {
                        out.push((Pred::Iri(RDF_FIRST), neg));
                        out.push((Pred::Iri(RDF_REST), neg));
                    }
                }
                Term::Iri(p) => out.push((Pred::Iri(p), neg)),
                _ => out.push((Pred::Any, neg)),
            }
        }
    }

    fn produces(&self, r: usize) -> Vec<Pred<'a>> {
        let mut out = Vec::new();
        let rule: &'a Rule = self.all[r];
        for (ci, c) in rule.conclusion.iter().enumerate() {
            match &c[1] {
                Term::Iri(p) if is_builtin(p) => {}
                Term::Iri(p) => out.push(Pred::Iri(p)),
                _ => match self.var_preds.get(&(r, ci)) {
                    Some(Some(set)) => out.extend(set.iter().map(|p| Pred::Iri(p))),
                    _ => out.push(Pred::Any),
                },
            }
        }
        out
    }
}

/// Stratify a document's forward `rules` (with its `backward` rules as proof-time
/// dependencies) over its stored `facts`. See the module docs for the contract.
///
/// Errors only under [`NegationCycles::Reject`], when some rule negates through a
/// dependency cycle.
pub(crate) fn stratify(
    rules: &[Rule],
    backward: &[Rule],
    facts: &[[Term; 3]],
    cycles: NegationCycles,
) -> Result<Strata, String> {
    // Fast exit: no store-scoped non-monotonic operator anywhere, so one stratum is exact.
    if !rules
        .iter()
        .chain(backward)
        .any(|r| has_store_negation(&r.premise))
    {
        return Ok(Strata::single());
    }
    let nf = rules.len();
    let mut an = Analysis {
        all: rules.iter().chain(backward).collect(),
        facts,
        fact_values: FxHashMap::default(),
        consulted: FxHashSet::default(),
        var_preds: FxHashMap::default(),
    };
    an.resolve_var_preds();
    let n_rules = an.all.len();
    let mut deps: Vec<Vec<Dep<'_>>> = Vec::with_capacity(n_rules);
    for r in 0..n_rules {
        let rule = an.all[r];
        let mut d = Vec::new();
        an.premise_deps(r, &rule.premise, false, 0, &mut d);
        deps.push(d);
    }
    let prods: Vec<Vec<Pred<'_>>> = (0..n_rules).map(|r| an.produces(r)).collect();
    let mut consulted: Vec<String> = an.consulted.iter().map(|p| p.to_string()).collect();
    consulted.sort_unstable();

    // Fast exit: every negated predicate is a base predicate (no rule derives it).
    let produced: FxHashSet<Pred<'_>> = prods.iter().flatten().copied().collect();
    let any_produced = produced.contains(&Pred::Any);
    let negates_derived = deps.iter().flatten().any(|&(p, neg)| {
        neg && match p {
            Pred::Any => !produced.is_empty(),
            Pred::Iri(_) => any_produced || produced.contains(&p),
        }
    });
    if !negates_derived {
        return Ok(Strata {
            consulted,
            ..Strata::single()
        });
    }

    // Bipartite dependency graph: rule nodes `0..n_rules`, then one node per predicate
    // (`Pred::Any` included, so an unresolved producer reaches an unresolved consumer).
    // producer rule → predicate (weight 0) → consumer rule (weight 1 when negative).
    let mut pred_ix: FxHashMap<Pred<'_>, usize> = FxHashMap::default();
    pred_ix.insert(Pred::Any, n_rules);
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
    for (r, ps) in prods.iter().enumerate() {
        for p in ps {
            let e = edges.entry(r).or_default();
            match p {
                Pred::Any => e.extend(pred_nodes.iter().copied()),
                Pred::Iri(_) => {
                    e.insert(pred_ix[p]);
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
                Pred::Any => "an unresolved variable predicate or clause".to_string(),
            })
            .unwrap_or_default();
        let via_any = prods
            .iter()
            .enumerate()
            .any(|(u, ps)| scc_of[u] == scc_of[s] && ps.contains(&Pred::Any));
        let via = if via_any {
            " (through a rule whose conclusion has an unresolvable variable predicate, which \
             may derive any predicate)"
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
        return Ok(Strata {
            warning,
            consulted,
            ..Strata::single()
        });
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
        consulted,
    })
}
