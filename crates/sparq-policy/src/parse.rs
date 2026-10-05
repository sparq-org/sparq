//! Parse an ODRL policy expressed in RDF into the typed [`Policy`] model.
//!
//! The parser runs SPARQL queries (via [`sparq_engine::query`]) over the loaded
//! policy graph — ODRL evaluation *is* a SPARQL/SHACL/N3 workload, so extracting
//! the model with the engine sparq already ships keeps this crate in-family and
//! dependency-light (no bespoke RDF walker).
//!
//! Rule and constraint nodes are matched by **variable** (never by a literal
//! blank-node label), because real ODRL policies overwhelmingly express rules,
//! constraints and duties as *blank nodes*; the whole rule structure is joined
//! in one query per (rule-kind, attribute) so a blank-node constraint is bound
//! through its incident edge, not re-named.
//!
//! Input forms accepted by [`parse_policy_str`]: any RDF serialization
//! `sparq_core::Graph::load_str` accepts (`turtle`, `ntriples`, …). The policy
//! IRI is whichever subject is `a odrl:Policy`/`Set`/`Offer`/`Agreement`, or —
//! if none is typed — whichever subject carries `odrl:permission`/`prohibition`.
//! [OPUS-4.8]

use crate::model::{
    Action, ConflictStrategy, Constraint, ConstraintNode, Duty, LogicalConstraint, LogicalOperator,
    Operator, Policy, Rule, Value, ODRL_NS,
};
use oxrdf::{Literal, Term};
use sparq_core::Graph;
use std::collections::{BTreeMap, BTreeSet};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// Parse an ODRL policy from an RDF string in `format` (e.g. `"turtle"`).
///
/// # Errors
///
/// Returns `Err` if the RDF does not parse, if a query over it fails, or if the
/// policy is REFUSED as ambiguous/degenerate (multiple `odrl:conflict`
/// strategies, a malformed/empty/nested RDF-collection combinator operand, or a
/// malformed RDF-collection `odrl:rightOperand` — see [`parse_policy`]). A
/// well-formed RDF document with no ODRL rules parses to an empty [`Policy`]
/// (which then denies everything — fail-closed).
pub fn parse_policy_str(rdf: &str, format: &str) -> Result<Policy, String> {
    let graph = Graph::load_str(rdf, format)?;
    parse_policy(&graph)
}

/// Parse an ODRL policy from an already-loaded [`Graph`].
///
/// # Errors
///
/// Returns `Err` if a query over the graph fails, or if the policy is REFUSED
/// (fail-closed) as ambiguous/degenerate: multiple `odrl:conflict` strategies
/// (the `policy_conflict` precedent), or a LogicalConstraint combinator with a
/// malformed (broken-tail/cyclic/forked), EMPTY (`( )`), or nested-list
/// collection operand (`fold_list_operands` — degrading those per-operand would
/// widen decisions on one rule kind or the other), or a constraint whose
/// `odrl:rightOperand` is a malformed (broken-tail/dangling/cyclic/forked, or a
/// member-less cell asserting `rdf:rest` with no `rdf:first`) collection
/// (`fold_rights` — honouring its valid prefix, or reading a member-less head as
/// an ordinary unmatchable value, would drop authored members from the set
/// encoding, likewise widening).
pub fn parse_policy(graph: &Graph) -> Result<Policy, String> {
    let iri = policy_iri(graph)?;
    // Bulk-load the graph's RDF collection shapes ONCE (cons cells + the member-less
    // rest-only heads), so a multi-valued `odrl:rightOperand ( <a> <b> )` list can be
    // folded into the set encoding — or refused — by every constraint parse below.
    // [FABLE-5] sq-ueydm.
    let lists = rdf_list_table(graph)?;
    let permissions = rules(graph, "permission", true, &lists)?;
    let prohibitions = rules(graph, "prohibition", false, &lists)?;
    let conflict = policy_conflict(graph)?;
    let party_collections = party_collections(graph)?;
    Ok(Policy {
        iri,
        permissions,
        prohibitions,
        conflict,
        party_collections,
    })
}

/// The IRIs this graph identifies as an `odrl:PartyCollection` — retained on the
/// [`Policy`] so a consumer that FREEZES a rule into an identity-matched head can route
/// collection-valued heads to a membership-aware path. [SONNET-4.6] sq-rf9uv.
///
/// Two independent signals, unioned:
///
/// - `?c a odrl:PartyCollection` — the explicit ODRL 2.2 type declaration.
/// - `?m odrl:partOf ?c` where `?c` is not explicitly `a odrl:AssetCollection` — the
///   object of a membership edge stated *in the policy document*. A document that says
///   `<alice> odrl:partOf <lab>` has identified `<lab>` as a collection just as surely as
///   the type triple, and real ODRL policies routinely state one without the other.
///   `odrl:partOf` is shared with asset-collection membership, so an object carrying no
///   type at all is admitted here: over-inclusion only routes a head to the
///   membership-aware path, which is the conservative direction, whereas missing a real
///   party collection is the fail-OPEN one.
///
/// Blank nodes and literals are skipped: only an IRI can be an ACP head, so only an IRI
/// can be a head the consumer needs to classify. Like [`policy_conflict`] this is
/// deliberately NOT scoped to the policy node — an unrelated subject asserting the type
/// still contributes, again the more fail-closed direction.
fn party_collections(graph: &Graph) -> Result<BTreeSet<String>, String> {
    let res = sparq_engine::query(
        graph,
        &format!(
            "SELECT ?c WHERE {{ {{ ?c a <{ODRL_NS}PartyCollection> }} \
             UNION {{ ?m <{ODRL_NS}partOf> ?c . \
             FILTER NOT EXISTS {{ ?c a <{ODRL_NS}AssetCollection> }} }} }}"
        ),
    )?;
    Ok(res
        .rows
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .filter_map(|t| match t {
            Term::NamedNode(n) => Some(n.into_string()),
            _ => None,
        })
        .collect())
}

