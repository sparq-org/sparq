// AUTHORED-BY Claude Opus 4.8
//! The Solid **N3 Patch** engine (`text/n3`).
//!
//! Implements the Solid Protocol N3 Patch document shape
//! (<https://solidproject.org/TR/protocol#n3-patch>): a single `solid:InsertDeletePatch` resource
//! with `solid:inserts`, `solid:deletes`, and `solid:where` formula objects (each a `{ … }` graph).
//! The patch is applied to the target resource's existing graph and the result re-serialised.
//!
//! ## What this implements
//!
//! - **`solid:inserts` + `solid:deletes`** (concrete triples, no variables): the delete-then-insert
//!   set operations over the target graph, with the spec's preconditions (every `deletes` triple
//!   must be present; an `InsertDeletePatch` must carry at least one of inserts/deletes).
//! - **`solid:where`** (the templated/conditional form): the variable solver. The `where` formula is
//!   a basic graph pattern (BGP) of triple *patterns* (terms may be SPARQL variables **or
//!   blank-node existentials** — see below). It is matched against the target's graph by conjunctive
//!   unification; the **single** resulting binding substitutes into the `inserts`/`deletes`
//!   templates, which are then applied. See the precise spec-faithful solution-count rule below.
//!
//! ## Spec-faithful solution-count semantics (the load-bearing call)
//!
//! The Solid Protocol §N3 Patch is explicit and unambiguous: *"If `?conditions` is non-empty, find
//! all (possibly empty) variable mappings such that all of the resulting triples occur in the
//! dataset. **If no such mapping exists, or if multiple mappings exist, the server MUST respond with
//! a `409` status code.**"* So a non-empty `where` requires **exactly one** solution — **zero** and
//! **more-than-one** are *both* a `409 Conflict`, NOT a no-op and NOT a per-solution fan-out. (This
//! differs from a SPARQL `DELETE/INSERT … WHERE`, which fans out over every solution; the Solid N3
//! Patch deliberately does not.) The where's templates therefore apply exactly once, with the unique
//! binding. An *empty* `where { }` constrains nothing and the patch must be fully concrete.
//!
//! ## Static (parse-time) constraints, per spec
//!
//! - The `inserts`/`deletes` formulae **MUST NOT contain variables that do not occur in the
//!   `where` formula** — a free (unbound) variable in a template is a structural error (`422`), never
//!   silently left as a variable in the output graph.
//! - The `inserts`/`deletes` formulae **MUST NOT contain blank nodes** (`422`). A blank node in the
//!   `where` pattern is fine, but it is an **existential variable** scoped to that formula — `_:x`
//!   means "there exists some term", matches like a variable, and two `_:x` occurrences in the same
//!   `where` must bind to the same term (a join). It does NOT match only its parser-generated id.
//!   (Concrete blank nodes in the *target* graph are matched as usual when a pattern names them via
//!   a variable/existential.)
//!
//! Per the house rule, the patch document is parsed with the vetted `oxttl` N3 parser (formulas →
//! blank-node-named graphs), never hand-parsed.

use std::collections::HashMap;

use oxrdf::{GraphName, NamedOrBlankNode, Term, Triple, Variable};
use oxttl::n3::{N3Parser, N3Quad, N3Term};

use crate::error::ServerError;

/// The Solid terms vocabulary IRIs used by an N3 Patch.
const SOLID_INSERTS: &str = "http://www.w3.org/ns/solid/terms#inserts";
const SOLID_DELETES: &str = "http://www.w3.org/ns/solid/terms#deletes";
const SOLID_WHERE: &str = "http://www.w3.org/ns/solid/terms#where";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const SOLID_INSERT_DELETE_PATCH: &str = "http://www.w3.org/ns/solid/terms#InsertDeletePatch";

/// A term in a triple pattern: a concrete RDF term, a (named) SPARQL variable, or a `where`-scoped
/// existential (a blank node in the `solid:where` formula).
///
/// Used uniformly for the `where`, `inserts`, and `deletes` formulae. A formula with no variables
/// collapses to all-`Term` patterns, which is exactly the concrete (un-templated) patch case.
///
/// ## Why blank nodes in `where` are existentials, not concrete terms
///
/// In N3-Patch (and SPARQL) a blank node in a graph PATTERN is an existential variable: `_:x`
/// means "there exists some term", scoped to that formula, and repeated occurrences of the same
/// label within the formula must bind to the **same** term (a join). It must NOT match only the
/// parser-generated concrete blank-node id. We therefore lift each `where`-clause blank node into a
/// dedicated [`PatTerm::WhereBlank`] existential, keyed by its (formula-local) label, so the BGP
/// solver treats it like a variable. Blank nodes in the `inserts`/`deletes` templates remain
/// forbidden (a parse-time `422`), so this variant only ever originates from a `where` formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatTerm {
    /// A concrete RDF term (named node, blank node, or literal).
    Term(Term),
    /// A SPARQL variable (`?x`) — to be bound by the `where` solver.
    Var(Variable),
    /// A `where`-scoped existential, from a blank node in the `solid:where` formula. The held
    /// string is the blank node's label (e.g. `x` for `_:x`); two occurrences of the same label in
    /// one `where` formula are the same existential and must unify to the same term.
    WhereBlank(String),
}

/// A triple pattern (subject/predicate/object, each possibly a variable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatTriple {
    pub subject: PatTerm,
    pub predicate: PatTerm,
    pub object: PatTerm,
}

/// A parsed N3 Patch: the `where` basic graph pattern plus the delete/insert templates.
///
/// When `where` is empty, `deletes`/`inserts` are guaranteed (by parse-time validation) to be
/// variable-free, i.e. concrete triples.
#[derive(Debug, Default, Clone)]
pub struct N3Patch {
    /// The `solid:where` basic graph pattern (empty ⇒ an unconditional, fully-concrete patch).
    pub conditions: Vec<PatTriple>,
    /// The `solid:deletes` template.
    pub deletes: Vec<PatTriple>,
    /// The `solid:inserts` template.
    pub inserts: Vec<PatTriple>,
}

