//! Parse an ODRL policy expressed in RDF into the typed [`Policy`] model.
//!
//! The parser extracts policy fields through metered predicate-index scans and
//! SPARQL queries over the loaded graph. Independent field scans keep constraint
//! multiplicities additive; rule membership and constraint attachments are shared
//! across direct constraints, logical constraints and expansion checks.
//!
//! Rule and constraint nodes retain their graph identity, including blank nodes.
//! Predicate scans preserve the node-listing query's index order, while compound
//! combinators are still bound by variable in the engine.
//!
//! Input forms accepted by [`parse_policy_str`]: any RDF serialization
//! `sparq_core::Graph::load_str` accepts (`turtle`, `ntriples`, …). The policy
//! IRI is whichever subject is `a odrl:Policy`/`Set`/`Offer`/`Agreement`, or —
//! if none is typed — whichever subject carries `odrl:permission`/`prohibition`.

use crate::model::{
    Action, ConflictStrategy, Constraint, ConstraintNode, Duty, LogicalConstraint, LogicalOperator,
    Operator, Policy, Rule, Value, ODRL_NS,
};
use crate::validate::ValidatedPolicy;
use oxrdf::{Literal, Term};
use sparq_core::dict::Id;
use sparq_core::meter::{Dim, Exceeded, Ledger, Limits, TermRef};
use sparq_core::temporal::Temporal;
use sparq_core::Graph;
use sparq_engine::{PreparedQuery, QueryBudget, QueryFailure, QueryResult};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

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
pub fn parse_policy_str(rdf: &str, format: &str) -> Result<ValidatedPolicy, String> {
    let graph = Graph::load_str(rdf, format)?;
    let mut ledger = Ledger::new(policy_limits(&graph, Some(rdf.len())));
    parse_policy_metered(&graph, &mut ledger)
}

/// [`parse_policy_str`] plus the document's [`target_memberships`]: the policy and the
/// `(member, collection)` asset evidence for its collection targets, from one load.
///
/// # Errors
///
/// As [`parse_policy_str`].
pub fn parse_policy_str_with_memberships(
    rdf: &str,
    format: &str,
) -> Result<(ValidatedPolicy, BTreeSet<(String, String)>), String> {
    let graph = Graph::load_str(rdf, format)?;
    let mut ledger = Ledger::new(policy_limits(&graph, Some(rdf.len())));
    let policy = parse_policy_metered(&graph, &mut ledger)?;
    let members = target_memberships_metered(&policy, &graph, &mut ledger)?;
    Ok((policy, members))
}

/// Default ceilings for policy parsing, before input-specific sizing.
///
/// Depth retains the corpus-compatible compound descent bound. Nodes retain the
/// former per-compound allowance, up to a policy-wide ceiling derived by probing
/// the main parser's repeated-compound workload. Bytes cover retained terms and
/// expanded atom copies: wrappers use a source-sized base (or graph fallback)
/// plus an input-sized retention allowance and a node-bounded copy allowance.
/// Query rows cover twice the extraction-pass count times the graph's triple count
/// (at least one). Corpus parity is checked against the frozen main
/// parser in `tests/parse_differential.rs`. That corpus needs shallow nesting
/// and small compounds; defaults retain the legacy margins. Shared compounds
/// additionally require an allowance for their expanded copies.
///
/// This graph-independent template uses the node ceiling as its row-sizing
/// reference. The ordinary wrappers size nodes, bytes and rows to the input. Set
/// `nodes`, `bytes`, `depth` or `query_rows` higher before constructing a
/// [`Ledger`] to parse a larger policy with [`parse_policy_metered`].
pub fn default_policy_limits() -> Limits {
    let mut limits = Limits::default();
    limits.depth = MAX_CONSTRAINT_DEPTH;
    limits.nodes = POLICY_NODE_CEILING;
    limits.bytes = 4 * 1024 * 1024;
    limits.query_rows = POLICY_QUERY_COUNT
        .saturating_mul(POLICY_NODE_CEILING)
        .saturating_mul(2);
    limits
}

// Count rule nodes through the two indexed incident-edge scans. This does not
// materialize a query result or inspect constraint values.
fn policy_limits(graph: &Graph, source_bytes: Option<usize>) -> Limits {
    let mut limits = default_policy_limits();
    let mut rules = BTreeSet::new();
    for kind in ["permission", "prohibition"] {
        let p = graph
            .dict()
            .lookup(&oxrdf::NamedNode::new_unchecked(format!("{ODRL_NS}{kind}")).into());
        let scan = graph.store().scan(&[None, Some(p), None]);
        for row in scan.rows.iter() {
            rules.insert(scan.to_spo(row)[2]);
        }
    }
    let count = |property: &str| {
        let p = graph
            .dict()
            .lookup(&oxrdf::NamedNode::new_unchecked(property.to_owned()).into());
        graph.store().scan(&[None, Some(p), None]).rows.len()
    };
    // The old allowance resets per attached compound, including several on one
    // rule. List cells are additional accounted work, rather than operand copies.
    let constraints = count(&format!("{ODRL_NS}constraint"));
    let list_cells = count(&format!("{RDF_NS}first"));
    limits.nodes = MAX_CONSTRAINT_NODES
        .saturating_mul(rules.len().max(constraints))
        .saturating_add(list_cells)
        .min(POLICY_NODE_CEILING);
    if let Some(size) = source_bytes {
        limits.bytes = size.saturating_mul(4);
    }
    // Source bytes bound retained input terms; expanded model atoms also own
    // strings. Include their policy-wide allowance so compact shared compounds
    // accepted by the legacy parser retain their result. Explicit ledgers keep
    // exactly the caller's byte cap.
    let mut sizing = Ledger::new(Limits::default());
    let mut largest_term = 0usize;
    for (id, _) in graph.dict().iter() {
        if let Ok(size) = sizing.stored_term(graph.dict(), id) {
            largest_term = largest_term.max(size);
        }
    }
    let retained = graph.len().saturating_mul(largest_term).saturating_mul(3);
    // A folded set can contain the whole term vocabulary, so one stored term
    // is not an upper bound on an atom's owned right-hand value.
    let expanded = limits
        .nodes
        .saturating_mul(sizing.spent(Dim::Bytes))
        .saturating_mul(3);
    limits.bytes = limits.bytes.max(retained).saturating_add(expanded);
    limits.query_rows = POLICY_QUERY_COUNT
        .saturating_mul(graph.len().max(1))
        .saturating_mul(2);
    limits
}

// Upper bound on the fixed bulk queries and predicate scans, including memberships.
const POLICY_QUERY_COUNT: usize = 64;
const POLICY_NODE_CEILING: usize = 1_906_000;

/// Parse RDF using the caller's ledger for all policy extraction and assembly.
/// RDF loading itself retains [`Graph::load_str`]'s input contract.
/// # Errors
/// As [`parse_policy_str`], including typed ledger refusals read with
/// [`constraint_budget_refusal`].
pub fn parse_policy_str_metered(
    rdf: &str,
    format: &str,
    ledger: &mut Ledger,
) -> Result<ValidatedPolicy, String> {
    let graph = Graph::load_str(rdf, format)?;
    parse_policy_metered(&graph, ledger)
}

/// Parse the policy and target memberships with the same caller-owned ledger.
/// # Errors
/// As [`parse_policy_str_metered`]; membership queries also consume query rows.
pub fn parse_policy_str_with_memberships_metered(
    rdf: &str,
    format: &str,
    ledger: &mut Ledger,
) -> Result<(ValidatedPolicy, BTreeSet<(String, String)>), String> {
    let graph = Graph::load_str(rdf, format)?;
    let policy = parse_policy_metered(&graph, ledger)?;
    let members = target_memberships_metered(&policy, &graph, ledger)?;
    Ok((policy, members))
}

fn query_metered(graph: &Graph, sparql: &str, ledger: &mut Ledger) -> Result<QueryResult, String> {
    let prepared = PreparedQuery::parse(sparql)?;
    let budget = QueryBudget {
        max_rows: Some(ledger.remaining(Dim::QueryRows)),
        ..QueryBudget::default()
    };
    let result = sparq_engine::query_prepared_with_budget_detailed(graph, &prepared, &budget)
        .map_err(|err| match err {
            QueryFailure::Budget(_) => budget_error(ConstraintBudget::Rows),
            other => other.to_string(),
        })?;
    ledger
        .charge(Dim::QueryRows, result.len())
        .map_err(|e| ledger_error(e, ConstraintBudget::Rows))?;
    Ok(result)
}