/// Extract the policy's declared `odrl:conflict` conflict-resolution strategy, if any.
/// [OPUS-4.8] sq-ihqbl.
///
/// The value is classified via [`ConflictStrategy::from_iri`], so an unrecognised term
/// is preserved as [`ConflictStrategy::Unknown`] (and later *refused*) rather than
/// silently dropped.
///
/// **Fail-closed on ambiguity.** For an authorization guard it is unsound to take only
/// the first-sorted `?c`: a benign strategy that happens to sort first (e.g.
/// `odrl:prohibit`) would mask a co-asserted unimplementable one (`odrl:perm`, an unknown
/// IRI), and the graph would be mis-classified as admissible. So we gather **every**
/// distinct declared strategy and *refuse* (`Err`) when more than one is present — a graph
/// declaring multiple conflicting resolution strategies is ambiguous and cannot be honoured
/// deterministically. (Any multi-value set necessarily contains a non-`Prohibit` strategy,
/// so this only ever refuses a graph that declares a non-default strategy, never a benign
/// deny-overrides one.) We deliberately do **not** tie `?p` to a specific policy node: an
/// unrelated subject asserting `odrl:conflict` then contributes to the refusal set, which is
/// strictly the *more* fail-closed direction. [OPUS-4.8] sq-ihqbl.
fn policy_conflict(graph: &Graph) -> Result<Option<ConflictStrategy>, String> {
    let res = sparq_engine::query(
        graph,
        &format!("SELECT DISTINCT ?c WHERE {{ ?p <{ODRL_NS}conflict> ?c }} ORDER BY ?c"),
    )?;
    let mut strategies: Vec<ConflictStrategy> = res
        .rows
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .map(|t| ConflictStrategy::from_iri(&term_str(&t)))
        .collect();
    strategies.dedup();
    match strategies.len() {
        0 => Ok(None),
        1 => Ok(Some(strategies.remove(0))),
        _ => Err(format!(
            "policy declares multiple conflicting `odrl:conflict` strategies ({strategies:?}); \
             an ambiguous conflict-resolution set is refused (fail-closed) rather than resolved \
             to whichever term sorts first"
        )),
    }
}

/// A stable string key identifying a rule/constraint/duty node, used to group
/// the flat rows of a join query back into structured rules. IRIs and blank
/// nodes both get a distinct key.
fn node_key(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        Term::Literal(l) => format!("\"{}\"", l.value()),
        #[allow(unreachable_patterns)]
        _ => String::new(),
    }
}

fn term_str(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => n.as_str().to_owned(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        Term::Literal(l) => l.value().to_owned(),
        #[allow(unreachable_patterns)]
        _ => String::new(),
    }
}

fn policy_iri(graph: &Graph) -> Result<Option<String>, String> {
    let typed = sparq_engine::query(
        graph,
        &format!(
            "SELECT ?p WHERE {{ ?p a ?t . \
             VALUES ?t {{ <{ODRL_NS}Policy> <{ODRL_NS}Set> <{ODRL_NS}Offer> <{ODRL_NS}Agreement> }} }}"
        ),
    )?;
    if let Some(Term::NamedNode(n)) = typed
        .rows
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next().flatten())
    {
        return Ok(Some(n.into_string()));
    }
    let withrule = sparq_engine::query(
        graph,
        &format!(
            "SELECT ?p WHERE {{ ?p ?r ?x . \
             VALUES ?r {{ <{ODRL_NS}permission> <{ODRL_NS}prohibition> }} }}"
        ),
    )?;
    Ok(
        match withrule
            .rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next().flatten())
        {
            Some(Term::NamedNode(n)) => Some(n.into_string()),
            _ => None,
        },
    )
}

/// Index `?rule -> attribute` rows from a single query into per-rule lists.
fn group_by_first(graph: &Graph, sparql: &str) -> Result<BTreeMap<String, Vec<Term>>, String> {
    let res = sparq_engine::query(graph, sparql)?;
    let mut out: BTreeMap<String, Vec<Term>> = BTreeMap::new();
    for row in res.rows {
        let mut it = row.into_iter();
        let (Some(Some(key_t)), Some(Some(val_t))) = (it.next(), it.next()) else {
            continue;
        };
        out.entry(node_key(&key_t)).or_default().push(val_t);
    }
    Ok(out)
}

/// All rules of a kind (`"permission"`/`"prohibition"`). `with_duties` controls
/// whether duty obligations are parsed (permissions only in the base case).
/// `lists` is the graph's pre-loaded RDF-collection table (see `rdf_list_table`),
/// consumed by the constraint parses to fold list-valued right operands.
fn rules(
    graph: &Graph,
    kind: &str,
    with_duties: bool,
    lists: &ListTable,
) -> Result<Vec<Rule>, String> {
    // Enumerate the rule nodes first (keyed), then attach attributes by join.
    // (BIND a copy so the projection has two distinct variables — the engine
    // rejects `SELECT ?rule ?rule`.)
    let nodes = group_by_first(
        graph,
        &format!("SELECT ?rule ?rule2 WHERE {{ ?policy <{ODRL_NS}{kind}> ?rule . BIND(?rule AS ?rule2) }}"),
    )?;

    let actions = group_by_first(
        graph,
        &format!("SELECT ?rule ?a WHERE {{ ?policy <{ODRL_NS}{kind}> ?rule . ?rule <{ODRL_NS}action> ?a }}"),
    )?;
    let targets = group_by_first(
        graph,
        &format!("SELECT ?rule ?t WHERE {{ ?policy <{ODRL_NS}{kind}> ?rule . ?rule <{ODRL_NS}target> ?t }}"),
    )?;
    let assignees = group_by_first(
        graph,
        &format!("SELECT ?rule ?p WHERE {{ ?policy <{ODRL_NS}{kind}> ?rule . ?rule <{ODRL_NS}assignee> ?p }}"),
    )?;
    let assigners = group_by_first(
        graph,
        &format!("SELECT ?rule ?p WHERE {{ ?policy <{ODRL_NS}{kind}> ?rule . ?rule <{ODRL_NS}assigner> ?p }}"),
    )?;

    let constraints = constraints_for(graph, kind, "constraint", lists)?;
    let logical_constraints = logical_constraints_for(graph, kind, lists)?;
    let duties = if with_duties {
        duties_for(graph, kind)?
    } else {
        BTreeMap::new()
    };

    let mut out = Vec::new();
    for (rule_key, rule_terms) in nodes {
        let node = rule_terms.first().cloned().expect("group has >=1 row");
        let action = first_str(&actions, &rule_key)
            .map(Action)
            .unwrap_or_else(Action::use_);
        out.push(Rule {
            id: term_str(&node),
            action,
            target: first_str(&targets, &rule_key),
            assignee: first_str(&assignees, &rule_key),
            assigner: first_str(&assigners, &rule_key),
            constraints: constraints.get(&rule_key).cloned().unwrap_or_default(),
            logical_constraints: logical_constraints
                .get(&rule_key)
                .cloned()
                .unwrap_or_default(),
            duties: duties.get(&rule_key).cloned().unwrap_or_default(),
        });
    }
    Ok(out)
}

