//! [OPUS-4.8] sq-xor3: the WRITE/update path enforcement (design doc §4.4 / §7 item 6).
//!
//! The read path ([`crate::PodStore::query_as`]) restricts a query to the session's
//! `auth:read` graph set. This module is its write-path mirror: before a SPARQL Update
//! (`INSERT`/`DELETE`/`DELETE…INSERT…WHERE`, `CLEAR`/`DROP`/`CREATE`/`LOAD`) is allowed
//! to mutate the store, every graph it could write is checked against the actor's
//! WAC/ACP **write** permission — `acl:Write` (→ [`Mode::Write`]) for a delete/clear and
//! `acl:Write` OR `acl:Append` (→ [`Mode::Write`] / [`Mode::Append`]) for a pure insert.
//! The permission model is exactly the read path's: the same `∪ allow ∖ ∪ deny`
//! per-mode graph sets from the materialized auth view ([`crate::AuthIndex::accessible`]).
//!
//! `.acl`/`.acr` documents need no special mode here: the WAC/ACP rules already
//! translate `acl:Control` on a resource into `auth:write` on its `.acl`/`.acr` graph
//! (design doc §3.3 — "Control ⇒ read+write of the ACL graph itself"), so requiring
//! `Write` on the `.acl`/`.acr` graph IS requiring Control on the resource — enforced
//! through exactly the same auth view, no Solid-specific branch. Writing one DOES,
//! however, invalidate the materialized view (the rules changed), so it triggers
//! re-materialization on success.
//!
//! Fail-closed, like the read path:
//!
//! - the **default graph** is never writable (pod data never lives there — design doc
//!   §2.1); any default-graph target is denied;
//! - a write to a graph the actor cannot write in the required mode is denied and the
//!   store is **not** mutated (the check runs entirely before [`update_in_place`]);
//! - a `DELETE/INSERT … WHERE` template with a `GRAPH ?var` slot is resolved
//!   *precisely* ([OPUS-4.8] sq-biss, extended to `USING`/`WITH` by sq-cnor): the
//!   operation's WHERE pattern is evaluated to enumerate the CONCRETE graphs `?var`
//!   actually binds to, and write is required only on THOSE graphs (per the read path's
//!   per-graph model) — not on every store graph. This stays fail-closed: the WHERE is
//!   evaluated against **exactly the dataset the engine will instantiate the templates
//!   over** — the full store, or, when the operation carries a `USING (NAMED)` / `WITH`
//!   clause, the same active dataset the engine's `build_using` assembles, re-expressed
//!   as an explicit `FROM`/`FROM NAMED` clause (see the soundness note on
//!   [`resolve_var_graphs`]) — so the graphs checked are exactly the graphs the apply
//!   could touch; if any binding is not a writable named graph the update is denied, and
//!   any binding that cannot be reduced to a concrete named graph (a blank-node graph
//!   name, or a WHERE the analysis cannot evaluate) falls back to the conservative
//!   all-graphs check below (never a hole);
//! - a target whose graph name cannot be determined statically by the above — a
//!   `CLEAR`/`DROP` of `ALL`/`NAMED` graphs, or a `GRAPH ?var` slot that fell back —
//!   is treated *conservatively*: the actor must be able to write **every** named graph
//!   currently in the store, or the whole update is denied. This is sound (it can only
//!   reject an update the precise analysis might have allowed), never permissive.
//!
//! After a permitted update that touched an `.acl`/`.acr`/group document, the auth view
//! is automatically re-materialized (epoch bump → session cache dropped), so a changed
//! rule takes effect on the next query/update — the design doc's
//! "after any acl/acr/group-doc write, re-materialize" requirement (§4.4).
//!
//! `.acl`/`.acr` documents are recognized by naming convention. A **group document** is
//! not: `https://pod.ex/groups` is an ordinary resource IRI. It is recognized instead by
//! REFERENCE — the store hands [`check`] the set of graphs the current access-control
//! documents name via `acl:agentGroup` ([`crate::loader::referenced_group_docs`]), and a
//! write to any of them re-materializes just as an `.acl` write does. Before that set was
//! threaded through, a statically-targeted `INSERT DATA`/`DELETE DATA` on a group document
//! silently left the auth view stale: a removed `vcard:hasMember` kept granting access
//! until something else forced a re-materialization.

use crate::loader::{ACL_SUFFIX, ACR_SUFFIX};
use crate::{AuthIndex, Mode, Session};
use oxrdf::{NamedNode, Term, Variable};
use rustc_hash::FxHashSet;
use sparq_engine::QueryBudget;
use spargebra::algebra::{GraphPattern, GraphTarget, QueryDataset};
use spargebra::term::{GraphName, GraphNamePattern};
use spargebra::{GraphUpdateOperation, Query, SparqlParser, Update};

/// The write permission a single graph target requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    /// A pure-insert touch: satisfied by `acl:Write` OR `acl:Append` (WAC: Append adds
    /// without removing). Maps to "graph ∈ accessible(Write) ∪ accessible(Append)".
    WriteOrAppend,
    /// A delete / clear / drop: requires `acl:Write` (Append cannot remove). Writing an
    /// `.acl`/`.acr` graph is also a `Write` need — but the auth view only ever grants
    /// `auth:write` on those graphs to `acl:Control` holders, so this is Control-gated
    /// without a special case (see the module docs).
    Write,
}

/// Why [`resolve_var_graphs`] could not produce a precise per-graph requirement set.
/// [FABLE-5] sq-yhlf0.
enum Bail {
    /// Fall back to the conservative all-graphs wildcard at this [`Need`] — the historic
    /// behaviour for an un-evaluable WHERE or a blank-node graph binding. Sound: a superset
    /// of the real write set can only deny, never leak.
    Wildcard(Need),
    /// The binding SELECT exhausted the caller's [`QueryBudget`], carrying the engine's
    /// `"query budget exceeded (…)"` message. NOT a wildcard fallback: the wildcard is the
    /// *cheap* check, so falling back could permit an update whose apply must then re-run
    /// the very WHERE that just ran out of budget. Surfaced as a deny instead — fail-closed
    /// and honest about which limit tripped.
    Budget(String),
}