// Predicate-bound scans are contiguous index slices. Charge every scanned
// triple, including rows a consumer filters out, before decoding any terms.
fn scan_property(
    graph: &Graph,
    iri: &str,
    ledger: &mut Ledger,
    mut visit: impl FnMut(Id, Id),
) -> Result<(), String> {
    let predicate = graph
        .dict()
        .lookup(&oxrdf::NamedNode::new_unchecked(iri).into());
    let scan = graph.store().scan(&[None, Some(predicate), None]);
    ledger
        .charge(Dim::QueryRows, scan.rows.len())
        .map_err(|e| ledger_error(e, ConstraintBudget::Rows))?;
    for row in scan.rows.iter() {
        let [subject, _, object] = scan.to_spo(row);
        visit(subject, object);
    }
    Ok(())
}

/// The `member odrl:partOf collection` edges `graph` states (default graph) whose
/// `collection` is the target of one of `policy`'s rules, as `(member, collection)`.
///
/// A rule matches a request on an asset that is the rule's target or a member of it under
/// the request's asset evidence ([`crate::Request::asset_matches`]). An enforcement point
/// that evaluates a policy per target graph therefore evaluates each of these members too,
/// with the edge as the request's [`crate::Request::with_asset_membership`] evidence.
/// Being a rule target makes `collection` an asset collection here, whatever
/// [`Policy::party_collections`] also records for it. Blank nodes and literals are skipped.
///
/// # Errors
///
/// Returns `Err` if the query over the graph fails.
pub fn target_memberships(
    policy: &Policy,
    graph: &Graph,
) -> Result<BTreeSet<(String, String)>, String> {
    let mut ledger = Ledger::new(policy_limits(graph, None));
    target_memberships_metered(policy, graph, &mut ledger)
}