fn first_str(m: &BTreeMap<String, Vec<Term>>, key: &str) -> Option<String> {
    m.get(key).and_then(|v| v.first()).map(term_str)
}

/// One RDF collection cons cell (`rdf:first`/`rdf:rest`), keyed in the list table
/// by its node key. `rest` is the node key of the tail (the next cell, or
/// `rdf:nil`); `None` when the cell is malformed (an `rdf:first` with no
/// `rdf:rest`), which the walk reads as end-of-list. [FABLE-5] sq-ueydm.
struct ListCell {
    first: Term,
    rest: Option<String>,
    /// The node asserts SEVERAL distinct `rdf:first`/`rdf:rest` pairs (a forked
    /// list). The kept pair stays deterministic, but a well-formedness check must
    /// treat the cell as malformed: honouring one deterministic fork of an
    /// ambiguous collection could silently drop authored members. [FABLE-5] sq-dkuff.
    ambiguous: bool,
}

/// The graph's pre-loaded RDF-collection shape table (see [`rdf_list_table`]):
/// every well-keyed cons cell, PLUS the member-less HEAD-position nodes that are
/// collection-*shaped* but carry no `rdf:first`. Both are needed by every
/// consumer, because a node's absence from `cells` alone does not mean "not a
/// collection". [FABLE-5] sq-srjuc.
struct ListTable {
    /// Cons cells keyed by node key (`?n rdf:first ?f`).
    cells: BTreeMap<String, ListCell>,
    /// Nodes asserting `rdf:rest` but NO `rdf:first`. Such a node is invisible to
    /// `cells` (which is keyed on `rdf:first`), so without this set it would read
    /// as an ordinary IRI/blank-node value or operand and silently bypass
    /// collection validation. (A MID-chain rest-only cell is already caught by
    /// [`well_formed_list`]'s dangling-tail check; this set is what catches one in
    /// HEAD position.) [FABLE-5] sq-dkuff.
    rest_only: BTreeMap<String, ()>,
}

impl ListTable {
    /// Is `key` collection-SHAPED — either a cons cell or a member-less rest-only
    /// node? Both shapes must be validated as a collection rather than read as an
    /// ordinary value/operand. [FABLE-5] sq-srjuc.
    fn is_collection_shaped(&self, key: &str) -> bool {
        self.cells.contains_key(key) || self.rest_only.contains_key(key)
    }
}

/// Bulk-load the graph's RDF collection shapes ONCE: every cons cell (`?n rdf:first
/// ?f`, optional `?n rdf:rest ?r`) keyed by node key, plus the `rest_only` set of
/// nodes asserting `rdf:rest` with no `rdf:first`. A `rightOperand` or combinator
/// operand bound to a list *head* can then be folded into its member terms — or
/// refused — without per-constraint queries (the same bulk-query + in-memory-assembly
/// pattern as [`logical_constraints_for`]). A malformed cell asserting several
/// `rdf:first`/`rdf:rest` values keeps its first-bound pair (degenerate, but the
/// load stays deterministic and terminating) and is flagged `ambiguous`, so every
/// consumer goes through [`well_formed_list`] and refuses it. [FABLE-5] sq-ueydm.
fn rdf_list_table(graph: &Graph) -> Result<ListTable, String> {
    let q = format!(
        "SELECT ?n ?f ?r WHERE {{ \
           ?n <{RDF_NS}first> ?f . \
           OPTIONAL {{ ?n <{RDF_NS}rest> ?r }} \
         }}"
    );
    let res = sparq_engine::query(graph, &q)?;
    let mut cells: BTreeMap<String, ListCell> = BTreeMap::new();
    for row in res.rows {
        let mut it = row.into_iter();
        let n = it.next().flatten();
        let f = it.next().flatten();
        let r = it.next().flatten();
        let (Some(n), Some(f)) = (n, f) else { continue };
        cells
            .entry(node_key(&n))
            // A SECOND row for the same cell node = a genuinely different
            // `rdf:first`/`rdf:rest` binding (a triple matches at most once) —
            // mark the fork so well-formedness checks can refuse it. [FABLE-5]
            // sq-dkuff.
            .and_modify(|c| c.ambiguous = true)
            .or_insert(ListCell {
                first: f,
                rest: r.as_ref().map(node_key),
                ambiguous: false,
            });
    }
    let rest_res = sparq_engine::query(
        graph,
        &format!("SELECT ?n ?r WHERE {{ ?n <{RDF_NS}rest> ?r }}"),
    )?;
    let mut rest_only: BTreeMap<String, ()> = BTreeMap::new();
    for row in rest_res.rows {
        let Some(Some(n)) = row.into_iter().next() else {
            continue;
        };
        let key = node_key(&n);
        if !cells.contains_key(&key) {
            rest_only.insert(key, ());
        }
    }
    Ok(ListTable { cells, rest_only })
}

