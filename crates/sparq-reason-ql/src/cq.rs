// sq-t5bne (epic sq-pbz04): the STRICT, FAIL-CLOSED CQ-shape gate.
// sq-pbz04.3.1: broaden the sound input fragment — UCQ (B1), literal-object atoms
// (B2 is in emit.rs), FILTER on distinguished-only vars (B3), constant-only VALUES over
// distinguished vars (B4), intensional-atom guard (B6).
//
// PerfectRef (and every OWL 2 QL query-rewriting algorithm) is sound + complete only for
// (unions of) CONJUNCTIVE QUERIES — the Pérez–Arenas–Gutiérrez "AND-only" fragment with an
// outer projection/distinct. Firing the rewrite on anything else SILENTLY LOSES OR INVENTS
// answers (the research records call this the #1 unsoundness trap: applicability is the
// load-bearing condition). So this gate is the soundness keystone of the whole crate: it is
// the FIRST thing every public rewriting entry point runs, and it REJECTS — never silently
// degrades — any query outside the CQ fragment as [`CqError::OutOfScope`].
//
// Crucially this module carries NO `cfg(feature = "experimental")`: the gate is always
// compiled and always cheap (a structural walk of the parsed algebra, no TBox, no rewriting),
// so the fail-closed classification is available even to a caller that has NOT enabled the
// rewriter. The rewriter (behind `experimental`) calls into the gate; it can never bypass it.
//
// ## Broadened shapes (sq-pbz04.3.1) and their soundness arguments
//
// B1. UCQ (top-level UNION of CQ branches): PerfectRef is defined over UCQs in the source
//     paper (Calvanese et al., JAR 2007). Each branch is rewired independently and the results
//     unioned. Certain answers distribute over union in DL-Lite_R. FAIL-CLOSED: each branch
//     must independently pass the CQ gate; a branch leaving a projected variable unbound is
//     rejected because UCQ answer-arity is uniform.
//
// B3. FILTER on distinguished-only variables: For q'(x) = q(x) ∧ F(x) with vars(F) ⊆
//     distinguished, certain answers are ground bindings of the answer vars, and F reads only
//     those. The rewriter passes F through unmodified — SPARQL evaluation semantics apply on
//     both sides of the rewrite. FAIL-CLOSED: a FILTER mentioning any non-distinguished
//     variable is rejected (its value on an anonymous witness is undefined).
//
// B4. Constant-only VALUES over distinguished vars: A VALUES block whose vars are all
//     distinguished and whose rows are fully bound constants is equivalent to a disjunction
//     of equality filters over answer variables. B3's argument applies verbatim. FAIL-CLOSED:
//     UNDEF cells or non-distinguished variables → rejected.
//
// B6. Intensional-atom guard (a deliberate NARROWING): the gate currently admits role atoms
//     whose predicate is semantics-bearing schema vocabulary (rdfs:subClassOf,
//     rdfs:subPropertyOf, rdfs:domain, rdfs:range, and the owl: axiom vocabulary). The
//     rewriter evaluates those over ABox triples only — silently MISSING TBox-entailed schema
//     facts (reflexivity, transitive closure, entailed subsumptions). That violates the
//     completeness half of the certain-answer contract. Fix: such atoms are now REJECTED as
//     `OutOfScope("intensional/schema-vocabulary atom")`. Annotation predicates
//     (rdfs:label/comment/seeAlso/isDefinedBy) remain admitted as plain role atoms.
//
// B5. Non-recursive property-path desugaring (sq-pbz04.3.2, design record §5 B5). The three
//     recursion-free path constructors are SPARQL-1.1-algebra query equivalences, applied to the
//     peeled body BEFORE the entailment rewrite (after which B1/PerfectRef soundness applies to
//     the result):
//       - sequence      `X p1/p2 Y  ≡  X p1 ?f . ?f p2 Y`   (fresh NON-distinguished `?f`);
//       - inverse       `X ^p Y     ≡  Y p X`               (subject/object swap);
//       - alternation   `X p1|p2 Y  ≡  { X p1 Y } UNION { X p2 Y }`  (branch multiplication
//                                                            into the B1 UCQ machinery).
//     STEP 0 finding (verified against the vendored spargebra parser, `add_to_triple_or_path_patterns`):
//     the PARSER already lowers a top-level SEQUENCE to a BGP joined by a fresh blank node, and a
//     top-level INVERSE of a simple predicate to a swapped BGP triple — so those never reach the
//     gate as a `Path`; the emitter already lifts the blank-node intermediate to a fresh
//     existential (sq-pbz04.3.6). The ONLY non-recursive form that survives parsing as a
//     `GraphPattern::Path` is ALTERNATION (whose arms may nest sequence/inverse/named-node), so
//     [`desugar_paths_dnf`] desugars exactly those surviving `Path` nodes — it does NOT
//     re-translate what the parser already normalised (no double-translation). The fresh
//     sequence intermediate is a SHARED variable across its two atoms, so PerfectRef's
//     applicability condition correctly treats it as bound (identical to the blank-node path).
//     `+` / `*` / `?` (zero-length-path semantics) and negated property sets stay
//     PERMANENT-REJECT (fail-closed) — they reintroduce the recursion/reflexivity QL excludes.
//
// ## Unchanged PERMANENT-REJECT shapes
//
// OPTIONAL (non-monotone), MINUS (non-monotone), recursive / zero-length / negated property
// paths (`+`/`*`/`?`/`!p` — reintroduce recursion/reflexivity), GRAPH (named-graph rewriting
// semantics unsettled), BIND/Extend (arithmetic, not CQ), aggregation, SERVICE, sub-SELECT,
// LATERAL, RDF-star quoted triples, variable predicates — all stay REJECTED with their
// original reason strings.

use spargebra::algebra::{GraphPattern, PropertyPathExpression};
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;
use std::collections::HashSet;

/// Why a query is not a conjunctive query the QL rewriter may soundly handle. Carries a
/// human-readable reason naming the offending construct, so a caller (or the conformance
/// harness) can surface an honest `OutOfScope(reason)` rather than a misleading wrong answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CqError {
    /// The query is outside the conjunctive-query fragment the rewriter is sound for. The
    /// string names the disqualifying construct (e.g. `"OPTIONAL (LeftJoin)"`).
    OutOfScope(String),
}

impl std::fmt::Display for CqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Positional `format!` arg (NOT `{reason}`) — avoids the CodeQL
            // rust/unused-variable false positive flagged in the shared contract.
            CqError::OutOfScope(reason) => write!(f, "query out of QL rewriting scope: {}", reason),
        }
    }
}

impl std::error::Error for CqError {}