fn target_memberships_metered(
    policy: &Policy,
    graph: &Graph,
    ledger: &mut Ledger,
) -> Result<BTreeSet<(String, String)>, String> {
    let targets: BTreeSet<&str> = policy
        .permissions
        .iter()
        .chain(policy.prohibitions.iter())
        .filter_map(|r| r.target.as_deref())
        .collect();
    if targets.is_empty() {
        return Ok(BTreeSet::new());
    }
    let res = query_metered(
        graph,
        &format!("SELECT ?m ?c WHERE {{ ?m <{ODRL_NS}partOf> ?c }}"),
        ledger,
    )?;
    Ok(res
        .rows
        .into_iter()
        .filter_map(
            |r| match (r.first().cloned().flatten(), r.get(1).cloned().flatten()) {
                (Some(Term::NamedNode(m)), Some(Term::NamedNode(c)))
                    if targets.contains(c.as_str()) =>
                {
                    Some((m.into_string(), c.into_string()))
                }
                _ => None,
            },
        )
        .collect())
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
/// encoding, likewise widening), or a rule's compound constraint is past the
/// constraint-assembly budget (nesting depth or total operand expansion;
/// [`constraint_budget_refusal`] tells that refusal from any other parse error).
/// The result is validated ([`Policy::validate`]).
pub fn parse_policy(graph: &Graph) -> Result<ValidatedPolicy, String> {
    let mut ledger = Ledger::new(policy_limits(graph, None));
    parse_policy_metered(graph, &mut ledger)
}

/// Parse an already-loaded policy using one ledger across every rule and query.
/// Counters remain spent on failure; entered compound depths are always balanced.
/// # Errors
/// As [`parse_policy`]. Ledger refusals carry [`ConstraintBudget`] tags.
pub fn parse_policy_metered(graph: &Graph, ledger: &mut Ledger) -> Result<ValidatedPolicy, String> {
    let iri = policy_iri(graph, ledger)?;
    // Bulk-load the graph's RDF collection shapes ONCE (cons cells + the member-less
    // rest-only heads), so a multi-valued `odrl:rightOperand ( <a> <b> )` list can be
    // folded into the set encoding — or refused — by every constraint parse below.
    // sq-ueydm.
    let lists = rdf_list_table(graph, ledger)?;
    let permissions = rules(graph, "permission", true, &lists, ledger)?;
    let prohibitions = rules(graph, "prohibition", false, &lists, ledger)?;
    refuse_unsupported_rule_heads(graph, &lists, ledger)?;
    let conflict = policy_conflict(graph, ledger)?;
    let party_collections = party_collections(graph, ledger)?;
    Policy {
        iri,
        permissions,
        prohibitions,
        conflict,
        party_collections,
    }
    .validate()
}

/// The IRIs this graph identifies as an `odrl:PartyCollection` — retained on the
/// [`Policy`] so a consumer that FREEZES a rule into an identity-matched head can route
/// collection-valued heads to a membership-aware path. sq-rf9uv.
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
fn party_collections(graph: &Graph, ledger: &mut Ledger) -> Result<BTreeSet<String>, String> {
    let res = query_metered(
        graph,
        &format!(
            "SELECT ?c WHERE {{ {{ ?c a <{ODRL_NS}PartyCollection> }} \
             UNION {{ ?m <{ODRL_NS}partOf> ?c . \
             FILTER NOT EXISTS {{ ?c a <{ODRL_NS}AssetCollection> }} }} }}"
        ),
        ledger,
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
/// sq-ihqbl.
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
/// strictly the *more* fail-closed direction. sq-ihqbl.
fn policy_conflict(graph: &Graph, ledger: &mut Ledger) -> Result<Option<ConflictStrategy>, String> {
    let res = query_metered(
        graph,
        &format!("SELECT DISTINCT ?c WHERE {{ ?p <{ODRL_NS}conflict> ?c }} ORDER BY ?c"),
        ledger,
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

fn policy_iri(graph: &Graph, ledger: &mut Ledger) -> Result<Option<String>, String> {
    let typed = query_metered(
        graph,
        &format!(
            "SELECT ?p WHERE {{ ?p a ?t . \
             VALUES ?t {{ <{ODRL_NS}Policy> <{ODRL_NS}Set> <{ODRL_NS}Offer> <{ODRL_NS}Agreement> }} }}"
        ), ledger)?;
    if let Some(Term::NamedNode(n)) = typed
        .rows
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next().flatten())
    {
        return Ok(Some(n.into_string()));
    }
    let withrule = query_metered(
        graph,
        &format!(
            "SELECT ?p WHERE {{ ?p ?r ?x . \
             VALUES ?r {{ <{ODRL_NS}permission> <{ODRL_NS}prohibition> }} }}"
        ),
        ledger,
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
fn group_by_first(
    graph: &Graph,
    sparql: &str,
    ledger: &mut Ledger,
) -> Result<BTreeMap<String, Vec<Term>>, String> {
    let res = query_metered(graph, sparql, ledger)?;
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
    ledger: &mut Ledger,
) -> Result<Vec<Rule>, String> {
    // Enumerate the rule nodes first (keyed), then attach attributes by join.
    // (BIND a copy so the projection has two distinct variables — the engine
    // rejects `SELECT ?rule ?rule`.)
    let nodes = group_by_first(
        graph,
        &format!("SELECT ?rule ?rule2 WHERE {{ ?policy <{ODRL_NS}{kind}> ?rule . BIND(?rule AS ?rule2) }}"), ledger)?;

    ensure_constraint_roots(graph, lists, ledger)?;
    let roots_ref = lists.roots.borrow();
    let rule_ids = roots_ref
        .as_ref()
        .expect("loaded above")
        .rules_for_kind(kind);
    let actions = rule_attributes(graph, rule_ids, "action", ledger)?;
    let targets = rule_attributes(graph, rule_ids, "target", ledger)?;
    let assignees = rule_attributes(graph, rule_ids, "assignee", ledger)?;
    let assigners = rule_attributes(graph, rule_ids, "assigner", ledger)?;

    let mut constraints = constraints_for(graph, kind, lists, ledger)?;
    let mut logical_constraints = logical_constraints_for(graph, kind, lists, ledger)?;
    let mut duties = if with_duties {
        duties_for(graph, kind, ledger)?
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
            constraints: constraints.remove(&rule_key).unwrap_or_default(),
            logical_constraints: logical_constraints.remove(&rule_key).unwrap_or_default(),
            duties: duties.remove(&rule_key).unwrap_or_default(),
        });
    }
    Ok(out)
}

// A semijoin by dictionary id avoids re-evaluating rule membership for each
// attribute value. Predicate index order retains the original first value.
fn rule_attributes(
    graph: &Graph,
    rules: &BTreeSet<Id>,
    property: &str,
    ledger: &mut Ledger,
) -> Result<BTreeMap<String, Vec<Term>>, String> {
    let mut out: BTreeMap<String, Vec<Term>> = BTreeMap::new();
    scan_property(graph, &format!("{ODRL_NS}{property}"), ledger, |s, o| {
        if rules.contains(&s) {
            out.entry(node_key(&graph.dict().term(s)))
                .or_default()
                .push(graph.dict().term(o));
        }
    })?;
    Ok(out)
}

fn first_str(m: &BTreeMap<String, Vec<Term>>, key: &str) -> Option<String> {
    m.get(key).and_then(|v| v.first()).map(term_str)
}

/// One RDF collection cons cell (`rdf:first`/`rdf:rest`), keyed in the list table
/// by its node key. `rest` is the node key of the tail (the next cell, or
/// `rdf:nil`); `None` when the cell is malformed (an `rdf:first` with no
/// `rdf:rest`), which the walk reads as end-of-list. sq-ueydm.
struct ListCell {
    first: Term,
    rest: Option<String>,
    /// The node asserts SEVERAL distinct `rdf:first`/`rdf:rest` pairs (a forked
    /// list). The kept pair stays deterministic, but a well-formedness check must
    /// treat the cell as malformed: honouring one deterministic fork of an
    /// ambiguous collection could silently drop authored members. sq-dkuff.
    ambiguous: bool,
}

/// The graph's pre-loaded RDF-collection shape table (see [`rdf_list_table`]):
/// every well-keyed cons cell, PLUS the member-less HEAD-position nodes that are
/// collection-*shaped* but carry no `rdf:first`. Both are needed by every
/// consumer, because a node's absence from `cells` alone does not mean "not a
/// collection". sq-srjuc.
struct ListTable {
    /// Cons cells keyed by node key (`?n rdf:first ?f`).
    cells: BTreeMap<String, ListCell>,
    /// Nodes asserting `rdf:rest` but NO `rdf:first`. Such a node is invisible to
    /// `cells` (which is keyed on `rdf:first`), so without this set it would read
    /// as an ordinary IRI/blank-node value or operand and silently bypass
    /// collection validation. (A MID-chain rest-only cell is already caught by
    /// [`well_formed_list`]'s dangling-tail check; this set is what catches one in
    /// HEAD position.) sq-dkuff.
    rest_only: BTreeMap<String, ()>,
    resolved: RefCell<BTreeMap<String, Option<Rc<[Term]>>>>,
    // Both rule kinds and logical atoms reuse the same property scans.
    raw: RefCell<Option<BTreeMap<String, RawConstraint>>>,
    atoms: RefCell<Option<BTreeMap<String, Constraint>>>,
    expansion_checked: std::cell::Cell<bool>,
    roots: RefCell<Option<ConstraintRoots>>,
    charged_cells: RefCell<BTreeSet<String>>,
    #[cfg(test)]
    walks: std::cell::Cell<usize>,
}

impl ListTable {
    /// Is `key` collection-SHAPED — either a cons cell or a member-less rest-only
    /// node? Both shapes must be validated as a collection rather than read as an
    /// ordinary value/operand. sq-srjuc.
    fn is_collection_shaped(&self, key: &str) -> bool {
        self.cells.contains_key(key) || self.rest_only.contains_key(key)
    }
}

/// Bulk-load the graph's RDF collection shapes ONCE: separate first/rest scans
/// merge each cons cell by node key, alongside the `rest_only` set of
/// nodes asserting `rdf:rest` with no `rdf:first`. A `rightOperand` or combinator
/// operand bound to a list *head* can then be folded into its member terms — or
/// refused — without per-constraint queries (the same bulk-query + in-memory-assembly
/// pattern as [`logical_constraints_for`]). A malformed cell asserting several
/// `rdf:first`/`rdf:rest` values keeps its first-bound pair (degenerate, but the
/// load stays deterministic and terminating) and is flagged `ambiguous`, so every
/// consumer goes through [`well_formed_list`] and refuses it. sq-ueydm.
fn rdf_list_table(graph: &Graph, ledger: &mut Ledger) -> Result<ListTable, String> {
    let q = format!("SELECT ?n ?f WHERE {{ ?n <{RDF_NS}first> ?f }}");
    let res = query_metered(graph, &q, ledger)?;
    let mut cells: BTreeMap<String, ListCell> = BTreeMap::new();
    for row in res.rows {
        let mut it = row.into_iter();
        let n = it.next().flatten();
        let f = it.next().flatten();
        let (Some(n), Some(f)) = (n, f) else { continue };
        cells
            .entry(node_key(&n))
            // A SECOND row for the same cell node = a genuinely different
            // `rdf:first`/`rdf:rest` binding (a triple matches at most once) —
            // mark the fork so well-formedness checks can refuse it.
            // sq-dkuff.
            .and_modify(|c| c.ambiguous = true)
            .or_insert(ListCell {
                first: f,
                rest: None,
                ambiguous: false,
            });
    }
    let rest_res = query_metered(
        graph,
        &format!("SELECT ?n ?r WHERE {{ ?n <{RDF_NS}rest> ?r }}"),
        ledger,
    )?;
    let mut rest_only: BTreeMap<String, ()> = BTreeMap::new();
    for row in rest_res.rows {
        let mut it = row.into_iter();
        let (Some(n), Some(r)) = (it.next().flatten(), it.next().flatten()) else {
            continue;
        };
        let key = node_key(&n);
        if let Some(cell) = cells.get_mut(&key) {
            if cell.rest.is_some() {
                cell.ambiguous = true;
            } else {
                cell.rest = Some(node_key(&r));
            }
        } else {
            rest_only.insert(key, ());
        }
    }
    Ok(ListTable {
        cells,
        rest_only,
        raw: RefCell::new(None),
        atoms: RefCell::new(None),
        expansion_checked: std::cell::Cell::new(false),
        roots: RefCell::new(None),
        resolved: RefCell::new(BTreeMap::new()),
        charged_cells: RefCell::new(BTreeSet::new()),
        #[cfg(test)]
        walks: std::cell::Cell::new(0),
    })
}

/// Walk an RDF list from `head_key` and return its member terms ONLY when the
/// collection is WELL-FORMED: every cell has an unambiguous `rdf:first`/`rdf:rest`
/// pair, no cell repeats (no cycle), and the walk terminates at `rdf:nil`
/// explicitly. Returns `None` for a broken tail (missing `rdf:rest`), a dangling
/// tail (an `rdf:rest` pointing at a non-cell that is not `rdf:nil`), a cycle, a
/// forked (ambiguous) cell, or a member-less rest-only node (`rdf:rest` with no
/// `rdf:first` — absent from `cells`, so it never walks as a collection).
/// sq-dkuff.
///
/// A *prefix* of a malformed list must never be honoured where the members carry
/// authorization semantics: dropping an authored member from an `odrl:and` operand
/// set makes the compound EASIER to satisfy than authored, and dropping one from an
/// `odrl:rightOperand` set encoding weakens the set relation (`isNoneOf` excludes
/// fewer values) — both the widening direction. Shared by [`fold_list_operands`]
/// (sq-dkuff) and [`fold_rights`] (sq-srjuc).
fn well_formed_list(
    head_key: &str,
    lists: &ListTable,
    ledger: &mut Ledger,
) -> Result<Option<Rc<[Term]>>, String> {
    if let Some(cached) = lists.resolved.borrow().get(head_key) {
        return Ok(cached.clone());
    }
    #[cfg(test)]
    lists.walks.set(lists.walks.get() + 1);
    let nil = format!("<{RDF_NS}nil>");
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cur = head_key.to_owned();
    let result = loop {
        if cur == nil {
            break Some(Rc::from(out));
        }
        let Some(cell) = lists.cells.get(&cur) else {
            break None;
        };
        if cell.ambiguous || !seen.insert(cur.clone()) {
            break None;
        }
        if !lists.charged_cells.borrow().contains(&cur) {
            ledger
                .reserve(&mut out, 1, Dim::Nodes)
                .map_err(|e| ledger_error(e, ConstraintBudget::Nodes))?;
            lists.charged_cells.borrow_mut().insert(cur.clone());
        } else {
            out.try_reserve(1)
                .map_err(|_| budget_error(ConstraintBudget::Nodes))?;
        }
        ledger
            .term(TermRef::from(&cell.first))
            .map_err(|e| ledger_error(e, ConstraintBudget::Bytes))?;
        out.push(cell.first.clone());
        let Some(rest) = &cell.rest else { break None };
        cur.clone_from(rest);
    };
    lists
        .resolved
        .borrow_mut()
        .insert(head_key.to_owned(), result.clone());
    Ok(result)
}

/// Is `op` one of the ODRL set-relation operators (`isPartOf`/`isAnyOf`/`isNoneOf`)
/// that consume the `|`/space/comma set encoding — the only operators a
/// MULTI-valued `rightOperand` can be faithfully folded for? sq-ueydm.
fn is_set_operator(op: Option<&Term>) -> bool {
    matches!(
        op.and_then(|t| Operator::from_iri(&term_str(t))),
        Some(Operator::IsPartOf | Operator::IsAnyOf | Operator::IsNoneOf)
    )
}

/// Fold a constraint's collected `rightOperand` objects — expanding any RDF list
/// (`rdf:first`/`rdf:rest` chain) into its members — into a single [`Value`].
/// sq-ueydm.
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
///   (fail-closed, consistent with every other malformed-constraint path). That guard
///   is fail-closed only on a permission; on a prohibition the whole policy is refused
///   instead ([`Policy::validate`]).
///
/// Members are deduplicated by node key. An empty list (`rdf:nil` directly as the
/// object) has no cons cell, so it falls through as the plain nil IRI — unmatchable
/// by ordinary values, as before.
///
/// **A NESTED-list member REFUSES the whole parse (`Err`)** — mirroring the sibling
/// refusal in [`fold_list_operands`]. Kept as an opaque blank-node string it would be
/// an unmatchable member, which is fail-closed only for the POSITIVE set operators:
/// under `isNoneOf` it excludes nothing, silently dropping the authored exclusion on a
/// permission (and narrowing a set-op prohibition's carve-out). #3982.
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
/// permission grant. sq-srjuc.
fn fold_rights(
    op: Option<&Term>,
    rights: &[Term],
    lists: &ListTable,
    ledger: &mut Ledger,
) -> Result<Option<Value>, String> {
    let mut members: Vec<Term> = Vec::new();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for r in rights {
        let key = node_key(r);
        // Collection-SHAPED (a cons cell OR a member-less rest-only node) → it must
        // validate as a collection; a rest-only head is invisible to the cell table,
        // so testing only `cells` would read it as an ordinary value. sq-srjuc.
        let expanded = if lists.is_collection_shaped(&key) {
            well_formed_list(&key, lists, ledger)?.ok_or_else(|| {
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
            Rc::from([r.clone()])
        };
        if lists.is_collection_shaped(&key) {
            let nil = format!("<{RDF_NS}nil>");
            if let Some(m) = expanded.iter().find(|m| {
                let mkey = node_key(m);
                mkey == nil || lists.is_collection_shaped(&mkey)
            }) {
                return Err(format!(
                    "a constraint's collection rightOperand ({key}) has a NESTED-list \
                     member ({}); nested collections are not a supported operand shape \
                     (an unmatchable member would silently drop an `isNoneOf` exclusion), \
                     so the policy is refused (fail-closed)",
                    node_key(m)
                ));
            }
        }
        for m in expanded.iter() {
            // Every collection member is checked before the node-key deduplication. #3902.
            if let Some(err) = ill_typed_operand(m) {
                return Err(err);
            }
            if seen.insert(node_key(m), ()).is_none() {
                ledger
                    .term(TermRef::from(m))
                    .map_err(|e| ledger_error(e, ConstraintBudget::Bytes))?;
                members.push(m.clone());
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
            // The set encoding is textual: a typed member (a number, a dateTime) would
            // lose its type and match a same-lexical string, so the set degrades to the
            // guard (Unknown) instead.
            if members.iter().any(typed_member) {
                return Ok(None);
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
/// into one constraint — not first-binding-wins. sq-ueydm.
#[derive(Default)]
struct RawConstraint {
    left: Option<Term>,
    op: Option<Term>,
    rights: Vec<Term>,
    rights_seen: BTreeMap<String, ()>,
    is_logical: bool,
    /// Set when the node carries several DISTINCT `leftOperand` objects. #3832.
    left_conflict: bool,
    /// Set when the node carries several DISTINCT `operator` objects. #3832.
    op_conflict: bool,
    /// Set when the node uses a feature the evaluator does not implement (an
    /// `odrl:unit`). Such a constraint becomes the unsatisfiable guard, which the
    /// evaluator reads as Unknown.
    unsupported: bool,
    /// The refusal for the first ill-typed right operand seen, recorded BEFORE the
    /// node-key deduplication could collapse it into a same-lexical sibling. #3902.
    ill_typed: Option<String>,
}

/// The refusal for a right operand with no value of its datatype: a timezone-free (or
/// otherwise ill-formed) `xsd:dateTimeStamp`, whose timezone XSD 1.1 §3.4.28 requires.
/// Comparing its lexical would decide access on a literal the datatype rejects, and
/// making just its constraint unsatisfiable would disable a prohibition it gates, so, like
/// a malformed collection operand, the whole policy is refused (fail-closed on both rule
/// kinds). Checked on every operand before any deduplication. XSD whitespace collapse
/// applies first, as the datatype's lexical space requires. #3902.
fn ill_typed_operand(t: &Term) -> Option<String> {
    let Term::Literal(l) = t else { return None };
    if l.datatype().as_str() != DATE_TIME_STAMP {
        return None;
    }
    let collapsed = l
        .value()
        .trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
    if Temporal::of_lit(collapsed, DATE_TIME_STAMP).is_some() {
        return None;
    }
    Some(format!(
        "a constraint's rightOperand {l} is not a valid value of its datatype \
         (xsd:dateTimeStamp requires a well-formed lexical with a timezone), so the policy \
         is refused (fail-closed)"
    ))
}

const DATE_TIME_STAMP: &str = "http://www.w3.org/2001/XMLSchema#dateTimeStamp";

/// Bind a single-valued field from one result row, flagging `conflict` when a
/// DIFFERENT value (by node key) has already been bound. #3832.
fn absorb_single(slot: &mut Option<Term>, conflict: &mut bool, v: Option<Term>) {
    let Some(v) = v else { return };
    match slot {
        None => *slot = Some(v),
        Some(cur) => {
            if node_key(cur) != node_key(&v) {
                *conflict = true;
            }
        }
    }
}

impl RawConstraint {
    /// Merge one result row into the accumulator: `left`/`op` are single-valued
    /// per the ODRL model — a second DISTINCT value is recorded as a conflict
    /// (refused at [`RawConstraint::build`], never first-binding-wins: which
    /// binding comes first is not stable under result ordering, #3832); `right`
    /// objects accumulate (deduplicated by node key, in row order); `is_logical`
    /// is sticky.
    fn absorb(
        &mut self,
        left: Option<Term>,
        op: Option<Term>,
        right: Option<Term>,
        is_logical: bool,
    ) {
        absorb_single(&mut self.left, &mut self.left_conflict, left);
        absorb_single(&mut self.op, &mut self.op_conflict, op);
        if let Some(r) = right {
            if self.ill_typed.is_none() {
                self.ill_typed = ill_typed_operand(&r);
            }
            if self.rights_seen.insert(node_key(&r), ()).is_none() {
                self.rights.push(r);
            }
        }
        self.is_logical |= is_logical;
    }

    /// Fold the accumulated right operands (expanding RDF lists) and build the
    /// [`Constraint`] — anything malformed degrades to the unsatisfiable guard,
    /// except a malformed COLLECTION right operand, which refuses the whole parse
    /// (`Err`; see [`fold_rights`]). sq-srjuc.
    ///
    /// A node with several distinct `leftOperand`/`operator` objects also refuses
    /// the parse (`Err`): degrading it to the unsatisfiable guard would DISABLE a
    /// prohibition (the widening direction), so — as for a malformed collection —
    /// the ambiguous shape is refused on both rule kinds. #3832.
    fn build(&self, lists: &ListTable, ledger: &mut Ledger) -> Result<Constraint, String> {
        for (conflict, field) in [
            (self.left_conflict, "odrl:leftOperand"),
            (self.op_conflict, "odrl:operator"),
        ] {
            if conflict {
                return Err(format!(
                    "a constraint carries several distinct {field} objects; a constraint \
                     is single-valued in that position and evaluating an arbitrarily \
                     chosen one is not stable under result ordering, so the policy is \
                     refused (fail-closed)"
                ));
            }
        }
        if let Some(err) = &self.ill_typed {
            return Err(err.clone());
        }
        for term in self
            .left
            .iter()
            .chain(self.op.iter())
            .chain(self.rights.iter())
        {
            ledger
                .term(TermRef::from(term))
                .map_err(|e| ledger_error(e, ConstraintBudget::Bytes))?;
        }
        let right = fold_rights(self.op.as_ref(), &self.rights, lists, ledger)?;
        if self.unsupported {
            return Ok(unsatisfiable_constraint());
        }
        Ok(build_constraint(self.left.clone(), self.op.clone(), right))
    }
}

#[derive(Default)]
struct ConstraintRoots {
    permission: Vec<(String, String)>,
    prohibition: Vec<(String, String)>,
    permission_rules: BTreeSet<Id>,
    prohibition_rules: BTreeSet<Id>,
}

impl ConstraintRoots {
    fn rules_for_kind(&self, kind: &str) -> &BTreeSet<Id> {
        match kind {
            "permission" => &self.permission_rules,
            "prohibition" => &self.prohibition_rules,
            _ => unreachable!("fixed rule kind"),
        }
    }

    fn for_kind(&self, kind: &str) -> &[(String, String)] {
        match kind {
            "permission" => &self.permission,
            "prohibition" => &self.prohibition,
            _ => unreachable!("fixed rule kind"),
        }
    }
}

fn ensure_constraint_roots(
    graph: &Graph,
    lists: &ListTable,
    ledger: &mut Ledger,
) -> Result<(), String> {
    if lists.roots.borrow().is_some() {
        return Ok(());
    }
    let mut permissions = BTreeSet::new();
    let mut prohibitions = BTreeSet::new();
    for (kind, rules) in [
        ("permission", &mut permissions),
        ("prohibition", &mut prohibitions),
    ] {
        scan_property(graph, &format!("{ODRL_NS}{kind}"), ledger, |_, rule| {
            rules.insert(rule);
        })?;
    }
    let mut roots = ConstraintRoots::default();
    scan_property(
        graph,
        &format!("{ODRL_NS}constraint"),
        ledger,
        |rule, constraint| {
            let permission = permissions.contains(&rule);
            let prohibition = prohibitions.contains(&rule);
            if permission || prohibition {
                let pair = (
                    node_key(&graph.dict().term(rule)),
                    node_key(&graph.dict().term(constraint)),
                );
                if permission {
                    roots.permission.push(pair.clone());
                }
                if prohibition {
                    roots.prohibition.push(pair);
                }
            }
        },
    )?;
    roots.permission_rules = permissions;
    roots.prohibition_rules = prohibitions;
    *lists.roots.borrow_mut() = Some(roots);
    Ok(())
}

/// Parse the *atomic* constraints attached via `odrl:constraint` to each rule of
/// `kind`, keyed by rule node. Constraint *nodes* are bound by variable, so
/// blank-node constraints are handled correctly.
///
/// A `?c` that is a compound `odrl:LogicalConstraint` (it carries an
/// `odrl:and`/`odrl:or`/`odrl:xone` refining set rather than a direct
/// `leftOperand`/`operator`/`rightOperand`) is **skipped here** — it is parsed by
/// [`logical_constraints_for`] into the rule's `logical_constraints` instead, so it
/// is *not* mis-read as a structurally-incomplete atomic constraint (which would
/// wrongly fail the whole rule closed). sq-a0zef.
///
/// A multi-valued `odrl:rightOperand` — several objects (`odrl:rightOperand <a>,
/// <b>`) or an RDF list (`odrl:rightOperand ( <a> <b> )`) — is folded into the
/// `|`-separated set encoding via [`fold_rights`] instead of taking the first
/// binding. sq-ueydm.
fn constraints_for(
    graph: &Graph,
    kind: &str,
    lists: &ListTable,
    ledger: &mut Ledger,
) -> Result<BTreeMap<String, Vec<Constraint>>, String> {
    // The predicate scan retains the node-listing query's index order, shared
    // with logical assembly and the policy-wide expansion preflight.
    ensure_constraint_roots(graph, lists, ledger)?;
    let roots = lists.roots.borrow();
    let order = roots.as_ref().expect("loaded above").for_kind(kind);
    ensure_raw_constraints(graph, lists, ledger)?;
    let acc_ref = lists.raw.borrow();
    let acc = acc_ref.as_ref().expect("loaded above");
    let missing = RawConstraint::default();
    let mut out: BTreeMap<String, Vec<Constraint>> = BTreeMap::new();
    for (rkey, ckey) in order {
        let raw = acc.get(ckey).unwrap_or(&missing);
        if !raw.is_logical {
            ledger
                .charge(Dim::Nodes, 1)
                .map_err(|e| ledger_error(e, ConstraintBudget::Nodes))?;
            out.entry(rkey.clone())
                .or_default()
                .push(raw.build(lists, ledger)?);
        }
    }
    Ok(out)
}

fn ensure_raw_constraints(
    graph: &Graph,
    lists: &ListTable,
    ledger: &mut Ledger,
) -> Result<(), String> {
    if lists.raw.borrow().is_none() {
        let mut raw = BTreeMap::new();
        absorb_properties(graph, &mut raw, true, ledger)?;
        *lists.raw.borrow_mut() = Some(raw);
    }
    Ok(())
}

// One scan per property: a node's field multiplicities never join one another.
fn absorb_properties(
    graph: &Graph,
    acc: &mut BTreeMap<String, RawConstraint>,
    logical: bool,
    ledger: &mut Ledger,
) -> Result<(), String> {
    let mut direct_rights = BTreeSet::new();
    for property in [
        "leftOperand",
        "operator",
        "rightOperand",
        "rightOperandReference",
        "unit",
    ] {
        scan_property(graph, &format!("{ODRL_NS}{property}"), ledger, |c, v| {
            let ckey = node_key(&graph.dict().term(c));
            let v = graph.dict().term(v);
            let raw = acc.entry(ckey.clone()).or_default();
            match property {
                "leftOperand" => raw.absorb(Some(v), None, None, false),
                "operator" => raw.absorb(None, Some(v), None, false),
                "rightOperand" => {
                    direct_rights.insert(ckey.clone());
                    raw.absorb(None, None, Some(v), false);
                }
                // The old OPTIONAL reused ?right: a direct operand takes precedence
                // over references. Preserve that behavior when both are authored.
                "rightOperandReference" if !direct_rights.contains(&ckey) => {
                    raw.absorb(None, None, Some(v), false)
                }
                "unit" => raw.unsupported = true,
                _ => {}
            }
        })?;
    }
    if logical {
        for property in ["and", "or", "xone"] {
            scan_property(graph, &format!("{ODRL_NS}{property}"), ledger, |c, _| {
                acc.entry(node_key(&graph.dict().term(c)))
                    .or_default()
                    .is_logical = true;
            })?;
        }
    }
    Ok(())
}

/// Parse the compound `odrl:LogicalConstraint` refinements attached (via
/// `odrl:constraint`) to each rule of `kind`, keyed by rule node. sq-a0zef.
///
/// A parsed `odrl:LogicalConstraint` node's combinator + operand node-keys, before
/// recursive assembly into a [`LogicalConstraint`]. sq-a0zef.
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
/// Separate bulk property queries build node tables for the *whole* policy graph
/// (all combinator edges and atomic constraint fields); the per-rule compound constraints are then
/// assembled recursively in-memory, with cycle protection so a malformed self-/mutually
/// referential `LogicalConstraint` cannot loop (it short-circuits to the unsatisfiable
/// guard — fail-closed).
fn logical_constraints_for(
    graph: &Graph,
    kind: &str,
    lists: &ListTable,
    ledger: &mut Ledger,
) -> Result<BTreeMap<String, Vec<LogicalConstraint>>, String> {
    // (1) Every combinator edge in the graph: (lc-node, combinator, operand-node), in
    // graph order. A node appearing as a subject here is a LogicalConstraint.
    let edges_q = format!(
        "SELECT ?lc ?logop ?operand WHERE {{ \
           ?lc ?logop ?operand . \
           VALUES ?logop {{ <{ODRL_NS}and> <{ODRL_NS}or> <{ODRL_NS}xone> }} \
         }}"
    );
    let edges_res = query_metered(graph, &edges_q, ledger)?;
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
    if lists.atoms.borrow().is_none() {
        ensure_raw_constraints(graph, lists, ledger)?;
        let raw_ref = lists.raw.borrow();
        let atoms = raw_ref
            .as_ref()
            .expect("loaded above")
            .iter()
            // The old atom query required a leftOperand. Other subjects retain
            // their direct-constraint reading or become a missing-atom guard.
            .filter(|(_, raw)| raw.left.is_some())
            .map(|(key, raw)| raw.build(lists, ledger).map(|c| (key.clone(), c)))
            .collect::<Result<_, String>>()?;
        *lists.atoms.borrow_mut() = Some(atoms);
    }
    let atoms_ref = lists.atoms.borrow();
    let atoms = atoms_ref.as_ref().expect("loaded above");

    // (2b) Fold LIST-valued combinator operands (`odrl:or ( <c1> <c2> )`) into their
    // member node keys before assembly; a malformed/degenerate collection REFUSES the
    // whole parse (fail-closed on both rule kinds). HEAD-position rest-only cells come
    // from the pre-loaded `lists.rest_only` set. sq-dkuff.
    fold_list_operands(&mut lc_defs, atoms, lists, ledger)?;

    // (3) Atomic and compound attachments use the same first-seen roots.
    ensure_constraint_roots(graph, lists, ledger)?;
    let roots_ref = lists.roots.borrow();
    let roots = roots_ref.as_ref().expect("loaded above");
    if !lists.expansion_checked.get() {
        preflight_expansions(roots, &lc_defs, ledger)?;
        lists.expansion_checked.set(true);
    }
    let mut out: BTreeMap<String, Vec<LogicalConstraint>> = BTreeMap::new();
    for (rkey, ckey) in roots.for_kind(kind) {
        // Only assemble compound nodes here (atomic direct constraints are handled by
        // constraints_for); a node is compound iff it has a combinator edge.
        if lc_defs.contains_key(ckey) {
            let mut stack = BTreeMap::new();
            // Every rule and compound consumes the same policy-wide ledger.
            let lc = assemble_logical(ckey, &lc_defs, atoms, &mut stack, ledger)?;
            out.entry(rkey.clone()).or_default().push(lc);
        }
    }
    Ok(out)
}

fn preflight_expansions(
    roots: &ConstraintRoots,
    defs: &BTreeMap<String, LcDef>,
    ledger: &Ledger,
) -> Result<(), String> {
    fn size(
        key: &str,
        defs: &BTreeMap<String, LcDef>,
        memo: &mut BTreeMap<String, usize>,
        active: &mut BTreeSet<String>,
    ) -> Option<usize> {
        if active.contains(key) || active.len() > MAX_CONSTRAINT_DEPTH {
            return None;
        }
        if let Some(n) = memo.get(key) {
            return Some(*n);
        }
        active.insert(key.to_owned());
        let def = &defs[key];
        let mut n = def.operands.len();
        if n > MAX_CONSTRAINT_NODES {
            return None;
        }
        for operand in &def.operands {
            if defs.contains_key(operand) {
                n = n.saturating_add(size(operand, defs, memo, active)?);
            }
            if n > MAX_CONSTRAINT_NODES {
                return None;
            }
        }
        active.remove(key);
        memo.insert(key.to_owned(), n);
        Some(n)
    }
    let mut total = 0usize;
    let mut memo = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (rule, key) in roots.permission.iter().chain(&roots.prohibition) {
        if defs.contains_key(key) && seen.insert((rule.clone(), key.clone())) {
            if let Some(n) = size(key, defs, &mut memo, &mut BTreeSet::new()) {
                total = total.saturating_add(n);
            }
        }
    }
    if total > ledger.remaining(Dim::Nodes) {
        return Err(budget_error(ConstraintBudget::Nodes));
    }
    Ok(())
}

/// Fold LIST-valued combinator operands into their member node keys. sq-dkuff.
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
    ledger: &mut Ledger,
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
                let Some(members) = well_formed_list(&okey, lists, ledger)? else {
                    return Err(format!(
                        "a LogicalConstraint combinator has a MALFORMED collection operand \
                         ({okey}: broken/dangling tail, cycle, or forked cell); honouring a \
                         prefix could widen the authored constraint set, so the policy is \
                         refused (fail-closed)"
                    ));
                };
                for member in members.iter() {
                    ledger
                        .term(TermRef::from(member))
                        .map_err(|e| ledger_error(e, ConstraintBudget::Bytes))?;
                    let mkey = node_key(member);
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
/// cycle protection (`active` tracks the ancestor chain) and under `ledger`. A malformed
/// node — not in the combinator table and not a known atomic, or part of a cycle —
/// becomes the unsatisfiable-guard atomic constraint (fail-closed). sq-a0zef.
///
/// # Errors
///
/// Returns `Err` when `ledger` is exhausted — see [`Ledger`]. The whole policy is
/// then refused rather than any one operand degraded, because a budget says nothing about
/// what the authored constraint meant: degrading an operand to the unsatisfiable guard
/// widens a prohibition's compound (the carve-out stops firing), and dropping it widens a
/// permission's `odrl:and`.
fn assemble_logical(
    lc_key: &str,
    lc_defs: &BTreeMap<String, LcDef>,
    atoms: &BTreeMap<String, Constraint>,
    active: &mut BTreeMap<String, ()>,
    ledger: &mut Ledger,
) -> Result<LogicalConstraint, String> {
    let def = lc_defs
        .get(lc_key)
        .expect("assemble_logical called on a non-LC node");
    active.insert(lc_key.to_owned(), ());
    let mut operands = Vec::new();
    ledger
        .reserve(&mut operands, def.operands.len(), Dim::Nodes)
        .map_err(expansion_error)?;
    for okey in &def.operands {
        let node = if lc_defs.contains_key(okey) {
            if active.contains_key(okey) {
                // A cycle in the LogicalConstraint graph — fail closed rather than loop.
                ConstraintNode::Atomic(unsatisfiable_constraint())
            } else {
                ledger
                    .enter()
                    .map_err(|e| ledger_error(e, ConstraintBudget::Depth))?;
                let sub = assemble_logical(okey, lc_defs, atoms, active, ledger);
                ledger.leave();
                ConstraintNode::Compound(sub?)
            }
        } else if let Some(c) = atoms.get(okey) {
            charge_constraint(c, ledger)?;
            ConstraintNode::Atomic(c.clone())
        } else {
            // An operand that is neither a recognised compound nor a well-formed atomic
            // constraint → unsatisfiable guard (fail-closed).
            ConstraintNode::Atomic(unsatisfiable_constraint())
        };
        operands.push(node);
    }
    active.remove(lc_key);
    Ok(LogicalConstraint {
        id: lc_key.to_owned(),
        operator: def.operator,
        operands,
    })
}

/// Nested `odrl:LogicalConstraint` levels [`assemble_logical`] may descend.
///
/// Assembly is recursive, and the combinator graph comes from the policy document, so
/// without a depth bound a chain of n nested compounds costs n stack frames.
///
/// Measured, not guessed: bisected down against every policy the `sparq-policy` and
/// `sparq-solid` test corpora parse, the smallest value that still parses all of them is
/// **1** descent below a rule's outermost compound — an `odrl:or` of `odrl:and`s, the ODRL
/// test suite's 062 shape. The value here is far above that (ODRL policies do not nest
/// combinators deeply) and far below any depth that could exhaust a stack.
const MAX_CONSTRAINT_DEPTH: usize = 32;

/// Legacy per-compound operand allowance used to size the policy-wide limit.
///
/// `active` tracks only the ancestor chain, so a node reachable by several paths through
/// an ACYCLIC combinator graph is assembled once per path: a "diamond" document whose
/// every level has two operands pointing at the same next level describes 2^n operand
/// nodes in O(n) triples, and the assembled tree is that size. Bounding the total number
/// of nodes expanded caps that at a size independent of how the document is shaped.
///
/// Measured, not guessed: bisected the same way as [`MAX_CONSTRAINT_DEPTH`], the smallest
/// value that still parses every policy in the `sparq-policy` and `sparq-solid` test
/// corpora is **6** nodes. The value here is far above that so no plausibly-authored
/// policy meets it. Assembly now charges the policy-wide ledger, and this value
/// supplies its compatibility sizing and the expansion preflight's legacy threshold.
const MAX_CONSTRAINT_NODES: usize = 1_024;

fn budget_error(kind: ConstraintBudget) -> String {
    format!(
        "{} {}{}]: the policy exceeds its configured work allowance",
        CONSTRAINT_BUDGET_REFUSAL,
        CONSTRAINT_BUDGET_TAG,
        kind.tag()
    )
}

fn ledger_error(e: Exceeded, allocating: ConstraintBudget) -> String {
    budget_error(match e.dim {
        Dim::Depth | Dim::TermDepth => ConstraintBudget::Depth,
        Dim::Nodes => ConstraintBudget::Nodes,
        Dim::Bytes | Dim::DocumentBytes | Dim::TermBytes | Dim::SourceBytes => {
            ConstraintBudget::Bytes
        }
        Dim::QueryRows => ConstraintBudget::Rows,
        Dim::Alloc => allocating,
        _ => ConstraintBudget::Unspecified,
    })
}

fn expansion_error(e: Exceeded) -> String {
    if matches!(e.dim, Dim::Nodes | Dim::Alloc) {
        budget_error(ConstraintBudget::Expansion)
    } else {
        ledger_error(e, ConstraintBudget::Expansion)
    }
}

fn charge_constraint(c: &Constraint, ledger: &mut Ledger) -> Result<(), String> {
    // Size the owned model strings before cloning the atom into an expanded tree.
    ledger
        .term(TermRef::Iri(&c.left))
        .map_err(|e| ledger_error(e, ConstraintBudget::Bytes))?;
    match &c.right {
        Value::Iri(v) | Value::Str(v) | Value::DateTime(v) => {
            ledger
                .term(TermRef::Iri(v))
                .map_err(|e| ledger_error(e, ConstraintBudget::Bytes))?;
        }
        Value::Num(_) => {}
    }
    Ok(())
}

/// Which assembly budget a policy parse was refused by ([`constraint_budget_refusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConstraintBudget {
    /// Compound descent or nesting of RDF triple terms.
    Depth,
    /// Compatible tag for nodes refused while expanding a compound.
    Expansion,
    /// Policy-wide list cells or planned operand nodes.
    Nodes,
    /// Term byte allowances, including per-term and document caps.
    Bytes,
    /// Extraction result rows or query intermediate row limits.
    Rows,
    /// A budget this build does not name individually.
    Unspecified,
}

impl ConstraintBudget {
    /// The tag this kind is carried as.
    fn tag(self) -> &'static str {
        match self {
            ConstraintBudget::Depth => "constraint-depth",
            ConstraintBudget::Expansion => "constraint-expansion",
            ConstraintBudget::Nodes => "constraint-nodes",
            ConstraintBudget::Bytes => "policy-bytes",
            ConstraintBudget::Rows => "policy-query-rows",
            ConstraintBudget::Unspecified => "unspecified",
        }
    }

    /// The kind a tag stands for.
    fn from_tag(tag: &str) -> ConstraintBudget {
        for kind in [
            ConstraintBudget::Depth,
            ConstraintBudget::Expansion,
            ConstraintBudget::Nodes,
            ConstraintBudget::Bytes,
            ConstraintBudget::Rows,
        ] {
            if kind.tag() == tag {
                return kind;
            }
        }
        ConstraintBudget::Unspecified
    }
}

/// The text a policy refused by its constraint-assembly budget puts in its error — the one
/// refusal that says "this document's compound constraints are too big to assemble", not
/// "this document is not an ODRL policy".
///
/// Read it with [`constraint_budget_refusal`] rather than by matching this text.
pub const CONSTRAINT_BUDGET_REFUSAL: &str = "ODRL constraint-assembly budget exceeded";

/// The opening of the kind tag every such refusal carries; the kind's own tag and a `]`
/// follow.
const CONSTRAINT_BUDGET_TAG: &str = "[budget=";

/// Which constraint-assembly budget `err` was refused by, or `None` when `err` is any
/// other parse error.
///
/// `err` is an error from [`parse_policy`], [`parse_policy_str`] or
/// [`parse_policy_str_with_memberships`], including one a caller wrapped in its own
/// sentence: the kind tag is read wherever it appears in the message.
///
/// # Examples
///
/// ```
/// assert_eq!(sparq_policy::constraint_budget_refusal("some other parse error"), None);
/// ```
pub fn constraint_budget_refusal(err: &str) -> Option<ConstraintBudget> {
    let at = err.find(CONSTRAINT_BUDGET_TAG)? + CONSTRAINT_BUDGET_TAG.len();
    let rest = &err[at..];
    let end = rest.find(']')?;
    Some(ConstraintBudget::from_tag(&rest[..end]))
}

/// A literal set member that is not a plain, `xsd:string` or language-tagged string.
fn typed_member(t: &Term) -> bool {
    let Term::Literal(l) = t else { return false };
    l.language().is_none() && l.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#string"
}

/// The unsatisfiable-guard atomic constraint used for a malformed/unknown operand
/// (a constraint that can never be satisfied — fail-closed). Shared by [`build_constraint`]
/// and the compound-operand assembler. sq-a0zef.
fn unsatisfiable_constraint() -> Constraint {
    Constraint {
        left: MALFORMED.to_owned(),
        operator: Operator::Neq,
        right: Value::Iri(MALFORMED.to_owned()),
    }
}

pub(crate) const MALFORMED: &str = "urn:sparq-policy:malformed";

pub(crate) fn is_unsatisfiable_guard(c: &Constraint) -> bool {
    c.left == MALFORMED
}

pub(crate) fn node_has_guard(n: &ConstraintNode) -> bool {
    match n {
        ConstraintNode::Atomic(c) => is_unsatisfiable_guard(c),
        ConstraintNode::Compound(lc) => lc.operands.iter().any(node_has_guard),
    }
}

/// `odrl:refinement` on a rule's action, target or assignee is not supported: the rule
/// would be read without it, so a refined permission grants more than authored. And a
/// prohibition whose action, target or assignee is a blank node (a refined action, an
/// anonymous collection) can never match a request, so it never fires and a sibling
/// permission grants. Both shapes refuse the whole policy (fail-closed).
fn refuse_unsupported_rule_heads(
    graph: &Graph,
    lists: &ListTable,
    ledger: &mut Ledger,
) -> Result<(), String> {
    ensure_constraint_roots(graph, lists, ledger)?;
    let roots_ref = lists.roots.borrow();
    let roots = roots_ref.as_ref().expect("loaded above");
    let mut refined = BTreeSet::new();
    scan_property(graph, &format!("{ODRL_NS}refinement"), ledger, |s, _| {
        refined.insert(s);
    })?;
    let mut refined_head = None;
    let mut blank_prohibition = None;
    for property in ["action", "target", "assignee"] {
        scan_property(graph, &format!("{ODRL_NS}{property}"), ledger, |s, o| {
            let prohibition = roots.prohibition_rules.contains(&s);
            if refined_head.is_none()
                && refined.contains(&o)
                && (prohibition || roots.permission_rules.contains(&s))
            {
                refined_head = Some(o);
            }
            if blank_prohibition.is_none()
                && prohibition
                && matches!(graph.dict().term(o), Term::BlankNode(_))
            {
                blank_prohibition = Some(s);
            }
        })?;
    }
    if let Some(n) = refined_head {
        let n = node_key(&graph.dict().term(n));
        return Err(format!(
            "a rule's action, target or assignee ({n}) carries an odrl:refinement, which is \
             not supported; reading the rule without it would widen what it grants or \
             disable what it forbids, so the policy is refused (fail-closed)"
        ));
    }
    if let Some(r) = blank_prohibition {
        let r = node_key(&graph.dict().term(r));
        return Err(format!(
            "prohibition {r} has a blank-node action, target or assignee, which can never \
             match a request; the prohibition would never fire and a sibling permission \
             would grant, so the policy is refused (fail-closed)"
        ));
    }
    Ok(())
}

/// Build a [`Constraint`], turning anything malformed/unknown into an
/// unsatisfiable guard (fail-closed): the enclosing rule can then never match.
/// `right` is the already-folded value (see [`fold_rights`]): `None` covers both
/// a missing right operand and an unfoldable multi-value. sq-ueydm.
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
fn duties_for(
    graph: &Graph,
    kind: &str,
    ledger: &mut Ledger,
) -> Result<BTreeMap<String, Vec<Duty>>, String> {
    let actions = group_by_first(
        graph,
        &format!("SELECT ?d ?a WHERE {{ ?d <{ODRL_NS}action> ?a }}"),
        ledger,
    )?;
    let constrained = group_by_first(
        graph,
        &format!("SELECT ?d ?c WHERE {{ ?d <{ODRL_NS}constraint> ?c }}"),
        ledger,
    )?;
    let q = format!("SELECT ?rule ?d WHERE {{ ?rule <{ODRL_NS}duty> ?d . FILTER EXISTS {{ ?policy <{ODRL_NS}{kind}> ?rule }} }}");
    let res = query_metered(graph, &q, ledger)?;
    let mut out: BTreeMap<String, Vec<Duty>> = BTreeMap::new();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for row in res.rows {
        let mut it = row.into_iter();
        let rule_t = it.next().flatten();
        let d_t = it.next().flatten();

        let (Some(rule_t), Some(d_t)) = (rule_t, d_t) else {
            continue;
        };
        let rkey = node_key(&rule_t);
        let dkey = format!("{rkey}|{}", node_key(&d_t));
        if seen.insert(dkey, ()).is_some() {
            continue;
        }
        let key = node_key(&d_t);
        let action = first_str(&actions, &key)
            .map(Action)
            .unwrap_or_else(Action::use_);
        out.entry(rkey).or_default().push(Duty {
            id: term_str(&d_t),
            action,
            constraints: if constrained.contains_key(&key) {
                vec![unsatisfiable_constraint()]
            } else {
                Vec::new()
            },
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

#[cfg(test)]
mod metered_tests {
    use super::*;

    #[test]
    fn indexed_scan_charges_candidates_before_visiting() {
        let graph = Graph::load_str(
            "<urn:a> <urn:p> <urn:x> . <urn:b> <urn:p> <urn:y> . <urn:c> <urn:other> <urn:z> .",
            "turtle",
        )
        .unwrap();
        let mut limits = Limits::default();
        limits.query_rows = 1;
        let mut ledger = Ledger::new(limits);
        let mut visited = Vec::new();
        let err =
            scan_property(&graph, "urn:p", &mut ledger, |s, o| visited.push((s, o))).unwrap_err();
        assert_eq!(
            constraint_budget_refusal(&err),
            Some(ConstraintBudget::Rows)
        );
        assert!(visited.is_empty(), "charge before processing any candidate");
        assert_eq!(ledger.spent(Dim::QueryRows), 0);

        limits.query_rows = 2;
        let mut ledger = Ledger::new(limits);
        scan_property(&graph, "urn:p", &mut ledger, |s, o| visited.push((s, o))).unwrap();
        assert_eq!(ledger.spent(Dim::QueryRows), 2);
        assert_eq!(visited.len(), 2);
    }

    #[test]
    fn indexed_roots_preserve_query_order_and_share_scans() {
        let graph = Graph::load_str(
            &format!("@prefix o: <{ODRL_NS}> .
                <urn:p> o:permission <urn:shared>, _:permission ; o:prohibition <urn:shared>, _:prohibition .
                <urn:other> o:permission <urn:shared> .
                <urn:shared> o:constraint <urn:z>, <urn:a> .
                _:permission o:constraint _:c . _:prohibition o:constraint <urn:z> .
                <urn:unattached> o:constraint <urn:ignored> ; o:action o:read .
                <urn:shared> o:action o:read, o:use ; o:target <urn:z>, <urn:a> ;
                    o:assignee _:party, <urn:agent> ; o:assigner <urn:owner> .
                _:permission o:action o:write ; o:target <urn:literal-target> ."),
            "turtle",
        ).unwrap();
        let mut ledger = Ledger::new(Limits::default());
        let lists = rdf_list_table(&graph, &mut ledger).unwrap();
        ensure_constraint_roots(&graph, &lists, &mut ledger).unwrap();
        let spent = ledger.spent(Dim::QueryRows);
        ensure_constraint_roots(&graph, &lists, &mut ledger).unwrap();
        assert_eq!(
            ledger.spent(Dim::QueryRows),
            spent,
            "reuse the policy's roots"
        );
        let roots = lists.roots.borrow();
        for kind in ["permission", "prohibition"] {
            let query = format!("SELECT ?rule ?c WHERE {{ ?rule <{ODRL_NS}constraint> ?c . FILTER EXISTS {{ ?policy <{ODRL_NS}{kind}> ?rule }} }}");
            let rows = query_metered(&graph, &query, &mut Ledger::new(Limits::default())).unwrap();
            let expected: Vec<_> = rows
                .rows
                .iter()
                .map(|row| {
                    (
                        node_key(row[0].as_ref().unwrap()),
                        node_key(row[1].as_ref().unwrap()),
                    )
                })
                .collect();
            let roots = roots.as_ref().unwrap();
            assert_eq!(roots.for_kind(kind), expected);
            for property in ["action", "target", "assignee", "assigner"] {
                let query = format!("SELECT ?rule ?value WHERE {{ ?rule <{ODRL_NS}{property}> ?value . FILTER EXISTS {{ ?policy <{ODRL_NS}{kind}> ?rule }} }}");
                let expected =
                    group_by_first(&graph, &query, &mut Ledger::new(Limits::default())).unwrap();
                let actual =
                    rule_attributes(&graph, roots.rules_for_kind(kind), property, &mut ledger)
                        .unwrap();
                assert_eq!(actual, expected, "{kind} {property}");
            }
        }
    }

    #[test]
    fn indexed_rule_heads_preserve_refusals_and_membership() {
        let check = |rdf: &str| {
            let graph =
                Graph::load_str(&format!("@prefix o: <{ODRL_NS}> . {rdf}"), "turtle").unwrap();
            let mut ledger = Ledger::new(Limits::default());
            let lists = rdf_list_table(&graph, &mut ledger).unwrap();
            refuse_unsupported_rule_heads(&graph, &lists, &mut ledger)
        };
        for kind in ["permission", "prohibition"] {
            for property in ["action", "target", "assignee"] {
                let rdf = format!("<urn:p> o:{kind} <urn:r> . <urn:r> o:{property} <urn:head> . <urn:head> o:refinement <urn:c> .");
                let err = check(&rdf).unwrap_err();
                assert!(err.contains("carries an odrl:refinement"), "{err}");
                let rdf = format!("<urn:p> o:{kind} <urn:r> . <urn:r> o:{property} _:head .");
                let result = check(&rdf);
                if kind == "prohibition" {
                    assert!(result.unwrap_err().contains("blank-node"));
                } else {
                    assert!(result.is_ok());
                }
            }
        }
        assert!(check(
            "<urn:p> o:permission <urn:r> . <urn:r> o:action o:read .
            <urn:unattached> o:action _:head . _:head o:refinement <urn:c> ."
        )
        .is_ok());
        // A later refinement retains priority over an earlier blank head.
        let err = check(
            "<urn:p> o:prohibition <urn:r> . <urn:r> o:action _:head ;
            o:target <urn:target> . <urn:target> o:refinement <urn:c> .",
        )
        .unwrap_err();
        assert!(err.contains("carries an odrl:refinement"), "{err}");
    }

    #[test]
    fn query_cap_bounds_intermediate_rows_before_final_charge() {
        let graph = Graph::load_str(
            "<urn:a> <urn:p> <urn:x> . <urn:b> <urn:p> <urn:y> .",
            "turtle",
        )
        .unwrap();
        let mut limits = Limits::default();
        limits.query_rows = 1;
        let mut ledger = Ledger::new(limits);
        let err = query_metered(
            &graph,
            "SELECT ?s WHERE { VALUES ?s { <urn:a> <urn:b> } FILTER(STR(?s) = 'urn:a') }",
            &mut ledger,
        )
        .unwrap_err();
        assert_eq!(
            constraint_budget_refusal(&err),
            Some(ConstraintBudget::Rows)
        );
        assert_eq!(ledger.spent(Dim::QueryRows), 0);
    }

    #[test]
    fn shared_list_walk_is_cached_across_constraints() {
        let rdf = format!("@prefix o: <{ODRL_NS}> . <urn:list> <{RDF_NS}first> <urn:v> ; <{RDF_NS}rest> <{RDF_NS}nil> .");
        let graph = Graph::load_str(&rdf, "turtle").unwrap();
        let mut l = Ledger::new(Limits::default());
        let lists = rdf_list_table(&graph, &mut l).unwrap();
        let head: Term = oxrdf::NamedNode::new_unchecked("urn:list").into();
        let op: Term = oxrdf::NamedNode::new_unchecked(format!("{ODRL_NS}isAnyOf")).into();
        for _ in 0..100 {
            let value =
                fold_rights(Some(&op), std::slice::from_ref(&head), &lists, &mut l).unwrap();
            assert_eq!(value, Some(Value::Iri("urn:v".into())));
        }
        assert_eq!(lists.walks.get(), 1);
        assert_eq!(l.spent(Dim::Nodes), 1);
    }

    #[test]
    fn cached_list_members_still_consume_copy_bytes() {
        let graph = Graph::load_str(
            &format!("<urn:list> <{RDF_NS}first> <urn:v> ; <{RDF_NS}rest> <{RDF_NS}nil> ."),
            "turtle",
        )
        .unwrap();
        let mut l = Ledger::new(Limits::default());
        let lists = rdf_list_table(&graph, &mut l).unwrap();
        let head = oxrdf::NamedNode::new_unchecked("urn:list").into();
        let rights = std::slice::from_ref(&head);
        assert_eq!(
            fold_rights(None, rights, &lists, &mut l).unwrap(),
            Some(Value::Iri("urn:v".into()))
        );
        let mut limits = Limits::default();
        limits.bytes = l.spent(Dim::Bytes);
        let mut capped = Ledger::new(limits);
        capped.charge(Dim::Bytes, l.spent(Dim::Bytes)).unwrap();
        let err = fold_rights(None, rights, &lists, &mut capped).unwrap_err();
        assert_eq!(
            constraint_budget_refusal(&err),
            Some(ConstraintBudget::Bytes)
        );
        assert_eq!(lists.walks.get(), 1);
    }

    #[test]
    fn wide_compound_refuses_before_allocating_operands() {
        let mut defs = BTreeMap::new();
        defs.insert(
            "root".into(),
            LcDef {
                operator: LogicalOperator::And,
                operands: (0..1_000_000).map(|i| format!("atom{i}")).collect(),
                seen: BTreeMap::new(),
            },
        );
        let mut limits = Limits::default();
        limits.nodes = MAX_CONSTRAINT_NODES;
        let mut l = Ledger::new(limits);
        let err = assemble_logical(
            "root",
            &defs,
            &BTreeMap::new(),
            &mut BTreeMap::new(),
            &mut l,
        )
        .unwrap_err();
        assert_eq!(
            constraint_budget_refusal(&err),
            Some(ConstraintBudget::Expansion)
        );
        assert_eq!(l.spent(Dim::Nodes), 0);
    }

    #[test]
    fn failed_compound_restores_entered_depth() {
        let defs = BTreeMap::from([
            (
                "root".into(),
                LcDef {
                    operator: LogicalOperator::And,
                    operands: vec!["sub".into()],
                    seen: BTreeMap::new(),
                },
            ),
            (
                "sub".into(),
                LcDef {
                    operator: LogicalOperator::And,
                    operands: vec!["atom".into()],
                    seen: BTreeMap::new(),
                },
            ),
        ]);
        let atoms = BTreeMap::from([(
            "atom".into(),
            Constraint {
                left: "urn:left".into(),
                operator: Operator::Eq,
                right: Value::Iri("urn:right".into()),
            },
        )]);
        for (depth, bytes, expected) in [
            (32, 0, ConstraintBudget::Bytes),
            (0, usize::MAX, ConstraintBudget::Depth),
        ] {
            let mut limits = Limits::default();
            limits.depth = depth;
            limits.bytes = bytes;
            let mut l = Ledger::new(limits);
            let err =
                assemble_logical("root", &defs, &atoms, &mut BTreeMap::new(), &mut l).unwrap_err();
            assert_eq!(constraint_budget_refusal(&err), Some(expected));
            assert_eq!(l.spent(Dim::Depth), 0);
        }
    }
}