/// Parse a `text/n3` Solid N3 Patch document, resolving relative IRIs against `base_iri`.
///
/// Returns the where/delete/insert templates. A document that is not a well-formed single
/// `InsertDeletePatch`, or that violates a static spec constraint (an unbound template variable, a
/// blank node in a template), is an error (4xx) — never silently dropped.
pub fn parse_n3_patch(body: &[u8], base_iri: &str) -> Result<N3Patch, ServerError> {
    let parser = N3Parser::new()
        .with_base_iri(base_iri)
        .map_err(|e| ServerError::BadRequest(format!("invalid base IRI: {e}")))?;

    let mut quads: Vec<N3Quad> = Vec::new();
    for q in parser.for_slice(body) {
        let q = q.map_err(|e| ServerError::UnprocessablePatch(format!("invalid N3: {e}")))?;
        quads.push(q);
    }

    // Partition into the default graph (the patch description) and the formula graphs (the `{ … }`
    // blocks, each a distinct blank-node graph name).
    let mut description: Vec<&N3Quad> = Vec::new();
    for q in &quads {
        if q.graph_name == GraphName::DefaultGraph {
            description.push(q);
        }
    }

    // Find the patch resource's formula references: subject patch -> solid:inserts/deletes/where ->
    // a formula (blank-node graph name). Reject a patch that names an unknown predicate shape.
    let mut inserts_formula: Option<GraphName> = None;
    let mut deletes_formula: Option<GraphName> = None;
    let mut where_formula: Option<GraphName> = None;
    let mut declared_patch_type = false;

    for q in &description {
        let pred_iri = match &q.predicate {
            N3Term::NamedNode(n) => n.as_str(),
            // A variable/blank predicate in the patch description is not a valid patch shape.
            _ => {
                return Err(ServerError::UnprocessablePatch(
                    "patch description predicate must be an IRI".into(),
                ))
            }
        };
        match pred_iri {
            SOLID_INSERTS => inserts_formula = Some(formula_graph(&q.object)?),
            SOLID_DELETES => deletes_formula = Some(formula_graph(&q.object)?),
            SOLID_WHERE => where_formula = Some(formula_graph(&q.object)?),
            RDF_TYPE => {
                if matches!(&q.object, N3Term::NamedNode(n) if n.as_str() == SOLID_INSERT_DELETE_PATCH)
                {
                    declared_patch_type = true;
                }
            }
            // Any other predicate on the patch description is ignored (forward-compat), but we do not
            // accept a patch that declares NONE of inserts/deletes (checked below).
            _ => {}
        }
    }

    // The type triple is conventional; we do not hard-require it (some clients omit it), but if NO
    // patch operation is present at all the document is not an actionable patch.
    let _ = declared_patch_type;

    // The `where` BGP: triple patterns, variables allowed (blank nodes allowed here per spec).
    let conditions = match where_formula {
        Some(g) => pattern_triples(&quads, &g, FormulaKind::Where)?,
        None => Vec::new(),
    };
    // The deletes/inserts TEMPLATES: triple patterns, variables allowed but blank nodes forbidden.
    let deletes = match deletes_formula {
        Some(g) => pattern_triples(&quads, &g, FormulaKind::Template)?,
        None => Vec::new(),
    };
    let inserts = match inserts_formula {
        Some(g) => pattern_triples(&quads, &g, FormulaKind::Template)?,
        None => Vec::new(),
    };

    if deletes.is_empty() && inserts.is_empty() {
        return Err(ServerError::UnprocessablePatch(
            "an N3 patch must specify at least one of solid:inserts / solid:deletes".into(),
        ));
    }

    // Spec: the inserts/deletes formulae MUST NOT contain variables that do not occur in `where`.
    // A free template variable can never be bound ⇒ a 422 structural error, not a runtime surprise.
    let where_vars = collect_vars(&conditions);
    for t in deletes.iter().chain(inserts.iter()) {
        for v in pat_triple_vars(t) {
            if !where_vars.contains(v.as_str()) {
                return Err(ServerError::UnprocessablePatch(format!(
                    "variable ?{} appears in solid:inserts/deletes but not in solid:where",
                    v.as_str()
                )));
            }
        }
    }

    Ok(N3Patch {
        conditions,
        deletes,
        inserts,
    })
}

/// Apply a parsed patch to the target's existing triples, returning the new triple set.
///
/// Semantics (Solid-Protocol-aligned):
///
/// 1. If `where` (conditions) is non-empty, solve the BGP against `existing`. Per spec, there MUST be
///    **exactly one** solution: **zero** or **multiple** solutions ⇒ `409 Conflict`.
/// 2. Substitute the unique binding into the `deletes`/`inserts` templates. (An empty `where` ⇒ the
///    templates are already concrete, used as-is with the empty binding.)
/// 3. Every resulting `deletes` triple MUST be present in `existing` (else `409 Conflict` — a
///    precondition violation, not a silent no-op); they are removed, then `inserts` are added.
///
/// The result preserves the surviving triples' relative order and appends new inserts, de-duplicated;
/// the set semantics make a repeated apply idempotent.
///
/// `oxrdf::Triple` is `Eq + Hash` but NOT `Ord`, so set membership is a linear scan over the (small)
/// graph rather than a `BTreeSet`; for a single resource's triples this is correct and adequate.
pub fn apply_patch(existing: &[Triple], patch: &N3Patch) -> Result<Vec<Triple>, ServerError> {
    // 1+2: resolve the (single) binding and instantiate the templates into concrete triples.
    let binding = if patch.conditions.is_empty() {
        // No conditions: the templates are already concrete (validated at parse time). Use the empty
        // binding; instantiation is then a 1:1 conversion of every PatTerm::Term.
        Binding::default()
    } else {
        let solutions = solve_bgp(&patch.conditions, existing);
        match solutions.len() {
            1 => solutions.into_iter().next().unwrap(),
            // Spec: zero OR multiple mappings ⇒ 409 (NOT a no-op, NOT a per-solution fan-out).
            0 => {
                return Err(ServerError::Conflict(
                    "the solid:where clause has no solution against the target graph".into(),
                ))
            }
            _ => {
                return Err(ServerError::Conflict(
                    "the solid:where clause has multiple solutions; exactly one is required".into(),
                ))
            }
        }
    };

    let deletes = instantiate_all(&patch.deletes, &binding)?;
    let inserts = instantiate_all(&patch.inserts, &binding)?;

    // 3: every delete must be present (a missing one is a 409, not a silent no-op).
    for d in &deletes {
        if !existing.contains(d) {
            return Err(ServerError::Conflict(
                "a solid:deletes triple is not present in the resource".into(),
            ));
        }
    }

    // Keep the survivors (everything not in `deletes`), preserving order + de-duplicating.
    let mut result: Vec<Triple> = Vec::with_capacity(existing.len() + inserts.len());
    for t in existing {
        if deletes.contains(t) {
            continue;
        }
        if !result.contains(t) {
            result.push(t.clone());
        }
    }
    // Append inserts that are not already present (set union).
    for i in &inserts {
        if !result.contains(i) {
            result.push(i.clone());
        }
    }
    Ok(result)
}

// --- the BGP solver ----------------------------------------------------------------------------

/// A variable binding: variable-name → concrete RDF term.
type Binding = HashMap<String, Term>;

/// Solve a basic graph pattern (conjunction of triple patterns) against the graph, returning every
/// distinct, complete variable mapping under which all pattern triples occur in `graph`.
///
/// This is a straight backtracking join: for each pattern in turn, scan the graph for triples that
/// unify with the pattern under the partial binding so far, extend the binding, and recurse. The
/// graph for one resource is small, so the naive nested scan is correct and adequate (no index).
fn solve_bgp(patterns: &[PatTriple], graph: &[Triple]) -> Vec<Binding> {
    let mut solutions: Vec<Binding> = Vec::new();
    solve_from(patterns, 0, Binding::default(), graph, &mut solutions);
    solutions
}

/// Recursive backtracking step over the pattern list from index `idx`.
fn solve_from(
    patterns: &[PatTriple],
    idx: usize,
    binding: Binding,
    graph: &[Triple],
    out: &mut Vec<Binding>,
) {
    if idx == patterns.len() {
        // A complete mapping. De-duplicate (two patterns can bind identically) so the spec's
        // "multiple mappings" count is over DISTINCT solutions, not repeated identical ones.
        if !out.contains(&binding) {
            out.push(binding);
        }
        return;
    }
    let pat = &patterns[idx];
    for t in graph {
        if let Some(extended) = unify_triple(pat, t, &binding) {
            solve_from(patterns, idx + 1, extended, graph, out);
        }
    }
}