/// A conjunctive query in the shape the rewriter consumes: a flat conjunction of triple
/// patterns (the body atoms) under a set of DISTINGUISHED (answer / projected) variables.
/// Additionally may carry a pass-through filter expression and a constant-binding VALUES
/// block — see the broadening in sq-pbz04.3.1.
///
/// Built only by [`as_conjunctive_query`] or [`as_ucq`], which have already proven via the
/// fail-closed gate that the source is a genuine CQ. The `distinguished` set is the SELECT
/// projection (or, for ASK, empty — a boolean CQ); every other variable in `atoms` is
/// non-distinguished (existentially quantified), which is exactly the distinction
/// PerfectRef's applicability condition turns on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConjunctiveQuery {
    /// The projected (answer) variables — the distinguished variables of the CQ.
    pub distinguished: Vec<Variable>,
    /// The body: a conjunction of triple patterns.
    pub atoms: Vec<TriplePattern>,
    /// Whether the original query was DISTINCT (preserved so the rewriter can re-wrap).
    pub distinct: bool,
    /// Whether the original query was an ASK (boolean CQ — `distinguished` is then empty).
    pub ask: bool,
    /// SPARQL filter expressions from the CQ body that mention ONLY distinguished variables
    /// (B3: passed through unmodified, applied post-rewrite). Empty when no FILTER was present.
    ///
    /// These are `spargebra::algebra::Expression` values, but to keep the type opaque in the
    /// public surface (and avoid an implicit spargebra dep in downstream code that only calls
    /// `as_conjunctive_query`) we store them as opaque `GraphPattern::Filter` wrappers that
    /// the emitter unwraps. Stored here as the raw `spargebra::algebra::Expression` string for
    /// now; the emitter re-applies them by wrapping the UCQ body in `Filter { expr, inner }`.
    pub filter_exprs: Vec<spargebra::algebra::Expression>,
    /// Constant-only VALUES bindings over distinguished variables (B4: passed through
    /// unmodified, semantically equivalent to equality filters, applied post-rewrite).
    pub values_blocks: Vec<ValuesBlock>,
}

/// A constant-only VALUES block accepted by the B4 broadening: all variables are
/// distinguished and all cells are fully bound to ground terms (no UNDEF).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValuesBlock {
    /// The variables bound in this VALUES block (all distinguished, already checked).
    pub variables: Vec<Variable>,
    /// The rows: each row is one binding per variable. All cells are ground terms (no UNDEF).
    pub bindings: Vec<Vec<GroundTerm>>,
}

/// A UCQ (union of conjunctive queries) as the gate classifies it.
///
/// The outermost shape is a UNION of CQ branches, each independently passing the CQ gate.
/// The distinguished variables (projection) are at the UCQ level — uniform across all
/// branches (a branch leaving a projected variable unbound is rejected, fail-closed).
///
/// Built only by [`as_ucq`], which has already proven via the fail-closed gate that the
/// source query is a genuine UCQ. Single-branch UCQs are representable (and the
/// `as_conjunctive_query` convenience wrapper builds them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ucq {
    /// The projected (answer) variables — uniform across all branches.
    pub distinguished: Vec<Variable>,
    /// Whether the original SELECT was DISTINCT.
    pub distinct: bool,
    /// Whether the original query was an ASK.
    pub ask: bool,
    /// The CQ branches. Never empty (a single CQ is a one-branch UCQ).
    pub branches: Vec<ConjunctiveQuery>,
}

/// Classify `query` and, if it is a conjunctive query the rewriter may soundly handle, return
/// its [`ConjunctiveQuery`] form; otherwise return [`CqError::OutOfScope`] naming the
/// disqualifying construct. FAIL-CLOSED: anything not recognised as plainly a CQ is rejected.
///
/// Accepted shape (and ONLY this shape): `SELECT [DISTINCT] ?vars WHERE { conjunction of
/// triple patterns [, FILTER over distinguished-only vars] [, VALUES over distinguished vars
/// with constant rows] }`, or the same body under `ASK`. The body may be a single `Bgp` or a
/// tree of `Join`s whose leaves are all `Bgp`s — both reduce to one flat conjunction. Also
/// accepted (B3): a `Filter` wrapping a CQ body, when the filter expression mentions only
/// distinguished variables. Also accepted (B4): a `Values` block over distinguished variables
/// with fully-bound rows. Everything else is out:
///
/// - `LeftJoin` (OPTIONAL), `Minus`, `Union`, `Lateral`, `Group`/aggregation,
///   `Service`, `Graph`, `Extend` (BIND), `OrderBy`, `Slice`, `Path` (property paths) — none
///   has a certain-answer-preserving UCQ rewriting;
/// - CONSTRUCT / DESCRIBE — not a CQ-answering shape;
/// - a triple pattern whose predicate is a VARIABLE, or any RDF-star quoted-triple term — both
///   outside DL-Lite atom shape;
/// - a FILTER mentioning a non-distinguished variable — value on an anonymous witness is
///   undefined, so the filter semantics do not translate to certain answers (fail-closed B3);
/// - a VALUES block with UNDEF cells or non-distinguished variables — not equivalent to a
///   constant filter over answer vars (fail-closed B4);
/// - a triple pattern whose predicate is semantics-bearing schema vocabulary
///   (`rdfs:subClassOf`, `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`, `owl:` axiom
///   vocabulary) — the rewriter evaluates over ABox data only, so an intensional atom would
///   silently under-answer without this guard (fail-closed B6).
pub fn as_conjunctive_query(query: &Query) -> Result<ConjunctiveQuery, CqError> {
    let ucq = as_ucq(query)?;
    // A plain CQ is a one-branch UCQ; reject multi-branch (use `as_ucq` directly for those).
    if ucq.branches.len() != 1 {
        return Err(CqError::OutOfScope(
            "query is a UCQ (multiple UNION branches); use as_ucq for UCQ input".into(),
        ));
    }
    Ok(ucq.branches.into_iter().next().unwrap())
}

