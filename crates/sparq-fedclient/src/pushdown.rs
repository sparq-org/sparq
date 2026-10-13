//! Capability-aware pushdown (design §4.3) — **Phase 4**.
//!
//! `sparq-fedplan` decides *which* sources answer *which* patterns and in *what* join
//! order; it does **not** decide the precise sub-query each source is asked. That is this
//! module's job: for each **FedX-style exclusive group** — a connected sub-pattern whose
//! only relevant source is one member — build the **maximal evaluable sub-algebra** for that
//! source's [`Capability`](crate::source::Capability) and emit it as one
//! [`SubQuery`](crate::source::SubQuery), so the source answers the most precise query it
//! can rather than one single-pattern `SELECT` per leaf (the Phase-3 `lower_leaf` shape).
//!
//! # What Phase 4 ships (HONEST scope)
//!
//! * [`exclusive_groups`] — the **exclusive-group decomposition**. From the planner's
//!   per-pattern source selection ([`PatternSources`](sparq_fedplan::PatternSources)) it
//!   forms the maximal sets of patterns that (a) each have **exactly one** retained source,
//!   (b) the **same** source, and (c) are **connected** by a shared variable (transitively).
//!   These are exactly the sub-BGPs FedX pushes as one sub-query. A pattern matched by two
//!   or more sources is **not** in any exclusive group — it is a cross-source join the
//!   operators phase fans out (kept out of pushdown, never under-answered).
//! * [`push_group`] — the **maximal sub-algebra builder**. For one exclusive group it builds
//!   the most precise [`SubQuery`] the group's source can answer: the projection trimmed to
//!   exactly the **join + output** variables (everything else is internal to the group), the
//!   FILTER conjuncts the source's [`FilterClass`](crate::source::FilterClass) covers AND
//!   that pass the **exact common-variable check** (below), and `ORDER BY` / `LIMIT` only
//!   when the capability's `order_limit` allows. The shape is chosen by interface:
//!   - **full endpoint** → a multi-pattern `SELECT … WHERE { tp … FILTER(…) } ORDER/LIMIT`;
//!   - **brTPF / plain TPF** → a single triple pattern only (a fragment server's access unit
//!     is one pattern); a group of >1 pattern on a fragment source is *not* collapsed into
//!     one sub-query — pushdown emits one per pattern and the join stays client-side.
//! * [`common_variable_check`] — the **exact** check Comunica is documented to omit
//!   (issues #834/#609): a FILTER conjunct is pushed **only when every variable it
//!   references is bound by the group's patterns**. A conjunct over a variable the group
//!   does not produce would change the remote answer (the remote sees an *unbound* variable
//!   where local evaluation sees it bound by a sibling group), so it is kept local. This is
//!   the load-bearing safety invariant: push only the provably-identical sub-algebra.
//! * [`render_values_block`] / [`bind_block_size`] — the **bind-join block** primitive used
//!   to join *across* groups: a `VALUES` block for a full endpoint (block size
//!   [`DEFAULT_BIND_BLOCK`]), a `maxMpR`-bounded block for brTPF, no block for plain TPF.
//!   This mirrors `sparq-engine`'s `service.rs::render_values_block` / `bind_block_size`
//!   (`pub(crate)`, so re-declared here exactly as Phase 2 re-declares the `Transport` seam).
//!
//! # Correctness — pushdown only ever NARROWS
//!
//! Every transform here either removes solutions a source would have returned (a pushed
//! FILTER) or removes columns the caller does not need (a trimmed projection) or bounds the
//! rows requested (`VALUES`/`maxMpR`) — it never adds an answer the residual local join
//! would not reattach. The common-variable check is what makes that true for filters: a
//! conjunct whose variables are all group-bound evaluates identically remote-side and
//! local-side, so pushing it cannot drop a solution the local plan would keep. Anything a
//! source cannot evaluate (a filter that fails the class check or the common-variable check,
//! a cross-source pattern, an `ORDER`/`LIMIT` the capability omits) is left for the local
//! engine on the residual — pushdown is correctness-preserving by construction.
//!
//! # What is STUBBED / deferred (no overclaim)
//!
//! * The FILTER conjuncts are a **light, pre-parsed model** ([`Filter`]) — variable set +
//!   a rendered SPARQL fragment + the class it needs. Wiring the *real* parsed-query FILTER
//!   algebra (from `spargebra`) into this model, and the disjunctive/combined-filter
//!   decomposition the design §4.3 names, is Phase 5's job when the operators consume a whole
//!   query rather than a bare BGP. Phase 4 ships the per-conjunct pushability decision (class
//!   check + common-variable check) the operators call, tested on the model directly.
//! * The bind-join block here is the **rendered block + the size policy**; the operator that
//!   *gathers* upstream bindings, slices them into blocks, and streams the per-block matches
//!   is the Phase-5 `operators`/`stream` work. Phase 4 owns the block construction the
//!   operator emits, not the streaming feeder.
//! * `ORDER BY` is pushed as a passthrough of caller-supplied sort keys; the client does not
//!   yet parse a query's `ORDER BY` into keys (Phase 5) — `push_group` takes the keys as an
//!   argument so the lowering is testable now.
//
// [OPUS-4.8] sq-7byx (epic sq-dnko): Phase-4 capability-aware pushdown — exclusive-group
// decomposition + maximal sub-algebra builder + the exact common-variable check + the
// bind-join block primitive. Phase 4 owns this module; Phase 5 owns operators.rs/stream.rs.
// Flagged for Fable re-review when available.

use crate::planner::{is_varname, pattern_vars, render_pattern_clause, render_term, RenderError};
use crate::source::{BindJoin, Capability, FilterClass, Interface, SubQuery};
use sparq_fedplan::{Bgp, PatternSources, Term, TriplePattern};

// ─── Bind-join block size policy (mirrors sparq-engine service.rs, `pub(crate)` there) ──

/// Default `VALUES` bind-join block size for a full endpoint: how many distinct binding
/// tuples are pushed into one remote request. Mirrors `sparq-engine`'s
/// `service.rs::DEFAULT_BIND_BLOCK` (`pub(crate)`, not importable) — ~50, FedX's default
/// bound-join batch: large enough to amortise the round-trip, small enough to keep the
/// injected query bounded. [OPUS-4.8] sq-7byx.
pub const DEFAULT_BIND_BLOCK: usize = 50;