/// Try to unify a single triple pattern against a concrete triple under the current binding.
/// Returns the extended binding on success, `None` on a clash.
fn unify_triple(pat: &PatTriple, t: &Triple, binding: &Binding) -> Option<Binding> {
    let mut b = binding.clone();
    unify_term(&pat.subject, &subject_as_term(&t.subject), &mut b)?;
    unify_term(
        &pat.predicate,
        &Term::NamedNode(t.predicate.clone()),
        &mut b,
    )?;
    unify_term(&pat.object, &t.object, &mut b)?;
    Some(b)
}

/// Unify one pattern term against a concrete term, mutating the binding. Returns `None` on a clash
/// (a concrete-term mismatch, or a variable already bound to a different term).
fn unify_term(pat: &PatTerm, concrete: &Term, binding: &mut Binding) -> Option<()> {
    match pat {
        PatTerm::Term(expected) => {
            if expected == concrete {
                Some(())
            } else {
                None
            }
        }
        // A SPARQL variable and a `where`-scoped blank-node existential unify identically — bind on
        // first sight, clash on a re-bind to a different term. They live in DISJOINT binding-key
        // namespaces (see `binding_key`) so `?x` and `_:x` never alias each other.
        PatTerm::Var(_) | PatTerm::WhereBlank(_) => {
            let key = binding_key(pat);
            match binding.get(&key) {
                Some(already) if already == concrete => Some(()),
                Some(_) => None, // bound to a different term ⇒ clash
                None => {
                    binding.insert(key, concrete.clone());
                    Some(())
                }
            }
        }
    }
}

/// The binding-map key for a unifiable pattern term (a variable or a `where`-blank existential).
///
/// Real SPARQL variable names cannot contain a space (oxrdf's `Variable` validation excludes it), so
/// prefixing the blank-node existential key with `"_: "` (note the space) guarantees a `where`-blank
/// `_:x` and a SPARQL variable `?x` occupy DISJOINT key namespaces and can never collide. Returns an
/// empty string for a concrete term, which never reaches the binding map.
fn binding_key(pat: &PatTerm) -> String {
    match pat {
        PatTerm::Var(v) => v.as_str().to_string(),
        PatTerm::WhereBlank(label) => format!("_: {label}"),
        PatTerm::Term(_) => String::new(),
    }
}

/// Lift an `oxrdf` subject (named/blank node) into a `Term` so it unifies uniformly with an object.
fn subject_as_term(s: &NamedOrBlankNode) -> Term {
    match s {
        NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

// --- template instantiation --------------------------------------------------------------------

/// Instantiate every template triple pattern under the binding into a concrete triple.
fn instantiate_all(template: &[PatTriple], binding: &Binding) -> Result<Vec<Triple>, ServerError> {
    template.iter().map(|t| instantiate(t, binding)).collect()
}

/// Instantiate a single template triple under the binding. Every variable in a template is
/// guaranteed (by parse-time validation) to occur in `where`; if the where matched it is therefore
/// bound. A missing binding here would be an internal invariant break, surfaced as a 422 rather than
/// a panic.
fn instantiate(pat: &PatTriple, binding: &Binding) -> Result<Triple, ServerError> {
    let subject = match resolve(&pat.subject, binding)? {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
        Term::Literal(_) => {
            return Err(ServerError::UnprocessablePatch(
                "a patch triple subject cannot be a literal".into(),
            ))
        }
        // `oxrdf`'s `Term::Triple` (RDF 1.2 quoted triples). In the sparq workspace this crate
        // depends on oxrdf WITH `rdf-12` (the workspace-wide dep set), so the variant is always
        // present regardless of feature unification. Quoted triples are NOT part of N3 Patch, so a
        // subject that is a quoted triple is rejected as an unprocessable patch (422), fail-closed.
        Term::Triple(_) => {
            return Err(ServerError::UnprocessablePatch(
                "a patch triple subject cannot be a quoted triple".into(),
            ))
        }
    };
    let predicate = match resolve(&pat.predicate, binding)? {
        Term::NamedNode(n) => n,
        _ => {
            return Err(ServerError::UnprocessablePatch(
                "a patch triple predicate must be an IRI".into(),
            ))
        }
    };
    let object = resolve(&pat.object, binding)?;
    Ok(Triple::new(subject, predicate, object))
}

/// Resolve a pattern term to a concrete term under the binding.
fn resolve(pat: &PatTerm, binding: &Binding) -> Result<Term, ServerError> {
    match pat {
        PatTerm::Term(t) => Ok(t.clone()),
        PatTerm::Var(v) => binding.get(v.as_str()).cloned().ok_or_else(|| {
            // Should be unreachable given parse-time validation, but never silently drop a variable.
            ServerError::UnprocessablePatch(format!(
                "variable ?{} in a template was not bound by solid:where",
                v.as_str()
            ))
        }),
        // A `where`-scoped existential can only originate from a `where` formula — blank nodes are
        // rejected in inserts/deletes templates at parse time — so it can never reach template
        // instantiation. Surface the invariant break as a 422 rather than panicking.
        PatTerm::WhereBlank(label) => Err(ServerError::UnprocessablePatch(format!(
            "blank node _:{label} unexpectedly reached a template (internal invariant)"
        ))),
    }
}

// --- formula parsing ---------------------------------------------------------------------------

/// Whether a formula is the `where` BGP (blank nodes allowed) or an inserts/deletes template
/// (blank nodes forbidden per spec).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FormulaKind {
    Where,
    Template,
}

/// Extract the blank-node graph name a formula object refers to. In oxttl's N3 model a `{ … }`
/// formula is encoded as a blank node whose triples live in the graph of that name.
fn formula_graph(object: &N3Term) -> Result<GraphName, ServerError> {
    match object {
        N3Term::BlankNode(b) => Ok(GraphName::BlankNode(b.clone())),
        N3Term::NamedNode(n) => Ok(GraphName::NamedNode(n.clone())),
        _ => Err(ServerError::UnprocessablePatch(
            "solid:inserts/deletes/where object must be a formula".into(),
        )),
    }
}

/// Collect the triple PATTERNS in a formula graph (terms may be variables), converting each N3 term.
/// For a `Template` formula a blank node is a spec violation (422); for a `Where` formula it is fine.
fn pattern_triples(
    quads: &[N3Quad],
    graph: &GraphName,
    kind: FormulaKind,
) -> Result<Vec<PatTriple>, ServerError> {
    let mut out = Vec::new();
    for q in quads.iter().filter(|q| &q.graph_name == graph) {
        let subject = pat_term(&q.subject, kind, Position::Subject)?;
        let predicate = pat_term(&q.predicate, kind, Position::Predicate)?;
        let object = pat_term(&q.object, kind, Position::Object)?;
        out.push(PatTriple {
            subject,
            predicate,
            object,
        });
    }
    Ok(out)
}

/// Triple position, used to reject a literal/blank in an illegal slot.
#[derive(Clone, Copy)]
enum Position {
    Subject,
    Predicate,
    Object,
}

/// Convert an N3 term to a pattern term, enforcing position + formula-kind constraints.
///
/// NB: oxttl's `N3Term::Triple` (RDF 1.2 quoted triples) is present (this crate takes the sparq
/// workspace's `rdf-12` oxttl dep) and rejected below — quoted triples are not part of N3 Patch.
fn pat_term(t: &N3Term, kind: FormulaKind, pos: Position) -> Result<PatTerm, ServerError> {
    match t {
        N3Term::NamedNode(n) => Ok(PatTerm::Term(Term::NamedNode(n.clone()))),
        N3Term::Literal(l) => match pos {
            Position::Object => Ok(PatTerm::Term(Term::Literal(l.clone()))),
            Position::Subject => Err(ServerError::UnprocessablePatch(
                "a patch triple subject cannot be a literal".into(),
            )),
            Position::Predicate => Err(ServerError::UnprocessablePatch(
                "a patch triple predicate must be an IRI".into(),
            )),
        },
        N3Term::BlankNode(b) => {
            if kind == FormulaKind::Template {
                // Spec: the inserts/deletes formulae MUST NOT contain blank nodes.
                return Err(ServerError::UnprocessablePatch(
                    "blank nodes are not allowed in solid:inserts / solid:deletes".into(),
                ));
            }
            match pos {
                Position::Predicate => Err(ServerError::UnprocessablePatch(
                    "a patch triple predicate must be an IRI".into(),
                )),
                // A blank node in the `where` formula is an existential variable (scoped to this
                // formula), NOT a concrete term: it must match like a variable and repeated labels
                // must unify. Lift it into a `WhereBlank` keyed by its label so the BGP solver
                // binds it. (Forbidden in templates, handled above, so this is only ever a `where`.)
                _ => Ok(PatTerm::WhereBlank(b.as_str().to_string())),
            }
        }
        N3Term::Variable(v) => {
            // A variable predicate is permitted in a BGP pattern; on a real graph the predicate is
            // always an IRI, so a predicate variable simply binds to an IRI during the join.
            Ok(PatTerm::Var(v.clone()))
        }
        // `oxttl`'s `N3Term::Triple` (RDF 1.2 quoted triples). In the sparq workspace this crate
        // depends on oxttl WITH `rdf-12` (the workspace-wide dep set), so the variant is always
        // present regardless of feature unification. Quoted triples are NOT part of N3 Patch, so
        // any position is rejected, fail-closed.
        N3Term::Triple(_) => Err(ServerError::UnprocessablePatch(
            "quoted triples are not allowed in an N3 Patch".into(),
        )),
    }
}

// --- variable bookkeeping ----------------------------------------------------------------------

/// The set of variable names occurring anywhere in a set of triple patterns.
fn collect_vars(patterns: &[PatTriple]) -> std::collections::HashSet<String> {
    let mut s = std::collections::HashSet::new();
    for t in patterns {
        for v in pat_triple_vars(t) {
            s.insert(v.as_str().to_string());
        }
    }
    s
}

/// The named SPARQL variables (`?x`) occurring in a single triple pattern.
///
/// Deliberately excludes `where`-scoped blank-node existentials ([`PatTerm::WhereBlank`]): the only
/// caller is the parse-time "no free template variable" check, and a template can NEVER reference a
/// `where`-blank (blank nodes are forbidden in templates), so a `where`-blank is irrelevant to that
/// check and must not be conflated with a named variable name.
fn pat_triple_vars(t: &PatTriple) -> impl Iterator<Item = &Variable> {
    [&t.subject, &t.predicate, &t.object]
        .into_iter()
        .filter_map(|term| match term {
            PatTerm::Var(v) => Some(v),
            PatTerm::Term(_) | PatTerm::WhereBlank(_) => None,
        })
}

/// The PATCH document language, selected from the request `Content-Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchKind {
    /// `text/n3` — the Solid N3 Patch (insert/delete + `solid:where`).
    N3,
    /// `application/sparql-update` — the SPARQL 1.1 Update language. We support the **`INSERT DATA` /
    /// `DELETE DATA`** subset (concrete triples, no `WHERE` solver), which is what the Solid Protocol
    /// containment scenarios use; a templated `DELETE/INSERT … WHERE` is rejected (422) until the
    /// solver lands.
    SparqlUpdate,
}