/// Classify `query` into a [`Ucq`] — a union of conjunctive queries. A plain CQ yields a
/// one-branch UCQ. FAIL-CLOSED: every branch must independently pass the CQ gate (including
/// the B6 intensional-atom guard) and must bind every distinguished variable (uniform arity).
///
/// Soundness argument (B1): PerfectRef is defined over UCQs (Calvanese et al., JAR 2007).
/// Certain answers distribute over union in DL-Lite_R via the canonical-model property.
/// Per-branch rewriting and then union is sound and complete.
///
/// This is the primary gate the rewriter calls. `as_conjunctive_query` is a convenience
/// wrapper that additionally enforces the single-branch restriction.
pub fn as_ucq(query: &Query) -> Result<Ucq, CqError> {
    let (pattern, ask) = match query {
        Query::Select { pattern, .. } => (pattern, false),
        Query::Ask { pattern, .. } => (pattern, true),
        Query::Construct { .. } => {
            return Err(CqError::OutOfScope("CONSTRUCT is not a CQ-answering query".into()))
        }
        Query::Describe { .. } => {
            return Err(CqError::OutOfScope("DESCRIBE is not a CQ-answering query".into()))
        }
    };

    // Peel the allowed outer modifiers: Project and Distinct/Reduced.
    let mut distinct = false;
    let mut distinguished: Option<Vec<Variable>> = None;
    let mut body = pattern;
    loop {
        match body {
            GraphPattern::Project { inner, variables } => {
                if distinguished.is_none() {
                    distinguished = Some(variables.clone());
                }
                body = inner;
            }
            GraphPattern::Distinct { inner } => {
                distinct = true;
                body = inner;
            }
            GraphPattern::Reduced { inner } => {
                distinct = true;
                body = inner;
            }
            _ => break,
        }
    }

    let distinguished = if ask {
        Vec::new()
    } else {
        distinguished.ok_or_else(|| {
            CqError::OutOfScope("SELECT without a projection (unsupported shape)".into())
        })?
    };

    // B5 (sq-pbz04.3.2): desugar non-recursive property paths (`/`, `^`, `|`) and split the
    // top-level UNION tree, in ONE DNF pass, into a list of path-free conjunction bodies. This
    // subsumes the old top-level-union split: `desugar_paths_dnf` handles `Union` (concatenate
    // disjuncts), alternation `Path` nodes (branch multiplication), sequence/inverse nested in
    // an alternation arm (fresh non-distinguished intermediate / swap), and distributes `Join`
    // and `Filter` over unions so each resulting disjunct is a plain conjunction. It is a NO-OP
    // (single unchanged clone) for a path-free, single-branch body — so all existing CQ/UCQ
    // shapes are preserved exactly. Recursive/zero-length/negated path forms are REJECTED
    // fail-closed here. Non-conjunctive operators (OPTIONAL/MINUS/GRAPH/…) pass through unchanged
    // and are rejected downstream by `collect_conjunction` with their precise reason strings.
    let mut fresh = FreshVars::new(collect_body_var_names_owned(body));
    let disjuncts = desugar_paths_dnf(body, &mut fresh)?;

    let mut branches: Vec<ConjunctiveQuery> = Vec::with_capacity(disjuncts.len());
    for branch in &disjuncts {
        let cq = parse_cq_body(branch, &distinguished, distinct, ask)?;
        branches.push(cq);
    }

    if branches.is_empty() {
        return Err(CqError::OutOfScope("UCQ has no branches".into()));
    }

    Ok(Ucq {
        distinguished,
        distinct,
        ask,
        branches,
    })
}

/// A cap on the number of DNF disjuncts (UCQ branches) that property-path desugaring +
/// union-splitting may produce, before the gate rejects fail-closed. Alternation multiplies
/// branches (`2^k` for `k` independent 2-way alternations, more for `n`-way), so a bound keeps
/// a pathological path from blowing up the rewrite. Chosen well above any realistic CQ; a query
/// exceeding it is out of the sound-and-tractable fragment. sq-pbz04.3.2
const MAX_UCQ_BRANCHES: usize = 256;

/// A generator of FRESH non-distinguished variables for property-path sequence intermediates
/// (B5). Names use the distinctive `__ql_seq_` prefix and are collision-checked against every
/// variable name in the source query, so a fresh intermediate can never capture a user variable
/// (nor be projected — it is never added to the distinguished set). sq-pbz04.3.2
struct FreshVars {
    used: HashSet<String>,
    counter: usize,
}

impl FreshVars {
    fn new(used: HashSet<String>) -> Self {
        Self { used, counter: 0 }
    }

    /// A fresh variable whose name is present in neither the source query nor any earlier
    /// fresh name. The intermediate is inherently non-distinguished (never projected).
    fn next(&mut self) -> Variable {
        loop {
            let name = format!("__ql_seq_{}", self.counter);
            self.counter += 1;
            if self.used.insert(name.clone()) {
                return Variable::new_unchecked(name);
            }
        }
    }
}

/// Collect every variable NAME appearing anywhere in `p` (all triple/path/values positions and
/// filter expressions), recursing through the whole pattern tree, so [`FreshVars`] can avoid
/// capturing any of them. Best-effort-thorough: an unrecognised operator is still traversed via
/// its child patterns where the shape is known. sq-pbz04.3.2
fn collect_body_var_names(p: &GraphPattern, out: &mut HashSet<String>) {
    let push_term = |t: &TermPattern, out: &mut HashSet<String>| {
        if let TermPattern::Variable(v) = t {
            out.insert(v.as_str().to_string());
        }
    };
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                push_term(&tp.subject, out);
                if let NamedNodePattern::Variable(v) = &tp.predicate {
                    out.insert(v.as_str().to_string());
                }
                push_term(&tp.object, out);
            }
        }
        GraphPattern::Path { subject, object, .. } => {
            push_term(subject, out);
            push_term(object, out);
        }
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Lateral { left, right } => {
            collect_body_var_names(left, out);
            collect_body_var_names(right, out);
        }
        GraphPattern::Filter { expr, inner } => {
            for v in expression_vars(expr) {
                out.insert(v.as_str().to_string());
            }
            collect_body_var_names(inner, out);
        }
        GraphPattern::Extend { inner, variable, expression } => {
            out.insert(variable.as_str().to_string());
            for v in expression_vars(expression) {
                out.insert(v.as_str().to_string());
            }
            collect_body_var_names(inner, out);
        }
        GraphPattern::Values { variables, .. } => {
            for v in variables {
                out.insert(v.as_str().to_string());
            }
        }
        GraphPattern::Project { inner, variables } => {
            for v in variables {
                out.insert(v.as_str().to_string());
            }
            collect_body_var_names(inner, out);
        }
        GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Graph { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. } => {
            collect_body_var_names(inner, out);
        }
    }
}

/// Convenience wrapper for [`FreshVars::new`] that first collects the query's variable names.
fn collect_body_var_names_owned(p: &GraphPattern) -> HashSet<String> {
    let mut out = HashSet::new();
    collect_body_var_names(p, &mut out);
    out
}