/// Walk an RDF list from `head_key` and return its member terms ONLY when the
/// collection is WELL-FORMED: every cell has an unambiguous `rdf:first`/`rdf:rest`
/// pair, no cell repeats (no cycle), and the walk terminates at `rdf:nil`
/// explicitly. Returns `None` for a broken tail (missing `rdf:rest`), a dangling
/// tail (an `rdf:rest` pointing at a non-cell that is not `rdf:nil`), a cycle, a
/// forked (ambiguous) cell, or a member-less rest-only node (`rdf:rest` with no
/// `rdf:first` — absent from `cells`, so it never walks as a collection).
/// [FABLE-5] sq-dkuff.
///
/// A *prefix* of a malformed list must never be honoured where the members carry
/// authorization semantics: dropping an authored member from an `odrl:and` operand
/// set makes the compound EASIER to satisfy than authored, and dropping one from an
/// `odrl:rightOperand` set encoding weakens the set relation (`isNoneOf` excludes
/// fewer values) — both the widening direction. Shared by [`fold_list_operands`]
/// (sq-dkuff) and [`fold_rights`] (sq-srjuc).
fn well_formed_list(head_key: &str, lists: &ListTable) -> Option<Vec<Term>> {
    let nil = format!("<{RDF_NS}nil>");
    let mut out = Vec::new();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    let mut cur = head_key.to_owned();
    loop {
        if cur == nil {
            return Some(out);
        }
        // Dangling tail / member-less rest-only cell / not a list at all.
        let cell = lists.cells.get(&cur)?;
        if cell.ambiguous || seen.insert(cur.clone(), ()).is_some() {
            return None; // forked cell / cycle
        }
        out.push(cell.first.clone());
        // Broken tail (an `rdf:first` with no `rdf:rest`) → not well-formed.
        cur.clone_from(cell.rest.as_ref()?);
    }
}

/// Is `op` one of the ODRL set-relation operators (`isPartOf`/`isAnyOf`/`isNoneOf`)
/// that consume the `|`/space/comma set encoding — the only operators a
/// MULTI-valued `rightOperand` can be faithfully folded for? [FABLE-5] sq-ueydm.
fn is_set_operator(op: Option<&Term>) -> bool {
    matches!(
        op.and_then(|t| Operator::from_iri(&term_str(t))),
        Some(Operator::IsPartOf | Operator::IsAnyOf | Operator::IsNoneOf)
    )
}

/// Fold a constraint's collected `rightOperand` objects — expanding any RDF list
/// (`rdf:first`/`rdf:rest` chain) into its members — into a single [`Value`].
/// [FABLE-5] sq-ueydm.
///
/// * **No object** → `None` (the missing-right case: [`build_constraint`] turns it
///   into the unsatisfiable guard — fail-closed, as before).
/// * **Exactly one member** (a single object, or a one-element list) → the TYPED
///   [`value_of`] — the single-value path is byte-for-byte the pre-fold parse, so
///   numeric/dateTime right operands keep magnitude/instant comparison.
/// * **Several members** (several objects — `odrl:rightOperand <a>, <b>` — or a
///   multi-element list) → the `|`-joined set-encoding string the set-relation
///   operators (`isPartOf`/`isAnyOf`/`isNoneOf`) consume, **iff** the operator IS
///   one of those set operators and every member is cleanly encodable (non-empty,
///   free of the `|`/whitespace/`,` separator characters). A multi-value under a
///   non-set operator (`eq`, `lt`, …) is ambiguous, and a separator-carrying
///   member would corrupt the encoding (splitting into unintended members — a
///   fail-OPEN hazard) — both degrade to `None` → the unsatisfiable guard
///   (fail-closed, consistent with every other malformed-constraint path).
///
/// Members are deduplicated by node key; a nested-list member is NOT recursively
/// flattened (it stays a blank-node string — an unmatchable member, fail-closed),
/// and an empty list (`rdf:nil` directly as the object) has no cons cell, so it
/// falls through as the plain nil IRI — unmatchable by ordinary values, as before.
///
/// **A MALFORMED collection REFUSES the whole parse (`Err`)** — the
/// [`fold_list_operands`] / [`policy_conflict`] precedent. A collection-SHAPED
/// operand ([`ListTable::is_collection_shaped`] — a cons cell, or a member-less
/// HEAD-position node asserting `rdf:rest` with no `rdf:first`) is expanded only
/// when it is WELL-FORMED ([`well_formed_list`]); a broken/dangling tail, a
/// cycle, or a forked cell would otherwise contribute the valid PREFIX of the
/// collection, and dropping authored members from a set encoding WIDENS decisions in
/// both directions: `isNoneOf` on a *permission* excludes fewer purposes than
/// authored (a dropped member now grants), and any set-op constraint gating a
/// *prohibition* narrows the carve-out (deny-overrides bypassed). Per-constraint
/// degradation to the unsatisfiable guard is not a sound fallback either — an
/// unsatisfiable constraint on a prohibition DISABLES it, the same widening
/// direction — so the shape is refused outright, fail-closed on both rule kinds.
/// That last hazard is exactly why a member-less rest-only head must be refused
/// rather than read as an ordinary value: as a value it is an unmatchable blank
/// node, which leaves a prohibition's constraint unsatisfied and lets a sibling
/// permission grant. [FABLE-5] sq-srjuc.
fn fold_rights(
    op: Option<&Term>,
    rights: &[Term],
    lists: &ListTable,
) -> Result<Option<Value>, String> {
    let mut members: Vec<Term> = Vec::new();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for r in rights {
        let key = node_key(r);
        // Collection-SHAPED (a cons cell OR a member-less rest-only node) → it must
        // validate as a collection; a rest-only head is invisible to the cell table,
        // so testing only `cells` would read it as an ordinary value. [FABLE-5] sq-srjuc.
        let expanded = if lists.is_collection_shaped(&key) {
            well_formed_list(&key, lists).ok_or_else(|| {
                format!(
                    "a constraint has a MALFORMED collection rightOperand ({key}: \
                     broken/dangling tail, cycle, forked cell, or a member-less cell \
                     asserting `rdf:rest` with no `rdf:first`); honouring the valid \
                     PREFIX would drop authored members and widen decisions (an \
                     `isNoneOf` permission excludes fewer values than authored; a \
                     set-op prohibition's carve-out narrows), so the policy is refused \
                     (fail-closed)"
                )
            })?
        } else {
            vec![r.clone()]
        };
        for m in expanded {
            if seen.insert(node_key(&m), ()).is_none() {
                members.push(m);
            }
        }
    }
    Ok(match members.len() {
        0 => None, // no rightOperand object at all → missing-right (unsatisfiable)
        1 => Some(value_of(&members[0])), // single value stays TYPED (pre-fold path)
        _ => {
            if !is_set_operator(op) {
                return Ok(None); // ambiguous multi-value under a non-set operator
            }
            let strs: Vec<String> = members.iter().map(term_str).collect();
            if strs.iter().any(|s| {
                s.is_empty() || s.contains(['|', ',']) || s.chars().any(char::is_whitespace)
            }) {
                return Ok(None); // un-encodable member would corrupt the set encoding
            }
            Some(Value::Str(strs.join("|")))
        }
    })
}