/// Whether `e` is the engine's cooperative-budget abort (`"query budget exceeded (timeout)"`
/// / `(max-rows)` / `(max-bytes)` / `(cancelled)`) rather than an ordinary evaluation error.
/// String-matched because the engine reports errors as `String`; this mirrors how
/// `sparq-server`'s HTTP layer maps the same aborts to status codes. [FABLE-5] sq-yhlf0.
fn is_budget_error(e: &str) -> bool {
    e.contains("query budget exceeded")
}

/// A `DELETE/INSERT … WHERE` operation with at least one `GRAPH ?var` template slot,
/// captured so [`resolve_var_graphs`] can evaluate the WHERE and turn the variable slots
/// into concrete per-solution graph targets. [OPUS-4.8] sq-biss.
#[derive(Debug)]
struct VarGraphResolve {
    /// The operation's WHERE pattern (what determines the bindings).
    pattern: GraphPattern,
    /// The operation's `USING` dataset, if any (re-scopes WHERE evaluation just as the
    /// engine's apply does — keep the two in lock-step or the checked set diverges from
    /// the written set).
    using: Option<QueryDataset>,
    /// The graph-slot variables and the base need each carries (delete slots → `Write`,
    /// insert slots → `WriteOrAppend`). The same variable may appear with both needs
    /// (deleted and inserted under the same name); both are recorded.
    slots: Vec<(Variable, Need)>,
}

/// What a parsed update needs in order to be permitted.
#[derive(Debug, Default)]
struct WriteReqs {
    /// Concrete (named-graph, need) requirements gathered from static targets.
    graphs: Vec<(NamedNode, Need)>,
    /// The update targets the default graph (always denied — pod data is never there).
    touches_default: bool,
    /// `DELETE/INSERT … WHERE` operations carrying a `GRAPH ?var` template slot, resolved
    /// precisely in [`check`] before the wildcard fallback. [OPUS-4.8] sq-biss.
    var_graphs: Vec<VarGraphResolve>,
    /// The update has a target whose graph cannot be determined statically (a
    /// `CLEAR`/`DROP` `ALL`/`NAMED`, or a `GRAPH ?var` slot whose precise resolution fell
    /// back): the actor must be able to write every named graph in the store, in this
    /// mode, or the update is denied.
    wildcard: Option<Need>,
    /// The update is known to touch an auth-view input (an `.acl`/`.acr` or group
    /// document) — a successful update sets this so the caller re-materializes. Raised by
    /// [`check`] from the resolved target set and by any wildcard/variable-graph target.
    rematerialize_hint: bool,
}

/// Whether `iri` is an access-control document (`.acl`/`.acr`) by naming convention.
fn is_control_graph(iri: &str) -> bool {
    iri.ends_with(ACL_SUFFIX) || iri.ends_with(ACR_SUFFIX)
}

/// Whether writing `iri` should trigger re-materialization — i.e. whether `iri` is an
/// INPUT of the materialized auth view. Two kinds:
///
/// - `.acl`/`.acr` documents (the rules themselves), recognized by naming convention;
/// - **group documents**, recognized by REFERENCE: a graph some access-control document
///   names via `acl:agentGroup` ([`crate::loader::referenced_group_docs`]). A group
///   document has no naming convention — `https://pod.ex/groups` looks like any other
///   resource — so the only sound way to spot one is to ask what the current
///   authorization documents point at. Adding or removing a `vcard:hasMember` triple
///   there changes who holds a grant, so it must re-materialize exactly as an `.acl`
///   write does.
///
/// A wildcard update re-materializes unconditionally (it could touch either kind), and a
/// group document that becomes referenced only by a LATER `.acl` write is covered by that
/// `.acl` write's own re-materialization.
fn affects_auth_view(iri: &str, group_docs: &FxHashSet<String>) -> bool {
    is_control_graph(iri) || group_docs.contains(iri)
}

/// The need for a static named-graph target. Writing an `.acl`/`.acr` graph always needs
/// `Write` (Control-gated via the auth view, never satisfiable by an `Append` grant —
/// the rules grant Control-holders `auth:write` on the graph, not `auth:append`).
fn need_for_graph(iri: &str, base: Need) -> Need {
    if is_control_graph(iri) {
        Need::Write
    } else {
        base
    }
}

/// Record a static named-graph target with the appropriate need. Whether the target is an
/// auth-view input is decided later, in [`check`], over the FULL `reqs.graphs` set (which
/// by then also holds the precisely-resolved `GRAPH ?var` targets) — `analyze` has no
/// access to the store's referenced-group-document set.
fn push_graph(reqs: &mut WriteReqs, n: &NamedNode, base: Need) {
    let need = need_for_graph(n.as_str(), base);
    reqs.graphs.push((n.clone(), need));
}

/// Record the target of a data quad / template graph-name slot.
fn push_graph_name(reqs: &mut WriteReqs, g: &GraphName, base: Need) {
    match g {
        GraphName::DefaultGraph => reqs.touches_default = true,
        GraphName::NamedNode(n) => push_graph(reqs, n, base),
    }
}

/// Record the target of a `DELETE`/`INSERT` template quad slot. A concrete named-node or
/// default-graph slot is recorded immediately; a `GRAPH ?var` slot is collected into
/// `var_slots` so the enclosing operation can resolve it precisely against its WHERE
/// ([OPUS-4.8] sq-biss). The strongest need for any one variable wins.
fn push_graph_name_pattern(
    reqs: &mut WriteReqs,
    var_slots: &mut Vec<(Variable, Need)>,
    g: &GraphNamePattern,
    base: Need,
) {
    match g {
        GraphNamePattern::DefaultGraph => reqs.touches_default = true,
        GraphNamePattern::NamedNode(n) => push_graph(reqs, n, base),
        GraphNamePattern::Variable(v) => {
            // Don't escalate to the wildcard yet: collect the (var, need) so the
            // operation's WHERE can be evaluated and the concrete bound graphs checked.
            match var_slots.iter_mut().find(|(x, _)| x == v) {
                Some((_, n)) => *n = strongest(*n, base),
                None => var_slots.push((v.clone(), base)),
            }
        }
    }
}