/// Desugar all non-recursive property paths in `body` and split the top-level `UNION` tree, in
/// ONE pass, into the DNF DISJUNCT LIST — each disjunct a path-free conjunction body (a `Bgp`,
/// or a `Join`/`Filter`/`Values` tree of them) with NO top-level `Union`. FAIL-CLOSED on
/// recursive/zero-length/negated path forms. sq-pbz04.3.2 (design record §5 B5)
///
/// Distribution identities applied (all SPARQL-algebra equivalences, certain-answer preserving
/// by the B1 union argument): `Join(A, Union(B,C)) ≡ Union(Join(A,B), Join(A,C))` and
/// `Filter(e, Union(B,C)) ≡ Union(Filter(e,B), Filter(e,C))`. A path-free, union-free body maps
/// to a single unchanged disjunct, so every previously-accepted CQ/UCQ shape is preserved.
///
/// Non-conjunctive operators (OPTIONAL/MINUS/GRAPH/BIND/aggregation/SERVICE/ORDER BY/Slice/
/// nested Project/Distinct/LATERAL) are returned UNCHANGED as a single disjunct — the downstream
/// [`collect_conjunction`] walk then rejects them with their precise reason strings (we do NOT
/// recurse into them: the whole query is out of scope regardless of any path nested inside).
fn desugar_paths_dnf(body: &GraphPattern, fresh: &mut FreshVars) -> Result<Vec<GraphPattern>, CqError> {
    match body {
        // A BGP carries no property paths (spargebra lowers simple predicates + top-level
        // sequence/inverse into BGP triples during parsing). Return it unchanged; any blank-node
        // path intermediate the parser introduced is lifted to a fresh existential by the emitter.
        GraphPattern::Bgp { .. } => Ok(vec![body.clone()]),

        // A surviving `Path` node is a non-recursive alternation (whose arms may nest
        // sequence/inverse/named-node) — or a recursive/negated form that must be rejected.
        GraphPattern::Path { subject, path, object } => desugar_path(subject, path, object, fresh),

        // Join distributes over the union produced by desugaring either side (DNF).
        GraphPattern::Join { left, right } => {
            let dl = desugar_paths_dnf(left, fresh)?;
            let dr = desugar_paths_dnf(right, fresh)?;
            let product = dl.len().saturating_mul(dr.len());
            if product > MAX_UCQ_BRANCHES {
                return Err(too_many_branches());
            }
            let mut out = Vec::with_capacity(product);
            for a in &dl {
                for b in &dr {
                    out.push(GraphPattern::Join {
                        left: Box::new(a.clone()),
                        right: Box::new(b.clone()),
                    });
                }
            }
            Ok(out)
        }

        // Union: concatenate the disjunct lists (top-level UCQ split + alternation-induced union).
        GraphPattern::Union { left, right } => {
            let mut out = desugar_paths_dnf(left, fresh)?;
            out.extend(desugar_paths_dnf(right, fresh)?);
            if out.len() > MAX_UCQ_BRANCHES {
                return Err(too_many_branches());
            }
            Ok(out)
        }

        // Filter distributes over the union produced by desugaring its inner pattern. When the
        // inner has a single disjunct (the common no-path case) this re-wraps it unchanged, so a
        // plain distinguished-only FILTER keeps behaving exactly as before. When it splits (a
        // path alternation under a FILTER), each branch carries the filter — behaving identically
        // to a hand-written `{ … FILTER } UNION { … FILTER }` (the existing multi-branch-modifier
        // emitter guard in lib.rs then rejects it fail-closed).
        GraphPattern::Filter { expr, inner } => {
            let di = desugar_paths_dnf(inner, fresh)?;
            Ok(di
                .into_iter()
                .map(|d| GraphPattern::Filter {
                    expr: expr.clone(),
                    inner: Box::new(d),
                })
                .collect())
        }

        // VALUES carries only ground terms (no paths, no unions) — a single unchanged disjunct.
        GraphPattern::Values { .. } => Ok(vec![body.clone()]),

        // Everything else is outside the conjunctive fragment. Return UNCHANGED as one disjunct
        // so `collect_conjunction` rejects it with its precise message (fail-closed preserved).
        _ => Ok(vec![body.clone()]),
    }
}

/// Reject a query whose path desugaring exceeds [`MAX_UCQ_BRANCHES`] (fail-closed).
fn too_many_branches() -> CqError {
    CqError::OutOfScope(format!(
        "property-path / UNION desugaring produced more than {} UCQ branches — out of the \
         tractable CQ fragment (fail-closed)",
        MAX_UCQ_BRANCHES
    ))
}

/// Desugar a single property-path atom `subject PATH object` into a DNF disjunct list — each
/// disjunct a `Bgp` or `Join` of `Bgp`s. Introduces a fresh NON-distinguished variable for each
/// sequence intermediate (shared across the two atoms it joins), swaps subject/object for an
/// inverse, and multiplies alternation into separate disjuncts (branch multiplication). The
/// recursion-reintroducing forms — `+` (`OneOrMore`), `*` (`ZeroOrMore`), `?` (`ZeroOrOne`) —
/// and negated property sets (`!p`) are REJECTED fail-closed. sq-pbz04.3.2
fn desugar_path(
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
    fresh: &mut FreshVars,
) -> Result<Vec<GraphPattern>, CqError> {
    match path {
        // Named predicate → a single role/class triple `subject p object`.
        PropertyPathExpression::NamedNode(p) => {
            let tp = TriplePattern {
                subject: subject.clone(),
                predicate: NamedNodePattern::NamedNode(p.clone()),
                object: object.clone(),
            };
            Ok(vec![GraphPattern::Bgp { patterns: vec![tp] }])
        }
        // Inverse: `X ^p Y ≡ Y p X` — swap subject/object and recurse. Spargebra already lowers a
        // TOP-LEVEL inverse of a simple predicate; this arm fires for an inverse nested inside an
        // alternation arm (e.g. `^:p1 | :p2`).
        PropertyPathExpression::Reverse(inner) => desugar_path(object, inner, subject, fresh),
        // Sequence: `X a/b Y ≡ X a ?f . ?f b Y` with a fresh non-distinguished `?f` SHARED across
        // the two atoms (so PerfectRef's applicability condition treats it as bound). Fires for a
        // sequence nested inside an alternation arm (a top-level sequence is lowered by spargebra).
        PropertyPathExpression::Sequence(a, b) => {
            let mid = TermPattern::Variable(fresh.next());
            let da = desugar_path(subject, a, &mid, fresh)?;
            let db = desugar_path(&mid, b, object, fresh)?;
            let product = da.len().saturating_mul(db.len());
            if product > MAX_UCQ_BRANCHES {
                return Err(too_many_branches());
            }
            let mut out = Vec::with_capacity(product);
            for x in &da {
                for y in &db {
                    out.push(GraphPattern::Join {
                        left: Box::new(x.clone()),
                        right: Box::new(y.clone()),
                    });
                }
            }
            Ok(out)
        }
        // Alternation: `X a|b Y ≡ { X a Y } UNION { X b Y }` — concatenate the two disjunct lists
        // (branch multiplication into the B1 UCQ machinery).
        PropertyPathExpression::Alternative(a, b) => {
            let mut out = desugar_path(subject, a, object, fresh)?;
            out.extend(desugar_path(subject, b, object, fresh)?);
            if out.len() > MAX_UCQ_BRANCHES {
                return Err(too_many_branches());
            }
            Ok(out)
        }
        // `+` / `*` / `?` — recursion / zero-length-path semantics QL deliberately excludes.
        // The reason string carries "property path" so the fail-closed classification is stable.
        PropertyPathExpression::OneOrMore(_)
        | PropertyPathExpression::ZeroOrMore(_)
        | PropertyPathExpression::ZeroOrOne(_) => Err(CqError::OutOfScope(
            "recursive / zero-length property path (+, *, ?) reintroduces the recursion / \
             reflexivity OWL 2 QL excludes — out of scope (fail-closed)"
                .into(),
        )),
        // Negated property set `!p` — not a DL-Lite atom (it is a complement over predicates).
        PropertyPathExpression::NegatedPropertySet(_) => Err(CqError::OutOfScope(
            "negated property set (!p) in a property path is not a DL-Lite atom — out of scope \
             (fail-closed)"
                .into(),
        )),
    }
}