/// Classify a PATCH `Content-Type` into a supported [`PatchKind`].
///
/// An ABSENT Content-Type is a 400 Bad Request (`content-type-reject` — a write MUST declare its
/// type); a PRESENT-but-unsupported type is a 415. The two Solid PATCH media types are `text/n3` and
/// `application/sparql-update`.
pub fn classify_patch_media_type(content_type: Option<&str>) -> Result<PatchKind, ServerError> {
    let raw =
        content_type.ok_or_else(|| ServerError::BadRequest("missing PATCH content-type".into()))?;
    let essence = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "text/n3" => Ok(PatchKind::N3),
        "application/sparql-update" => Ok(PatchKind::SparqlUpdate),
        other => Err(ServerError::UnsupportedMediaType(other.to_string())),
    }
}

/// Parse the supported `application/sparql-update` subset (`INSERT DATA { … }` / `DELETE DATA { … }`)
/// into the same [`N3Patch`] shape the N3 engine applies.
///
/// Only the **concrete-data** forms are supported: `INSERT DATA { triples }` and
/// `DELETE DATA { triples }` (each optional, both may appear). The inner group is a set of ground
/// triples in Turtle syntax, so it is parsed with the vetted `oxttl` Turtle parser (the house rule —
/// never hand-parse RDF), resolving relative IRIs against `base_iri`. A templated
/// `DELETE/INSERT … WHERE`, a `WITH`/`USING`, or any graph-management verb is **rejected (422)** — the
/// `WHERE` solver is not wired for SPARQL-Update yet, and silently ignoring it would mis-apply the
/// patch. The braces are extracted by depth-tracked scanning (no nested groups in `… DATA`).
pub fn parse_sparql_update(body: &[u8], base_iri: &str) -> Result<N3Patch, ServerError> {
    let text = std::str::from_utf8(body)
        .map_err(|_| ServerError::UnprocessablePatch("SPARQL-Update must be UTF-8".into()))?;

    let mut inserts: Vec<PatTriple> = Vec::new();
    let mut deletes: Vec<PatTriple> = Vec::new();
    let mut saw_any = false;
    // Collected SPARQL prologue declarations, translated to Turtle so the `oxttl` parser of each DATA
    // group sees the prefixes. A SPARQL `PREFIX p: <iri>` becomes Turtle `@prefix p: <iri> .`; a
    // SPARQL `BASE <iri>` becomes `@base <iri> .`.
    let mut prologue = String::new();
    // Scan the body case-insensitively for `INSERT DATA` / `DELETE DATA`, parsing each `{ … }` block.
    // Anything that is not one of these two concrete-data verbs (e.g. a bare `DELETE`/`INSERT` with a
    // WHERE, `WITH`, `LOAD`, `CLEAR`, …) is unsupported and fails closed. `to_ascii_lowercase` only
    // remaps ASCII A–Z, so `lower` shares byte indices with `text` (slicing stays in sync).
    let lower = text.to_ascii_lowercase();
    let mut cursor = 0usize;
    while cursor < text.len() {
        // Skip whitespace, then an OPTIONAL `;` operation separator (SPARQL 1.1 Update separates
        // operations with `;`, e.g. `DELETE DATA { … } ; INSERT DATA { … }`, and permits a trailing
        // `;`). We accept and skip it between supported INSERT DATA / DELETE DATA operations — applied
        // in order via the shared engine — and re-skip whitespace after it. (Without this, a standard
        // semicolon-separated multi-op patch hit the unsupported-verb branch and 422'd.)
        let lo = &lower[cursor..];
        let ws = lo.len() - lo.trim_start().len();
        cursor += ws;
        let lo = &lower[cursor..];
        if let Some(rest) = lo.strip_prefix(';') {
            cursor += lo.len() - rest.len(); // advance past the ';'
            let lo = &lower[cursor..];
            let ws = lo.len() - lo.trim_start().len();
            cursor += ws;
        }
        let lo = &lower[cursor..];
        if lo.is_empty() {
            break;
        }
        // SPARQL `PREFIX p: <iri>` / `BASE <iri>` declarations: consume to end of line and translate
        // to a Turtle `@prefix`/`@base … .` for the DATA-group parser. A `@`-prefixed Turtle-style
        // prologue (`@prefix … .`) is also accepted and copied through verbatim.
        if lo.starts_with("prefix")
            || lo.starts_with("base")
            || lo.starts_with("@prefix")
            || lo.starts_with("@base")
        {
            let nl = text[cursor..].find('\n').unwrap_or(text.len() - cursor);
            let line = text[cursor..cursor + nl].trim();
            if line.starts_with('@') {
                // Already Turtle-shaped; keep as-is (ensure it terminates with a '.').
                let stmt = line.trim_end_matches('.').trim();
                prologue.push_str(stmt);
                prologue.push_str(" .\n");
            } else {
                // SPARQL prologue → Turtle directive (lower-cased keyword).
                let mut parts = line.splitn(2, char::is_whitespace);
                let kw = parts.next().unwrap_or("").to_ascii_lowercase();
                let body = parts
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches('.')
                    .trim();
                prologue.push('@');
                prologue.push_str(&kw);
                prologue.push(' ');
                prologue.push_str(body);
                prologue.push_str(" .\n");
            }
            cursor += nl;
            continue;
        }
        let kind = if lo.starts_with("insert data") {
            cursor += "insert data".len();
            Some(false)
        } else if lo.starts_with("delete data") {
            cursor += "delete data".len();
            Some(true)
        } else {
            None
        };
        let Some(is_delete) = kind else {
            // An unsupported verb (templated INSERT/DELETE with WHERE, WITH, LOAD, CLEAR, …).
            return Err(ServerError::UnprocessablePatch(
                "only INSERT DATA / DELETE DATA SPARQL-Update is supported".into(),
            ));
        };
        // Find the `{ … }` group.
        let after = &text[cursor..];
        let open = after.find('{').ok_or_else(|| {
            ServerError::UnprocessablePatch("SPARQL-Update DATA block missing '{'".into())
        })?;
        let group_start = cursor + open + 1;
        // DATA blocks contain no nested groups; the matching close is the next '}'.
        let close_rel = text[group_start..].find('}').ok_or_else(|| {
            ServerError::UnprocessablePatch("SPARQL-Update DATA block missing '}'".into())
        })?;
        let group = &text[group_start..group_start + close_rel];
        // Parse the group as Turtle WITH the collected prologue prepended (so prefixed names resolve).
        let with_prologue = format!("{prologue}{group}");
        let triples = parse_to_concrete_triples(&with_prologue, base_iri)?;
        if is_delete {
            deletes.extend(triples);
        } else {
            inserts.extend(triples);
        }
        saw_any = true;
        cursor = group_start + close_rel + 1;
    }

    if !saw_any || (inserts.is_empty() && deletes.is_empty()) {
        return Err(ServerError::UnprocessablePatch(
            "SPARQL-Update must contain a non-empty INSERT DATA / DELETE DATA".into(),
        ));
    }

    Ok(N3Patch {
        conditions: Vec::new(),
        deletes,
        inserts,
    })
}