/// Accumulator grouping one constraint node's result rows: a multi-valued
/// `odrl:rightOperand` (several objects, or several rows from the OPTIONAL
/// combinations) yields SEVERAL rows for ONE constraint node, which are folded
/// into one constraint — not first-binding-wins. [FABLE-5] sq-ueydm.
#[derive(Default)]
struct RawConstraint {
    left: Option<Term>,
    op: Option<Term>,
    rights: Vec<Term>,
    rights_seen: BTreeMap<String, ()>,
    is_logical: bool,
}

impl RawConstraint {
    /// Merge one result row into the accumulator: first-bound `left`/`op` win
    /// (single-valued per the ODRL model), `right` objects accumulate
    /// (deduplicated by node key, in row order), `is_logical` is sticky.
    fn absorb(
        &mut self,
        left: Option<Term>,
        op: Option<Term>,
        right: Option<Term>,
        is_logical: bool,
    ) {
        if self.left.is_none() {
            self.left = left;
        }
        if self.op.is_none() {
            self.op = op;
        }
        if let Some(r) = right {
            if self.rights_seen.insert(node_key(&r), ()).is_none() {
                self.rights.push(r);
            }
        }
        self.is_logical |= is_logical;
    }

    /// Fold the accumulated right operands (expanding RDF lists) and build the
    /// [`Constraint`] — anything malformed degrades to the unsatisfiable guard,
    /// except a malformed COLLECTION right operand, which refuses the whole parse
    /// (`Err`; see [`fold_rights`]). [FABLE-5] sq-srjuc.
    fn build(self, lists: &ListTable) -> Result<Constraint, String> {
        let right = fold_rights(self.op.as_ref(), &self.rights, lists)?;
        Ok(build_constraint(self.left, self.op, right))
    }
}

/// Parse the *atomic* constraints attached (via `odrl:<pred>`) to each rule of
/// `kind`, keyed by rule node. Constraint *nodes* are bound by variable, so
/// blank-node constraints are handled correctly.
///
/// A `?c` that is a compound `odrl:LogicalConstraint` (it carries an
/// `odrl:and`/`odrl:or`/`odrl:xone` refining set rather than a direct
/// `leftOperand`/`operator`/`rightOperand`) is **skipped here** — it is parsed by
/// [`logical_constraints_for`] into the rule's `logical_constraints` instead, so it
/// is *not* mis-read as a structurally-incomplete atomic constraint (which would
/// wrongly fail the whole rule closed). [OPUS-4.8] sq-a0zef.
///
/// A multi-valued `odrl:rightOperand` — several objects (`odrl:rightOperand <a>,
/// <b>`) or an RDF list (`odrl:rightOperand ( <a> <b> )`) — is folded into the
/// `|`-separated set encoding via [`fold_rights`] instead of taking the first
/// binding. [FABLE-5] sq-ueydm.
fn constraints_for(
    graph: &Graph,
    kind: &str,
    pred: &str,
    lists: &ListTable,
) -> Result<BTreeMap<String, Vec<Constraint>>, String> {
    // One row per (rule, constraint-node, left, operator, right, is-logical).
    // LEFT/OP/RIGHT are OPTIONAL so a structurally incomplete constraint still
    // surfaces (and is turned into an unsatisfiable guard — fail-closed). `?and`
    // binds iff the node is a LogicalConstraint (any of the three combinators), so
    // such a node is routed to logical_constraints_for instead of here.
    let q = format!(
        "SELECT ?rule ?c ?left ?op ?right ?and WHERE {{ \
           ?policy <{ODRL_NS}{kind}> ?rule . \
           ?rule <{ODRL_NS}{pred}> ?c . \
           OPTIONAL {{ ?c <{ODRL_NS}leftOperand> ?left }} \
           OPTIONAL {{ ?c <{ODRL_NS}operator> ?op }} \
           OPTIONAL {{ ?c <{ODRL_NS}rightOperand> ?right }} \
           OPTIONAL {{ ?c <{ODRL_NS}rightOperandReference> ?right }} \
           OPTIONAL {{ ?c ?logop ?and . \
             VALUES ?logop {{ <{ODRL_NS}and> <{ODRL_NS}or> <{ODRL_NS}xone> }} }} \
         }}"
    );
    let res = sparq_engine::query(graph, &q)?;
    // Group rows by (rule, constraint-node) — a multi-valued rightOperand yields
    // several rows for ONE node, folded via `RawConstraint` (not first-binding-wins
    // — sq-ueydm); `order` preserves first-seen row order for the output.
    let mut acc: BTreeMap<String, RawConstraint> = BTreeMap::new();
    let mut order: Vec<(String, String)> = Vec::new();
    for row in res.rows {
        let mut it = row.into_iter();
        let rule_t = it.next().flatten();
        let c_t = it.next().flatten();
        let left = it.next().flatten();
        let op = it.next().flatten();
        let right = it.next().flatten();
        let is_logical = it.next().flatten().is_some();
        let (Some(rule_t), Some(c_t)) = (rule_t, c_t) else {
            continue;
        };
        let rkey = node_key(&rule_t);
        let ckey = format!("{rkey}|{}", node_key(&c_t));
        if !acc.contains_key(&ckey) {
            order.push((rkey, ckey.clone()));
        }
        acc.entry(ckey)
            .or_default()
            .absorb(left, op, right, is_logical);
    }
    let mut out: BTreeMap<String, Vec<Constraint>> = BTreeMap::new();
    for (rkey, ckey) in order {
        let raw = acc.remove(&ckey).expect("accumulated above");
        // A compound LogicalConstraint is parsed by logical_constraints_for, not as
        // a malformed atomic constraint. [OPUS-4.8] sq-a0zef.
        if raw.is_logical {
            continue;
        }
        out.entry(rkey).or_default().push(raw.build(lists)?);
    }
    Ok(out)
}