/// Parse one UCQ branch as a CQ body, collecting atoms and (optionally) FILTER expressions
/// and VALUES blocks. Returns a `ConjunctiveQuery` with the branch's body.
///
/// `top_distinguished` is the outer UCQ's projected variables — used for the B3/B4 guard
/// (filter/values must only mention those). `distinct` and `ask` are UCQ-level metadata.
fn parse_cq_body(
    body: &GraphPattern,
    top_distinguished: &[Variable],
    distinct: bool,
    ask: bool,
) -> Result<ConjunctiveQuery, CqError> {
    let mut atoms = Vec::new();
    let mut filter_exprs = Vec::new();
    let mut values_blocks = Vec::new();

    collect_conjunction(body, top_distinguished, &mut atoms, &mut filter_exprs, &mut values_blocks)?;

    if atoms.is_empty() {
        return Err(CqError::OutOfScope("empty query body (no triple patterns)".into()));
    }

    // B1 uniform-arity check: every projected variable must appear in this branch.
    // An ASK has no projection, so the check is vacuous.
    if !ask {
        for v in top_distinguished {
            let in_atoms = atoms.iter().any(|tp| triple_mentions_var(tp, v));
            // Also allow the variable to appear in a values block binding (it is bound there).
            let in_values = values_blocks.iter().any(|vb| vb.variables.contains(v));
            if !in_atoms && !in_values {
                return Err(CqError::OutOfScope(format!(
                    "UCQ branch does not bind projected variable ?{} (non-uniform arity is not sound)",
                    v.as_str()
                )));
            }
        }
    }

    Ok(ConjunctiveQuery {
        distinguished: top_distinguished.to_vec(),
        atoms,
        distinct,
        ask,
        filter_exprs,
        values_blocks,
    })
}

/// Whether a triple pattern mentions a given variable in any position (subject, predicate, object).
fn triple_mentions_var(tp: &TriplePattern, v: &Variable) -> bool {
    let in_subj = matches!(&tp.subject, TermPattern::Variable(u) if u == v);
    let in_pred = matches!(&tp.predicate, NamedNodePattern::Variable(u) if u == v);
    let in_obj = matches!(&tp.object, TermPattern::Variable(u) if u == v);
    in_subj || in_pred || in_obj
}

/// Walk a CQ body, flattening nested `Join`s into one conjunction of triple patterns, and
/// collecting (B3) FILTER expressions and (B4) VALUES blocks that pass the distinguished-only
/// guard. Any non-conjunctive operator encountered is a hard `OutOfScope` rejection (fail-closed).
fn collect_conjunction(
    p: &GraphPattern,
    distinguished: &[Variable],
    atoms: &mut Vec<TriplePattern>,
    filter_exprs: &mut Vec<spargebra::algebra::Expression>,
    values_blocks: &mut Vec<ValuesBlock>,
) -> Result<(), CqError> {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                check_atom_shape(tp)?;
                atoms.push(tp.clone());
            }
            Ok(())
        }
        GraphPattern::Join { left, right } => {
            collect_conjunction(left, distinguished, atoms, filter_exprs, values_blocks)?;
            collect_conjunction(right, distinguished, atoms, filter_exprs, values_blocks)
        }
        // B3: FILTER — accept iff ALL variables in the expression are distinguished.
        GraphPattern::Filter { expr, inner } => {
            // First collect the inner CQ body.
            collect_conjunction(inner, distinguished, atoms, filter_exprs, values_blocks)?;
            // Check that every variable in expr is a distinguished variable.
            let expr_vars = expression_vars(expr);
            for v in &expr_vars {
                if !distinguished.iter().any(|d| d == v) {
                    return Err(CqError::OutOfScope(format!(
                        "FILTER expression mentions non-distinguished variable ?{} \
                         (value on an anonymous witness is undefined — fail-closed B3)",
                        v.as_str()
                    )));
                }
            }
            // Pass-through: store the expression for the emitter.
            filter_exprs.push(expr.clone());
            Ok(())
        }
        // B4: VALUES — accept iff all vars are distinguished AND all rows fully bound.
        GraphPattern::Values { variables, bindings } => {
            // Guard: all variables must be distinguished.
            for v in variables {
                if !distinguished.iter().any(|d| d == v) {
                    return Err(CqError::OutOfScope(format!(
                        "inline VALUES binds non-distinguished variable ?{} \
                         (not equivalent to a constant filter — fail-closed B4)",
                        v.as_str()
                    )));
                }
            }
            // Guard: no UNDEF cells.
            let mut rows: Vec<Vec<GroundTerm>> = Vec::with_capacity(bindings.len());
            for row in bindings {
                let mut bound_row: Vec<GroundTerm> = Vec::with_capacity(row.len());
                for cell in row {
                    match cell {
                        Some(gt) => bound_row.push(gt.clone()),
                        None => {
                            return Err(CqError::OutOfScope(
                                "inline VALUES has UNDEF cell (not equivalent to a constant filter — fail-closed B4)".into()
                            ));
                        }
                    }
                }
                rows.push(bound_row);
            }
            values_blocks.push(ValuesBlock {
                variables: variables.clone(),
                bindings: rows,
            });
            Ok(())
        }
        // Every other operator is outside the conjunctive fragment — REJECT, naming it.
        GraphPattern::LeftJoin { .. } => {
            Err(CqError::OutOfScope("OPTIONAL (LeftJoin) is not conjunctive".into()))
        }
        GraphPattern::Union { .. } => {
            // A Union inside the body (after the top-level branch split) is nested UNION —
            // not a plain top-level UCQ pattern; reject as non-conjunctive.
            Err(CqError::OutOfScope("nested UNION inside a CQ branch is not conjunctive".into()))
        }
        GraphPattern::Minus { .. } => Err(CqError::OutOfScope("MINUS is not conjunctive".into())),
        GraphPattern::Path { .. } => Err(CqError::OutOfScope(
            "property path is not conjunctive (reintroduces recursion QL excludes)".into(),
        )),
        GraphPattern::Graph { .. } => {
            Err(CqError::OutOfScope("named-graph (GRAPH) is out of scope".into()))
        }
        GraphPattern::Extend { .. } => {
            Err(CqError::OutOfScope("BIND (Extend) is not conjunctive".into()))
        }
        GraphPattern::Group { .. } => {
            Err(CqError::OutOfScope("aggregation (GROUP BY) is not conjunctive".into()))
        }
        GraphPattern::Service { .. } => {
            Err(CqError::OutOfScope("SERVICE (federation) is out of scope".into()))
        }
        GraphPattern::OrderBy { .. } => {
            Err(CqError::OutOfScope("ORDER BY inside the body is out of scope".into()))
        }
        GraphPattern::Slice { .. } => {
            Err(CqError::OutOfScope("LIMIT/OFFSET inside the body is out of scope".into()))
        }
        GraphPattern::Distinct { .. } | GraphPattern::Reduced { .. } => Err(CqError::OutOfScope(
            "nested DISTINCT/REDUCED is out of scope".into(),
        )),
        GraphPattern::Project { .. } => {
            Err(CqError::OutOfScope("nested sub-SELECT is out of scope".into()))
        }
        // `Lateral` exists because the vendored spargebra is built with `sep-0006` (the
        // workspace dep enables it). Match it unconditionally.
        GraphPattern::Lateral { .. } => {
            Err(CqError::OutOfScope("LATERAL is not conjunctive".into()))
        }
    }
}