/// The bind-join block size a source's [`Capability`] supports — the maximum number of
/// upstream binding tuples the bind-join operator may push in one request to this source:
///
/// * [`BindJoin::Values`] → [`DEFAULT_BIND_BLOCK`] (a full endpoint accepts an arbitrarily
///   large `VALUES` block; the default bounds the injected query);
/// * [`BindJoin::MaxMpR(n)`](BindJoin::MaxMpR) → `n` (the brTPF `hydra:maxMpR` ceiling — the
///   server promises to honour *at most* `n` mappings per request, so the block must not
///   exceed it), clamped to at least 1 so a tuple is still pushed one-per-request rather than
///   silently disabling the bind-join;
/// * [`BindJoin::None`] → `0` (plain TPF has no bind-join — every join is client-side).
///
/// [OPUS-4.8] sq-7byx.
pub fn bind_block_size(cap: &Capability) -> usize {
    match cap.bind_join {
        BindJoin::Values => DEFAULT_BIND_BLOCK,
        BindJoin::MaxMpR(n) => (n as usize).max(1),
        BindJoin::None => 0,
    }
}

/// Render a `VALUES` block binding `vars` to each tuple in `tuples`, in the SPARQL 1.1
/// syntax accepted inside a group graph pattern — the cross-group bind-join primitive.
///
/// Single-variable blocks use the short `VALUES ?v { v1 v2 }` form; multi-variable blocks
/// use the parenthesised `VALUES (?a ?b) { (a1 b1) (a2 b2) }` form, the same shape as
/// `sparq-engine`'s `service.rs::render_values_block`. Every variable must be a SPARQL
/// `VARNAME`, every row must hold exactly one bound term (an IRI or a literal) per variable,
/// and each term is written by the same strict renderer as a pattern position, so the block
/// is exactly one `VALUES` clause whatever the inputs hold; anything else is a
/// [`RenderError`]. `vars` empty or `tuples` empty yields an empty string (no block to push).
/// [OPUS-4.8] sq-7byx.
pub fn render_values_block(vars: &[String], tuples: &[Vec<Term>]) -> Result<String, RenderError> {
    use std::fmt::Write as _;
    if vars.is_empty() || tuples.is_empty() {
        return Ok(String::new());
    }
    if let Some(v) = vars.iter().find(|v| !is_varname(v)) {
        return Err(RenderError::Var(v.clone()));
    }
    let cell = |t: &Term| match t {
        Term::Var(v) => Err(RenderError::Values(format!("variable ?{} in place of a term", v.0))),
        _ => render_term(t),
    };
    let mut s = String::new();
    if vars.len() == 1 {
        let _ = write!(s, "VALUES ?{} {{", vars[0]);
    } else {
        s.push_str("VALUES (");
        for (i, v) in vars.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "?{}", v);
        }
        s.push_str(") {");
    }
    for tuple in tuples {
        if tuple.len() != vars.len() {
            return Err(RenderError::Values(format!(
                "{} terms for {} variables",
                tuple.len(),
                vars.len()
            )));
        }
        if vars.len() == 1 {
            let _ = write!(s, " {}", cell(&tuple[0])?);
        } else {
            s.push_str(" (");
            for (i, t) in tuple.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&cell(t)?);
            }
            s.push(')');
        }
    }
    s.push_str(" }");
    Ok(s)
}

/// One `ORDER BY` key pushed to a source: a variable, ascending or descending.
/// [OPUS-4.8] sq-7byx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderKey {
    /// The variable name, without the leading `?`. Must be a SPARQL `VARNAME`.
    pub var: String,
    /// `true` for `DESC(?var)`, `false` for ascending `?var`.
    pub descending: bool,
}

impl OrderKey {
    /// An ascending key on `var`.
    pub fn asc(var: impl Into<String>) -> OrderKey {
        OrderKey { var: var.into(), descending: false }
    }

    /// A descending key on `var`.
    pub fn desc(var: impl Into<String>) -> OrderKey {
        OrderKey { var: var.into(), descending: true }
    }

    /// The key as SPARQL, or `None` when `var` is not a SPARQL `VARNAME`.
    fn render(&self) -> Option<String> {
        if !is_varname(&self.var) {
            return None;
        }
        Some(if self.descending { format!("DESC(?{})", self.var) } else { format!("?{}", self.var) })
    }
}

// ─── A light FILTER-conjunct model (the seam Phase 5 wires the parsed algebra into) ─────

/// One FILTER conjunct as the pushdown layer sees it: the variables it references, a
/// rendered SPARQL expression fragment, and the [`FilterClass`] a source must support to
/// evaluate it remotely.
///
/// This is the deliberately-light Phase-4 model — the operators phase (Phase 5) lowers the
/// parsed query's FILTER algebra (and decomposes a conjunction / disjunction) into a slice of
/// these. Phase 4 owns the **per-conjunct pushability decision** over this model: a conjunct
/// is pushed only when the source's class covers `needs` AND it passes the common-variable
/// check ([`common_variable_check`]). [OPUS-4.8] sq-7byx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    /// The variable names this conjunct references (without the leading `?`). All must be
    /// bound by the group for the conjunct to be pushable (the common-variable check).
    pub vars: Vec<String>,
    /// The SPARQL expression, e.g. `?age > 18` or `?x = <http://ex/a>`. It is pushed inside
    /// `FILTER(…)` only when it parses as exactly one expression that references no variable
    /// outside [`Filter::vars`] and contains no `EXISTS` (see [`push_group`]); otherwise the
    /// conjunct stays local.
    pub expr: String,
    /// The [`FilterClass`] the source must support to evaluate this conjunct remotely. A
    /// simple equality / `IN` needs [`FilterClass::Equality`]; anything richer needs
    /// [`FilterClass::Full`].
    pub needs: FilterClass,
}

impl Filter {
    /// A conjunct over `vars` with rendered expression `expr` needing class `needs`.
    pub fn new(vars: Vec<String>, expr: impl Into<String>, needs: FilterClass) -> Filter {
        Filter {
            vars,
            expr: expr.into(),
            needs,
        }
    }
}