/// Parse the compound `odrl:LogicalConstraint` refinements attached (via
/// `odrl:constraint`) to each rule of `kind`, keyed by rule node. [OPUS-4.8] sq-a0zef.
///
/// A parsed `odrl:LogicalConstraint` node's combinator + operand node-keys, before
/// recursive assembly into a [`LogicalConstraint`]. [OPUS-4.8] sq-a0zef.
struct LcDef {
    operator: LogicalOperator,
    /// Operand node-keys in graph order (de-duplicated).
    operands: Vec<String>,
    /// De-dup set for the operand node-keys.
    seen: BTreeMap<String, ()>,
}

/// A `LogicalConstraint` node carries one combinator property
/// (`odrl:and`/`odrl:or`/`odrl:xone`) whose objects are the operand nodes (`odrl:and
/// <c1>, <c2>` — several objects of the one property). Each operand is parsed into a
/// [`ConstraintNode`]: a nested `LogicalConstraint` recurses, anything else becomes an
/// atomic [`Constraint`] via [`build_constraint`] (a structurally-incomplete atomic
/// operand becomes the unsatisfiable guard — fail-closed, never a silent pass).
///
/// Two bulk queries build node tables for the *whole* policy graph (all combinator
/// edges; all atomic constraint fields); the per-rule compound constraints are then
/// assembled recursively in-memory, with cycle protection so a malformed self-/mutually
/// referential `LogicalConstraint` cannot loop (it short-circuits to the unsatisfiable
/// guard — fail-closed).
fn logical_constraints_for(
    graph: &Graph,
    kind: &str,
    lists: &ListTable,
) -> Result<BTreeMap<String, Vec<LogicalConstraint>>, String> {
    // (1) Every combinator edge in the graph: (lc-node, combinator, operand-node), in
    // graph order. A node appearing as a subject here is a LogicalConstraint.
    let edges_q = format!(
        "SELECT ?lc ?logop ?operand WHERE {{ \
           ?lc ?logop ?operand . \
           VALUES ?logop {{ <{ODRL_NS}and> <{ODRL_NS}or> <{ODRL_NS}xone> }} \
         }}"
    );
    let edges_res = sparq_engine::query(graph, &edges_q)?;
    let mut lc_defs: BTreeMap<String, LcDef> = BTreeMap::new();
    for row in edges_res.rows {
        let mut it = row.into_iter();
        let (Some(Some(lc_t)), Some(Some(logop_t)), Some(Some(operand_t))) =
            (it.next(), it.next(), it.next())
        else {
            continue;
        };
        let Some(operator) = LogicalOperator::from_iri(&term_str(&logop_t)) else {
            continue;
        };
        let lckey = node_key(&lc_t);
        let okey = node_key(&operand_t);
        let def = lc_defs.entry(lckey).or_insert_with(|| LcDef {
            operator,
            operands: Vec::new(),
            seen: BTreeMap::new(),
        });
        if def.seen.insert(okey.clone(), ()).is_none() {
            def.operands.push(okey);
        }
    }

    // (2) Every atomic constraint node's fields, keyed by node. A node with no
    // combinator edge but with these fields is an atomic operand.
    let atoms_q = format!(
        "SELECT ?c ?left ?op ?right WHERE {{ \
           ?c <{ODRL_NS}leftOperand> ?left . \
           OPTIONAL {{ ?c <{ODRL_NS}operator> ?op }} \
           OPTIONAL {{ ?c <{ODRL_NS}rightOperand> ?right }} \
           OPTIONAL {{ ?c <{ODRL_NS}rightOperandReference> ?right }} \
         }}"
    );
    let atoms_res = sparq_engine::query(graph, &atoms_q)?;
    // Group rows per atomic node and fold a multi-valued rightOperand (several
    // objects / an RDF list) into the set encoding — the same `RawConstraint`
    // accumulation the direct-constraint parse uses (sq-ueydm; previously
    // first-binding-wins).
    let mut atom_acc: BTreeMap<String, RawConstraint> = BTreeMap::new();
    for row in atoms_res.rows {
        let mut it = row.into_iter();
        let c_t = it.next().flatten();
        let left = it.next().flatten();
        let op = it.next().flatten();
        let right = it.next().flatten();
        let Some(c_t) = c_t else { continue };
        atom_acc
            .entry(node_key(&c_t))
            .or_default()
            .absorb(left, op, right, false);
    }
    let atoms: BTreeMap<String, Constraint> = atom_acc
        .into_iter()
        .map(|(ckey, raw)| raw.build(lists).map(|c| (ckey, c)))
        .collect::<Result<_, String>>()?;

    // (2b) Fold LIST-valued combinator operands (`odrl:or ( <c1> <c2> )`) into their
    // member node keys before assembly; a malformed/degenerate collection REFUSES the
    // whole parse (fail-closed on both rule kinds). HEAD-position rest-only cells come
    // from the pre-loaded `lists.rest_only` set. [FABLE-5] sq-dkuff.
    fold_list_operands(&mut lc_defs, &atoms, lists)?;

    // (3) The rule → direct-constraint-node map (which of a rule's `odrl:constraint`
    // objects are LogicalConstraint nodes), in rule/graph order.
    let rule_lc_q = format!(
        "SELECT ?rule ?c WHERE {{ \
           ?policy <{ODRL_NS}{kind}> ?rule . \
           ?rule <{ODRL_NS}constraint> ?c . \
         }}"
    );
    let rule_lc_res = sparq_engine::query(graph, &rule_lc_q)?;
    let mut out: BTreeMap<String, Vec<LogicalConstraint>> = BTreeMap::new();
    let mut seen_rule_c: BTreeMap<String, ()> = BTreeMap::new();
    for row in rule_lc_res.rows {
        let mut it = row.into_iter();
        let (Some(Some(rule_t)), Some(Some(c_t))) = (it.next(), it.next()) else {
            continue;
        };
        let rkey = node_key(&rule_t);
        let ckey = node_key(&c_t);
        if seen_rule_c.insert(format!("{rkey}|{ckey}"), ()).is_some() {
            continue;
        }
        // Only assemble compound nodes here (atomic direct constraints are handled by
        // constraints_for); a node is compound iff it has a combinator edge.
        if lc_defs.contains_key(&ckey) {
            let mut stack = BTreeMap::new();
            let lc = assemble_logical(&ckey, &lc_defs, &atoms, &mut stack);
            out.entry(rkey).or_default().push(lc);
        }
    }
    Ok(out)
}

