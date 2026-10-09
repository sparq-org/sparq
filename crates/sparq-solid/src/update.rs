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
//! - an update's WHERE sees only the session's **read** view ([`scope_reads`]): a
//!   conditional write needs read access to its condition, exactly as a query would;
//! - the **default graph** is never writable (pod data never lives there — design doc
//!   §2.1); any default-graph target is denied;
//! - a write to a graph the actor cannot write in the required mode is denied and the
//!   store is **not** mutated (the check runs entirely before [`update_in_place`]);
//! - a `DELETE/INSERT … WHERE` template with a `GRAPH ?var` slot is authorized against
//!   the graphs it is actually instantiated for: the engine evaluates the WHERE once,
//!   under the read view, and hands the concrete destination graphs of each operation
//!   to [`authorize_writes`] before applying that operation. There is no second
//!   evaluation, so the graphs authorized are exactly the graphs written. A destination
//!   that is not a writable named graph denies the update before it writes anything.
//!   Such an operation must be the whole request: a request with several operations
//!   that includes one is refused;
//! - a target whose graph name cannot be determined statically — a `CLEAR`/`DROP` of
//!   `ALL`/`NAMED` graphs — is treated *conservatively*: the actor must be able to write
//!   **every** named graph currently in the store, or the whole update is denied.
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
use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashSet;
use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, GraphTarget, OrderExpression};
use spargebra::term::{GraphName, GraphNamePattern, NamedNodePattern};
use spargebra::{GraphUpdateOperation, Update};

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