impl Filter {
    /// Whether `expr` can be written into a pushed `FILTER(…)` unchanged: the SPARQL parser
    /// reads `FILTER(<expr>)` as exactly one filter over one expression, every variable that
    /// expression references is in [`Filter::vars`] (so the common-variable check sees all of
    /// them), and it has no `EXISTS` (whose graph pattern could reference others).
    fn expr_is_pushable(&self) -> bool {
        use spargebra::algebra::GraphPattern;
        // An endpoint applies `\u`/`\U` escapes before parsing; this parser does not.
        if self.expr.contains("\\u") || self.expr.contains("\\U") {
            return false;
        }
        let parse = |sparql: String| match spargebra::SparqlParser::new().parse_query(&sparql) {
            Ok(spargebra::Query::Select { pattern: GraphPattern::Project { inner, .. }, .. }) => Some(*inner),
            _ => None,
        };
        let empty = |p: &GraphPattern| matches!(p, GraphPattern::Bgp { patterns } if patterns.is_empty());
        // The parser folds consecutive FILTERs into one conjunction, so the text is also parsed
        // as a BIND operand: only a single expression parses to the same tree in both places.
        let Some(GraphPattern::Filter { expr, inner }) =
            parse(format!("SELECT * WHERE {{ FILTER({}) }}", self.expr))
        else {
            return false;
        };
        let Some(GraphPattern::Extend { inner: bound_inner, expression, .. }) =
            parse(format!("SELECT * WHERE {{ BIND(({}) AS ?__sparq_pushed) }}", self.expr))
        else {
            return false;
        };
        if !empty(&inner) || !empty(&bound_inner) || expr != expression {
            return false;
        }
        let mut vars = Vec::new();
        expression_vars(&expr, &mut vars) && vars.iter().all(|v| self.vars.iter().any(|d| d == v))
    }
}

/// Collect the variable names `e` references into `out`; `false` when `e` contains `EXISTS`.
fn expression_vars(e: &spargebra::algebra::Expression, out: &mut Vec<String>) -> bool {
    use spargebra::algebra::Expression as E;
    match e {
        E::NamedNode(_) | E::Literal(_) => true,
        E::Variable(v) | E::Bound(v) => {
            out.push(v.as_str().to_owned());
            true
        }
        E::Or(a, b)
        | E::And(a, b)
        | E::Equal(a, b)
        | E::SameTerm(a, b)
        | E::Greater(a, b)
        | E::GreaterOrEqual(a, b)
        | E::Less(a, b)
        | E::LessOrEqual(a, b)
        | E::Add(a, b)
        | E::Subtract(a, b)
        | E::Multiply(a, b)
        | E::Divide(a, b) => expression_vars(a, out) && expression_vars(b, out),
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => expression_vars(a, out),
        E::In(a, list) => expression_vars(a, out) && list.iter().all(|x| expression_vars(x, out)),
        E::If(a, b, c) => expression_vars(a, out) && expression_vars(b, out) && expression_vars(c, out),
        E::Coalesce(list) | E::FunctionCall(_, list) => list.iter().all(|x| expression_vars(x, out)),
        E::Exists(_) => false,
    }
}

/// Whether a source whose capability advertises `have` can evaluate a conjunct that
/// `needs` a given class. The order is `None < Equality < Full`: a source covers a conjunct
/// iff its class is at least the conjunct's. [OPUS-4.8] sq-7byx.
fn class_covers(have: FilterClass, needs: FilterClass) -> bool {
    fn rank(c: FilterClass) -> u8 {
        match c {
            FilterClass::None => 0,
            FilterClass::Equality => 1,
            FilterClass::Full => 2,
        }
    }
    rank(have) >= rank(needs)
}

// ─── Exclusive groups (FedX) ────────────────────────────────────────────────────────────

/// A FedX **exclusive group**: a connected set of BGP patterns whose only relevant source is
/// one member. The pushdown layer sends the whole group to that source as one sub-query (for
/// a full endpoint) — the most precise query the source can answer for that sub-pattern.
///
/// `patterns` are BGP pattern indices, ascending (deterministic). `source` is the single
/// retained source index (into the descriptor / adapter slice). [OPUS-4.8] sq-7byx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusiveGroup {
    /// The single source index every pattern in this group is exclusively matched by.
    pub source: usize,
    /// The BGP pattern indices in this group, ascending.
    pub patterns: Vec<usize>,
}

/// Derive the FedX exclusive groups from the planner's per-pattern source selection.
///
/// A pattern is **exclusive** when [`select_sources`](sparq_fedplan::select_sources) retained
/// **exactly one** source for it. Two exclusive patterns are merged into one group when they
/// share that one source **and** are connected by a shared variable (directly or
/// transitively, via a union-find over the exclusive patterns). The result is the maximal
/// set of such groups, each addressed to one source.
///
/// Patterns with **zero** retained sources (the planner found no source that can contribute —
/// the BGP is unsatisfiable against this source set) and patterns with **two or more**
/// retained sources (a cross-source join) are **excluded** from every group: they are not
/// pushable as part of a single-source sub-query and are handled by the operators phase
/// (a multi-source leaf fans out; an empty leaf yields no rows). Pushdown never over-claims
/// a pattern into a group it does not exclusively belong to.
///
/// Determinism: groups are returned sorted by their smallest pattern index, and each group's
/// `patterns` ascending. [OPUS-4.8] sq-7byx.
pub fn exclusive_groups(selection: &[PatternSources], bgp: &Bgp) -> Vec<ExclusiveGroup> {
    // The exclusive patterns and their single source.
    let mut exclusive: Vec<(usize, usize)> = Vec::new(); // (pattern_index, source_index)
    for ps in selection {
        if ps.candidates.len() == 1 {
            exclusive.push((ps.pattern, ps.candidates[0].source));
        }
    }
    if exclusive.is_empty() {
        return Vec::new();
    }
    exclusive.sort_by_key(|&(p, _)| p);

    // Union-find over the exclusive patterns (indexed by their position in `exclusive`).
    let n = exclusive.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]]; // path-halving
            x = parent[x];
        }
        x
    }
    // Merge two exclusive patterns iff SAME source AND they share a variable in the BGP.
    for i in 0..n {
        for j in (i + 1)..n {
            if exclusive[i].1 != exclusive[j].1 {
                continue; // different source — never the same exclusive group.
            }
            if bgp.shares_var(exclusive[i].0, exclusive[j].0) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    // Collect components → groups. (The union-find roots are computed by index, so the
    // component key is the representative index; the per-pattern source + pattern come from
    // `exclusive[i]`.)
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<usize, (usize, Vec<usize>)> = BTreeMap::new();
    let roots: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    for (i, &(pattern, source)) in exclusive.iter().enumerate() {
        let entry = groups.entry(roots[i]).or_insert((source, Vec::new()));
        entry.1.push(pattern);
    }
    let mut out: Vec<ExclusiveGroup> = groups
        .into_values()
        .map(|(source, mut patterns)| {
            patterns.sort_unstable();
            ExclusiveGroup { source, patterns }
        })
        .collect();
    // Deterministic order: by the group's smallest pattern index.
    out.sort_by_key(|g| g.patterns.first().copied().unwrap_or(usize::MAX));
    out
}

/// The variables a group's patterns produce (bound terms in any position), de-duplicated,
/// in pattern-then-position order. These are the variables a pushed FILTER conjunct may
/// reference and the maximum a projection can keep. [OPUS-4.8] sq-7byx.
pub fn group_vars(group: &ExclusiveGroup, bgp: &Bgp) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for &pi in &group.patterns {
        if let Some(tp) = bgp.patterns.get(pi) {
            for t in [&tp.subject, &tp.predicate, &tp.object] {
                if let Term::Var(v) = t {
                    if !out.iter().any(|n| n == &v.0) {
                        out.push(v.0.clone());
                    }
                }
            }
        }
    }
    out
}