/// A triple pattern is a DL-Lite-shaped atom only if its predicate is a CONSTANT IRI (a class
/// atom `?x rdf:type A` or a role atom `?x R ?y` with `R` a named property) and neither subject
/// nor object is an RDF-star quoted triple. A variable predicate (`?x ?p ?y`) cannot be matched
/// to a DL-Lite inclusion, so the query is out of scope (fail-closed, not silently un-rewritten).
///
/// B6 guard: a predicate that is SEMANTICS-BEARING schema vocabulary (rdfs:subClassOf,
/// rdfs:subPropertyOf, rdfs:domain, rdfs:range, and the owl: axiom vocabulary) is REJECTED
/// as `OutOfScope("intensional/schema-vocabulary atom")` — the rewriter evaluates over ABox
/// data only and would silently under-answer a schema-vocabulary query without this guard.
/// Annotation predicates (rdfs:label/comment/seeAlso/isDefinedBy) are admitted as plain role
/// atoms (no QL TBox axiom changes their extension). sq-pbz04.3.1
///
/// Body blank-node lifting (sq-pbz04.3.6): blank nodes in subject/object positions are now
/// ADMITTED — they are non-distinguished existentials and are lifted to fresh `Unbound` ids by
/// the emitter (`emit::cq_to_atoms`). The gate does not reject them; the emitter handles the
/// mapping with the correct freshness/sharing semantics. sq-pbz04.3.6
fn check_atom_shape(tp: &TriplePattern) -> Result<(), CqError> {
    use spargebra::term::NamedNodePattern;
    match &tp.predicate {
        NamedNodePattern::Variable(_) => {
            return Err(CqError::OutOfScope(
                "triple pattern with a variable predicate is not a DL-Lite atom".into(),
            ));
        }
        NamedNodePattern::NamedNode(n) => {
            // B6: reject intensional schema-vocabulary predicates.
            if is_intensional_schema_predicate(n.as_str()) {
                return Err(CqError::OutOfScope(format!(
                    "intensional/schema-vocabulary atom: predicate <{}> is semantics-bearing \
                     schema vocabulary; the rewriter evaluates over ABox data only and would \
                     silently under-answer this query (fail-closed B6)",
                    n.as_str()
                )));
            }
        }
    }
    if is_quoted_triple(&tp.subject) || is_quoted_triple(&tp.object) {
        return Err(CqError::OutOfScope(
            "RDF-star quoted-triple term is out of DL-Lite scope".into(),
        ));
    }
    Ok(())
}

/// Returns `true` if `pred_iri` is a semantics-bearing schema predicate that the QL rewriter
/// evaluates over the ABox only (and would therefore silently under-answer a query using it).
///
/// ADMITTED annotation predicates: `rdfs:label`, `rdfs:comment`, `rdfs:seeAlso`,
/// `rdfs:isDefinedBy` — no QL TBox axiom changes their extension, so the rewriter can treat
/// them as ordinary role atoms.
///
/// REJECTED schema predicates (the B6 intensional-atom guard): rdfs:subClassOf,
/// rdfs:subPropertyOf, rdfs:domain, rdfs:range, and the owl: vocabulary (owl:inverseOf,
/// owl:equivalentClass, owl:equivalentProperty, owl:disjointWith, owl:propertyDisjointWith,
/// owl:complementOf, owl:onProperty, owl:someValuesFrom, owl:allValuesFrom, owl:unionOf,
/// owl:intersectionOf, owl:oneOf, owl:hasValue, and any other owl: predicate).
fn is_intensional_schema_predicate(iri: &str) -> bool {
    const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
    const OWL: &str = "http://www.w3.org/2002/07/owl#";
    const ADMITTED_RDFS: &[&str] = &["label", "comment", "seeAlso", "isDefinedBy"];

    if let Some(local) = iri.strip_prefix(RDFS) {
        // Annotations are admitted; everything else (subClassOf, subPropertyOf, domain, range,
        // member, …) is schema vocabulary the rewriter must not evaluate over ABox data.
        !ADMITTED_RDFS.contains(&local)
    } else {
        // Any owl: predicate is schema vocabulary (B6).
        iri.starts_with(OWL)
    }
}

fn is_quoted_triple(t: &TermPattern) -> bool {
    // `TermPattern::Triple` exists because the vendored spargebra is built with `sparql-12`
    // (the workspace dep enables it). A `cfg(feature = "sparql-12")` here would test THIS
    // crate's features, not spargebra's, so match the variant directly.
    matches!(t, TermPattern::Triple(_))
}

/// Collect all `Variable`s mentioned in a SPARQL expression (shallow recursive walk). Used by
/// the B3 FILTER guard to verify the filter only mentions distinguished variables.
fn expression_vars(expr: &spargebra::algebra::Expression) -> Vec<Variable> {
    let mut out = Vec::new();
    collect_expr_vars(expr, &mut out);
    out
}