/// Escalate to the wildcard requirement (the strongest seen wins: Control > Write >
/// WriteOrAppend).
fn raise_wildcard(reqs: &mut WriteReqs, need: Need) {
    reqs.rematerialize_hint = true; // a wildcard could touch a control doc
    reqs.wildcard = Some(match reqs.wildcard {
        None => need,
        Some(prev) => strongest(prev, need),
    });
}

fn strongest(a: Need, b: Need) -> Need {
    fn rank(n: Need) -> u8 {
        match n {
            Need::WriteOrAppend => 0,
            Need::Write => 1,
        }
    }
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

/// A `CLEAR`/`DROP` graph target needs `Write` (it removes triples).
fn push_clear_drop_target(reqs: &mut WriteReqs, target: &GraphTarget) {
    match target {
        GraphTarget::DefaultGraph => reqs.touches_default = true,
        GraphTarget::NamedNode(n) => push_graph(reqs, n, Need::Write),
        // NAMED / ALL touch every (named) graph — conservative wildcard at Write.
        GraphTarget::NamedGraphs | GraphTarget::AllGraphs => raise_wildcard(reqs, Need::Write),
    }
}

/// Walk a parsed update and collect everything it needs permission to write.
fn analyze(upd: &Update) -> WriteReqs {
    let mut reqs = WriteReqs::default();
    for op in &upd.operations {
        match op {
            GraphUpdateOperation::InsertData { data } => {
                for q in data {
                    push_graph_name(&mut reqs, &q.graph_name, Need::WriteOrAppend);
                }
            }
            GraphUpdateOperation::DeleteData { data } => {
                for q in data {
                    push_graph_name(&mut reqs, &q.graph_name, Need::Write);
                }
            }
            GraphUpdateOperation::DeleteInsert { delete, insert, using, pattern } => {
                // Deletes always need Write; inserts need Write-or-Append. The WHERE
                // pattern only READS, so it is not a write target. A `GRAPH ?var` slot is
                // collected into `var_slots` and resolved precisely against this
                // operation's own WHERE/USING (not the whole-update conservative
                // wildcard). [OPUS-4.8] sq-biss.
                let mut var_slots: Vec<(Variable, Need)> = Vec::new();
                for d in delete {
                    push_graph_name_pattern(&mut reqs, &mut var_slots, &d.graph_name, Need::Write);
                }
                for i in insert {
                    push_graph_name_pattern(&mut reqs, &mut var_slots, &i.graph_name, Need::WriteOrAppend);
                }
                if !var_slots.is_empty() {
                    reqs.var_graphs.push(VarGraphResolve {
                        pattern: (**pattern).clone(),
                        using: using.clone(),
                        slots: var_slots,
                    });
                }
            }
            GraphUpdateOperation::Load { destination, .. } => {
                push_graph_name(&mut reqs, destination, Need::WriteOrAppend);
            }
            GraphUpdateOperation::Clear { graph, .. } | GraphUpdateOperation::Drop { graph, .. } => {
                push_clear_drop_target(&mut reqs, graph);
            }
            GraphUpdateOperation::Create { graph, .. } => {
                // Creating a named graph entry is a write to that graph.
                push_graph(&mut reqs, graph, Need::Write);
            }
        }
    }
    reqs
}

/// Re-express an update's `USING (NAMED)` / `WITH` re-scope as a query `FROM`/`FROM NAMED`
/// dataset clause that makes the query side's [`sparq_engine::query`] assemble the SAME
/// active dataset the engine's apply builds via `Dataset::build_using`. [OPUS-4.8] sq-cnor.
///
/// `build_using` and the query-side `build_active` treat the `default` (FROM) graphs
/// identically (merge the listed store graphs; an absent one contributes nothing). They
/// diverge only on `named`:
///
/// - `Some(list)` (explicit `USING NAMED`): both keep exactly `list`. We pass it through
///   unchanged — an absent name denotes the empty graph on the query side and contributes
///   no binding under `build_using` either, so the `GRAPH ?g` enumeration is identical.
/// - `None` (the parser's encoding of `WITH`, which re-scopes only the default graph):
///   `build_using` keeps ALL the store's named graphs, but a query's empty `FROM NAMED`
///   keeps NONE. We therefore materialize `named` as an explicit `FROM NAMED` of every
///   store named graph — **including the reserved [`AUTH_GRAPH`](crate::AUTH_GRAPH) view**.
///   This is load-bearing for soundness ([OPUS-4.8] sq-cnor): `build_using(named: None)`
///   keeps the auth view in the active dataset, so `GRAPH ?g` CAN bind to it and the engine
///   CAN record a Delta to it. If we excluded the auth view here the precise resolved set
///   would *under-count* the engine write set — a binding to the auth view would escape the
///   per-graph check and let a `WITH … DELETE/INSERT { GRAPH ?g … }` transiently mutate the
///   authorization view. Including it means a binding to the auth view appears in the precise
///   set, and since no session is ever granted write/append on the auth view itself,
///   [`allowed`] denies — fail-closed. (Contrast [`store_named_graphs`], used by the
///   conservative *wildcard* path, which deliberately excludes the auth view.)
fn rescope_dataset(graph: &sparq_core::Graph, u: &QueryDataset) -> QueryDataset {
    let named = match &u.named {
        Some(list) => list.clone(),
        // WITH: make the implicit "all store named graphs" explicit for the query side —
        // the SAME set `build_using(named: None)` keeps, auth view included.
        None => engine_using_named_graphs(graph),
    };
    QueryDataset { default: u.default.clone(), named: Some(named) }
}

/// Resolve a `DELETE/INSERT … WHERE` operation's `GRAPH ?var` template slots into the
/// concrete (named-graph, need) requirements its solutions actually produce.
/// [OPUS-4.8] sq-biss.
///
/// `Ok(reqs)` is the precise per-graph requirement set: write is needed only on the
/// graphs `?var` actually binds to. [`Bail::Wildcard`] means the resolution must fall back
/// to the conservative all-graphs wildcard at that `Need` (fail-closed); the caller raises
/// it. [`Bail::Budget`] means the binding SELECT ran out of the caller's [`QueryBudget`] —
/// a hard deny, never a fallback (see [`Bail`]).
///
/// # Soundness — why the WHERE runs over the FULL store, not the actor's read view
///
/// The apply step ([`sparq_engine::update_in_place`]) instantiates the templates by
/// evaluating this exact WHERE pattern over the **full store**, then writes the resulting
/// graph slots. To check the right set of graphs we must enumerate the SAME bindings the
/// engine will — anything narrower (e.g. the actor's authorized read view) could miss a
/// graph the apply would write and open a hole, which the bead's note explicitly warns
/// against. Evaluating over the full store is *not* an information leak: we never return
/// rows to the actor — only an allow/deny verdict, and we deny unless the actor can WRITE
/// every graph the apply could touch (a graph the actor cannot even read is certainly not
/// writable, so it forces a deny). The verdict is exactly the one the conservative
/// wildcard already computed, only tightened to the graphs that genuinely appear as
/// targets.
///
/// # `USING`/`WITH` re-scope ([OPUS-4.8] sq-cnor)
///
/// When the operation carries a `USING (NAMED)` / `WITH` clause the apply re-scopes the
/// WHERE through `Dataset::build_using`, which a *plain* SELECT (`dataset: None`) does NOT
/// reproduce: `build_using`'s `named: None` (the parser's encoding of `WITH`, which only
/// re-scopes the DEFAULT graph) keeps ALL the store's named graphs, whereas a query whose
/// `FROM NAMED` list is empty keeps NONE — so a naive serialized SELECT would *under-count*
/// the `GRAPH ?g` bindings and open a hole. We close the gap by handing the binding SELECT
/// the EXACT same active dataset the engine builds, re-expressed as a query dataset clause
/// ([`rescope_dataset`]): the `USING` default graphs become `FROM`, and the named set is
/// either the explicit `USING NAMED` list or — for `WITH` (`named: None`) — an explicit
/// `FROM NAMED` of every store named graph (the set `build_using` keeps). The query side's
/// [`sparq_engine::query`] then assembles the identical active dataset
/// (`dataset::build_active` with `default`/`named` matching `build_using` graph-for-graph;
/// the two diverge only on a view intersection, and no dataset view is installed on the
/// write-check path — `check` calls the full-store `query`, not `query_view`). The
/// enumerated bindings therefore equal the engine's, so the resolution stays precise even
/// under `USING`/`WITH`.
///
/// The `WITH` named set explicitly INCLUDES the reserved [`AUTH_GRAPH`](crate::AUTH_GRAPH)
/// view, because `build_using(named: None)` does: `GRAPH ?g` can therefore bind to the auth
/// view and the engine can record a write to it. That binding shows up in the precise set
/// here, and since no session is granted write/append on the auth view, [`allowed`] denies —
/// fail-closed. (Were the auth view dropped from the materialized set the precise resolution
/// would under-count the engine write set and a `WITH … { GRAPH ?g … }` could transiently
/// mutate the authorization view; that hole is closed by [`engine_using_named_graphs`].)
///
/// Fail-closed cases that still fall back to the wildcard (rather than risk a hole):
///
/// - the WHERE cannot be evaluated (parse/serialize/engine error) — `Bail::Wildcard(need)`;
/// - a slot binds to a **blank-node** graph name: the engine *would* write to it
///   ([`gnp_subst`] accepts blank nodes), but the auth view only ever grants write on
///   named graphs, so write can never be proven — `Bail::Wildcard(need)`.
///
/// Bindings the engine *drops* (an unbound variable, or a literal/triple where a graph
/// name is required — [`gnp_subst`] returns `None`) produce no write and are ignored.
///
/// # Budget ([FABLE-5] sq-yhlf0)
///
/// The binding SELECT is a real query evaluation on the WRITE path, so it runs under the
/// caller's `budget` — an unlimited budget (every non-budgeted entry point) is byte-for-byte
/// the previous behaviour. It is deliberately NOT folded into the `Bail::Wildcard` fallback:
/// the wildcard is a *cheaper* check that could still permit the update, and the apply would
/// then re-evaluate the same exhausted WHERE. Reporting the exhaustion is both honest and
/// fail-closed — nothing is mutated.
fn resolve_var_graphs(
    graph: &sparq_core::Graph,
    r: &VarGraphResolve,
    budget: &QueryBudget,
) -> Result<Vec<(NamedNode, Need)>, Bail> {
    // The strongest need across this operation's slots is the fallback level (and the
    // floor for any escalation): if we must give up, give up at the level that protects
    // every slot.
    let fallback = r.slots.iter().map(|(_, n)| *n).fold(Need::WriteOrAppend, strongest);

    // Build `SELECT ?v… { WHERE }` and evaluate it exactly as the engine instantiates the
    // templates. With no `USING`/`WITH` the active dataset is the full store
    // (`dataset: None`); with a re-scope we re-express the engine's `build_using` active
    // dataset as an explicit FROM/FROM NAMED clause so the SELECT enumerates the SAME
    // bindings the apply will ([OPUS-4.8] sq-cnor).
    let dataset = r.using.as_ref().map(|u| rescope_dataset(graph, u));
    let select = Query::Select {
        dataset,
        pattern: GraphPattern::Project {
            inner: Box::new(r.pattern.clone()),
            variables: r.slots.iter().map(|(v, _)| v.clone()).collect(),
        },
        base_iri: None,
    };
    let result = match sparq_engine::query_with_budget(graph, &select.to_string(), budget) {
        Ok(r) => r,
        Err(e) if is_budget_error(&e) => return Err(Bail::Budget(e)),
        Err(_) => return Err(Bail::Wildcard(fallback)), // un-evaluable WHERE → conservative
    };

    // Column index of each slot variable in the result (a projected variable absent from
    // the row vars never binds → contributes no write).
    let col = |v: &Variable| result.vars.iter().position(|x| x == v);

    let mut out: Vec<(NamedNode, Need)> = Vec::new();
    for (v, need) in &r.slots {
        let Some(i) = col(v) else { continue };
        for row in &result.rows {
            match row.get(i).and_then(|c| c.as_ref()) {
                Some(Term::NamedNode(n)) => {
                    // Per-graph need, honouring the control-doc convention exactly as a
                    // static target would. (`check` flags re-materialization when a
                    // resolved target is an `.acl`/`.acr` graph.)
                    out.push((n.clone(), need_for_graph(n.as_str(), *need)));
                }
                // A blank-node graph name IS written by the engine but can never be
                // authorized → fail closed to the wildcard.
                Some(Term::BlankNode(_)) => return Err(Bail::Wildcard(fallback)),
                // Unbound, or a literal/triple term: the engine drops the quad
                // (`gnp_subst` → None), so no write happens — nothing to authorize.
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Does the session have `need` on the concrete graph `g`?
fn allowed(auth: &AuthIndex, s: &Session, g: &NamedNode, need: Need) -> bool {
    let has = |mode: Mode| auth.accessible(s, mode).iter().any(|x| x == g);
    match need {
        Need::Write => has(Mode::Write),
        Need::WriteOrAppend => has(Mode::Write) || has(Mode::Append),
    }
}

/// Every named graph currently in the store except the reserved auth view. Used by the
/// conservative *wildcard* check (CLEAR/DROP ALL|NAMED, or a var-graph op that bailed): the
/// actor must hold write on every *user* graph, and the auth view is never user-writable, so
/// excluding it here is correct — the auth view is protected by re-materialization, not by a
/// write grant.
fn store_named_graphs(graph: &sparq_core::Graph) -> Vec<NamedNode> {
    graph
        .named
        .iter()
        .filter_map(|(n, _)| match n {
            Term::NamedNode(nn) if nn.as_str() != crate::AUTH_GRAPH => Some(nn.clone()),
            _ => None,
        })
        .collect()
}

/// Every named-node graph the engine's `Dataset::build_using(named: None)` keeps in the
/// active dataset for a `WITH` re-scope — i.e. EVERY store named graph, **including the
/// reserved [`AUTH_GRAPH`](crate::AUTH_GRAPH) view** (and any future reserved view). This is
/// the set [`rescope_dataset`] must hand the binding SELECT so the precise resolved
/// var-graph set EQUALS the engine write set under `WITH`; see that function for why the auth
/// view must be present (fail-closed deny, not a silent under-count). [OPUS-4.8] sq-cnor.
///
/// (Blank-node graph names cannot be expressed in a `FROM NAMED` clause and contribute no
/// binding the actor could ever be authorized for, so they are excluded here exactly as the
/// query-side dataset builder excludes them; a var-graph slot that binds to a blank-node
/// graph is independently caught fail-closed by [`resolve_var_graphs`].)
fn engine_using_named_graphs(graph: &sparq_core::Graph) -> Vec<NamedNode> {
    graph
        .named
        .iter()
        .filter_map(|(n, _)| match n {
            Term::NamedNode(nn) => Some(nn.clone()),
            _ => None,
        })
        .collect()
}

/// The outcome of [`check`]: either the (deduped) set of graph names the permitted
/// update may touch — used to decide re-materialization — or a deny reason.
pub(crate) struct Permit {
    /// Whether a re-materialization should follow a successful apply.
    pub rematerialize: bool,
}

/// Authorize an update string for `session` against `auth` over the dataset `graph`,
/// WITHOUT mutating anything. `Ok(Permit)` means every target is writable; `Err(msg)`
/// is a deny (fail-closed) and the caller must not apply the update.
///
/// `budget` bounds the ONE evaluation this check may perform — the `GRAPH ?var` binding
/// SELECT of [`resolve_var_graphs`]. Everything else here is static algebra analysis plus
/// per-graph auth-index lookups, so the whole check is bounded once that query is
/// ([FABLE-5] sq-yhlf0). Pass [`QueryBudget::unlimited`] for the historic behaviour.
pub(crate) fn check(
    graph: &sparq_core::Graph,
    auth: &AuthIndex,
    session: &Session,
    sparql: &str,
    group_docs: &FxHashSet<String>,
    budget: &QueryBudget,
) -> Result<Permit, String> {
    let upd = SparqlParser::new().parse_update(sparql).map_err(|e| e.to_string())?;
    let mut reqs = analyze(&upd);

    if reqs.touches_default {
        return Err(
            "update denied: writes to the default graph are not permitted (pod data lives in \
             named graphs only)"
                .to_owned(),
        );
    }

    // Precise resolution of `GRAPH ?var` template slots ([OPUS-4.8] sq-biss): evaluate
    // each such operation's WHERE to enumerate the concrete graphs it actually targets,
    // and fold them into the static per-graph requirements — instead of demanding write
    // on every store graph. A resolution that cannot be reduced safely raises the
    // conservative wildcard (handled below). Done BEFORE the static loop runs the checks,
    // so resolved targets are authorized on exactly the same footing as static ones.
    // (Resolve first, then fold in — `resolve_var_graphs` borrows `reqs.var_graphs`, the
    // folding mutates the rest of `reqs`.)
    let resolutions: Vec<Result<Vec<(NamedNode, Need)>, Bail>> =
        reqs.var_graphs.iter().map(|r| resolve_var_graphs(graph, r, budget)).collect();
    for res in resolutions {
        match res {
            Ok(resolved) => {
                // A precisely-resolved variable-graph op still re-materializes on success,
                // unconditionally. The `affects_auth_view` sweep at the end of `check`
                // would now catch its resolved `.acl`/`.acr` and referenced-group-document
                // targets anyway, but this stays as belt-and-braces: it also covers a
                // target that is not YET a referenced group document, and
                // re-materialization is only a cost, never a security hole. Matches the
                // conservative path's "any dynamic write re-materializes" guarantee.
                // [OPUS-4.8] sq-biss; [SONNET-4.6] issue #55.
                if !resolved.is_empty() {
                    reqs.rematerialize_hint = true;
                }
                reqs.graphs.extend(resolved);
            }
            // Fail-closed: fall back to demanding write on every store graph.
            Err(Bail::Wildcard(need)) => raise_wildcard(&mut reqs, need),
            // Fail-closed AND honest: the binding SELECT ran out of budget, so the precise
            // set is unknown and the apply would have to re-run the same WHERE. Deny with
            // the engine's own budget message; nothing has been mutated. [FABLE-5] sq-yhlf0.
            Err(Bail::Budget(e)) => return Err(e),
        }
    }

    // Static per-graph requirements (now including the precisely-resolved variable-graph
    // targets).
    for (g, need) in &reqs.graphs {
        if !allowed(auth, session, g, *need) {
            return Err(format!(
                "update denied: session lacks {} permission on <{}>",
                need_label(*need),
                g.as_str()
            ));
        }
    }

    // Conservative wildcard requirement: must be able to write EVERY store graph.
    if let Some(need) = reqs.wildcard {
        for g in store_named_graphs(graph) {
            // For a wildcard the per-graph need still respects the control-doc
            // convention (writing an .acl under a CLEAR ALL needs the Write grant that
            // only Control-holders have).
            let g_need = strongest(need, need_for_graph(g.as_str(), need));
            if !allowed(auth, session, &g, g_need) {
                return Err(format!(
                    "update denied: a graph-wildcard operation (variable GRAPH target or \
                     CLEAR/DROP ALL|NAMED) requires {} permission on every graph, but the \
                     session lacks it on <{}>",
                    need_label(need),
                    g.as_str()
                ));
            }
        }
    }

    // Auth-view inputs among the permitted targets — the `.acl`/`.acr` documents AND the
    // group documents the current access-control documents reference. Raised over the
    // whole resolved set (static + precisely-resolved `GRAPH ?var` targets) rather than
    // per-push, so a group-document write triggers re-materialization on exactly the same
    // footing as an `.acl` write.
    if reqs.graphs.iter().any(|(g, _)| affects_auth_view(g.as_str(), group_docs)) {
        reqs.rematerialize_hint = true;
    }

    let rematerialize = reqs.rematerialize_hint || reqs.wildcard.is_some();
    Ok(Permit { rematerialize })
}

fn need_label(n: Need) -> &'static str {
    match n {
        Need::WriteOrAppend => "write/append",
        Need::Write => "write",
    }
}

// --- [OPUS-4.8] sq-3jtd.1: differential test — PodStore's precise variable-GRAPH write set
//     EQUALS the engine's actual write set ----------------------------------------------------
//
// The security invariant the rest of this module rests on (module docs, line 26ff): for a
// `DELETE/INSERT … WHERE` with a `GRAPH ?var` slot, `resolve_var_graphs` must enumerate
// EXACTLY the set of named graphs `sparq_engine::update_in_place` will actually write — no
// MORE (an over-approximation only costs a false denial) and crucially no LESS (an
// under-approximation = a write that escapes authorization = a security hole).
//
// These tests are the differential GUARD around that invariant (and around sq-cnor's
// USING/WITH precision work): they compute the precise set from `resolve_var_graphs` AND the
// real write set from `sparq_engine::update_in_place_capturing` (whose `UpdateEffect::Delta`
// records the slot of every graph the engine actually touched) and assert the two are EQUAL —
// or, for the cases `resolve_var_graphs` deliberately bails on (USING/WITH), assert the
// fallback is an OVER-approximation of the real write set (sound: a superset can only deny,
// never leak).
//
// Shapes mirror PSS's `setAclPointer`/`putContainer` (gh-47): a `DELETE/INSERT … WHERE` whose
// WHERE has an OPTIONAL (the binding may or may not match → the graph set varies), including a
// case where the OPTIONAL does NOT bind (a naive resolver could over- or under-count), and a
// WITH-clause variant (WITH re-scopes the operation's default graph).
#[cfg(test)]
mod differential_writeset_tests {
    use super::*;
    use sparq_core::Graph;
    use sparq_engine::{update_in_place_capturing, UpdateEffect};
    use std::collections::BTreeSet;

    /// A tiny PSS-shaped pod: two resource graphs (`r1`, `r2`) each holding a `title`; only
    /// `r1` ALSO holds the optional `aclPointer` triple that PSS's `setAclPointer` keys its
    /// OPTIONAL off. A third graph `r3` holds an unrelated triple (no title) so a title-keyed
    /// WHERE never binds it — present to prove the resolver does not over-count the store.
    ///
    /// CRITICAL ([OPUS-4.8] sq-cnor): the fixture ALSO seeds a `title` triple in the reserved
    /// [`crate::AUTH_GRAPH`] view. `Dataset::build_using(named: None)` (the `WITH` re-scope)
    /// keeps the auth view, so a title-keyed `GRAPH ?g` WHERE makes `?g` bind to it and the
    /// engine records a write there. The guard tests therefore expect the engine write set to
    /// INCLUDE the auth view, and the WITH guard catches the original under-count: the prior
    /// `rescope_dataset` dropped the auth view from the materialized `FROM NAMED` set, so the
    /// precise resolved set MISSED it while the engine wrote it — `precise != written`. With
    /// the auth view restored to the materialized set the two are equal again (and the
    /// production `check` path then DENIES, since no session is write-granted on the auth view).
    fn pss_dataset() -> Graph {
        let nq = "\
<https://pod.ex/r1#it> <https://ex.dev/ns#title> \"r1\" <https://pod.ex/r1> .
<https://pod.ex/r1#it> <https://ex.dev/ns#aclPointer> <https://pod.ex/r1.acl> <https://pod.ex/r1> .
<https://pod.ex/r2#it> <https://ex.dev/ns#title> \"r2\" <https://pod.ex/r2> .
<https://pod.ex/r3#it> <https://ex.dev/ns#other> \"r3\" <https://pod.ex/r3> .
<https://sparq.dev/auth#it> <https://ex.dev/ns#title> \"auth\" <urn:sparq:auth> .
";
        Graph::load_dataset(nq, "nquads").expect("fixture loads")
    }

    const AUTH: &str = crate::AUTH_GRAPH;

    /// The set of named graphs `sparq_engine::update_in_place` ACTUALLY writes for `sparql`,
    /// observed via the engine's own resolved effect log (`UpdateEffect::Delta { slot, … }`
    /// records every graph slot the engine touched). Default-graph writes (slot `None`) are
    /// reported separately via the bool — none of these PSS shapes target the default graph.
    ///
    /// Blank-node graph slots ARE included in the named set (keyed by their `_:id`): a graph
    /// name is, per RDF, a NamedNode OR a BlankNode, so dropping the blank-node case could mask
    /// a real engine write and defeat the differential guard. Any OTHER `Some(term)` shape (a
    /// literal or triple-term graph name) is malformed for a graph slot — we fail loudly so a
    /// regression that starts emitting such a slot is caught immediately rather than swallowed.
    fn engine_write_set(graph: &mut Graph, sparql: &str) -> (BTreeSet<String>, bool) {
        let effects = update_in_place_capturing(graph, sparql, &QueryBudget::unlimited())
            .expect("engine applies the update");
        let mut named = BTreeSet::new();
        let mut touched_default = false;
        for e in &effects {
            if let UpdateEffect::Delta { slot, .. } = e {
                match slot {
                    Some(Term::NamedNode(n)) => {
                        named.insert(n.as_str().to_owned());
                    }
                    // A blank-node graph name is a valid graph slot; record it (keyed by its
                    // blank-node label) so a write here cannot silently slip past the diff.
                    Some(Term::BlankNode(b)) => {
                        named.insert(format!("_:{}", b.as_str()));
                    }
                    // Literal / triple-term graph slots are malformed — never expected.
                    Some(other) => {
                        panic!("engine emitted a non-graph-name slot in a Delta effect: {other:?}")
                    }
                    None => touched_default = true,
                }
            }
        }
        (named, touched_default)
    }

    /// PodStore's PRECISE variable-GRAPH target set for `sparql`, as computed by
    /// `resolve_var_graphs` over `graph`. `Some(set)` is the precise resolution; `None` means
    /// the resolver bailed to the conservative all-graphs wildcard (USING/WITH, blank-node
    /// binding, or an un-evaluable WHERE) — i.e. it demands write on EVERY store graph.
    fn resolve_var_graph_set(graph: &Graph, sparql: &str) -> Option<BTreeSet<String>> {
        let upd = SparqlParser::new().parse_update(sparql).expect("parses");
        let reqs = analyze(&upd);
        assert!(
            !reqs.var_graphs.is_empty(),
            "test update must carry a GRAPH ?var slot"
        );
        let mut out = BTreeSet::new();
        for r in &reqs.var_graphs {
            match resolve_var_graphs(graph, r, &QueryBudget::unlimited()) {
                Ok(resolved) => {
                    for (n, _need) in resolved {
                        out.insert(n.as_str().to_owned());
                    }
                }
                // Any operation that fell back to the wildcard makes the WHOLE update's
                // precise set undefined — report the fallback.
                Err(_) => return None,
            }
        }
        Some(out)
    }

    /// CASE 1 — OPTIONAL that BINDS, no re-scope: the precise resolver set must EQUAL the
    /// engine write set exactly. PSS `setAclPointer` shape: rewrite the pointer of every
    /// resource that has one. `?g` binds to the resources where the OPTIONAL matched.
    #[test]
    fn optional_bound_precise_set_equals_engine_write_set() {
        // DELETE the old pointer / INSERT a new one, keyed on the resource carrying a title;
        // the pointer triple sits behind an OPTIONAL, so `?p` is bound only for r1.
        let upd = "\
            DELETE { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> ?p } } \
            INSERT { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> <https://pod.ex/new.acl> } } \
            WHERE  { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t . \
                                OPTIONAL { ?s <https://ex.dev/ns#aclPointer> ?p } } }";

        let graph = pss_dataset();
        let precise = resolve_var_graph_set(&graph, upd).expect("no re-scope -> precise");

        let mut applied = pss_dataset();
        let (written, default) = engine_write_set(&mut applied, upd);
        assert!(!default, "no default-graph write in this shape");

        // The DELETE half only produces a quad for r1 (where OPTIONAL bound `?p`); the INSERT
        // half produces a quad for every graph with a title — r1, r2 AND the seeded auth view
        // (a plain no-USING update runs over the full store, which build_using also keeps).
        // resolve_var_graphs must enumerate EXACTLY that set — never r3 (no title), never the
        // whole store.
        assert_eq!(
            precise, written,
            "precise resolve_var_graphs set must EQUAL the engine's actual write set"
        );
        let expect: BTreeSet<String> = ["https://pod.ex/r1", "https://pod.ex/r2", AUTH]
            .map(String::from)
            .into();
        assert_eq!(
            written, expect,
            "engine wrote exactly r1+r2+auth (every titled graph)"
        );
    }

    /// CASE 2 — OPTIONAL that does NOT bind for some rows: the precise set must still equal the
    /// engine write set. A naive resolver that counted the OPTIONAL's graph even when it is
    /// unbound (or skipped a graph because the OPTIONAL didn't match) would over/under-count.
    /// Here the DELETE template references the OPTIONAL var `?p`: for r2 (no pointer) `?p` is
    /// unbound, so the engine drops the DELETE quad — r2 is written ONLY via the INSERT half.
    #[test]
    fn optional_unbound_precise_set_equals_engine_write_set() {
        // DELETE keyed on the (possibly-unbound) OPTIONAL var; INSERT keyed on the title.
        // r1: OPTIONAL binds -> delete + insert. r2: OPTIONAL unbound -> insert only. r3: no
        // title -> never bound at all.
        let upd = "\
            DELETE { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> ?p } } \
            INSERT { GRAPH ?g { ?s <https://ex.dev/ns#mark> \"x\" } } \
            WHERE  { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t . \
                                OPTIONAL { ?s <https://ex.dev/ns#aclPointer> ?p } } }";

        let graph = pss_dataset();
        let precise = resolve_var_graph_set(&graph, upd).expect("no re-scope -> precise");

        let mut applied = pss_dataset();
        let (written, default) = engine_write_set(&mut applied, upd);
        assert!(!default);

        // `?g` binds to every titled graph — r1, r2 and the auth view. The engine writes all
        // of them — r1 via delete+insert, r2/auth via insert only (their DELETE quad is
        // dropped, `?p` unbound). r3 is never bound. The precise set must be exactly that, with
        // NO phantom r3 and NO escalation to all-store.
        assert_eq!(
            precise, written,
            "OPTIONAL-unbound: precise set must EQUAL the engine write set"
        );
        let expect: BTreeSet<String> = ["https://pod.ex/r1", "https://pod.ex/r2", AUTH]
            .map(String::from)
            .into();
        assert_eq!(
            written, expect,
            "engine wrote exactly r1+r2+auth (r3 unbound)"
        );
    }

    /// CASE 3 — an OPTIONAL whose WHERE key is ABSENT everywhere, so `?g` binds to NOTHING:
    /// the precise set must be EMPTY and equal the engine's (empty) write set. A resolver that
    /// fell back to the wildcard here would wrongly demand write on the whole store for a
    /// no-op update.
    #[test]
    fn optional_empty_binding_precise_set_equals_empty_engine_write_set() {
        // No resource carries `<ex:missing>`, so `?g` never binds; the update is a no-op.
        let upd = "\
            DELETE { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> ?p } } \
            WHERE  { GRAPH ?g { ?s <https://ex.dev/ns#missing> ?t . \
                                OPTIONAL { ?s <https://ex.dev/ns#aclPointer> ?p } } }";

        let graph = pss_dataset();
        let precise = resolve_var_graph_set(&graph, upd).expect("no re-scope -> precise");

        let mut applied = pss_dataset();
        let (written, default) = engine_write_set(&mut applied, upd);
        assert!(!default);

        assert!(written.is_empty(), "engine writes nothing (no binding)");
        assert_eq!(
            precise, written,
            "empty binding -> empty precise set, no wildcard"
        );
    }

    /// CASE 4 — the WITH-clause variant (PSS shapes can carry `WITH`). [OPUS-4.8] sq-cnor
    /// tightened `resolve_var_graphs` so a `USING`/`WITH` re-scope is now resolved PRECISELY:
    /// the binding SELECT is handed the same active dataset the apply's `build_using` builds
    /// (`rescope_dataset` re-expresses `WITH`'s implicit "all store named graphs" as an explicit
    /// `FROM NAMED` list), so it enumerates the SAME `GRAPH ?g` bindings the engine writes. The
    /// precise set must now EQUAL the engine write set — no longer the conservative wildcard.
    ///
    /// SOUNDNESS GUARD ([OPUS-4.8] sq-cnor): the fixture seeds a `title` triple in the auth
    /// view, which `build_using(named: None)` keeps, so the engine's write set INCLUDES the
    /// auth view. The earlier `rescope_dataset` excluded the auth view from the materialized
    /// `FROM NAMED` list, so the precise resolved set MISSED it — `precise != written` — an
    /// under-count that let a `WITH … { GRAPH ?g … }` slip a write to the authorization view
    /// past the per-graph check. This assertion FAILS on that prior code and PASSES once the
    /// auth view is restored to the materialized set; the production `check` then DENIES the
    /// op fail-closed (no session is write-granted on the auth view).
    #[test]
    fn with_clause_resolves_precisely_to_engine_write_set() {
        // WITH re-scopes the operation's DEFAULT graph to r1; the GRAPH ?g slot still ranges
        // over named graphs. The resolver re-expresses the re-scope as a FROM NAMED clause and
        // resolves precisely.
        let upd = "\
            WITH <https://pod.ex/r1> \
            DELETE { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> ?p } } \
            INSERT { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> <https://pod.ex/new.acl> } } \
            WHERE  { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t . \
                                OPTIONAL { ?s <https://ex.dev/ns#aclPointer> ?p } } }";

        let graph = pss_dataset();
        let precise =
            resolve_var_graph_set(&graph, upd).expect("WITH re-scope now resolves precisely (sq-cnor)");

        let mut applied = pss_dataset();
        let (written, default) = engine_write_set(&mut applied, upd);

        // WITH re-scopes the operation's DEFAULT graph, but EVERY quad pattern in the DELETE,
        // INSERT and WHERE templates here is explicitly `GRAPH ?g { … }`-scoped, so no triple
        // lands in (or is read from) the default graph: the WITH target is never written.
        assert!(
            !default,
            "WITH default-graph re-scope must NOT produce a default-graph write when every \
             template quad is explicitly GRAPH ?g-scoped"
        );

        // The precise set now EQUALS the engine's actual write set — exactly r1+r2 AND the auth
        // view (every titled graph build_using keeps). The sq-cnor under-count is closed; no
        // escalation to the all-store wildcard, and crucially the auth view is no longer missed.
        assert_eq!(
            precise, written,
            "WITH: precise resolve_var_graphs set must EQUAL the engine's actual write set"
        );
        assert!(
            written.contains(AUTH),
            "the engine's WITH write set includes the auth view (build_using keeps it) — the \
             precise set must too, or a write to the auth view escapes the per-graph check"
        );
        let expect: BTreeSet<String> = ["https://pod.ex/r1", "https://pod.ex/r2", AUTH]
            .map(String::from)
            .into();
        assert_eq!(
            written, expect,
            "engine wrote exactly r1+r2+auth under WITH"
        );
    }

    /// CASE 5 — `USING NAMED` restricts the GRAPH ?g range to the listed graphs only ([OPUS-4.8]
    /// sq-cnor). The binding SELECT must mirror `build_using`'s explicit named set: `?g` binds
    /// only within the `USING NAMED` graphs, so a resource with a title that is NOT named in the
    /// USING clause must NOT appear in the precise set (and the engine does not write it). This
    /// proves the re-scope is honoured precisely, not just passed through as the whole store.
    #[test]
    fn using_named_restricts_precise_set_to_listed_graphs() {
        // Only r1 is in USING NAMED; r2 has a title too but is excluded. `?g` can bind ONLY r1.
        let upd = "\
            DELETE { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> ?p } } \
            INSERT { GRAPH ?g { ?s <https://ex.dev/ns#aclPointer> <https://pod.ex/new.acl> } } \
            USING NAMED <https://pod.ex/r1> \
            WHERE  { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t . \
                                OPTIONAL { ?s <https://ex.dev/ns#aclPointer> ?p } } }";

        let graph = pss_dataset();
        let precise =
            resolve_var_graph_set(&graph, upd).expect("USING NAMED resolves precisely (sq-cnor)");

        let mut applied = pss_dataset();
        let (written, default) = engine_write_set(&mut applied, upd);
        assert!(!default, "no default-graph write in this shape");

        assert_eq!(
            precise, written,
            "USING NAMED: precise set must EQUAL the engine write set (only the listed graph)"
        );
        let expect: BTreeSet<String> = ["https://pod.ex/r1"].map(String::from).into();
        assert_eq!(
            written, expect,
            "USING NAMED <r1> restricts the write set to r1 (r2 excluded despite its title)"
        );
    }
}