/// Fold LIST-valued combinator operands into their member node keys. [FABLE-5] sq-dkuff.
///
/// ODRL examples mostly write a combinator's operands as several objects of the one
/// property (`odrl:and <c1>, <c2>` — the SolidLab suite's form, handled by the edges
/// query above), but a LIST-valued combinator (`odrl:or ( <c1> <c2> )`) appears in the
/// wild. The edges query binds such an object to the RDF-collection HEAD node — neither
/// a compound nor an atomic constraint — so pre-fold it degraded to the unsatisfiable
/// guard. That was fail-closed for a *permission*'s compound (never permits), but on a
/// **prohibition** it silently DISABLED the compound (a never-satisfied carve-out never
/// fires → deny-overrides is bypassed — the widening direction). This pass expands the
/// head into its member node keys via the pre-loaded [`ListCell`] table, in place and
/// in list order, deduplicated against the combinator's already-seen operands.
///
/// **Strictly narrow:** a key is expanded ONLY when it would otherwise degrade — it is
/// a list head (has a cons cell) AND is neither a known compound nor a known atomic
/// constraint node, so no currently-working parse changes (a pathological node that is
/// both a cons cell and a real constraint keeps its constraint reading).
///
/// **Malformed/degenerate collections REFUSE the whole parse (`Err`)** — the
/// [`policy_conflict`] ambiguity precedent — rather than degrading to the
/// unsatisfiable guard, because per-operand degradation is NOT fail-closed on both
/// rule kinds: an unsatisfiable operand inside a *prohibition*'s compound disables
/// the carve-out (deny-overrides bypassed — widening), and honouring a valid
/// *prefix* of a broken list makes an `odrl:and` easier to satisfy than authored
/// (widening on a permission). Refused shapes: a collection that is not
/// well-formed ([`well_formed_list`]: broken/dangling tail, cycle, forked cell), a
/// HEAD-position rest-only cell (`rdf:rest` with no `rdf:first` — `rest_only`,
/// invisible to the cells table), an *empty* operand (`rdf:nil` directly — a
/// degenerate combinator), and a nested-list member (one level only, mirroring
/// [`fold_rights`]). The constraint-reading precedence applies to ALL of these:
/// a node that is a known compound or atomic constraint keeps that reading (even
/// `rdf:nil` or a cons-cell node), so no previously-working parse is refused.
fn fold_list_operands(
    lc_defs: &mut BTreeMap<String, LcDef>,
    atoms: &BTreeMap<String, Constraint>,
    lists: &ListTable,
) -> Result<(), String> {
    let nil = format!("<{RDF_NS}nil>");
    // Snapshot the compound-node keys: the mutation below never adds/removes defs,
    // only rewrites operand lists, so membership checks stay sound.
    let lc_keys: BTreeMap<String, ()> = lc_defs.keys().map(|k| (k.clone(), ())).collect();
    // A node with NO constraint reading (neither compound nor atomic) — only such
    // nodes are folded/refused; a real constraint keeps its reading.
    let no_reading = |k: &str| !lc_keys.contains_key(k) && !atoms.contains_key(k);
    let expandable = |k: &str| lists.cells.contains_key(k) && no_reading(k);
    let empty_op = |k: &str| *k == nil && no_reading(k);
    let rest_only_op = |k: &str| lists.rest_only.contains_key(k) && no_reading(k);
    for def in lc_defs.values_mut() {
        if !def
            .operands
            .iter()
            .any(|k| expandable(k) || empty_op(k) || rest_only_op(k))
        {
            continue;
        }
        let old = std::mem::take(&mut def.operands);
        for okey in old {
            if empty_op(&okey) {
                return Err(
                    "a LogicalConstraint combinator has an EMPTY collection operand (`( )`); \
                     a degenerate compound cannot be honoured deterministically on both rule \
                     kinds and is refused (fail-closed)"
                        .to_owned(),
                );
            }
            if rest_only_op(&okey) {
                return Err(format!(
                    "a LogicalConstraint combinator has a MALFORMED collection operand \
                     ({okey}: a cons cell asserting `rdf:rest` but no `rdf:first`); a \
                     member-less cell cannot be honoured and silently degrading it would \
                     widen decisions on a prohibition, so the policy is refused (fail-closed)"
                ));
            }
            if expandable(&okey) {
                let Some(members) = well_formed_list(&okey, lists) else {
                    return Err(format!(
                        "a LogicalConstraint combinator has a MALFORMED collection operand \
                         ({okey}: broken/dangling tail, cycle, or forked cell); honouring a \
                         prefix could widen the authored constraint set, so the policy is \
                         refused (fail-closed)"
                    ));
                };
                for member in members {
                    let mkey = node_key(&member);
                    if (mkey == nil || lists.is_collection_shaped(&mkey)) && no_reading(&mkey) {
                        return Err(format!(
                            "a LogicalConstraint combinator collection operand ({okey}) has a \
                             NESTED-list member ({mkey}); nested collections are not a \
                             supported operand shape and are refused (fail-closed)"
                        ));
                    }
                    if def.seen.insert(mkey.clone(), ()).is_none() {
                        def.operands.push(mkey);
                    }
                }
            } else {
                // Already deduplicated at absorb time — keep its position.
                def.operands.push(okey);
            }
        }
    }
    Ok(())
}