fn collect_expr_vars(expr: &spargebra::algebra::Expression, out: &mut Vec<Variable>) {
    use spargebra::algebra::Expression;
    match expr {
        Expression::Variable(v) => out.push(v.clone()),
        Expression::Or(a, b) | Expression::And(a, b) | Expression::Equal(a, b)
        | Expression::SameTerm(a, b) | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b) | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b) | Expression::Add(a, b) | Expression::Subtract(a, b)
        | Expression::Multiply(a, b) | Expression::Divide(a, b) => {
            collect_expr_vars(a, out);
            collect_expr_vars(b, out);
        }
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
            collect_expr_vars(a, out);
        }
        Expression::FunctionCall(_, args) => {
            for arg in args {
                collect_expr_vars(arg, out);
            }
        }
        Expression::In(e, es) => {
            collect_expr_vars(e, out);
            for sub in es {
                collect_expr_vars(sub, out);
            }
        }
        Expression::Bound(v) => {
            out.push(v.clone());
        }
        Expression::Exists(p) => {
            // Exists sub-pattern — collect vars from it. This is not a CQ shape and will have
            // been rejected at the body-walk level, but we walk it defensively.
            collect_pattern_vars(p, out);
        }
        Expression::If(cond, then_, else_) => {
            collect_expr_vars(cond, out);
            collect_expr_vars(then_, out);
            collect_expr_vars(else_, out);
        }
        Expression::Coalesce(es) => {
            for sub in es {
                collect_expr_vars(sub, out);
            }
        }
        // Literals, named nodes — no variables.
        Expression::NamedNode(_) | Expression::Literal(_) => {}
    }
}

/// Collect variables mentioned at the top level of a `GraphPattern` (not recursive into
/// sub-selects). Used only by the `Exists` arm of `collect_expr_vars` as a defensive walk.
fn collect_pattern_vars(p: &GraphPattern, out: &mut Vec<Variable>) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                if let TermPattern::Variable(v) = &tp.subject {
                    out.push(v.clone());
                }
                if let NamedNodePattern::Variable(v) = &tp.predicate {
                    out.push(v.clone());
                }
                if let TermPattern::Variable(v) = &tp.object {
                    out.push(v.clone());
                }
            }
        }
        GraphPattern::Join { left, right } => {
            collect_pattern_vars(left, out);
            collect_pattern_vars(right, out);
        }
        _ => {} // defensive: don't recurse into sub-selects / aggregations
    }
}