/// What a parsed update needs in order to be permitted.
#[derive(Debug, Default)]
struct WriteReqs {
    /// Concrete (named-graph, need) requirements gathered from static targets.
    graphs: Vec<(NamedNode, Need)>,
    /// The update targets the default graph (always denied — pod data is never there).
    touches_default: bool,
    /// Some `DELETE/INSERT … WHERE` template has a `GRAPH ?var` slot. Those graphs are
    /// known only once the WHERE is evaluated, so the store authorizes them during the
    /// apply ([`authorize_writes`]).
    var_graphs: bool,
    /// The update has a target whose graph cannot be determined statically (a
    /// `CLEAR`/`DROP` `ALL`/`NAMED`): the actor must be able to write every named graph in the store, in this
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
/// default-graph slot is recorded immediately; a `GRAPH ?var` slot is authorized once the
/// apply has instantiated it ([`authorize_writes`]).
fn push_graph_name_pattern(reqs: &mut WriteReqs, g: &GraphNamePattern, base: Need) {
    match g {
        GraphNamePattern::DefaultGraph => reqs.touches_default = true,
        GraphNamePattern::NamedNode(n) => push_graph(reqs, n, base),
        GraphNamePattern::Variable(_) => reqs.var_graphs = true,
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
            GraphUpdateOperation::DeleteInsert { delete, insert, .. } => {
                // Deletes always need Write; inserts need Write-or-Append. The WHERE
                // pattern only READS, so it is not a write target. A `GRAPH ?var` slot is
                // authorized against the graphs it is instantiated for, during the apply.
                for d in delete {
                    push_graph_name_pattern(&mut reqs, &d.graph_name, Need::Write);
                }
                for i in insert {
                    push_graph_name_pattern(&mut reqs, &i.graph_name, Need::WriteOrAppend);
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

/// The outcome of [`check`]: either the (deduped) set of graph names the permitted
/// update may touch — used to decide re-materialization — or a deny reason.
pub(crate) struct Permit {
    /// Whether a re-materialization should follow a successful apply.
    pub rematerialize: bool,
    /// Some template writes to a `GRAPH ?var`: the apply must authorize the graphs it
    /// instantiates with [`authorize_writes`].
    pub var_graphs: bool,
}

/// Check that every graph an update's patterns name is in `readable` (the session's read
/// view), so a `DELETE`/`INSERT … WHERE` reads exactly what a query by the same session
/// could. A conditional write needs read access to its condition. The caller then evaluates
/// the WHERE, in [`check`] and in the apply, under a view of exactly `readable`, so
/// `GRAPH ?var` ranges over readable graphs only and an unreadable graph is never scanned.
///
/// - `GRAPH <g>` in a WHERE (or inside its `EXISTS`) requires `g` to be readable, or the
///   update is denied.
/// - A default-graph pattern reads the `USING`/`WITH` graphs, each of which must be
///   readable. Without `USING`/`WITH` it reads the store's default graph, which no session
///   may read (the read path's default graph is empty), so the update is denied.
pub(crate) fn scope_reads(upd: &Update, readable: &FxHashSet<Term>) -> Result<(), String> {
    for op in &upd.operations {
        if let GraphUpdateOperation::DeleteInsert { using, pattern, .. } = op {
            let mut scope = ReadScope { readable, default_read: false };
            scope.pattern(pattern, false)?;
            if scope.default_read {
                let Some(ds) = using else {
                    return Err("update denied: the WHERE reads the default graph, which is \
                                not readable (pod data lives in named graphs only)"
                        .to_owned());
                };
                for g in &ds.default {
                    scope.require(g)?;
                }
            }
        }
    }
    Ok(())
}

/// Whether `upd` has an operation that evaluates a pattern over the store (a
/// `DELETE`/`INSERT … WHERE`), the only kind [`scope_reads`] changes.
pub(crate) fn evaluates_patterns(upd: &Update) -> bool {
    upd.operations.iter().any(|op| matches!(op, GraphUpdateOperation::DeleteInsert { .. }))
}

/// The walk behind [`scope_reads`].
struct ReadScope<'a> {
    readable: &'a FxHashSet<Term>,
    /// A triple or path pattern outside every `GRAPH` block was seen.
    default_read: bool,
}

impl ReadScope<'_> {
    fn require(&self, g: &NamedNode) -> Result<(), String> {
        if self.readable.contains(&Term::NamedNode(g.clone())) {
            Ok(())
        } else {
            Err(format!("update denied: session lacks read permission on <{}>", g.as_str()))
        }
    }

    fn pattern(&mut self, p: &GraphPattern, in_graph: bool) -> Result<(), String> {
        match p {
            GraphPattern::Bgp { patterns } => {
                self.default_read |= !in_graph && !patterns.is_empty();
            }
            GraphPattern::Path { .. } => self.default_read |= !in_graph,
            GraphPattern::Graph { name, inner } => {
                self.pattern(inner, true)?;
                if let NamedNodePattern::NamedNode(g) = name {
                    self.require(g)?;
                }
            }
            GraphPattern::Join { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right }
            | GraphPattern::Lateral { left, right } => {
                self.pattern(left, in_graph)?;
                self.pattern(right, in_graph)?;
            }
            GraphPattern::LeftJoin { left, right, expression } => {
                self.pattern(left, in_graph)?;
                self.pattern(right, in_graph)?;
                if let Some(e) = expression {
                    self.expression(e, in_graph)?;
                }
            }
            GraphPattern::Filter { expr, inner } => {
                self.expression(expr, in_graph)?;
                self.pattern(inner, in_graph)?;
            }
            GraphPattern::Extend { inner, expression, .. } => {
                self.pattern(inner, in_graph)?;
                self.expression(expression, in_graph)?;
            }
            GraphPattern::OrderBy { inner, expression } => {
                self.pattern(inner, in_graph)?;
                for o in expression {
                    let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = o;
                    self.expression(e, in_graph)?;
                }
            }
            GraphPattern::Group { inner, aggregates, .. } => {
                self.pattern(inner, in_graph)?;
                for (_, a) in aggregates {
                    if let AggregateExpression::FunctionCall { expr, .. } = a {
                        self.expression(expr, in_graph)?;
                    }
                }
            }
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => self.pattern(inner, in_graph)?,
            GraphPattern::Values { .. } => {}
            // A SERVICE body is evaluated by the remote endpoint, not over this store.
            GraphPattern::Service { .. } => {}
        }
        Ok(())
    }

    fn expression(&mut self, e: &Expression, in_graph: bool) -> Result<(), String> {
        match e {
            Expression::Exists(p) => self.pattern(p, in_graph)?,
            Expression::Or(a, b)
            | Expression::And(a, b)
            | Expression::Equal(a, b)
            | Expression::SameTerm(a, b)
            | Expression::Greater(a, b)
            | Expression::GreaterOrEqual(a, b)
            | Expression::Less(a, b)
            | Expression::LessOrEqual(a, b)
            | Expression::Add(a, b)
            | Expression::Subtract(a, b)
            | Expression::Multiply(a, b)
            | Expression::Divide(a, b) => {
                self.expression(a, in_graph)?;
                self.expression(b, in_graph)?;
            }
            Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
                self.expression(a, in_graph)?;
            }
            Expression::If(a, b, c) => {
                self.expression(a, in_graph)?;
                self.expression(b, in_graph)?;
                self.expression(c, in_graph)?;
            }
            Expression::In(a, list) => {
                self.expression(a, in_graph)?;
                for x in list {
                    self.expression(x, in_graph)?;
                }
            }
            Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
                for x in list {
                    self.expression(x, in_graph)?;
                }
            }
            Expression::NamedNode(_)
            | Expression::Literal(_)
            | Expression::Variable(_)
            | Expression::Bound(_) => {}
        }
        Ok(())
    }
}

/// Authorize a parsed update's statically known targets for `session` against `auth`
/// over the dataset `graph`, WITHOUT mutating anything. Run [`scope_reads`] on it first.
/// `Ok(Permit)` means every static target is writable; `Err(msg)` is a deny (fail-closed)
/// and the caller must not apply the update. Static algebra analysis plus per-graph
/// auth-index lookups only: nothing is evaluated here. `GRAPH ?var` template targets are
/// authorized during the apply, by [`authorize_writes`] (see [`Permit::var_graphs`]).
pub(crate) fn check(
    graph: &sparq_core::Graph,
    auth: &AuthIndex,
    session: &Session,
    upd: &Update,
    group_docs: &FxHashSet<String>,
) -> Result<Permit, String> {
    let mut reqs = analyze(upd);

    // A `GRAPH ?var` target is authorized while its operation is applied, so a denial can
    // only be all-or-nothing when that operation is the whole request.
    if reqs.var_graphs && upd.operations.len() > 1 {
        return Err(
            "update denied: a DELETE/INSERT … WHERE with a variable GRAPH target must be sent \
             as a request of its own"
                .to_owned(),
        );
    }

    if reqs.touches_default {
        return Err(
            "update denied: writes to the default graph are not permitted (pod data lives in \
             named graphs only)"
                .to_owned(),
        );
    }

    // Static per-graph requirements.
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
    // whole static set rather than per-push, so a group-document write triggers re-materialization on exactly the same
    // footing as an `.acl` write.
    if reqs.graphs.iter().any(|(g, _)| affects_auth_view(g.as_str(), group_docs)) {
        reqs.rematerialize_hint = true;
    }

    let rematerialize = reqs.rematerialize_hint || reqs.wildcard.is_some();
    Ok(Permit { rematerialize, var_graphs: reqs.var_graphs })
}

/// Authorize the graphs one `DELETE`/`INSERT … WHERE` is about to change, as the engine
/// instantiated them from its single evaluation of the WHERE (the authorizer of
/// [`sparq_engine::update_in_place_algebra_with_budget`]). Deleting needs `Write`,
/// inserting `Write` or `Append` (`Write` on an `.acl`/`.acr`). The default graph and
/// blank-node graph names are never writable. Sets `*auth_input` when a changed graph is
/// an auth-view input, so the caller re-materializes.
pub(crate) fn authorize_writes(
    auth: &AuthIndex,
    session: &Session,
    group_docs: &FxHashSet<String>,
    deletes: &[Option<Term>],
    inserts: &[Option<Term>],
    auth_input: &mut bool,
) -> Result<(), String> {
    let slots = deletes
        .iter()
        .map(|g| (g, Need::Write))
        .chain(inserts.iter().map(|g| (g, Need::WriteOrAppend)));
    for (slot, base) in slots {
        let g = match slot {
            Some(Term::NamedNode(g)) => g,
            None => {
                return Err("update denied: writes to the default graph are not permitted (pod \
                            data lives in named graphs only)"
                    .to_owned())
            }
            Some(other) => {
                return Err(format!("update denied: {other} is not a writable graph"));
            }
        };
        let need = need_for_graph(g.as_str(), base);
        if !allowed(auth, session, g, need) {
            return Err(format!(
                "update denied: session lacks {} permission on <{}>",
                need_label(need),
                g.as_str()
            ));
        }
        *auth_input |= affects_auth_view(g.as_str(), group_docs);
    }
    Ok(())
}

fn need_label(n: Need) -> &'static str {
    match n {
        Need::WriteOrAppend => "write/append",
        Need::Write => "write",
    }
}