/// Parse a Turtle triple block into concrete (variable-free) [`PatTriple`]s, resolving relative IRIs
/// against `base_iri`. Used by the SPARQL-Update `INSERT/DELETE DATA` path.
fn parse_to_concrete_triples(group: &str, base_iri: &str) -> Result<Vec<PatTriple>, ServerError> {
    use oxttl::TurtleParser;
    let parser = TurtleParser::new()
        .with_base_iri(base_iri)
        .map_err(|e| ServerError::BadRequest(format!("invalid base IRI: {e}")))?;
    let mut out = Vec::new();
    for t in parser.for_slice(group.as_bytes()) {
        let t = t.map_err(|e| {
            ServerError::UnprocessablePatch(format!("invalid SPARQL-Update DATA triple: {e}"))
        })?;
        // A DATA block holds GROUND triples; every term is carried through as a concrete
        // [`PatTerm::Term`] (the apply engine treats these as ground, with the empty binding). The
        // containment scenarios use only IRIs/literals.
        out.push(PatTriple {
            subject: PatTerm::Term(Term::from(t.subject)),
            predicate: PatTerm::Term(Term::NamedNode(t.predicate)),
            object: PatTerm::Term(t.object),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://pod.example/alice/data";

    const PREFIXES: &str = "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
        @prefix foaf: <http://xmlns.com/foaf/0.1/> .\n";

    fn triple(s: &str, p: &str, o_literal: &str) -> Triple {
        Triple::new(
            oxrdf::NamedNode::new(s).unwrap(),
            oxrdf::NamedNode::new(p).unwrap(),
            oxrdf::Literal::new_simple_literal(o_literal),
        )
    }

    /// A triple whose object is a named node (for where-binding tests where vars bind to IRIs).
    fn triple_obj_iri(s: &str, p: &str, o: &str) -> Triple {
        Triple::new(
            oxrdf::NamedNode::new(s).unwrap(),
            oxrdf::NamedNode::new(p).unwrap(),
            oxrdf::NamedNode::new(o).unwrap(),
        )
    }

    /// Build a concrete (variable-free) triple pattern with a literal object, for the programmatic
    /// `N3Patch` construction tests below.
    fn concrete_pat_triple(s: &str, p: &str, o_literal: &str) -> PatTriple {
        PatTriple {
            subject: PatTerm::Term(Term::NamedNode(oxrdf::NamedNode::new(s).unwrap())),
            predicate: PatTerm::Term(Term::NamedNode(oxrdf::NamedNode::new(p).unwrap())),
            object: PatTerm::Term(Term::Literal(oxrdf::Literal::new_simple_literal(o_literal))),
        }
    }

    #[test]
    fn media_type_selects_n3_or_sparql_update() {
        assert_eq!(
            classify_patch_media_type(Some("text/n3")).unwrap(),
            PatchKind::N3
        );
        assert_eq!(
            classify_patch_media_type(Some("text/n3; charset=utf-8")).unwrap(),
            PatchKind::N3
        );
        // SPARQL-Update (INSERT/DELETE DATA subset) is now supported.
        assert_eq!(
            classify_patch_media_type(Some("application/sparql-update")).unwrap(),
            PatchKind::SparqlUpdate
        );
        // An unsupported PATCH type ⇒ 415, never a silent accept.
        assert_eq!(
            classify_patch_media_type(Some("application/json"))
                .unwrap_err()
                .status()
                .as_u16(),
            415
        );
        // An ABSENT Content-Type ⇒ 400 (content-type-reject), distinct from unsupported-415.
        assert_eq!(
            classify_patch_media_type(None)
                .unwrap_err()
                .status()
                .as_u16(),
            400
        );
    }

    #[test]
    fn sparql_update_insert_delete_data_parses_to_concrete_triples() {
        let doc = "PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
            DELETE DATA { <#me> foaf:name \"Old\" . }\n\
            INSERT DATA { <#me> foaf:name \"New\" . }\n";
        let patch = parse_sparql_update(doc.as_bytes(), BASE).unwrap();
        assert!(patch.conditions.is_empty());
        assert_eq!(patch.deletes.len(), 1);
        assert_eq!(patch.inserts.len(), 1);
    }

    #[test]
    fn sparql_update_accepts_semicolon_separated_multi_op() {
        // Regression (roborev Medium): a standard `;`-separated multi-op patch must parse — the parser
        // previously 422'd on the `;` between DELETE DATA and INSERT DATA. Both ops apply, in order.
        let doc = "PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
            DELETE DATA { <#me> foaf:name \"Old\" . } ; INSERT DATA { <#me> foaf:name \"New\" . }\n";
        let patch = parse_sparql_update(doc.as_bytes(), BASE).unwrap();
        assert!(patch.conditions.is_empty());
        assert_eq!(patch.deletes.len(), 1);
        assert_eq!(patch.inserts.len(), 1);

        // And it applies end-to-end through the shared engine: Old → New.
        let existing = vec![triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Old",
        )];
        let result = apply_patch(&existing, &patch).unwrap();
        assert_eq!(
            result,
            vec![triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "New"
            )]
        );
    }

    #[test]
    fn sparql_update_accepts_trailing_semicolon() {
        // A trailing `;` after the last operation is also tolerated (SPARQL 1.1 permits it).
        let doc = "PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
            INSERT DATA { <#me> foaf:name \"New\" . } ;\n";
        let patch = parse_sparql_update(doc.as_bytes(), BASE).unwrap();
        assert_eq!(patch.inserts.len(), 1);
        assert!(patch.deletes.is_empty());
    }

    #[test]
    fn sparql_update_rejects_templated_where() {
        // A templated `DELETE … WHERE` is not the supported DATA subset ⇒ 422 (never silently ignored).
        let doc = "DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }";
        let err = parse_sparql_update(doc.as_bytes(), BASE).unwrap_err();
        assert_eq!(err.status().as_u16(), 422);
    }

    #[test]
    fn parses_insert_and_delete() {
        let doc = format!(
            "{PREFIXES}\
            _:patch a solid:InsertDeletePatch;\n\
              solid:deletes {{ <#me> foaf:name \"Old\" . }};\n\
              solid:inserts {{ <#me> foaf:name \"New\" . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        assert_eq!(patch.deletes.len(), 1);
        assert_eq!(patch.inserts.len(), 1);
        assert!(patch.conditions.is_empty());
        // Apply it and confirm the concrete instantiation resolved <#me> against the base.
        let existing = vec![triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Old",
        )];
        let result = apply_patch(&existing, &patch).unwrap();
        assert_eq!(
            result,
            vec![triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "New"
            )]
        );
    }

    #[test]
    fn insert_only_patch_is_valid() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:inserts {{ <#me> foaf:name \"Alice\" . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        assert!(patch.deletes.is_empty());
        assert_eq!(patch.inserts.len(), 1);
    }

    #[test]
    fn empty_patch_is_rejected() {
        let doc = format!("{PREFIXES}_:patch a solid:InsertDeletePatch.\n");
        let err = parse_n3_patch(doc.as_bytes(), BASE).unwrap_err();
        assert_eq!(err.status().as_u16(), 422);
    }

    // --- where-clause solver tests -------------------------------------------------------------

    /// A `where` matching exactly one solution drives a templated delete+insert (the happy path).
    #[test]
    fn where_single_binding_drives_delete_insert() {
        // The classic "rename" patch: bind ?name to the current value, delete it, insert the new one.
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where   {{ <#me> foaf:name ?name . }};\n\
              solid:deletes {{ <#me> foaf:name ?name . }};\n\
              solid:inserts {{ <#me> foaf:name \"New\" . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        assert_eq!(patch.conditions.len(), 1);

        let existing = vec![triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Old",
        )];
        let result = apply_patch(&existing, &patch).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "New"
        )));
        assert!(!result.contains(&triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Old"
        )));
    }

    /// A multi-pattern `where` conjunctively joins on a shared variable and binds it once.
    #[test]
    fn where_conjunctive_join_on_shared_var() {
        // where { <#me> foaf:knows ?p . ?p foaf:name ?n } — join ?p across two patterns.
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where {{ <#me> foaf:knows ?p . ?p foaf:name ?n . }};\n\
              solid:inserts {{ <#me> foaf:name ?n . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        let existing = vec![
            triple_obj_iri(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/knows",
                "https://pod.example/bob#me",
            ),
            triple(
                "https://pod.example/bob#me",
                "http://xmlns.com/foaf/0.1/name",
                "Bob",
            ),
        ];
        let result = apply_patch(&existing, &patch).unwrap();
        // ?n bound to "Bob" via the join ⇒ inserts <#me> foaf:name "Bob".
        assert!(result.contains(&triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Bob"
        )));
    }

    /// Spec: a `where` with ZERO solutions ⇒ 409 Conflict (NOT a silent no-op).
    #[test]
    fn where_zero_solutions_is_409() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where   {{ <#me> foaf:name ?name . }};\n\
              solid:deletes {{ <#me> foaf:name ?name . }};\n\
              solid:inserts {{ <#me> foaf:name \"New\" . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        // Target has NO foaf:name triple ⇒ the where can't be satisfied.
        let existing = vec![triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/age",
            "30",
        )];
        let err = apply_patch(&existing, &patch).unwrap_err();
        assert_eq!(err.status().as_u16(), 409);
    }

    /// Spec: a `where` with MULTIPLE solutions ⇒ 409 Conflict (Solid N3 Patch requires exactly one
    /// mapping — it does NOT fan out per solution the way a SPARQL DELETE/INSERT WHERE would).
    #[test]
    fn where_multiple_solutions_is_409() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where   {{ <#me> foaf:name ?name . }};\n\
              solid:deletes {{ <#me> foaf:name ?name . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        // Two foaf:name values ⇒ two distinct bindings of ?name ⇒ multiple solutions ⇒ 409.
        let existing = vec![
            triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "Alice",
            ),
            triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "Alicia",
            ),
        ];
        let err = apply_patch(&existing, &patch).unwrap_err();
        assert_eq!(err.status().as_u16(), 409);
    }

    /// A `where` that binds a variable used to FILTER which delete applies: only the matched triple
    /// is removed, the other foaf:name-on-a-different-subject triple survives.
    #[test]
    fn where_filters_which_delete_applies() {
        // where { ?s foaf:age "30" . ?s foaf:name ?n } — pins ?s to the person aged 30, then deletes
        // only THAT person's name. A second person's name (different subject) is untouched.
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where   {{ ?s foaf:age \"30\" . ?s foaf:name ?n . }};\n\
              solid:deletes {{ ?s foaf:name ?n . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        let alice_name = triple(
            "https://pod.example/alice/data#alice",
            "http://xmlns.com/foaf/0.1/name",
            "Alice",
        );
        let bob_name = triple(
            "https://pod.example/alice/data#bob",
            "http://xmlns.com/foaf/0.1/name",
            "Bob",
        );
        let existing = vec![
            triple(
                "https://pod.example/alice/data#alice",
                "http://xmlns.com/foaf/0.1/age",
                "30",
            ),
            alice_name.clone(),
            bob_name.clone(),
        ];
        let result = apply_patch(&existing, &patch).unwrap();
        // Alice (aged 30) loses her name; Bob keeps his; Alice's age survives.
        assert!(!result.contains(&alice_name));
        assert!(result.contains(&bob_name));
        assert!(result.contains(&triple(
            "https://pod.example/alice/data#alice",
            "http://xmlns.com/foaf/0.1/age",
            "30"
        )));
    }

    /// Spec: a variable in inserts/deletes that does NOT occur in `where` is a static 422.
    #[test]
    fn unbound_template_variable_is_422() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where   {{ <#me> foaf:name ?name . }};\n\
              solid:inserts {{ <#me> foaf:nick ?missing . }}.\n"
        );
        let err = parse_n3_patch(doc.as_bytes(), BASE).unwrap_err();
        assert_eq!(err.status().as_u16(), 422);
    }

    /// A variable in inserts/deletes WITHOUT any `where` clause is unbound ⇒ 422 (was the old
    /// "variables in inserts/deletes are deferred" rejection; now it's the spec's unbound-var rule).
    #[test]
    fn variable_in_template_without_where_is_422() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:inserts {{ ?s foaf:name \"x\" . }}.\n"
        );
        let err = parse_n3_patch(doc.as_bytes(), BASE).unwrap_err();
        assert_eq!(err.status().as_u16(), 422);
    }

    /// Spec: blank nodes are forbidden in the inserts/deletes templates (422).
    #[test]
    fn blank_node_in_template_is_422() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:inserts {{ <#me> foaf:knows _:someone . }}.\n"
        );
        let err = parse_n3_patch(doc.as_bytes(), BASE).unwrap_err();
        assert_eq!(err.status().as_u16(), 422);
    }

    /// An EMPTY where { } constrains nothing; the patch must then be fully concrete and applies as
    /// an unconditional insert/delete.
    #[test]
    fn empty_where_is_unconditional() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where {{ }};\n\
              solid:inserts {{ <#me> foaf:name \"Alice\" . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        assert!(patch.conditions.is_empty());
        let result = apply_patch(&[], &patch).unwrap();
        assert_eq!(result.len(), 1);
    }

    // --- where-clause blank-node-as-existential-variable tests --------------------------------

    /// A blank node in `where` is an EXISTENTIAL VARIABLE, not a concrete term: `_:x foaf:name ?n`
    /// must match a subject by its predicate/object and bind, exactly as `?x foaf:name ?n` would.
    /// (Regression: blank nodes were previously parsed as concrete `Term::BlankNode`, which only
    /// matched the parser-generated id and so never matched a real named subject ⇒ a spurious 409.)
    #[test]
    fn where_blank_node_is_existential_variable() {
        // where { _:x foaf:age "30" . _:x foaf:name ?n } — _:x is an existential that binds to the
        // subject; ?n captures that subject's name; insert it onto <#me>.
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where {{ _:x foaf:age \"30\" . _:x foaf:name ?n . }};\n\
              solid:inserts {{ <#me> foaf:name ?n . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        // Both where patterns share the existential _:x.
        assert_eq!(patch.conditions.len(), 2);
        let existing = vec![
            triple(
                "https://pod.example/alice/data#alice",
                "http://xmlns.com/foaf/0.1/age",
                "30",
            ),
            triple(
                "https://pod.example/alice/data#alice",
                "http://xmlns.com/foaf/0.1/name",
                "Alice",
            ),
        ];
        let result = apply_patch(&existing, &patch).unwrap();
        // _:x bound to <#alice> (aged 30), ?n bound to "Alice" ⇒ inserts <#me> foaf:name "Alice".
        assert!(result.contains(&triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Alice"
        )));
    }

    /// A repeated blank-node LABEL within ONE `where` formula is the SAME existential and must
    /// unify consistently — i.e. it joins two patterns, exactly like a shared `?var` would. Here
    /// `_:x` must be the SAME subject across both patterns; only the subject satisfying both binds.
    #[test]
    fn where_repeated_blank_label_unifies_as_join() {
        // where { _:x foaf:knows <#bob> . _:x foaf:name ?n } — the join via _:x pins the person who
        // knows Bob; ?n is their name. Only Alice both knows Bob AND has a name here.
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where {{ _:x foaf:knows <#bob> . _:x foaf:name ?n . }};\n\
              solid:inserts {{ <#out> foaf:name ?n . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        let existing = vec![
            // Alice knows Bob and has a name ⇒ satisfies the _:x join.
            triple_obj_iri(
                "https://pod.example/alice/data#alice",
                "http://xmlns.com/foaf/0.1/knows",
                "https://pod.example/alice/data#bob",
            ),
            triple(
                "https://pod.example/alice/data#alice",
                "http://xmlns.com/foaf/0.1/name",
                "Alice",
            ),
            // Carol has a name but does NOT know Bob ⇒ must NOT bind _:x (else this would be a 2nd
            // solution and a 409). The repeated-_:x join must exclude her.
            triple(
                "https://pod.example/alice/data#carol",
                "http://xmlns.com/foaf/0.1/name",
                "Carol",
            ),
        ];
        let result = apply_patch(&existing, &patch).unwrap();
        // Exactly one solution: _:x = <#alice>, ?n = "Alice".
        assert!(result.contains(&triple(
            "https://pod.example/alice/data#out",
            "http://xmlns.com/foaf/0.1/name",
            "Alice"
        )));
        // Carol's name must NOT have leaked through (the join excluded her).
        assert!(!result.contains(&triple(
            "https://pod.example/alice/data#out",
            "http://xmlns.com/foaf/0.1/name",
            "Carol"
        )));
    }

    /// The spec-faithful exactly-one-solution → otherwise-409 rule holds for the blank-node-as-var
    /// case too: TWO distinct subjects satisfying the existential ⇒ multiple solutions ⇒ 409.
    #[test]
    fn where_blank_var_multiple_solutions_is_409() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where {{ _:x foaf:name ?n . }};\n\
              solid:inserts {{ <#out> foaf:name ?n . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        // Two subjects each have a name ⇒ two distinct (_:x, ?n) bindings ⇒ 409.
        let existing = vec![
            triple(
                "https://pod.example/alice/data#alice",
                "http://xmlns.com/foaf/0.1/name",
                "Alice",
            ),
            triple(
                "https://pod.example/alice/data#bob",
                "http://xmlns.com/foaf/0.1/name",
                "Bob",
            ),
        ];
        let err = apply_patch(&existing, &patch).unwrap_err();
        assert_eq!(err.status().as_u16(), 409);
    }

    /// And ZERO solutions for the blank-node-as-var case is also a 409 (not a no-op).
    #[test]
    fn where_blank_var_zero_solutions_is_409() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where {{ _:x foaf:name ?n . }};\n\
              solid:inserts {{ <#out> foaf:name ?n . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        // No triple has a foaf:name ⇒ the existential can't bind ⇒ 409.
        let existing = vec![triple(
            "https://pod.example/alice/data#alice",
            "http://xmlns.com/foaf/0.1/age",
            "30",
        )];
        let err = apply_patch(&existing, &patch).unwrap_err();
        assert_eq!(err.status().as_u16(), 409);
    }

    /// A `where`-blank `_:x` and a SPARQL variable `?x` of the SAME spelling must NOT alias — they
    /// live in disjoint binding-key namespaces. Here `?x` and `_:x` bind to DIFFERENT subjects, so
    /// if they aliased the patch would spuriously 409 (clash) instead of yielding one solution.
    #[test]
    fn where_blank_and_named_var_same_spelling_do_not_alias() {
        // where { ?x foaf:knows _:x . _:x foaf:name ?n } — ?x is alice, _:x is bob; the spellings
        // collide but the namespaces don't, so this has exactly one solution.
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where {{ ?x foaf:knows _:x . _:x foaf:name ?n . }};\n\
              solid:inserts {{ <#out> foaf:name ?n . }}.\n"
        );
        let patch = parse_n3_patch(doc.as_bytes(), BASE).unwrap();
        let existing = vec![
            triple_obj_iri(
                "https://pod.example/alice/data#alice",
                "http://xmlns.com/foaf/0.1/knows",
                "https://pod.example/alice/data#bob",
            ),
            triple(
                "https://pod.example/alice/data#bob",
                "http://xmlns.com/foaf/0.1/name",
                "Bob",
            ),
        ];
        let result = apply_patch(&existing, &patch).unwrap();
        // ?x = <#alice>, _:x = <#bob>, ?n = "Bob" — one solution, no spurious clash.
        assert!(result.contains(&triple(
            "https://pod.example/alice/data#out",
            "http://xmlns.com/foaf/0.1/name",
            "Bob"
        )));
    }

    /// Blank nodes remain FORBIDDEN (422) in the inserts/deletes templates — the existential rule
    /// applies ONLY to `where`. (Guards against the fix accidentally relaxing the template rule.)
    #[test]
    fn blank_node_in_delete_template_is_422() {
        let doc = format!(
            "{PREFIXES}\
            _:patch solid:where   {{ <#me> foaf:name ?n . }};\n\
              solid:deletes {{ <#me> foaf:knows _:someone . }}.\n"
        );
        let err = parse_n3_patch(doc.as_bytes(), BASE).unwrap_err();
        assert_eq!(err.status().as_u16(), 422);
    }

    #[test]
    fn malformed_n3_is_rejected() {
        let err = parse_n3_patch(b"this is not n3 <<<", BASE).unwrap_err();
        // Either 422 (unprocessable patch parse) — never a silent accept.
        assert_eq!(err.status().as_u16(), 422);
    }

    #[test]
    fn apply_inserts_and_deletes() {
        let existing = vec![
            triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "Old",
            ),
            triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/age",
                "30",
            ),
        ];
        let patch = N3Patch {
            conditions: vec![],
            deletes: vec![concrete_pat_triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "Old",
            )],
            inserts: vec![concrete_pat_triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "New",
            )],
        };
        let result = apply_patch(&existing, &patch).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "New"
        )));
        assert!(!result.contains(&triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Old"
        )));
    }

    #[test]
    fn deleting_an_absent_triple_is_a_conflict() {
        let existing = vec![triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Alice",
        )];
        let patch = N3Patch {
            conditions: vec![],
            deletes: vec![concrete_pat_triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "NotThere",
            )],
            inserts: vec![],
        };
        let err = apply_patch(&existing, &patch).unwrap_err();
        assert_eq!(err.status().as_u16(), 409);
    }

    #[test]
    fn apply_is_idempotent_for_inserts() {
        let existing = vec![triple(
            "https://pod.example/alice/data#me",
            "http://xmlns.com/foaf/0.1/name",
            "Alice",
        )];
        let patch = N3Patch {
            conditions: vec![],
            deletes: vec![],
            inserts: vec![concrete_pat_triple(
                "https://pod.example/alice/data#me",
                "http://xmlns.com/foaf/0.1/name",
                "Alice",
            )],
        };
        // Inserting an existing triple is a no-op (set union), not a duplicate.
        let result = apply_patch(&existing, &patch).unwrap();
        assert_eq!(result.len(), 1);
    }

    // --- RDF 1.2 quoted triples (rdf-12) are rejected fail-closed sq-gg0qq.2 ----------
    // In the sparq workspace this crate takes oxrdf/oxttl WITH `rdf-12`, so `Term::Triple` /
    // `N3Term::Triple` are real inhabitants. N3 Patch has no quoted triples: both entry points
    // must refuse them with UnprocessablePatch (422), never accept or panic.

    /// `pat_term` refuses a quoted triple in every position and both formula kinds.
    #[test]
    fn pat_term_rejects_quoted_triple_in_every_position() {
        let qt = N3Term::Triple(Box::new(triple(
            "https://s.example/s",
            "https://p.example/p",
            "o",
        )));
        for kind in [FormulaKind::Where, FormulaKind::Template] {
            for pos in [Position::Subject, Position::Predicate, Position::Object] {
                match pat_term(&qt, kind, pos) {
                    Err(ServerError::UnprocessablePatch(msg)) => {
                        assert!(msg.contains("quoted triples"), "unexpected message: {msg}");
                    }
                    other => panic!("expected UnprocessablePatch, got {other:?}"),
                }
            }
        }
    }

    /// `instantiate` refuses a template subject variable that resolved to a quoted triple.
    #[test]
    fn instantiate_rejects_quoted_triple_subject_binding() {
        let pat = PatTriple {
            subject: PatTerm::Var(Variable::new("s").unwrap()),
            predicate: PatTerm::Term(Term::NamedNode(
                oxrdf::NamedNode::new("https://p.example/p").unwrap(),
            )),
            object: PatTerm::Term(Term::Literal(oxrdf::Literal::new_simple_literal("o"))),
        };
        let mut binding = Binding::new();
        binding.insert(
            "s".to_string(),
            Term::Triple(Box::new(triple(
                "https://s.example/s",
                "https://p.example/p",
                "o",
            ))),
        );
        match instantiate(&pat, &binding) {
            Err(ServerError::UnprocessablePatch(msg)) => {
                assert!(msg.contains("quoted triple"), "unexpected message: {msg}");
            }
            other => panic!("expected UnprocessablePatch, got {other:?}"),
        }
    }
}