// Convenience: check whether `query` is a single conjunctive query (not a UCQ).
// Does NOT reject UCQs — callers that need to distinguish should use `as_ucq`.
// This is the original public entry point; it now returns `OutOfScope` for multi-branch UCQs.
// NOTE: this is re-exported as `crate::as_conjunctive_query` from lib.rs.

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::SparqlParser;

    fn parse(q: &str) -> Query {
        SparqlParser::new().parse_query(q).expect("parse")
    }

    fn accepts(q: &str) -> ConjunctiveQuery {
        as_conjunctive_query(&parse(q)).expect("should be a CQ")
    }

    fn accepts_ucq(q: &str) -> Ucq {
        as_ucq(&parse(q)).expect("should be a UCQ")
    }

    fn rejects(q: &str) -> String {
        match as_conjunctive_query(&parse(q)) {
            Err(CqError::OutOfScope(r)) => r,
            Ok(_) => panic!("expected OutOfScope rejection for: {}", q),
        }
    }

    fn rejects_ucq(q: &str) -> String {
        match as_ucq(&parse(q)) {
            Err(CqError::OutOfScope(r)) => r,
            Ok(_) => panic!("expected UCQ OutOfScope rejection for: {}", q),
        }
    }

    const PRE: &str = "PREFIX : <http://ex/> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>";

    // ---- existing acceptance cases (must not regress) ----

    #[test]
    fn accepts_single_bgp_select() {
        let cq = accepts(&format!("{PRE} SELECT ?x WHERE {{ ?x rdf:type :A }}"));
        assert_eq!(cq.atoms.len(), 1);
        assert_eq!(cq.distinguished.len(), 1);
        assert!(!cq.ask);
    }

    #[test]
    fn accepts_multi_atom_conjunction() {
        let cq = accepts(&format!(
            "{PRE} SELECT ?x ?y WHERE {{ ?x rdf:type :A . ?x :r ?y }}"
        ));
        assert_eq!(cq.atoms.len(), 2);
    }

    #[test]
    fn accepts_distinct() {
        let cq = accepts(&format!("{PRE} SELECT DISTINCT ?x WHERE {{ ?x rdf:type :A }}"));
        assert!(cq.distinct);
    }

    #[test]
    fn accepts_ask() {
        let cq = accepts(&format!("{PRE} ASK WHERE {{ ?x rdf:type :A }}"));
        assert!(cq.ask);
        assert!(cq.distinguished.is_empty());
    }

    // ---- existing rejection cases (must not regress) ----

    #[test]
    fn rejects_optional() {
        assert!(rejects(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A OPTIONAL {{ ?x :r ?y }} }}"
        ))
        .contains("OPTIONAL"));
    }

    #[test]
    fn rejects_minus() {
        assert!(rejects(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A MINUS {{ ?x rdf:type :B }} }}"
        ))
        .contains("MINUS"));
    }

    #[test]
    fn rejects_property_path() {
        assert!(rejects(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x :r+ ?y }}"
        ))
        .contains("property path"));
    }

    #[test]
    fn rejects_bind() {
        assert!(rejects(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A BIND(:B AS ?y) }}"
        ))
        .contains("BIND"));
    }

    #[test]
    fn rejects_aggregation() {
        let r = rejects(&format!(
            "{PRE} SELECT (COUNT(?x) AS ?n) WHERE {{ ?x rdf:type :A }}"
        ));
        assert!(
            r.contains("aggregation") || r.contains("BIND") || r.contains("GROUP"),
            "aggregation query must be rejected as non-conjunctive, got: {}",
            r
        );
    }

    #[test]
    fn rejects_bare_group_by() {
        let r = rejects(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A }} GROUP BY ?x"
        ));
        assert!(
            r.contains("aggregation") || r.contains("GROUP"),
            "GROUP BY must be rejected as non-conjunctive, got: {}",
            r
        );
    }

    #[test]
    fn rejects_variable_predicate() {
        assert!(rejects(&format!("{PRE} SELECT ?x WHERE {{ ?x ?p :A }}"))
            .contains("variable predicate"));
    }

    #[test]
    fn rejects_construct() {
        assert!(rejects(&format!(
            "{PRE} CONSTRUCT {{ ?x rdf:type :A }} WHERE {{ ?x rdf:type :A }}"
        ))
        .contains("CONSTRUCT"));
    }

    // ---- B1: UCQ acceptance ----

    #[test]
    fn ucq_two_branch_accepted() {
        // { ?x a :A } UNION { ?x a :B } — a two-branch UCQ over the same projected var ?x.
        let ucq = accepts_ucq(&format!(
            "{PRE} SELECT ?x WHERE {{ {{ ?x rdf:type :A }} UNION {{ ?x rdf:type :B }} }}"
        ));
        assert_eq!(ucq.branches.len(), 2);
        assert_eq!(ucq.distinguished.len(), 1);
    }

    #[test]
    fn ucq_three_branch_accepted() {
        let ucq = accepts_ucq(&format!(
            "{PRE} SELECT ?x WHERE {{ {{ ?x rdf:type :A }} UNION {{ ?x rdf:type :B }} UNION {{ ?x rdf:type :C }} }}"
        ));
        assert_eq!(ucq.branches.len(), 3);
    }

    #[test]
    fn as_conjunctive_query_rejects_multi_branch_ucq() {
        // as_conjunctive_query must reject a UCQ — the caller must use as_ucq.
        let q = parse(&format!(
            "{PRE} SELECT ?x WHERE {{ {{ ?x rdf:type :A }} UNION {{ ?x rdf:type :B }} }}"
        ));
        match as_conjunctive_query(&q) {
            Err(CqError::OutOfScope(r)) => {
                assert!(r.contains("UCQ") || r.contains("UNION"), "expected UCQ rejection, got: {}", r);
            }
            Ok(_) => panic!("as_conjunctive_query must reject a multi-branch UCQ"),
        }
    }

    // ---- B3: FILTER on distinguished variables ----

    #[test]
    fn filter_on_distinguished_var_accepted() {
        // ?x is projected (distinguished) so the filter is pass-through sound.
        let cq = accepts(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A FILTER(?x != :Bad) }}"
        ));
        assert_eq!(cq.atoms.len(), 1);
        assert_eq!(cq.filter_exprs.len(), 1, "filter expression must be collected");
    }

    #[test]
    fn filter_on_non_distinguished_var_rejected() {
        // ?y is NOT projected — its value on an anonymous witness is undefined (B3 fail-closed).
        let r = rejects(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A . ?x :r ?y FILTER(?y != :Bad) }}"
        ));
        assert!(
            r.contains("non-distinguished") || r.contains("FILTER"),
            "non-distinguished FILTER must be rejected; got: {}",
            r
        );
    }

    // ---- B4: constant-only VALUES over distinguished vars ----

    #[test]
    fn values_over_distinguished_with_constants_accepted() {
        // ?x is projected; VALUES binds it to two constants — equivalent to equality filter.
        let cq = accepts(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A VALUES ?x {{ :C1 :C2 }} }}"
        ));
        assert_eq!(cq.atoms.len(), 1);
        assert_eq!(cq.values_blocks.len(), 1, "VALUES block must be collected");
        assert_eq!(cq.values_blocks[0].bindings.len(), 2);
    }

    #[test]
    fn values_over_non_distinguished_var_rejected() {
        // ?y is NOT projected — VALUES over it is not equivalent to a constant filter (B4 fail-closed).
        let r = rejects(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A . ?x :r ?y VALUES ?y {{ :C }} }}"
        ));
        assert!(
            r.contains("non-distinguished") || r.contains("VALUES"),
            "non-distinguished VALUES must be rejected; got: {}",
            r
        );
    }

    // Note: UNDEF in VALUES — spargebra parses UNDEF as `None` in the binding row.
    // We verify the rejection path in broadened_shapes.rs integration tests.

    // ---- B6: intensional-atom guard ----

    #[test]
    fn rdfs_sub_class_of_predicate_rejected() {
        // ?c rdfs:subClassOf :A — intensional schema vocabulary (B6 fail-closed).
        let r = rejects(
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> PREFIX : <http://ex/> \
             SELECT ?c WHERE { ?c rdfs:subClassOf :A }",
        );
        assert!(
            r.contains("intensional") || r.contains("schema"),
            "rdfs:subClassOf predicate must be rejected as intensional; got: {}",
            r
        );
    }

    #[test]
    fn rdfs_sub_property_of_predicate_rejected() {
        let r = rejects(
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> PREFIX : <http://ex/> \
             SELECT ?p WHERE { ?p rdfs:subPropertyOf :r }",
        );
        assert!(
            r.contains("intensional") || r.contains("schema"),
            "rdfs:subPropertyOf predicate must be rejected as intensional; got: {}",
            r
        );
    }

    #[test]
    fn rdfs_domain_predicate_rejected() {
        let r = rejects(
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> PREFIX : <http://ex/> \
             SELECT ?p WHERE { ?p rdfs:domain :A }",
        );
        assert!(
            r.contains("intensional") || r.contains("schema"),
            "rdfs:domain predicate must be rejected as intensional; got: {}",
            r
        );
    }

    #[test]
    fn owl_inverse_of_predicate_rejected() {
        let r = rejects(
            "PREFIX owl: <http://www.w3.org/2002/07/owl#> PREFIX : <http://ex/> \
             SELECT ?r WHERE { ?r owl:inverseOf :s }",
        );
        assert!(
            r.contains("intensional") || r.contains("schema"),
            "owl:inverseOf predicate must be rejected as intensional; got: {}",
            r
        );
    }

    #[test]
    fn rdfs_annotation_predicates_admitted() {
        // rdfs:label and rdfs:comment are annotation predicates — admitted as plain role atoms.
        let cq = accepts(
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> PREFIX : <http://ex/> \
             SELECT ?x ?label WHERE { ?x rdfs:label ?label }",
        );
        assert_eq!(cq.atoms.len(), 1, "rdfs:label must be admitted as a plain role atom");
    }

    #[test]
    fn rdfs_see_also_admitted() {
        let cq = accepts(
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> PREFIX : <http://ex/> \
             SELECT ?x ?ref WHERE { ?x rdfs:seeAlso ?ref }",
        );
        assert_eq!(cq.atoms.len(), 1, "rdfs:seeAlso must be admitted as a plain role atom");
    }

    #[test]
    fn ucq_with_intensional_branch_rejected() {
        // B6 fail-closed: a UCQ branch containing rdfs:subClassOf must be rejected.
        let r = rejects_ucq(
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
             PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
             PREFIX : <http://ex/> \
             SELECT ?x WHERE { { ?x rdfs:subClassOf :A } UNION { ?x rdf:type :B } }",
        );
        assert!(
            r.contains("intensional") || r.contains("schema"),
            "UCQ with intensional branch must be rejected; got: {}",
            r
        );
    }

    // ---- B6 mutation-kill guard: existing FILTER rejection must still fire before B6 check ----

    #[test]
    fn filter_on_distinguished_vars_only_does_not_trigger_b6() {
        // A plain FILTER on the distinguished var ?x — passes the B3 guard.
        let cq = accepts(&format!(
            "{PRE} SELECT ?x WHERE {{ ?x rdf:type :A FILTER(?x != :Bad) }}"
        ));
        assert!(!cq.filter_exprs.is_empty());
    }
}