// ─── The exact common-variable check ────────────────────────────────────────────────────

/// The **exact common-variable check** (design §4.3, the check Comunica omits — #834/#609):
/// a FILTER conjunct may be pushed into a group **only when every variable it references is
/// bound by the group's patterns**.
///
/// If a conjunct references a variable the group does not produce, that variable is *unbound*
/// from the remote source's perspective but *bound* (by a sibling group) from the whole
/// query's perspective. Evaluating the conjunct remotely would therefore use a different
/// (unbound) value than local evaluation and could drop a solution the local plan keeps — so
/// such a conjunct is **not** pushed; it is kept local on the residual. Returns `true` iff
/// the conjunct is safe to push by this check alone (the caller separately checks the source
/// covers the conjunct's [`FilterClass`]). [OPUS-4.8] sq-7byx.
pub fn common_variable_check(filter: &Filter, group_vars: &[String]) -> bool {
    filter.vars.iter().all(|v| group_vars.iter().any(|g| g == v))
}

// ─── The maximal sub-algebra builder ────────────────────────────────────────────────────

/// The outcome of pushing one exclusive group to its source: the [`SubQuery`] to send, plus
/// the honest record of which FILTER conjuncts were pushed and which were kept local (the
/// residual the local engine must still evaluate). [OPUS-4.8] sq-7byx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedGroup {
    /// The maximal sub-query the source is asked (projection + pushed filters + order/limit).
    pub sub: SubQuery,
    /// Indices (into the `filters` slice passed to [`push_group`]) of the conjuncts pushed
    /// into `sub` — covered by the source's class AND passing the common-variable check.
    pub pushed_filters: Vec<usize>,
    /// Indices of the conjuncts kept **local** (class not covered, or common-variable check
    /// failed) — the residual the local engine evaluates after the source answers.
    pub local_filters: Vec<usize>,
}