/// Recursively assemble a [`LogicalConstraint`] from the pre-built node tables, with
/// cycle protection (`active` tracks the ancestor chain). A malformed node — not in the
/// combinator table and not a known atomic, or part of a cycle — becomes the
/// unsatisfiable-guard atomic constraint (fail-closed). [OPUS-4.8] sq-a0zef.
fn assemble_logical(
    lc_key: &str,
    lc_defs: &BTreeMap<String, LcDef>,
    atoms: &BTreeMap<String, Constraint>,
    active: &mut BTreeMap<String, ()>,
) -> LogicalConstraint {
    let def = lc_defs
        .get(lc_key)
        .expect("assemble_logical called on a non-LC node");
    active.insert(lc_key.to_owned(), ());
    let mut operands = Vec::with_capacity(def.operands.len());
    for okey in &def.operands {
        let node = if lc_defs.contains_key(okey) {
            if active.contains_key(okey) {
                // A cycle in the LogicalConstraint graph — fail closed rather than loop.
                ConstraintNode::Atomic(unsatisfiable_constraint())
            } else {
                ConstraintNode::Compound(assemble_logical(okey, lc_defs, atoms, active))
            }
        } else if let Some(c) = atoms.get(okey) {
            ConstraintNode::Atomic(c.clone())
        } else {
            // An operand that is neither a recognised compound nor a well-formed atomic
            // constraint → unsatisfiable guard (fail-closed).
            ConstraintNode::Atomic(unsatisfiable_constraint())
        };
        operands.push(node);
    }
    active.remove(lc_key);
    LogicalConstraint {
        id: lc_key.to_owned(),
        operator: def.operator,
        operands,
    }
}

/// The unsatisfiable-guard atomic constraint used for a malformed/unknown operand
/// (a constraint that can never be satisfied — fail-closed). Shared by [`build_constraint`]
/// and the compound-operand assembler. [OPUS-4.8] sq-a0zef.
fn unsatisfiable_constraint() -> Constraint {
    Constraint {
        left: "urn:sparq-policy:malformed".to_owned(),
        operator: Operator::Neq,
        right: Value::Iri("urn:sparq-policy:malformed".to_owned()),
    }
}

/// Build a [`Constraint`], turning anything malformed/unknown into an
/// unsatisfiable guard (fail-closed): the enclosing rule can then never match.
/// `right` is the already-folded value (see [`fold_rights`]): `None` covers both
/// a missing right operand and an unfoldable multi-value. [FABLE-5] sq-ueydm.
fn build_constraint(left: Option<Term>, op: Option<Term>, right: Option<Value>) -> Constraint {
    let (Some(left), Some(op), Some(right)) = (left, op, right) else {
        return unsatisfiable_constraint();
    };
    let operator = match Operator::from_iri(&term_str(&op)) {
        Some(o) => o,
        None => return unsatisfiable_constraint(),
    };
    Constraint {
        left: term_str(&left),
        operator,
        right,
    }
}

/// Parse duties (and their actions/constraints) per rule node.
fn duties_for(graph: &Graph, kind: &str) -> Result<BTreeMap<String, Vec<Duty>>, String> {
    let q = format!(
        "SELECT ?rule ?d ?a WHERE {{ \
           ?policy <{ODRL_NS}{kind}> ?rule . \
           ?rule <{ODRL_NS}duty> ?d . \
           OPTIONAL {{ ?d <{ODRL_NS}action> ?a }} \
         }}"
    );
    let res = sparq_engine::query(graph, &q)?;
    let mut out: BTreeMap<String, Vec<Duty>> = BTreeMap::new();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for row in res.rows {
        let mut it = row.into_iter();
        let rule_t = it.next().flatten();
        let d_t = it.next().flatten();
        let a_t = it.next().flatten();
        let (Some(rule_t), Some(d_t)) = (rule_t, d_t) else {
            continue;
        };
        let rkey = node_key(&rule_t);
        let dkey = format!("{rkey}|{}", node_key(&d_t));
        if seen.insert(dkey, ()).is_some() {
            continue;
        }
        let action = a_t
            .map(|t| Action(term_str(&t)))
            .unwrap_or_else(Action::use_);
        out.entry(rkey).or_default().push(Duty {
            id: term_str(&d_t),
            action,
            constraints: Vec::new(),
        });
    }
    Ok(out)
}

/// Classify a [`Term`] into a typed [`Value`] (numeric/dateTime/iri/string).
pub(crate) fn value_of(t: &Term) -> Value {
    match t {
        Term::NamedNode(n) => Value::Iri(n.as_str().to_owned()),
        Term::BlankNode(b) => Value::Str(format!("_:{}", b.as_str())),
        Term::Literal(l) => literal_value(l),
        #[allow(unreachable_patterns)]
        _ => Value::Str(String::new()),
    }
}

fn literal_value(l: &Literal) -> Value {
    let dt = l.datatype();
    let v = l.value();
    if is_datetime(dt) {
        Value::DateTime(v.to_owned())
    } else if is_numeric(dt) {
        match v.trim().parse::<f64>() {
            Ok(n) => Value::Num(n),
            Err(_) => Value::Str(v.to_owned()),
        }
    } else {
        Value::Str(v.to_owned())
    }
}

fn is_datetime(dt: oxrdf::NamedNodeRef<'_>) -> bool {
    matches!(
        dt.as_str().strip_prefix(XSD),
        Some("dateTime" | "date" | "dateTimeStamp")
    )
}

fn is_numeric(dt: oxrdf::NamedNodeRef<'_>) -> bool {
    matches!(
        dt.as_str().strip_prefix(XSD),
        Some(
            "integer"
                | "decimal"
                | "double"
                | "float"
                | "long"
                | "int"
                | "short"
                | "byte"
                | "nonNegativeInteger"
                | "positiveInteger"
                | "unsignedLong"
                | "unsignedInt"
        )
    )
}