/// Build the **maximal evaluable sub-algebra** the group's source can answer (design §4.3).
///
/// `output_vars` are the variables the whole query needs out of this group (its projection +
/// any variable it joins to another group on); the pushed projection is trimmed to exactly
/// the intersection of those with the group's variables — everything else is internal to the
/// group and need not come back. `filters` are the query's FILTER conjuncts; each is pushed
/// **iff** the source's [`Capability::pushable_filters`] class covers it AND it passes
/// [`common_variable_check`] — otherwise it is recorded in `local_filters` for the residual.
/// A conjunct whose expression is not exactly one SPARQL expression over its declared
/// variables (see [`Filter::expr`]) is kept local too.
/// `order_keys` and `limit` are pushed only when the capability's `order_limit` is set.
///
/// The pushed shape depends on the interface:
/// * [`Interface::Endpoint`] / [`Interface::LocalEngine`] → a **multi-pattern** `SELECT
///   <proj> WHERE { tp … FILTER(<expr>) … } ORDER BY <keys> LIMIT <n>` — the whole group as
///   one sub-query;
/// * [`Interface::BrTpf`] / [`Interface::Tpf`] → a fragment server's access unit is **one**
///   triple pattern, so a multi-pattern group is **not** collapsed: `push_group` emits a
///   single-pattern `SELECT` for the group's *first* pattern and pushes **no** filter /
///   order / limit (those capabilities are off for a fragment source); the remaining
///   patterns and the join stay client-side. (The operators phase issues one fragment fetch
///   per pattern; `push_group` is the per-group entry the endpoint path uses fully and the
///   fragment path uses minimally — honest, not an overclaim that a fragment server answers a
///   multi-pattern BGP.)
///
/// Returns `None` when the group is empty, names a pattern index out of range for the BGP,
/// has a pattern position that cannot be written as one SPARQL term (see
/// [`crate::planner::RenderError`]), or has an order key whose variable is not a SPARQL
/// `VARNAME`: fail closed rather than emit a partial or altered query. [OPUS-4.8] sq-7byx.
pub fn push_group(
    group: &ExclusiveGroup,
    bgp: &Bgp,
    cap: &Capability,
    output_vars: &[String],
    filters: &[Filter],
    order_keys: &[OrderKey],
    limit: Option<u64>,
) -> Option<PushedGroup> {
    if group.patterns.is_empty() {
        return None;
    }
    // Resolve the group's patterns up front (fail closed on a bad index).
    let mut pats: Vec<&TriplePattern> = Vec::with_capacity(group.patterns.len());
    for &pi in &group.patterns {
        pats.push(bgp.patterns.get(pi)?);
    }

    let gvars = group_vars(group, bgp);

    // Fragment sources answer ONE pattern; do not collapse a multi-pattern group, and push
    // no filter/order/limit (a fragment server evaluates none of them).
    let fragment = matches!(cap.interface, Interface::BrTpf | Interface::Tpf);

    // --- Filters: push a conjunct iff the source's class covers it AND it passes the exact
    // common-variable check. Fragment sources push none.
    let mut pushed_filters: Vec<usize> = Vec::new();
    let mut local_filters: Vec<usize> = Vec::new();
    for (i, f) in filters.iter().enumerate() {
        let pushable = !fragment
            && class_covers(cap.pushable_filters, f.needs)
            && common_variable_check(f, &gvars)
            && f.expr_is_pushable();
        if pushable {
            pushed_filters.push(i);
        } else {
            local_filters.push(i);
        }
    }

    // --- Projection: keep exactly the group vars the whole query needs out of this group.
    // For a fragment source the sub-query is the FIRST pattern only, so project that
    // pattern's vars that are needed (a fragment SELECT still narrows columns).
    let projectable: Vec<String> = if fragment {
        pattern_vars(pats[0])
    } else {
        gvars.clone()
    };
    // A conjunct evaluated locally needs the group's bindings of its variables too.
    let local_needs = |v: &String| local_filters.iter().any(|&i| filters[i].vars.contains(v));
    let project: Vec<String> = projectable
        .iter()
        .filter(|v| output_vars.iter().any(|o| o == *v) || local_needs(v))
        .cloned()
        .collect();

    // --- Render the pattern block (one pattern for a fragment source, all for an endpoint).
    let body = if fragment {
        render_pattern_clause(pats[0]).ok()?
    } else {
        pats.iter()
            .map(|tp| render_pattern_clause(tp))
            .collect::<Result<Vec<_>, _>>()
            .ok()?
            .join(" . ")
    };

    // --- Projection clause: `SELECT ?a ?b` or `SELECT *` when nothing narrower is requested
    // (an empty `output_vars` means "the caller wants whatever the group projects").
    let proj_clause = if project.is_empty() && output_vars.is_empty() {
        "*".to_string()
    } else if project.is_empty() {
        // The caller asked for vars, none of which this group produces: a `SELECT *` would
        // over-return, so project the empty set explicitly as the group's own vars (the
        // join still reattaches). Keep it precise: project the group's vars.
        let v = if fragment { pattern_vars(pats[0]) } else { gvars };
        if v.is_empty() {
            "*".to_string()
        } else {
            v.iter().map(|n| format!("?{}", n)).collect::<Vec<_>>().join(" ")
        }
    } else {
        project.iter().map(|n| format!("?{}", n)).collect::<Vec<_>>().join(" ")
    };

    // --- FILTER clause(s) for the pushed conjuncts.
    let mut where_clause = body;
    for &i in &pushed_filters {
        where_clause.push_str(&format!(" FILTER({})", filters[i].expr));
    }

    let mut sparql = format!("SELECT {} WHERE {{ {} }}", proj_clause, where_clause);

    // --- ORDER BY / LIMIT only when the capability covers them (never for a fragment source).
    if !fragment && cap.order_limit {
        if !order_keys.is_empty() {
            let keys = order_keys.iter().map(OrderKey::render).collect::<Option<Vec<_>>>()?;
            sparql.push_str(&format!(" ORDER BY {}", keys.join(" ")));
        }
        // A conjunct evaluated locally can drop rows, so the limit is only pushed without one.
        if let (Some(n), true) = (limit, local_filters.is_empty()) {
            sparql.push_str(&format!(" LIMIT {}", n));
        }
    }

    Some(PushedGroup {
        sub: SubQuery {
            sparql,
            project: if project.is_empty() && output_vars.is_empty() {
                // No explicit narrowing requested — let the query's projection stand.
                Vec::new()
            } else {
                project
            },
        },
        pushed_filters,
        local_filters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_fedplan::{SourceCandidate, Var};

    fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
    fn var(s: &str) -> Term {
        Term::Var(Var::new(s))
    }
    fn tp(s: Term, p: Term, o: Term) -> TriplePattern {
        TriplePattern::new(s, p, o)
    }
    /// Build a `PatternSources` selection naming, per pattern, the retained source indices.
    fn sel(per_pattern: &[&[usize]]) -> Vec<PatternSources> {
        per_pattern
            .iter()
            .enumerate()
            .map(|(pi, srcs)| PatternSources {
                pattern: pi,
                candidates: srcs
                    .iter()
                    .map(|&s| SourceCandidate {
                        source: s,
                        estimated_cardinality: 1.0,
                    })
                    .collect(),
            })
            .collect()
    }

    // ─── Exclusive-group derivation ──────────────────────────────────────────────────

    #[test]
    fn exclusive_group_merges_connected_same_source_patterns() {
        // ?s :p ?o  and  ?o :q ?z  — both exclusively source 0, share ?o ⇒ one group.
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/p"), var("o")),
            tp(var("o"), iri("http://ex/q"), var("z")),
        ]);
        let selection = sel(&[&[0], &[0]]);
        let groups = exclusive_groups(&selection, &bgp);
        assert_eq!(
            groups,
            vec![ExclusiveGroup {
                source: 0,
                patterns: vec![0, 1],
            }]
        );
    }

    #[test]
    fn exclusive_group_splits_different_sources() {
        // Same shape but pattern 1 is source 1 ⇒ two singleton groups, no merge.
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/p"), var("o")),
            tp(var("o"), iri("http://ex/q"), var("z")),
        ]);
        let selection = sel(&[&[0], &[1]]);
        let groups = exclusive_groups(&selection, &bgp);
        assert_eq!(
            groups,
            vec![
                ExclusiveGroup {
                    source: 0,
                    patterns: vec![0],
                },
                ExclusiveGroup {
                    source: 1,
                    patterns: vec![1],
                },
            ]
        );
    }

    #[test]
    fn exclusive_group_excludes_multi_and_zero_source_patterns() {
        // p0 exclusive src 0; p1 has TWO sources (cross-source — excluded); p2 zero (excluded).
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/p"), var("o")),
            tp(var("o"), iri("http://ex/q"), var("z")),
            tp(var("z"), iri("http://ex/r"), var("w")),
        ]);
        let selection = sel(&[&[0], &[0, 1], &[]]);
        let groups = exclusive_groups(&selection, &bgp);
        // Only p0 is exclusive; p1 (multi) and p2 (zero) are not in any group.
        assert_eq!(
            groups,
            vec![ExclusiveGroup {
                source: 0,
                patterns: vec![0],
            }]
        );
    }

    #[test]
    fn exclusive_group_does_not_merge_disconnected_same_source() {
        // Two patterns, same source 0, but NO shared variable ⇒ two groups (not one).
        let bgp = Bgp::new(vec![
            tp(var("a"), iri("http://ex/p"), var("b")),
            tp(var("c"), iri("http://ex/q"), var("d")),
        ]);
        let selection = sel(&[&[0], &[0]]);
        let groups = exclusive_groups(&selection, &bgp);
        assert_eq!(
            groups,
            vec![
                ExclusiveGroup {
                    source: 0,
                    patterns: vec![0],
                },
                ExclusiveGroup {
                    source: 0,
                    patterns: vec![1],
                },
            ]
        );
    }

    #[test]
    fn exclusive_group_transitive_chain() {
        // p0—p1 via ?b, p1—p2 via ?c: a transitive chain ⇒ ONE group of all three.
        let bgp = Bgp::new(vec![
            tp(var("a"), iri("http://ex/p"), var("b")),
            tp(var("b"), iri("http://ex/q"), var("c")),
            tp(var("c"), iri("http://ex/r"), var("d")),
        ]);
        let selection = sel(&[&[2], &[2], &[2]]);
        let groups = exclusive_groups(&selection, &bgp);
        assert_eq!(
            groups,
            vec![ExclusiveGroup {
                source: 2,
                patterns: vec![0, 1, 2],
            }]
        );
    }

    // ─── The exact common-variable check ─────────────────────────────────────────────

    #[test]
    fn common_variable_check_pushes_only_group_bound_filters() {
        let group_bound = vec!["s".to_string(), "o".to_string()];
        // All vars bound by the group ⇒ pushable.
        let f_ok = Filter::new(vec!["o".to_string()], "?o > 18", FilterClass::Full);
        assert!(common_variable_check(&f_ok, &group_bound));
        // A var NOT bound by the group (?z lives in a sibling group) ⇒ NOT pushable.
        let f_bad = Filter::new(vec!["z".to_string()], "?z > 18", FilterClass::Full);
        assert!(!common_variable_check(&f_bad, &group_bound));
        // Mixed: one bound, one not ⇒ NOT pushable (ALL vars must be bound).
        let f_mixed = Filter::new(
            vec!["o".to_string(), "z".to_string()],
            "?o > ?z",
            FilterClass::Full,
        );
        assert!(!common_variable_check(&f_mixed, &group_bound));
        // A constant-only conjunct (no vars) is trivially pushable.
        let f_const = Filter::new(vec![], "1 = 1", FilterClass::Equality);
        assert!(common_variable_check(&f_const, &group_bound));
    }

    // ─── Maximal sub-algebra builder (the pushed blocks) ─────────────────────────────

    #[test]
    fn push_group_endpoint_collapses_group_and_pushes_covered_filter() {
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/age"), var("age")),
            tp(var("s"), iri("http://ex/name"), var("name")),
        ]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0, 1],
        };
        let cap = Capability::endpoint();
        // Output: ?name only (?s is a join var also needed → include it).
        let output = vec!["s".to_string(), "name".to_string()];
        let filters = vec![
            // pushable: ?age is group-bound, Full covers it.
            Filter::new(vec!["age".to_string()], "?age > 18", FilterClass::Full),
            // NOT pushable: ?city is not in the group (sibling group var).
            Filter::new(vec!["city".to_string()], "?city = \"NYC\"", FilterClass::Full),
        ];
        let pushed = push_group(&group, &bgp, &cap, &output, &filters, &[], None).unwrap();
        // The whole group is one sub-query, projecting exactly ?s ?name, pushing FILTER(?age>18).
        assert_eq!(
            pushed.sub.sparql,
            "SELECT ?s ?name WHERE { ?s <http://ex/age> ?age . ?s <http://ex/name> ?name FILTER(?age > 18) }"
        );
        assert_eq!(pushed.sub.project, vec!["s".to_string(), "name".to_string()]);
        assert_eq!(pushed.pushed_filters, vec![0]); // first filter pushed
        assert_eq!(pushed.local_filters, vec![1]); // second kept local
    }

    #[test]
    fn push_group_endpoint_pushes_order_limit_when_capability_allows() {
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0],
        };
        let cap = Capability::endpoint(); // order_limit = true
        let pushed = push_group(
            &group,
            &bgp,
            &cap,
            &["s".to_string(), "o".to_string()],
            &[],
            &[OrderKey::asc("o")],
            Some(10),
        )
        .unwrap();
        assert_eq!(
            pushed.sub.sparql,
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o } ORDER BY ?o LIMIT 10"
        );
    }

    #[test]
    fn push_group_does_not_push_order_limit_when_capability_forbids() {
        // brTPF capability has order_limit = false AND is a fragment source.
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0],
        };
        let cap = Capability::brtpf(20);
        let pushed = push_group(
            &group,
            &bgp,
            &cap,
            &["s".to_string(), "o".to_string()],
            &[],
            &[OrderKey::asc("o")],
            Some(10),
        )
        .unwrap();
        // No ORDER/LIMIT pushed (capability forbids).
        assert_eq!(
            pushed.sub.sparql,
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }"
        );
    }

    #[test]
    fn push_group_fragment_source_does_not_collapse_or_filter() {
        // A 2-pattern group on a brTPF source: pushdown emits the FIRST pattern only and
        // pushes NO filter (a fragment server answers one pattern, evaluates no FILTER).
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/p"), var("o")),
            tp(var("o"), iri("http://ex/q"), var("z")),
        ]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0, 1],
        };
        let cap = Capability::brtpf(20);
        let filters = vec![Filter::new(
            vec!["o".to_string()],
            "?o > 18",
            FilterClass::Full,
        )];
        let pushed = push_group(
            &group,
            &bgp,
            &cap,
            &["s".to_string(), "o".to_string(), "z".to_string()],
            &filters,
            &[],
            None,
        )
        .unwrap();
        // First pattern only; no FILTER; the filter is recorded local.
        assert_eq!(
            pushed.sub.sparql,
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }"
        );
        assert!(pushed.pushed_filters.is_empty());
        assert_eq!(pushed.local_filters, vec![0]);
    }

    #[test]
    fn push_group_filter_kept_local_when_class_not_covered() {
        // A plain-TPF-like capability with FilterClass::None never pushes a filter even if
        // its vars are group-bound. (Use an endpoint cap with the filter needing Full but
        // a cap whose class is only Equality to exercise the class check on a non-fragment.)
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/age"), var("age"))]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0],
        };
        let mut cap = Capability::endpoint();
        cap.pushable_filters = FilterClass::Equality; // covers Equality, not Full
        let filters = vec![
            // needs Full > Equality ⇒ NOT pushable (class not covered), kept local.
            Filter::new(vec!["age".to_string()], "?age > 18", FilterClass::Full),
            // needs Equality ⇒ pushable.
            Filter::new(
                vec!["age".to_string()],
                "?age = 18",
                FilterClass::Equality,
            ),
        ];
        let pushed = push_group(
            &group,
            &bgp,
            &cap,
            &["s".to_string(), "age".to_string()],
            &filters,
            &[],
            None,
        )
        .unwrap();
        assert_eq!(pushed.pushed_filters, vec![1]);
        assert_eq!(pushed.local_filters, vec![0]);
        assert_eq!(
            pushed.sub.sparql,
            "SELECT ?s ?age WHERE { ?s <http://ex/age> ?age FILTER(?age = 18) }"
        );
    }

    #[test]
    fn push_group_trims_projection_to_needed_vars() {
        // Group produces ?s ?age ?name; the query needs only ?name → projection is just ?name.
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/age"), var("age")),
            tp(var("s"), iri("http://ex/name"), var("name")),
        ]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0, 1],
        };
        let cap = Capability::endpoint();
        let pushed = push_group(
            &group,
            &bgp,
            &cap,
            &["name".to_string()],
            &[],
            &[],
            None,
        )
        .unwrap();
        assert_eq!(pushed.sub.project, vec!["name".to_string()]);
        assert_eq!(
            pushed.sub.sparql,
            "SELECT ?name WHERE { ?s <http://ex/age> ?age . ?s <http://ex/name> ?name }"
        );
    }

    #[test]
    fn push_group_empty_output_is_select_star() {
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0],
        };
        let cap = Capability::endpoint();
        let pushed = push_group(&group, &bgp, &cap, &[], &[], &[], None).unwrap();
        assert_eq!(
            pushed.sub.sparql,
            "SELECT * WHERE { ?s <http://ex/p> ?o }"
        );
        assert!(pushed.sub.project.is_empty());
    }

    #[test]
    fn push_group_out_of_range_pattern_fails_closed() {
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![9], // out of range
        };
        let cap = Capability::endpoint();
        assert!(push_group(&group, &bgp, &cap, &[], &[], &[], None).is_none());
    }

    #[test]
    fn push_group_with_an_unrenderable_term_fails_closed() {
        // Any pattern in the group that cannot be written as one SPARQL term stops the whole
        // group, for an endpoint (all patterns) and a fragment source (the first pattern).
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/p"), var("o")),
            tp(var("o"), iri("http://ex/q> ?x <http://ex/r"), var("z")),
        ]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0, 1],
        };
        assert!(push_group(&group, &bgp, &Capability::endpoint(), &[], &[], &[], None).is_none());
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![1],
        };
        let cap = Capability::tpf();
        assert!(push_group(&group, &bgp, &cap, &[], &[], &[], None).is_none());
    }

    // ─── Bind-join block primitive ───────────────────────────────────────────────────

    #[test]
    fn bind_block_size_per_capability() {
        assert_eq!(bind_block_size(&Capability::endpoint()), DEFAULT_BIND_BLOCK);
        assert_eq!(bind_block_size(&Capability::brtpf(30)), 30);
        assert_eq!(bind_block_size(&Capability::brtpf(0)), 1); // clamped to >= 1
        assert_eq!(bind_block_size(&Capability::tpf()), 0); // no bind-join
    }

    #[test]
    fn render_values_block_single_and_multi_var() {
        // Single var → short form.
        let single = render_values_block(
            &["j".to_string()],
            &[vec![iri("http://ex/a")], vec![iri("http://ex/b")]],
        );
        assert_eq!(single.unwrap(), "VALUES ?j { <http://ex/a> <http://ex/b> }");
        // Multi var → parenthesised form.
        let multi = render_values_block(
            &["a".to_string(), "b".to_string()],
            &[
                vec![iri("http://ex/a1"), iri("http://ex/b1")],
                vec![iri("http://ex/a2"), iri("http://ex/b2")],
            ],
        );
        assert_eq!(
            multi.unwrap(),
            "VALUES (?a ?b) { (<http://ex/a1> <http://ex/b1>) (<http://ex/a2> <http://ex/b2>) }"
        );
        // Empty inputs → empty block (nothing to push).
        assert_eq!(render_values_block(&[], &[]).unwrap(), "");
        assert_eq!(render_values_block(&["j".to_string()], &[]).unwrap(), "");
    }

    // [OPUS-4.8] sq-qcnn.22: Additional direct unit tests for coverage ratchet
    #[test]
    fn group_vars_collects_in_position_order_dedup() {
        // Verify group_vars collects variables in s→p→o position order with dedup.
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/p"), var("o")),
            tp(var("o"), iri("http://ex/q"), var("z")),
        ]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0, 1],
        };
        let vars = group_vars(&group, &bgp);
        // s from pattern 0 subject, o from pattern 0 object + pattern 1 subject, z from pattern 1 object.
        assert_eq!(vars, vec!["s", "o", "z"]);
    }

    #[test]
    fn group_vars_single_pattern_bound_positions() {
        // Group over a single pattern with one variable; bound positions ignored.
        let bgp = Bgp::new(vec![tp(
            var("x"),
            iri("http://ex/type"),
            iri("http://ex/Person"),
        )]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0],
        };
        let vars = group_vars(&group, &bgp);
        assert_eq!(vars, vec!["x"]);
    }

    #[test]
    fn push_group_single_pattern_fragment_projects_vars() {
        // Fragment source: single-pattern group, select exactly what the group needs.
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0],
        };
        let cap = Capability::brtpf(50);
        let pushed = push_group(
            &group,
            &bgp,
            &cap,
            &["s".to_string(), "o".to_string()],
            &[],
            &[],
            None,
        )
        .unwrap();
        // Fragment: first pattern only; SELECT projects s, o.
        assert_eq!(pushed.sub.project, vec!["s".to_string(), "o".to_string()]);
        assert_eq!(
            pushed.sub.sparql,
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }"
        );
    }

    #[test]
    fn push_group_output_includes_cross_group_join_vars() {
        // Output must include vars needed for joins with other groups, even if not in final projection.
        let bgp = Bgp::new(vec![
            tp(var("s"), iri("http://ex/p1"), var("o1")),
            tp(var("s"), iri("http://ex/p2"), var("o2")),
        ]);
        let group = ExclusiveGroup {
            source: 0,
            patterns: vec![0, 1],
        };
        let cap = Capability::endpoint();
        // output_vars includes both s (cross-group join key) and o1 (final projection var).
        // push_group projects exactly the group vars present in output_vars. [OPUS-4.8] sq-qcnn.22
        let pushed = push_group(
            &group,
            &bgp,
            &cap,
            &["s".to_string(), "o1".to_string()],
            &[],
            &[],
            None,
        )
        .unwrap();
        // Both s and o1 in projection (s is join var, o1 is output).
        assert_eq!(
            pushed.sub.project,
            vec!["s".to_string(), "o1".to_string()]
        );
    }

    #[test]
    fn render_values_block_three_vars() {
        // Three-variable VALUES block: comprehensive test of the multi-var form.
        let block = render_values_block(
            &["x".to_string(), "y".to_string(), "z".to_string()],
            &[
                vec![iri("http://ex/a"), iri("http://ex/b"), iri("http://ex/c")],
                vec![iri("http://ex/d"), iri("http://ex/e"), iri("http://ex/f")],
            ],
        );
        assert_eq!(
            block.unwrap(),
            "VALUES (?x ?y ?z) { (<http://ex/a> <http://ex/b> <http://ex/c>) (<http://ex/d> <http://ex/e> <http://ex/f>) }"
        );
    }

    /// The pushed text parses as one query.
    fn parses(sparql: &str) -> bool {
        spargebra::SparqlParser::new().parse_query(sparql).is_ok()
    }

    #[test]
    fn values_cells_are_rendered_as_terms_or_refused() {
        let lit = |s: &str| Term::Literal(s.to_string());
        // Literals are re-serialised strictly; the block parses and keeps one row per tuple.
        let block = render_values_block(
            &["a".to_string(), "b".to_string()],
            &[vec![iri("http://ex/a"), lit("x } ; y")], vec![iri("http://ex/b"), lit("\"z\"@en")]],
        )
        .unwrap();
        assert_eq!(block, r#"VALUES (?a ?b) { (<http://ex/a> "x } ; y") (<http://ex/b> "z"@en) }"#);
        assert!(parses(&format!("SELECT * WHERE {{ {block} }}")));
        // Anything that is not one bound term per variable is refused.
        let one = |cells: Vec<Term>| render_values_block(&["a".to_string()], &[cells]);
        assert!(matches!(one(vec![iri("http://ex/a> } ; <x")]), Err(RenderError::Iri(_))));
        assert!(matches!(one(vec![lit("\"a\" } ; \"b\"")]), Err(RenderError::Literal(_))));
        assert!(matches!(one(vec![var("x")]), Err(RenderError::Values(_))));
        assert!(matches!(one(vec![]), Err(RenderError::Values(_))));
        assert!(matches!(one(vec![iri("http://ex/a"), iri("http://ex/b")]), Err(RenderError::Values(_))));
        assert!(matches!(
            render_values_block(&["a }".to_string()], &[vec![iri("http://ex/a")]]),
            Err(RenderError::Var(_))
        ));
    }

    #[test]
    fn order_keys_are_variables_with_a_direction() {
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup { source: 0, patterns: vec![0] };
        let cap = Capability::endpoint();
        let out = ["s".to_string()];
        let pushed =
            push_group(&group, &bgp, &cap, &out, &[], &[OrderKey::desc("o"), OrderKey::asc("s")], Some(3))
                .unwrap();
        assert_eq!(pushed.sub.sparql, "SELECT ?s WHERE { ?s <http://ex/p> ?o } ORDER BY DESC(?o) ?s LIMIT 3");
        assert!(parses(&pushed.sub.sparql));
        // A key that is not a variable name fails closed.
        assert!(push_group(&group, &bgp, &cap, &out, &[], &[OrderKey::asc("o } LIMIT 1")], None).is_none());
    }

    #[test]
    fn filters_push_only_as_one_expression_over_their_declared_variables() {
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup { source: 0, patterns: vec![0] };
        let f = |vars: &[&str], expr: &str| {
            Filter::new(vars.iter().map(|v| v.to_string()).collect(), expr, FilterClass::Full)
        };
        let filters = [
            f(&["o"], "?o > 1"),
            // Not exactly one expression.
            f(&["o"], "?o > 1) FILTER(true"),
            f(&["o"], "?o > 1) } SELECT * WHERE { ?a ?b ?c"),
            f(&["o"], "?o >"),
            f(&["o"], "?o > 1) AS ?y) FILTER(true) BIND((1"),
            // A variable the conjunct does not declare.
            f(&["o"], "?o = ?s"),
            // EXISTS.
            f(&["o"], "?o > 1 || EXISTS { ?a ?b ?c }"),
            // A code-point escape an endpoint would expand before parsing.
            f(&["o"], "?o != \"\\u0022) FILTER(true) #\""),
        ];
        let pushed =
            push_group(&group, &bgp, &Capability::endpoint(), &["s".to_string()], &filters, &[], None).unwrap();
        assert_eq!(pushed.pushed_filters, vec![0]);
        assert_eq!(pushed.local_filters, vec![1, 2, 3, 4, 5, 6, 7]);
        // The local conjuncts need `?o`, so it stays projected.
        assert_eq!(pushed.sub.sparql, "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o FILTER(?o > 1) }");
        assert!(parses(&pushed.sub.sparql));
    }

    #[test]
    fn a_local_filter_keeps_its_variables_and_the_limit_local() {
        let bgp = Bgp::new(vec![tp(var("s"), iri("http://ex/p"), var("o"))]);
        let group = ExclusiveGroup { source: 0, patterns: vec![0] };
        let out = ["s".to_string()];
        let keys = [OrderKey::asc("o")];
        let local = [Filter::new(vec!["o".to_string()], "?o > 1 && EXISTS { ?o ?p ?q }", FilterClass::Full)];
        let pushed = push_group(&group, &bgp, &Capability::endpoint(), &out, &local, &keys, Some(1)).unwrap();
        assert_eq!(pushed.local_filters, vec![0]);
        assert_eq!(pushed.sub.sparql, "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o } ORDER BY ?o");
        assert_eq!(pushed.sub.project, vec!["s".to_string(), "o".to_string()]);
        // With every conjunct pushed, the projection stays narrow and the limit is pushed.
        let remote = [Filter::new(vec!["o".to_string()], "?o > 1", FilterClass::Full)];
        let pushed = push_group(&group, &bgp, &Capability::endpoint(), &out, &remote, &keys, Some(1)).unwrap();
        assert_eq!(pushed.sub.sparql, "SELECT ?s WHERE { ?s <http://ex/p> ?o FILTER(?o > 1) } ORDER BY ?o LIMIT 1");
    }

    #[test]
    fn class_covers_hierarchy() {
        // FilterClass hierarchy: None < Equality < Full.
        assert!(class_covers(FilterClass::Full, FilterClass::Equality));
        assert!(class_covers(FilterClass::Full, FilterClass::Full));
        assert!(class_covers(FilterClass::Equality, FilterClass::Equality));
        assert!(!class_covers(FilterClass::Equality, FilterClass::Full));
        assert!(!class_covers(FilterClass::None, FilterClass::Equality));
        assert!(class_covers(FilterClass::Full, FilterClass::None));
    }
}
