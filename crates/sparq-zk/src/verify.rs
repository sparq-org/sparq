//! Verifier-side static re-check (plan §2.4, layer 3).
//!
//! The verifier never trusts prover-supplied plan structures: it re-derives
//! the stage-1 fragment's BGP patterns and join edges from the **query text
//! alone** (an independent `spargebra` parse), takes the manifest's *public*
//! per-pattern graph attributions, and computes which join edges could
//! equate values across graph boundaries. Every such edge carries a
//! **non-bnode obligation**: the joined value must be IRI- or literal-typed
//! (the only terms with cross-credential identity). A manifest that omits
//! one of the required obligations assumes a cross-graph bnode equality —
//! which RDF merge semantics forbid (Q6) — and is rejected.
//!
//! Relation to the prover-side guard ([`crate::trace`]'s rejection at
//! witness-build time): the prover sees concrete matched triples and rejects
//! precisely (only when an actual bnode is equated across actually-disjoint
//! attributions). The verifier cannot see values — bnodes are private — so
//! its check is necessarily **coarser**: an edge is flagged whenever the two
//! patterns' attribution sets admit *any* cross-graph pair (`|A_i ∪ A_j| >
//! 1`). Coarser is the safe direction — the verifier may demand a non-bnode
//! obligation the prover's concrete data never exercised, but can never
//! admit a correlating join the prover would have rejected. The two checks
//! are enforced independently (plan §2.4: a malicious prover cannot smuggle
//! a correlating join past an honest verifier).
//!
//! Fragment honesty (stage 1): SELECT/ASK over BGP + FILTER, with the
//! solution modifiers that do not change pattern input sets (DISTINCT /
//! REDUCED / LIMIT-OFFSET / projection). Everything else — property paths,
//! OPTIONAL, MINUS, UNION, `GRAPH` (contradictory with graph-set privacy,
//! plan §2.4), `FROM` / `FROM NAMED` dataset clauses (which would re-define
//! the dataset the fixed statement is about, so ignoring one mis-scopes the
//! consumer's reading of the verdict — sq-p2hci), VALUES, BIND, aggregates,
//! SERVICE — is rejected as outside the verifiable fragment, failing closed.
//!
//! **Wave-1 extended gate** (sq-3kd2g.3, epic sq-3kd2g / #1591): the separate
//! [`fragment_query`] entry point additionally accepts the *monotone*
//! extensions of `research/zksparql-fragment-extension.md` §3–§4 — property
//! path rewrites (incl. bounded `*`/`+`/`?` mapped to the `path_reach_d{k}`
//! statement family), UNION, VALUES, and subqueries — still failing closed on
//! everything else. The stage-1 entry points above stay byte-identical: they
//! gate the stage-1 compose verifier, whose manifest schema and circuit
//! dispatch cannot express the extensions yet (`sq-3kd2g.6` migrates it).

// [OPUS-4.8] sq-1zf94: `PatternKey`/`SlotPattern` are the SHAPES of a re-derived
// pattern's slots — already exposed through the `pub` fields of the values this
// module returns (`fragment_patterns`, `FragmentBranch.patterns`,
// `PathReach.subject`/`object`). Re-export them so a downstream verifier
// (`sparq-zk-compose`) can name and MATCH on them (the disclosed-solution term
// binding) without a direct `sparq-engine` dependency.
pub use sparq_engine::zk::{PatternKey, SlotPattern};
use spargebra::algebra::{Expression, GraphPattern, PropertyPathExpression};
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;
use std::collections::{BTreeSet, HashMap};

/// Verifier-side re-check failure. Every variant is a rejection: the
/// manifest/query pair must not be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The query text does not parse as SPARQL.
    Parse(String),
    /// The query is outside the stage-1 verifiable fragment.
    UnsupportedFragment(String),
    /// The manifest's attribution list does not match the query's pattern
    /// count (one public graph-set per BGP pattern, in query order).
    ArityMismatch { patterns: usize, attributions: usize },
    /// A pattern with an empty attribution set (a pattern no graph supports
    /// cannot contribute witnesses; the manifest is malformed).
    EmptyAttribution { pattern: usize },
    /// A required non-bnode join obligation is missing from the manifest:
    /// its join edge would otherwise assume a cross-graph bnode equality.
    MissingObligation(JoinEdge),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Parse(e) => write!(f, "query does not parse: {e}"),
            VerifyError::UnsupportedFragment(what) => {
                write!(f, "outside the stage-1 verifiable fragment: {what}")
            }
            VerifyError::ArityMismatch { patterns, attributions } => write!(
                f,
                "manifest attribution arity mismatch: query has {patterns} BGP patterns, manifest attributes {attributions}"
            ),
            VerifyError::EmptyAttribution { pattern } => {
                write!(f, "pattern {pattern} has an empty graph attribution set")
            }
            VerifyError::MissingObligation(edge) => write!(
                f,
                "rejected: join on ?{} between patterns {} and {} crosses graph boundaries without a non-bnode obligation — the manifest assumes a cross-graph bnode equality (Q6)",
                edge.variable, edge.patterns.0, edge.patterns.1
            ),
        }
    }
}

impl std::error::Error for VerifyError {}

/// One join edge of the query: `variable` is shared between the BGP patterns
/// at indices `patterns.0 < patterns.1` (indices into the query-order
/// pattern list returned by [`fragment_patterns`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JoinEdge {
    pub variable: String,
    pub patterns: (usize, usize),
}

/// Rejection reason for a `FROM` / `FROM NAMED` dataset clause. Shared by
/// every query-text entry point so the two gates give one answer.
const DATASET_CLAUSE_REASON: &str = "FROM / FROM NAMED (dataset clauses name graphs — contradictory with graph-set privacy; the committed union is the dataset)";

/// Fails closed on a query carrying a `FROM` / `FROM NAMED` dataset clause
/// (sq-p2hci).
///
/// A dataset clause RE-DEFINES the dataset the query is evaluated over, but
/// the statement every zk entry point gates is fixed: *these solutions are in
/// the answer over the **committed union***, with each pattern's witnesses
/// drawn from the manifest's public attribution set. Ignoring the clause is
/// therefore a CONSUMER-HONESTY defect, not merely an unsupported feature: a
/// consumer reading `SELECT * FROM <g1> …` off the manifest reads the verdict
/// as a claim scoped to `g1`, while the verified statement admits scans
/// attributed to any committed graph — and, since the FROM-restricted answer
/// is a *subset* of the union answer, a disclosed solution can be one the
/// query as written does not produce. The clause also cannot be honoured
/// here: the graph *set* is private by design (plan §2.4), so the verifier
/// cannot check that the named graphs are the attributed ones. Fail closed.
fn reject_dataset_clause(query: &Query) -> Result<(), VerifyError> {
    if query.dataset().is_some() {
        return Err(VerifyError::UnsupportedFragment(DATASET_CLAUSE_REASON.into()));
    }
    Ok(())
}

/// Re-derives the stage-1 fragment's BGP triple patterns from the query
/// text, in query (text) order, as the same [`PatternKey`] shape the
/// zk-trace records — so manifest pattern references can be cross-checked
/// against an independent parse. Rejects queries outside the fragment.
pub fn fragment_patterns(sparql: &str) -> Result<Vec<PatternKey>, VerifyError> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| VerifyError::Parse(e.to_string()))?;
    let pattern = match &query {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
        Query::Construct { .. } => {
            return Err(VerifyError::UnsupportedFragment("CONSTRUCT".into()))
        }
        Query::Describe { .. } => {
            return Err(VerifyError::UnsupportedFragment("DESCRIBE".into()))
        }
    };
    reject_dataset_clause(&query)?;
    let mut out = Vec::new();
    collect_patterns(pattern, &mut out)?;
    Ok(out)
}

fn collect_patterns(gp: &GraphPattern, out: &mut Vec<PatternKey>) -> Result<(), VerifyError> {
    match gp {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                out.push(triple_pattern_key(tp)?);
            }
            Ok(())
        }
        GraphPattern::Filter { expr, inner } => {
            // EXISTS/NOT EXISTS embed a graph pattern inside the FILTER
            // expression — non-monotone (NOT EXISTS) and traced unlabelled
            // by the engine seam; outside the stage-1 fragment.
            if expression_has_exists(expr) {
                return Err(VerifyError::UnsupportedFragment("EXISTS / NOT EXISTS".into()));
            }
            collect_patterns(inner, out)
        }
        GraphPattern::Join { left, right } => {
            collect_patterns(left, out)?;
            collect_patterns(right, out)
        }
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => collect_patterns(inner, out),
        GraphPattern::Path { .. } => {
            Err(VerifyError::UnsupportedFragment("property path".into()))
        }
        GraphPattern::LeftJoin { .. } => Err(VerifyError::UnsupportedFragment("OPTIONAL".into())),
        GraphPattern::Minus { .. } => Err(VerifyError::UnsupportedFragment("MINUS".into())),
        GraphPattern::Union { .. } => Err(VerifyError::UnsupportedFragment("UNION".into())),
        GraphPattern::Graph { .. } => Err(VerifyError::UnsupportedFragment(
            "GRAPH (contradictory with graph-set privacy, plan §2.4)".into(),
        )),
        GraphPattern::Extend { .. } => Err(VerifyError::UnsupportedFragment("BIND".into())),
        GraphPattern::Values { .. } => Err(VerifyError::UnsupportedFragment("VALUES".into())),
        GraphPattern::OrderBy { .. } => Err(VerifyError::UnsupportedFragment("ORDER BY".into())),
        GraphPattern::Group { .. } => Err(VerifyError::UnsupportedFragment("aggregates".into())),
        GraphPattern::Service { .. } => Err(VerifyError::UnsupportedFragment("SERVICE".into())),
        // Covers feature-gated variants the workspace enables on spargebra
        // (e.g. sep-0006 LATERAL): everything unknown is outside the fragment.
        other => Err(VerifyError::UnsupportedFragment(format!("{other:?}"))),
    }
}

/// Whether a FILTER expression contains an (NOT) EXISTS subpattern anywhere.
fn expression_has_exists(e: &Expression) -> bool {
    use Expression as E;
    match e {
        E::Exists(_) => true,
        E::NamedNode(_) | E::Literal(_) | E::Variable(_) | E::Bound(_) => false,
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
        | E::Divide(a, b) => expression_has_exists(a) || expression_has_exists(b),
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => expression_has_exists(a),
        E::In(a, list) => expression_has_exists(a) || list.iter().any(expression_has_exists),
        E::If(a, b, c) => {
            expression_has_exists(a) || expression_has_exists(b) || expression_has_exists(c)
        }
        E::Coalesce(list) => list.iter().any(expression_has_exists),
        E::FunctionCall(_, args) => args.iter().any(expression_has_exists),
    }
}

// [OPUS-4.8] FILTER query-binding (audit #5/#6/#10) — extract the query's
// FILTER comparisons and the variable→(pattern,slot) map so the compose
// verifier can require a matching, slot-bound, verdict-gated filter sub-proof.

/// A numeric comparison operator extracted from a query FILTER. Mirrors the
/// `filter_int` circuit's `OP_*` codes; `sparq-zk-compose`'s `FilterOp` maps
/// 1:1 to this (the manifest enum lives downstream, so this neutral enum is
/// what the query parser yields). `code()` is the in-circuit `op` value the
/// reconstructed public-input vector carries — the compose verifier compares
/// it against the bound sub-proof's `op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl FilterCmp {
    /// The `op` public-input value the `filter_int` circuit expects (identical
    /// to `sparq_zk_compose::manifest::FilterOp::code`).
    pub fn code(self) -> u32 {
        match self {
            FilterCmp::Lt => 0,
            FilterCmp::Le => 1,
            FilterCmp::Gt => 2,
            FilterCmp::Ge => 3,
            FilterCmp::Eq => 4,
            FilterCmp::Ne => 5,
        }
    }
}

/// One query FILTER comparison the verifier must find a bound `filter_int`
/// sub-proof for: `?variable op bound`, where `bound` is the xsd:integer
/// constant operand parsed from the literal. Only the integer-FILTER fragment
/// the `filter_int` circuit supports is extracted; any other FILTER shape
/// (float, string, arithmetic, `?a op ?b`, `IN`, …) makes the query fall
/// outside the bindable fragment and is rejected by `collect_filters`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryFilter {
    /// The variable the FILTER constrains (its operand column).
    pub variable: String,
    /// The comparison operator (already normalized to `?var op const` form;
    /// a `const op ?var` FILTER is flipped so the variable is on the left).
    pub op: FilterCmp,
    /// The xsd:integer constant operand, as a non-negative decimal.
    pub bound: u64,
}

/// xsd:integer datatype IRI (the only literal datatype the `filter_int`
/// circuit can bind — see `encode.rs` / `filter_int.nr`).
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// Flip a comparison so `const op ?var` becomes the equivalent `?var op' const`.
fn flip_cmp(op: FilterCmp) -> FilterCmp {
    match op {
        FilterCmp::Lt => FilterCmp::Gt,
        FilterCmp::Le => FilterCmp::Ge,
        FilterCmp::Gt => FilterCmp::Lt,
        FilterCmp::Ge => FilterCmp::Le,
        FilterCmp::Eq => FilterCmp::Eq,
        FilterCmp::Ne => FilterCmp::Ne,
    }
}

/// Parse one comparison operand pair `(a, b)` under `op` into a normalized
/// `QueryFilter` (`?var op const`). `None` if it is not a bindable
/// integer-FILTER (e.g. `?a op ?b`, a non-integer / non-canonical literal, or
/// `const op const`). A non-canonical xsd:integer lexical form (leading zero,
/// sign, whitespace) is rejected: `filter_int.nr` can only bind a canonical
/// non-negative decimal token, so a non-canonical bound could never be matched
/// by any honest proof and must fail closed (audit hardening, lexical-form
/// note).
fn comparison_filter(op: FilterCmp, a: &Expression, b: &Expression) -> Option<QueryFilter> {
    let var_of = |e: &Expression| match e {
        Expression::Variable(v) => Some(v.as_str().to_string()),
        _ => None,
    };
    let int_of = |e: &Expression| match e {
        Expression::Literal(l) if l.datatype().as_str() == XSD_INTEGER => {
            canonical_u64(l.value())
        }
        _ => None,
    };
    if let (Some(variable), Some(bound)) = (var_of(a), int_of(b)) {
        return Some(QueryFilter { variable, op, bound });
    }
    // `const op ?var` — flip so the variable is on the left.
    if let (Some(bound), Some(variable)) = (int_of(a), var_of(b)) {
        return Some(QueryFilter { variable, op: flip_cmp(op), bound });
    }
    None
}

/// Parse a canonical non-negative xsd:integer lexical form to `u64`. Rejects
/// leading zeros, signs, and any non-digit byte — only the exact token the
/// circuit's `enc_int_literal` replica can bind round-trips (mirrors
/// `filter_int.nr`'s digit-token constraints).
fn canonical_u64(lex: &str) -> Option<u64> {
    if lex.is_empty() || !lex.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if lex.len() > 1 && lex.starts_with('0') {
        return None; // non-canonical leading zero
    }
    lex.parse::<u64>().ok()
}

/// Walk a FILTER expression and collect every top-level integer comparison it
/// imposes. `AND` chains are flattened (each conjunct is its own obligation).
/// `?v != c` (spargebra `Not(Equal(..))`, [OPUS-4.8]) is recognised as a
/// `Ne` comparison. Any other FILTER node that is NOT a flat conjunction of
/// bindable integer comparisons (general negation `!(...)`, disjunction,
/// arithmetic, `?a op ?b`, function calls, non-integer literals, …) makes the
/// query fall outside the bindable fragment:
/// the function returns `Err(UnsupportedFragment)` so a FILTER the binding layer
/// cannot vouch for fails CLOSED rather than being silently disclosed unproven
/// (audit #10: a FILTER must have a bound sub-proof, so an unbindable FILTER
/// must be rejected, never ignored).
fn collect_filter_expr(e: &Expression, out: &mut Vec<QueryFilter>) -> Result<(), VerifyError> {
    use Expression as E;
    match e {
        E::And(a, b) => {
            collect_filter_expr(a, out)?;
            collect_filter_expr(b, out)
        }
        E::Less(a, b) => push_cmp(FilterCmp::Lt, a, b, out),
        E::LessOrEqual(a, b) => push_cmp(FilterCmp::Le, a, b, out),
        E::Greater(a, b) => push_cmp(FilterCmp::Gt, a, b, out),
        E::GreaterOrEqual(a, b) => push_cmp(FilterCmp::Ge, a, b, out),
        E::Equal(a, b) => push_cmp(FilterCmp::Eq, a, b, out),
        // [OPUS-4.8] SPARQL `?v != c` parses to `Not(Equal(..))` in spargebra
        // (there is no dedicated `NotEqual` node — confirmed against the
        // vendored spargebra `Expression` AST). Recognise `Not(Equal(var,const))`
        // / `Not(Equal(const,var))` and bind it to `FilterCmp::Ne` so valid
        // integer `!=` FILTERs are inside the bindable fragment. Any other `Not`
        // payload (e.g. `Not(Greater(..))`, `Not(And(..))`, `Not(Equal(?a,?b))`,
        // non-integer / non-canonical operands) still falls through `push_cmp`'s
        // / `comparison_filter`'s fail-closed reject path — `!=` does not loosen
        // anything beyond the equality shape `comparison_filter` already vets.
        E::Not(inner) => match inner.as_ref() {
            E::Equal(a, b) => push_cmp(FilterCmp::Ne, a, b, out),
            other => Err(VerifyError::UnsupportedFragment(format!(
                "FILTER `!(...)` is not a bindable integer `!=` (only `?var != <xsd:integer>` binds): {other:?}"
            ))),
        },
        other => Err(VerifyError::UnsupportedFragment(format!(
            "FILTER expression not a bindable integer comparison: {other:?}"
        ))),
    }
}

fn push_cmp(
    op: FilterCmp,
    a: &Expression,
    b: &Expression,
    out: &mut Vec<QueryFilter>,
) -> Result<(), VerifyError> {
    match comparison_filter(op, a, b) {
        Some(qf) => {
            out.push(qf);
            Ok(())
        }
        None => Err(VerifyError::UnsupportedFragment(
            "FILTER comparison is not `?var <op> <xsd:integer>` (the bindable fragment)".into(),
        )),
    }
}

/// Re-derive the query's FILTER obligations from the query text alone, in the
/// same independent `spargebra` parse `fragment_patterns` uses. Returns one
/// [`QueryFilter`] per integer comparison. Rejects any FILTER outside the
/// bindable integer fragment (so it cannot be silently disclosed unproven).
pub fn fragment_filters(sparql: &str) -> Result<Vec<QueryFilter>, VerifyError> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| VerifyError::Parse(e.to_string()))?;
    let pattern = match &query {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
        Query::Construct { .. } => return Err(VerifyError::UnsupportedFragment("CONSTRUCT".into())),
        Query::Describe { .. } => return Err(VerifyError::UnsupportedFragment("DESCRIBE".into())),
    };
    reject_dataset_clause(&query)?;
    let mut out = Vec::new();
    collect_filters_gp(pattern, &mut out)?;
    Ok(out)
}

fn collect_filters_gp(gp: &GraphPattern, out: &mut Vec<QueryFilter>) -> Result<(), VerifyError> {
    match gp {
        GraphPattern::Bgp { .. } => Ok(()),
        GraphPattern::Filter { expr, inner } => {
            if expression_has_exists(expr) {
                return Err(VerifyError::UnsupportedFragment("EXISTS / NOT EXISTS".into()));
            }
            collect_filter_expr(expr, out)?;
            collect_filters_gp(inner, out)
        }
        GraphPattern::Join { left, right } => {
            collect_filters_gp(left, out)?;
            collect_filters_gp(right, out)
        }
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => collect_filters_gp(inner, out),
        // Every other node is already rejected by `collect_patterns`; the
        // verifier always calls `recheck` (which runs `fragment_patterns`)
        // before/with the filter extraction, so a structurally-unsupported
        // query never reaches a point where its FILTERs matter. Fail closed
        // anyway for totality.
        other => Err(VerifyError::UnsupportedFragment(format!("{other:?}"))),
    }
}

/// For each variable, the set of `(pattern_index, slot_index)` positions it
/// occupies across the query-order BGP patterns. The compose verifier uses
/// this to require a FILTER's bound sub-proof to reference exactly the scanned
/// slot the FILTER variable binds to (audit #6).
pub fn variable_slots(patterns: &[PatternKey]) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for (pi, key) in patterns.iter().enumerate() {
        for (si, slot) in key.slots.iter().enumerate() {
            if let SlotPattern::Var(v) = slot {
                out.push((v.clone(), pi, si));
            }
        }
    }
    out
}

// =====================================================================
// [OPUS-4.8] sq-3kd2g.8 (epic sq-3kd2g / #1591): query-side FILTER/BIND
// expression-tree extraction. Generalizes the single-comparison
// `FilterCmp`/`QueryFilter` extraction above to a full typed expression
// tree over the design record (`research/zksparql-fragment-extension.md`)
// §5.1 IN-fragment operator set, producing the neutral tree the compose
// verifier (`sq-3kd2g.9`) binds a node-per-operator sub-proof estate
// against (§5.2 Option C).
//
// INVARIANT: the tree is re-derived from the query text ALONE (an
// independent `spargebra` parse), is EXACT (a structural image of the
// parsed expression — `!=`/`NOT IN` stay `Not(Eq)`/`Not(In)`, operand
// order is preserved, literal lexical forms are kept verbatim), and is
// FAIL-CLOSED: any expression form outside the §5.1 IN set yields
// `UnsupportedFragment`, never a silently-dropped or approximated node.
//
// This is the QUERY side only. Rooting each leaf variable to a scan-row
// slot ([`expr_leaf_vars`] + [`variable_slots`]) and binding each node to
// its datatype-bucketed circuit sub-proof is the compose verifier's job.
// The existing `fragment_filters`/`QueryFilter`/`FilterCmp` path is kept
// byte-identical: it still gates today's stage-1 compose verifier, whose
// manifest schema cannot express expression trees yet.
// =====================================================================

/// The operator at an [`ExprTree`] interior node — one tag per §5.1
/// IN-fragment operator class. Neutral: it names the SPARQL operator, not a
/// circuit; the compose verifier maps each tag to its datatype-bucketed node
/// circuit. `!=` and `NOT IN` are deliberately NOT tags — spargebra parses
/// them as `Not(Equal(..))` / `Not(In(..))` and the tree keeps that exact
/// structural shape (a `Not` over an `Eq`/`In` node).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprOp {
    // Logical connectives (§5.1 logical; three-valued EBV + error lane).
    And,
    Or,
    Not,
    // Comparisons — valid across every datatype lane (§5.1).
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
    SameTerm,
    // Membership against a constant list (§5.1 IN; `NOT IN` = `Not(In)`).
    In,
    // Conditionals (§5.1; need the error lane).
    If,
    Coalesce,
    // Arithmetic (§5.1; the `/` decimal-as-double caveat is downstream).
    Add,
    Sub,
    Mul,
    Div,
    UnaryPlus,
    UnaryMinus,
    // Term accessors (§5.1; over the leaf type/datatype/lang lanes).
    Str,
    Lang,
    Datatype,
    IsIri,
    IsBlank,
    IsLiteral,
    IsNumeric,
    // String functions (§5.1).
    StrLen,
    SubStr,
    UCase,
    LCase,
    StrStarts,
    StrEnds,
    Contains,
    StrBefore,
    StrAfter,
    EncodeForUri,
    Concat,
    // Numeric functions (§5.1).
    Abs,
    Ceil,
    Floor,
    Round,
    // Date/time component accessors (§5.1; `TZ` excluded — estate gap).
    Year,
    Month,
    Day,
    Hours,
    Minutes,
    Seconds,
    Timezone,
    // Bounded `REGEX`/`REPLACE` subset ONLY (§5.1 / `sq-y73`): the pattern
    // (and replacement/flags) must be constants within the literal/anchored
    // subset — a "full" regex fails closed (see `bounded_regex_ok`).
    Regex,
    Replace,
}

/// A neutral, typed SPARQL expression tree re-derived from the query text —
/// the representation the compose verifier binds a node-per-operator sub-proof
/// estate against (design record §5.2 Option C). Independent of both the
/// `spargebra` AST and the downstream manifest/circuit enums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprTree {
    /// A variable leaf. Rooted to a scan-row slot by the compose verifier via
    /// [`expr_leaf_vars`] + [`variable_slots`].
    Var(String),
    /// A constant term operand, parsed verbatim (lexical form preserved) from
    /// the query text so re-derivation is byte-exact.
    Const(oxrdf::Term),
    /// `BOUND(?v)` — resolved statically downstream (OPTIONAL is out of the
    /// fragment, so boundness is decidable from the query text alone, §5.1).
    /// Extraction records only the referenced variable.
    Bound(String),
    /// An interior operator node with its ordered operands.
    Node { op: ExprOp, args: Vec<ExprTree> },
}

/// One expression obligation re-derived from a query, in query order: either a
/// FILTER condition (whose root EBV must hold, error-as-unsatisfied) or a BIND
/// (`Extend`) target (the derived value bound into the disclosed solution).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprObligation {
    /// `FILTER(expr)` — the tree's root verdict is `true ∧ ¬error` (§5.2).
    Filter(ExprTree),
    /// `BIND(expr AS ?target)` — the tree evaluates to `?target`'s value.
    Bind { target: String, expr: ExprTree },
}

/// Re-derive the neutral typed tree of one SPARQL expression, FAIL-CLOSED on
/// any node outside the §5.1 IN set (EXISTS, non-deterministic builtins,
/// term constructors, estate-gap functions, a REGEX beyond the bounded
/// subset, custom / SPARQL-1.2 term functions, …). The tree is a structural
/// image of the parse: operand order and literal lexical forms are preserved
/// and `!=`/`NOT IN` remain `Not(Eq)`/`Not(In)`.
pub fn extract_expr(e: &Expression) -> Result<ExprTree, VerifyError> {
    use Expression as E;
    match e {
        E::Variable(v) => Ok(ExprTree::Var(v.as_str().to_string())),
        E::NamedNode(n) => Ok(ExprTree::Const(oxrdf::Term::NamedNode(n.clone()))),
        E::Literal(l) => Ok(ExprTree::Const(oxrdf::Term::Literal(l.clone()))),
        E::Bound(v) => Ok(ExprTree::Bound(v.as_str().to_string())),
        E::Or(a, b) => node2(ExprOp::Or, a, b),
        E::And(a, b) => node2(ExprOp::And, a, b),
        E::Equal(a, b) => node2(ExprOp::Eq, a, b),
        E::SameTerm(a, b) => node2(ExprOp::SameTerm, a, b),
        E::Greater(a, b) => node2(ExprOp::Gt, a, b),
        E::GreaterOrEqual(a, b) => node2(ExprOp::Ge, a, b),
        E::Less(a, b) => node2(ExprOp::Lt, a, b),
        E::LessOrEqual(a, b) => node2(ExprOp::Le, a, b),
        E::Add(a, b) => node2(ExprOp::Add, a, b),
        E::Subtract(a, b) => node2(ExprOp::Sub, a, b),
        E::Multiply(a, b) => node2(ExprOp::Mul, a, b),
        E::Divide(a, b) => node2(ExprOp::Div, a, b),
        E::UnaryPlus(a) => node1(ExprOp::UnaryPlus, a),
        E::UnaryMinus(a) => node1(ExprOp::UnaryMinus, a),
        E::Not(a) => node1(ExprOp::Not, a),
        E::In(a, list) => {
            let mut args = Vec::with_capacity(1 + list.len());
            args.push(extract_expr(a)?);
            for item in list {
                // §5.1: IN is membership against a CONSTANT list only — the
                // proof construction enumerates constant candidates. A
                // variable or operator node on the RHS is outside the estate.
                match item {
                    E::NamedNode(_) | E::Literal(_) => args.push(extract_expr(item)?),
                    other => {
                        return Err(VerifyError::UnsupportedFragment(format!(
                            "non-constant IN list item {other:?} (§5.1: IN takes a constant list)"
                        )));
                    }
                }
            }
            Ok(ExprTree::Node { op: ExprOp::In, args })
        }
        E::If(a, b, c) => Ok(ExprTree::Node {
            op: ExprOp::If,
            args: vec![extract_expr(a)?, extract_expr(b)?, extract_expr(c)?],
        }),
        E::Coalesce(list) => {
            let mut args = Vec::with_capacity(list.len());
            for item in list {
                args.push(extract_expr(item)?);
            }
            Ok(ExprTree::Node { op: ExprOp::Coalesce, args })
        }
        // EXISTS / NOT EXISTS (`Not(Exists(..))`) is closed-world / deferred:
        // reject anywhere it appears, not just at a FILTER's root.
        E::Exists(_) => Err(VerifyError::UnsupportedFragment(
            "EXISTS / NOT EXISTS in a FILTER/BIND expression".into(),
        )),
        E::FunctionCall(f, args) => extract_function(f, args),
    }
}

/// Build a unary operator node from one operand subtree.
fn node1(op: ExprOp, a: &Expression) -> Result<ExprTree, VerifyError> {
    Ok(ExprTree::Node { op, args: vec![extract_expr(a)?] })
}

/// Build a binary operator node, preserving operand order.
fn node2(op: ExprOp, a: &Expression, b: &Expression) -> Result<ExprTree, VerifyError> {
    Ok(ExprTree::Node { op, args: vec![extract_expr(a)?, extract_expr(b)?] })
}

/// Map a `spargebra` `Function` call to its [`ExprOp`], FAIL-CLOSED on every
/// function outside the §5.1 IN estate. Non-deterministic builtins, term
/// constructors, estate-gap functions, custom functions, SPARQL-1.2 term
/// functions and `ADJUST` are all rejected (the last group via the catch-all,
/// so no feature-gated variant needs naming here).
fn extract_function(f: &spargebra::algebra::Function, args: &[Expression]) -> Result<ExprTree, VerifyError> {
    use spargebra::algebra::Function as F;
    let op = match f {
        // Term accessors.
        F::Str => ExprOp::Str,
        F::Lang => ExprOp::Lang,
        F::Datatype => ExprOp::Datatype,
        F::IsIri => ExprOp::IsIri,
        F::IsBlank => ExprOp::IsBlank,
        F::IsLiteral => ExprOp::IsLiteral,
        F::IsNumeric => ExprOp::IsNumeric,
        // String functions.
        F::StrLen => ExprOp::StrLen,
        F::SubStr => ExprOp::SubStr,
        F::UCase => ExprOp::UCase,
        F::LCase => ExprOp::LCase,
        F::StrStarts => ExprOp::StrStarts,
        F::StrEnds => ExprOp::StrEnds,
        F::Contains => ExprOp::Contains,
        F::StrBefore => ExprOp::StrBefore,
        F::StrAfter => ExprOp::StrAfter,
        F::EncodeForUri => ExprOp::EncodeForUri,
        F::Concat => ExprOp::Concat,
        // Numeric functions.
        F::Abs => ExprOp::Abs,
        F::Ceil => ExprOp::Ceil,
        F::Floor => ExprOp::Floor,
        F::Round => ExprOp::Round,
        // Date/time component accessors (`TZ` is an estate gap — below).
        F::Year => ExprOp::Year,
        F::Month => ExprOp::Month,
        F::Day => ExprOp::Day,
        F::Hours => ExprOp::Hours,
        F::Minutes => ExprOp::Minutes,
        F::Seconds => ExprOp::Seconds,
        F::Timezone => ExprOp::Timezone,
        // Bounded subset only — the pattern must be a constant literal within
        // the literal/anchored subset (dispatched to a dedicated extractor).
        F::Regex => return extract_regex_like(ExprOp::Regex, args),
        F::Replace => return extract_regex_like(ExprOp::Replace, args),
        // --- FAIL-CLOSED: outside the §5.1 IN estate. ---
        // Non-deterministic builtins (§5.1 OUT — nothing provable).
        F::Rand | F::Now | F::Uuid | F::StrUuid | F::BNode => {
            return Err(VerifyError::UnsupportedFragment(format!(
                "non-deterministic builtin {f:?} (§5.1 OUT — nothing provable)"
            )));
        }
        // Term constructors — DEFERRED (need encoding-side term construction).
        F::Iri | F::StrDt | F::StrLang => {
            return Err(VerifyError::UnsupportedFragment(format!(
                "term constructor {f:?} (deferred; needs encoding-side term construction)"
            )));
        }
        // Estate gaps (§5.1): langMatches, TZ, hash digests — pending gap-fill.
        F::LangMatches | F::Tz | F::Md5 | F::Sha1 | F::Sha256 | F::Sha384 | F::Sha512 => {
            return Err(VerifyError::UnsupportedFragment(format!(
                "estate gap {f:?} (pending gap-fill sq-3kd2g.4)"
            )));
        }
        // Custom functions, SPARQL-1.2 term functions (Triple/Subject/…),
        // ADJUST — all out.
        other => {
            return Err(VerifyError::UnsupportedFragment(format!(
                "unsupported function {other:?}"
            )));
        }
    };
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        out.push(extract_expr(a)?);
    }
    Ok(ExprTree::Node { op, args: out })
}

/// `REGEX(text, pattern[, flags])` / `REPLACE(text, pattern, replacement
/// [, flags])` — accepted ONLY for the bounded subset (§5.1 / `sq-y73`):
/// `pattern` (and any `replacement`/`flags`) MUST be constant string literals,
/// `flags` MUST be absent or empty, and `pattern` MUST be literal-and-anchored
/// (plain characters with an optional leading `^` and/or trailing `$`, no
/// other regex metacharacter). Any non-constant pattern, non-empty flags, or a
/// metacharacter/char-class/backslash makes it a "full" regex and FAILS
/// CLOSED. This query-side subset is a deliberate STRICT under-approximation
/// of the estate's bounded capability — under-accepting is the fail-closed
/// direction. An accepted-and-empty `flags` argument is dropped so both the
/// 2-arg and empty-3-arg spellings normalize to the same tree.
fn extract_regex_like(op: ExprOp, args: &[Expression]) -> Result<ExprTree, VerifyError> {
    let reject = |m: &str| -> Result<ExprTree, VerifyError> {
        Err(VerifyError::UnsupportedFragment(format!(
            "REGEX/REPLACE beyond the bounded subset: {m}"
        )))
    };
    // REGEX: (text, pattern) or (text, pattern, flags).
    // REPLACE: (text, pattern, replacement) or (…, replacement, flags).
    let (fixed_len, has_replacement) = match op {
        ExprOp::Regex => (2usize, false),
        ExprOp::Replace => (3usize, true),
        _ => return reject("internal: non-regex op"),
    };
    if args.len() != fixed_len && args.len() != fixed_len + 1 {
        return reject("arity");
    }
    // The pattern operand (index 1) must be a constant, bounded literal.
    let pattern_lex = match const_string(&args[1]) {
        Some(s) => s,
        None => return reject("non-constant pattern"),
    };
    if !bounded_regex_ok(&pattern_lex) {
        return reject("pattern uses metacharacters outside the literal/anchored subset");
    }
    // REPLACE's replacement operand (index 2) must be a constant literal too.
    if has_replacement && const_string(&args[2]).is_none() {
        return reject("non-constant replacement");
    }
    // Any flags operand must be a constant, EMPTY literal.
    if let Some(flags) = args.get(fixed_len) {
        match const_string(flags) {
            Some(s) if s.is_empty() => {}
            _ => return reject("non-empty or non-constant flags"),
        }
    }
    // Build the tree WITHOUT the (empty) flags operand — the constant operands
    // are re-extracted (Const leaves), the `text` operand recurses.
    let mut out = Vec::with_capacity(fixed_len);
    for a in &args[..fixed_len] {
        out.push(extract_expr(a)?);
    }
    Ok(ExprTree::Node { op, args: out })
}

/// xsd:string datatype IRI — the only literal category `const_string` accepts.
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// The lexical value of a constant STRING literal expression — a simple
/// literal or an explicit `xsd:string` typed literal (oxrdf normalizes the
/// two to the same value) — else `None`. Used to vet REGEX/REPLACE
/// pattern/replacement/flags constancy. A typed non-string literal (numeric,
/// boolean, date/time, …) or a language-tagged string is NOT a valid
/// pattern/flags/replacement operand for the bounded string-matching circuit
/// semantics and fails closed, per the SPARQL contract that these arguments
/// are simple literals.
fn const_string(e: &Expression) -> Option<String> {
    match e {
        Expression::Literal(l)
            if l.language().is_none() && l.datatype().as_str() == XSD_STRING =>
        {
            Some(l.value().to_string())
        }
        _ => None,
    }
}

/// Whether a regex pattern lexical is within the query-side bounded subset:
/// plain literal characters with an OPTIONAL leading `^` and/or trailing `$`
/// anchor, and NO other regex metacharacter (`. * + ? ( ) [ ] { } | \` or an
/// interior `^`/`$`). Deliberately stricter than the estate's
/// char-class-capable bounded subset (`sq-y73`) — under-approximating is the
/// fail-closed direction.
fn bounded_regex_ok(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut start = 0;
    let mut end = bytes.len();
    if bytes.first() == Some(&b'^') {
        start = 1;
    }
    if end > start && bytes.get(end - 1) == Some(&b'$') {
        end -= 1;
    }
    const META: &[u8] = b".*+?()[]{}|\\^$";
    bytes[start..end].iter().all(|b| !META.contains(b))
}

/// Every variable a tree references at a leaf — plain `Var` leaves AND the
/// argument of a static `BOUND`. The compose verifier extends the
/// variable→(pattern,slot) map ([`variable_slots`]) to these expression leaves
/// so each rooted operand binds to the scan row it reads (design record §5.2:
/// "variable→(pattern,slot) mapping extended to expression leaves").
pub fn expr_leaf_vars(tree: &ExprTree) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_expr_leaf_vars(tree, &mut out);
    out
}

fn collect_expr_leaf_vars(tree: &ExprTree, out: &mut BTreeSet<String>) {
    match tree {
        ExprTree::Var(v) | ExprTree::Bound(v) => {
            out.insert(v.clone());
        }
        ExprTree::Const(_) => {}
        ExprTree::Node { args, .. } => {
            for a in args {
                collect_expr_leaf_vars(a, out);
            }
        }
    }
}

/// Re-derive every FILTER condition and BIND (`Extend`) target expression tree
/// from the query text alone (an independent `spargebra` parse), in query
/// EVALUATION order — inner-first over the algebra (a later FILTER/BIND nests
/// OUTSIDE an earlier one in spargebra, and a group's FILTERs apply at group
/// end per the SPARQL translation), left-to-right across joins — so a BIND
/// target is always emitted before any obligation that consumes it.
/// FAIL-CLOSED on any expression or structural form outside the §5.1 IN set. The neutral trees are what the compose verifier (`sq-3kd2g.9`) binds a
/// node-per-operator sub-proof estate against; this is the query side only.
///
/// This is a SEPARATE entry point from `fragment_filters` (the stage-1
/// single-comparison path, kept intact) and from `fragment_query` (the wave-1
/// structural gate, which still rejects BIND): it walks the same
/// BGP/Join/modifier spine and additionally extracts FILTER + BIND expression
/// trees. Structural forms it does not itself model (paths, UNION, VALUES,
/// GRAPH, OPTIONAL, MINUS, aggregates, ORDER BY, SERVICE, …) fail closed.
pub fn fragment_expr_trees(sparql: &str) -> Result<Vec<ExprObligation>, VerifyError> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| VerifyError::Parse(e.to_string()))?;
    let pattern = match &query {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
        Query::Construct { .. } => return Err(VerifyError::UnsupportedFragment("CONSTRUCT".into())),
        Query::Describe { .. } => return Err(VerifyError::UnsupportedFragment("DESCRIBE".into())),
    };
    reject_dataset_clause(&query)?;
    let mut out = Vec::new();
    collect_expr_obligations_gp(pattern, &mut out)?;
    Ok(out)
}

fn collect_expr_obligations_gp(
    gp: &GraphPattern,
    out: &mut Vec<ExprObligation>,
) -> Result<(), VerifyError> {
    match gp {
        GraphPattern::Bgp { .. } => Ok(()),
        // Spargebra nests LATER operations OUTSIDE earlier ones (a FILTER
        // over a BIND parses as `Filter { inner: Extend { .. } }`), so
        // evaluation order = inner-first: traverse the inner pattern BEFORE
        // appending the enclosing obligation.
        GraphPattern::Filter { expr, inner } => {
            collect_expr_obligations_gp(inner, out)?;
            out.push(ExprObligation::Filter(extract_expr(expr)?));
            Ok(())
        }
        GraphPattern::Extend { inner, variable, expression } => {
            collect_expr_obligations_gp(inner, out)?;
            out.push(ExprObligation::Bind {
                target: variable.as_str().to_string(),
                expr: extract_expr(expression)?,
            });
            Ok(())
        }
        GraphPattern::Join { left, right } => {
            collect_expr_obligations_gp(left, out)?;
            collect_expr_obligations_gp(right, out)
        }
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => collect_expr_obligations_gp(inner, out),
        // Everything else (paths, UNION, OPTIONAL, MINUS, GRAPH, VALUES,
        // aggregates/Group — so HAVING over an aggregate — ORDER BY, SERVICE,
        // …) is outside the structural surface this extractor models and FAILS
        // CLOSED. (`fragment_query` owns the accepted extended structural
        // fragment; this entry adds only FILTER + BIND expression extraction.)
        other => Err(VerifyError::UnsupportedFragment(format!("{other:?}"))),
    }
}

/// Per query-order BGP pattern, the constant term at each `(s, p, o)` slot
/// (`None` = a variable slot). The compose verifier encodes these and
/// byte-matches them against a scan sub-proof's bound `pattern_const_enc` to
/// confirm the disclosed scan actually answers the query's pattern (audit #10,
/// constant-swap). Returned in the same query order as [`fragment_patterns`]
/// (and `attributions`), as `oxrdf::Term`s so a downstream crate need not
/// depend on `sparq-engine`/`PatternKey`.
pub fn fragment_pattern_consts(
    patterns: &[PatternKey],
) -> Vec<[Option<oxrdf::Term>; 3]> {
    patterns
        .iter()
        .map(|key| {
            let slot = |i: usize| match &key.slots[i] {
                SlotPattern::Term(t) => Some(t.clone()),
                _ => None,
            };
            [slot(0), slot(1), slot(2)]
        })
        .collect()
}

fn triple_pattern_key(tp: &TriplePattern) -> Result<PatternKey, VerifyError> {
    let term_slot = |t: &TermPattern| -> Result<SlotPattern, VerifyError> {
        match t {
            TermPattern::NamedNode(n) => Ok(SlotPattern::Term(oxrdf::Term::NamedNode(n.clone()))),
            TermPattern::Literal(l) => Ok(SlotPattern::Term(oxrdf::Term::Literal(l.clone()))),
            TermPattern::Variable(v) => Ok(SlotPattern::Var(v.as_str().to_string())),
            // A bnode in a query pattern is a non-distinguished variable
            // whose scoping rules are easy to get wrong on both sides of the
            // proof; stage 1 requires explicit variables instead.
            TermPattern::BlankNode(_) => Err(VerifyError::UnsupportedFragment(
                "blank node in a triple pattern (use an explicit variable)".into(),
            )),
            other => Err(VerifyError::UnsupportedFragment(format!(
                "triple pattern term {other}"
            ))),
        }
    };
    let predicate = match &tp.predicate {
        NamedNodePattern::NamedNode(n) => SlotPattern::Term(oxrdf::Term::NamedNode(n.clone())),
        NamedNodePattern::Variable(v) => SlotPattern::Var(v.as_str().to_string()),
    };
    Ok(PatternKey {
        slots: [term_slot(&tp.subject)?, predicate, term_slot(&tp.object)?],
    })
}

fn pattern_variables(key: &PatternKey) -> BTreeSet<&str> {
    key.slots
        .iter()
        .filter_map(|s| match s {
            SlotPattern::Var(v) => Some(v.as_str()),
            _ => None,
        })
        .collect()
}

/// Computes the join edges that REQUIRE a non-bnode obligation, given the
/// query-order patterns and the manifest's public per-pattern graph
/// attributions (`attributions[i]` = the set of committed-graph indices
/// pattern `i`'s witnesses may be drawn from).
///
/// An edge is required whenever the two patterns' attributions admit any
/// cross-graph pair — i.e. unless both patterns are supported by the same
/// single graph (`|A_i ∪ A_j| > 1`). See the module docs for why this is
/// deliberately coarser than the prover-side guard.
pub fn cross_graph_join_obligations(
    patterns: &[PatternKey],
    attributions: &[BTreeSet<usize>],
) -> Result<Vec<JoinEdge>, VerifyError> {
    if patterns.len() != attributions.len() {
        return Err(VerifyError::ArityMismatch {
            patterns: patterns.len(),
            attributions: attributions.len(),
        });
    }
    if let Some(i) = attributions.iter().position(BTreeSet::is_empty) {
        return Err(VerifyError::EmptyAttribution { pattern: i });
    }
    let vars: Vec<BTreeSet<&str>> = patterns.iter().map(pattern_variables).collect();
    let mut edges = Vec::new();
    for i in 0..patterns.len() {
        for j in (i + 1)..patterns.len() {
            let cross_possible = attributions[i].union(&attributions[j]).count() > 1;
            if !cross_possible {
                continue;
            }
            for v in vars[i].intersection(&vars[j]) {
                edges.push(JoinEdge {
                    variable: (*v).to_string(),
                    patterns: (i, j),
                });
            }
        }
    }
    Ok(edges)
}

/// The layer-3 verifier gate: re-derives the fragment patterns from the
/// query text, computes the required non-bnode join obligations under the
/// manifest's attributions, and rejects unless every required edge is
/// declared by the manifest (`declared` = the manifest's non-bnode
/// obligations; extra declared edges are harmless — they only constrain the
/// prover further).
pub fn recheck(
    sparql: &str,
    attributions: &[BTreeSet<usize>],
    declared: &[JoinEdge],
) -> Result<Vec<JoinEdge>, VerifyError> {
    let patterns = fragment_patterns(sparql)?;
    let required = cross_graph_join_obligations(&patterns, attributions)?;
    for edge in &required {
        if !declared.contains(edge) {
            return Err(VerifyError::MissingObligation(edge.clone()));
        }
    }
    Ok(required)
}

// =========================================================================
// Wave-1 EXTENDED fragment gate (sq-3kd2g.3, epic sq-3kd2g / #1591; design
// record `research/zksparql-fragment-extension.md` §3–§4).
//
// [`fragment_query`] accepts the MONOTONE fragment extensions on top of
// stage 1 — property-path rewrites (predicate / inverse / sequence /
// alternative, plus the closures `?`/`*`/`+` mapped to the bounded
// `path_reach_d{k}` statement family), UNION (per-branch pattern lists for
// branch attribution), VALUES (public row re-derivation, `UNDEF` wildcards)
// and subqueries composed of in-fragment operators — and stays FAIL-CLOSED
// on everything else (OPTIONAL / MINUS / (NOT) EXISTS / GRAPH / SERVICE /
// BIND / aggregates / ORDER BY / dataset clauses / unrecognized forms).
// Acceptance is derived from the record's normative feature table (§3):
// the gate never accepts a query whose verified statement the fixed
// circuit family cannot express.
//
// The stage-1 entry points above (`fragment_patterns`, `fragment_filters`,
// `recheck`) are UNCHANGED on purpose: they gate the live stage-1 compose
// verifier (`sparq-zk-compose`), whose manifest schema and circuit
// dispatch cannot express these extensions yet. `sq-3kd2g.6` migrates
// compose onto this gate; until then both gates fail closed on everything
// outside their respective fragments.
// =========================================================================

/// Hard cap on the number of UNION branches [`fragment_query`] will expand a
/// query into. Joins distribute over UNION (`A . { B UNION C }` becomes the
/// branch set `A⨝B, A⨝C`), so nested unions / path alternatives multiply;
/// a query expanding past this cap is rejected (fail-closed) rather than
/// risking a resource-exhaustion path in the verifier.
pub const MAX_FRAGMENT_BRANCHES: usize = 64;

/// The closure kind of a [`PathReach`] obligation — which `path_reach_d{k}`
/// statement variant the manifest must bind.
///
/// The SPARQL closures `+`/`*` are UNBOUNDED; the circuit family proves the
/// **bounded** statement of the design record §4: a chain of committed
/// triples of length `ℓ` with `min_len() ≤ ℓ ≤ k`, where **`k` is a public
/// input disclosed in the manifest, NOT part of the query text**. Proofs at
/// different `k` are different statements; the composition verifier
/// (`sq-3kd2g.6`) must surface `k` to the consumer and reject a claimed
/// depth beyond the bound circuit member (record §4 requirements 1–3).
/// A bounded path proof asserts existence only — never absence and never
/// completeness of the reachable set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathClosure {
    /// `p?` — zero-length case or exactly one step (`k` pinned to 1).
    ZeroOrOne,
    /// `p*` — zero-length case or a chain of `1..=k` steps.
    ZeroOrMore,
    /// `p+` — a chain of `1..=k` steps (no zero-length case).
    OneOrMore,
}

impl PathClosure {
    /// Minimum chain length of the proved statement (0 admits the
    /// zero-length case, which additionally requires an **occurrence
    /// witness**: `μ(s) = μ(o)` alone is not enough, the term must occur in
    /// the committed union — record §4 requirement 5).
    pub fn min_len(self) -> usize {
        match self {
            PathClosure::OneOrMore => 1,
            PathClosure::ZeroOrOne | PathClosure::ZeroOrMore => 0,
        }
    }

    /// The depth bound `k` when the closure itself fixes it (`p?` proves at
    /// most one step, so `k = 1`). `None` for `*`/`+`: their `k` is a public
    /// manifest input the composition verifier must surface and check.
    pub fn fixed_k(self) -> Option<usize> {
        match self {
            PathClosure::ZeroOrOne => Some(1),
            PathClosure::ZeroOrMore | PathClosure::OneOrMore => None,
        }
    }
}

/// One bounded path-reachability obligation re-derived from the query text:
/// the `path_reach_d{k}` statement family member the manifest must bind for
/// a `p?` / `p*` / `p+` closure over a single predicate step.
///
/// Only closures over an ATOMIC step — a predicate IRI, possibly inverted
/// (`(^p)+` swaps the endpoints) — are expressible: the family proves chains
/// of committed triples that all carry the same predicate (design record
/// §4). Closures over compound paths (`(p1/p2)+`, `(p1|p2)*`, nested
/// closures) are rejected fail-closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathReach {
    /// Chain start (variable or constant term).
    pub subject: SlotPattern,
    /// The single step predicate (every chain triple carries it).
    pub predicate: oxrdf::NamedNode,
    /// Chain end (variable or constant term).
    pub object: SlotPattern,
    /// Which closure statement the manifest must bind.
    pub closure: PathClosure,
}

/// One VALUES block re-derived from the query text: inline PUBLIC rows the
/// disclosed solutions must be checked against (composition-level, no
/// circuit — the rows are constants of the query text). `None` cells are
/// `UNDEF` wildcards (compatible with any binding of that variable).
///
/// VALUES terms are IRIs or literals by grammar (no blank nodes), so a
/// VALUES-constrained join never equates blank nodes and contributes no Q6
/// obligation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuesBlock {
    /// The block's variables, in declaration order.
    pub variables: Vec<String>,
    /// The rows, one `Option<Term>` per variable (`None` = `UNDEF`).
    pub rows: Vec<Vec<Option<oxrdf::Term>>>,
}

/// One conjunctive branch of the extended fragment: the obligations a
/// manifest must discharge for a disclosed solution ATTRIBUTED TO this
/// branch (`UNION` semantics is per-solution branch attribution — the
/// verifier checks the attributed branch's obligations, design record §3.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FragmentBranch {
    /// BGP scan obligations in query-text order, INCLUDING the rewritten
    /// path forms: parser-flattened sequences/inverses and re-derived
    /// rewrites share this list. Non-projected intermediates are fresh
    /// variables in the reserved `!`-namespace (`!path:{n}` / `!bnode:{n}`,
    /// deterministic in walk order — `!` cannot occur in a parsed SPARQL
    /// variable name, so they never collide with query variables and both
    /// prover and verifier re-derive identical names from the query text).
    pub patterns: Vec<PatternKey>,
    /// Bounded path-reachability obligations (`p?`/`p*`/`p+`).
    pub path_reach: Vec<PathReach>,
    /// Integer FILTER obligations (the stage-1 bindable fragment; FILTERs
    /// above a UNION distribute into every branch). A FILTER outside the
    /// bindable fragment rejects the whole query — never silently dropped.
    pub filters: Vec<QueryFilter>,
    /// VALUES obligations (public row checks).
    pub values: Vec<ValuesBlock>,
}

/// The extended-fragment re-derivation of a query: its UNION branches (a
/// query without UNION/alternative has exactly one), the projected
/// variables, and the query form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentQuery {
    /// The branches, in query-text order (left UNION branch first; joins
    /// distribute over UNION). Never empty.
    pub branches: Vec<FragmentBranch>,
    /// The outer projection (empty for ASK). Variables NOT in this list are
    /// existential: the prover witnesses them without disclosing.
    pub projected: Vec<String>,
    /// `true` for ASK (non-emptiness claim), `false` for SELECT.
    pub ask: bool,
}

/// The Q6 obligations of one branch under the manifest's public per-pattern
/// graph attributions (the wave-1 analogue of
/// [`cross_graph_join_obligations`]; same safe-coarser discipline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchObligations {
    /// Join edges requiring a non-bnode obligation. Pattern indices span
    /// the branch's COMBINED obligation list: `0..patterns.len()` are the
    /// BGP scans, `patterns.len()..` are the [`PathReach`] obligations in
    /// order (a path endpoint variable joins like any pattern variable).
    pub join_edges: Vec<JoinEdge>,
    /// Indices into `branch.path_reach` whose chain links must be non-bnode:
    /// flagged whenever the path's attribution set admits any cross-graph
    /// pair (`|A| > 1`). Interior chain nodes are existential terms no join
    /// edge can see; a chain step boundary equates them exactly like a join,
    /// so a multi-graph attribution must carry the coarse obligation that
    /// every chain-equated term is IRI- or literal-typed. Zero-length
    /// endpoint equality (`p*`/`p?` with `μ(s) = μ(o)`) is covered the same
    /// way; single-graph attributions need nothing (within-graph bnode
    /// identity is exact, matching the stage-1 rule).
    pub path_link_non_bnode: Vec<usize>,
}

/// Deterministic fresh-name allocator for the extended gate. All fresh
/// names live in the reserved `!` namespace and are numbered in walk order
/// over the parse tree, so two independent parses of the same query text
/// derive identical names (parser-generated ANONYMOUS bnode labels are
/// random per parse and MUST NOT leak into derived names).
#[derive(Default)]
struct FreshNames {
    counter: usize,
    bnodes: HashMap<String, String>,
}

impl FreshNames {
    fn next(&mut self) -> usize {
        let n = self.counter;
        self.counter += 1;
        n
    }

    /// The fresh variable standing for a query bnode label (a bnode in a
    /// query pattern is a non-distinguished variable; same label = same
    /// variable). First-encounter order keys the name, never the label.
    fn bnode_var(&mut self, label: &str) -> String {
        if let Some(v) = self.bnodes.get(label) {
            return v.clone();
        }
        let v = format!("!bnode:{}", self.next());
        self.bnodes.insert(label.to_string(), v.clone());
        v
    }
}

/// Re-derives the extended (wave-1) fragment structure from the query text
/// alone — an independent `spargebra` parse, never trusting the manifest.
/// Rejects every construct outside the extended fragment with the design
/// record's documented reason (fail-closed).
pub fn fragment_query(sparql: &str) -> Result<FragmentQuery, VerifyError> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| VerifyError::Parse(e.to_string()))?;
    let (pattern, ask) = match &query {
        Query::Select { pattern, .. } => (pattern, false),
        Query::Ask { pattern, .. } => (pattern, true),
        Query::Construct { .. } => {
            return Err(VerifyError::UnsupportedFragment(
                "CONSTRUCT (graph-template result form, outside the result-membership property — instantiate templates client-side from a proved mapping)".into(),
            ))
        }
        Query::Describe { .. } => {
            return Err(VerifyError::UnsupportedFragment(
                "DESCRIBE (implementation-defined result; nothing well-defined to prove)".into(),
            ))
        }
    };
    reject_dataset_clause(&query)?;

    // Peel the OUTER solution-modifier stack (membership-indifferent:
    // LIMIT/OFFSET, DISTINCT/REDUCED) down to the query's own projection.
    // Any Project encountered DEEPER in the tree is a subquery.
    let mut node = pattern;
    while let GraphPattern::Slice { inner, .. }
    | GraphPattern::Distinct { inner }
    | GraphPattern::Reduced { inner } = node
    {
        node = inner.as_ref();
    }
    let (projected, body): (Vec<String>, &GraphPattern) =
        if let GraphPattern::Project { inner, variables } = node {
            (variables.iter().map(|v| v.as_str().to_string()).collect(), inner.as_ref())
        } else {
            // The parser always emits an outer Project; tolerate its absence
            // (an empty projection claims nothing extra — safe direction).
            (Vec::new(), node)
        };

    let mut fresh = FreshNames::default();
    let branches = collect_extended(body, &mut fresh)?;
    // ASK is a non-emptiness claim: no bindings are projected (the parser
    // still wraps the pattern in a Project of every in-scope variable —
    // peeled above, ignored here).
    let projected = if ask { Vec::new() } else { projected };
    Ok(FragmentQuery { branches, projected, ask })
}

/// Recursive extended-fragment collector: returns the query subtree's
/// branch set (its UNION-normal form). Every accepted construct is one of
/// the design record §3 table's IN rows; every other node is rejected with
/// the table's reason.
fn collect_extended(
    gp: &GraphPattern,
    fresh: &mut FreshNames,
) -> Result<Vec<FragmentBranch>, VerifyError> {
    match gp {
        GraphPattern::Bgp { patterns } => {
            let mut branch = FragmentBranch::default();
            for tp in patterns {
                branch.patterns.push(triple_pattern_key_ext(tp, fresh)?);
            }
            Ok(vec![branch])
        }
        GraphPattern::Path { subject, path, object } => {
            let s = term_slot_ext(subject, fresh)?;
            let o = term_slot_ext(object, fresh)?;
            path_branches(s, path, o, fresh)
        }
        GraphPattern::Join { left, right } => {
            let l = collect_extended(left, fresh)?;
            let r = collect_extended(right, fresh)?;
            join_branches(l, r)
        }
        GraphPattern::Union { left, right } => {
            let mut out = collect_extended(left, fresh)?;
            out.extend(collect_extended(right, fresh)?);
            if out.len() > MAX_FRAGMENT_BRANCHES {
                return Err(branch_cap_error(out.len()));
            }
            Ok(out)
        }
        GraphPattern::Filter { expr, inner } => {
            // Same discipline as stage 1: (NOT) EXISTS is out (closed-world
            // negation / deferred positive form), and a FILTER the binding
            // layer cannot vouch for rejects the query — never silently
            // disclosed unproven. FILTERs distribute into every branch of
            // their scope group (membership-preserving: a filtered solution
            // of the group is a filtered solution of its witnessing branch).
            if expression_has_exists(expr) {
                return Err(VerifyError::UnsupportedFragment("EXISTS / NOT EXISTS".into()));
            }
            let mut filters = Vec::new();
            collect_filter_expr(expr, &mut filters)?;
            let mut branches = collect_extended(inner, fresh)?;
            for b in &mut branches {
                b.filters.extend(filters.iter().cloned());
            }
            Ok(branches)
        }
        GraphPattern::Values { variables, bindings } => {
            Ok(vec![FragmentBranch {
                values: vec![values_block(variables, bindings)?],
                ..FragmentBranch::default()
            }])
        }
        // A Project below the outer modifier stack is a SUBQUERY: monotone
        // when composed of in-fragment operators. Its non-projected
        // variables are scoped to the subquery — rename them apart so an
        // identically-named outer variable is never conflated (inner
        // projection = existential quantification).
        GraphPattern::Project { inner, variables } => {
            let branches = collect_extended(inner, fresh)?;
            Ok(rename_subquery_apart(branches, variables, fresh))
        }
        // Set-semantics duplicate handling is membership-indifferent at any
        // depth (record §3.2).
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => {
            collect_extended(inner, fresh)
        }
        // A Slice HERE is inside a subquery (the top-level one was peeled):
        // it selects an implementation-nondeterministic subset of the inner
        // evaluation, so a membership witness for the unrestricted inner
        // evaluation is NOT a witness for every legal evaluation of the
        // sliced one. Fail closed.
        GraphPattern::Slice { .. } => Err(VerifyError::UnsupportedFragment(
            "LIMIT / OFFSET inside a subquery (nondeterministic subset selection — the membership statement is defined over the unrestricted inner evaluation)".into(),
        )),
        GraphPattern::OrderBy { .. } => Err(VerifyError::UnsupportedFragment(
            "ORDER BY (membership-indifferent but implies an unproved top-k claim; excluded pending an explicit order-not-proved manifest flag)".into(),
        )),
        GraphPattern::LeftJoin { .. } => Err(VerifyError::UnsupportedFragment(
            "OPTIONAL (non-monotone: an unbound optional side asserts no compatible extension exists — a closed-world claim; rewrite to Join or a UNION of explicit cases)".into(),
        )),
        GraphPattern::Minus { .. } => Err(VerifyError::UnsupportedFragment(
            "MINUS (closed-world set difference; non-monotone)".into(),
        )),
        GraphPattern::Graph { .. } => Err(VerifyError::UnsupportedFragment(
            "GRAPH (contradictory with graph-set privacy, plan §2.4)".into(),
        )),
        // `SELECT (COUNT(...) AS ?c)` parses to Extend-over-Group: report it
        // as aggregation, not BIND.
        GraphPattern::Extend { inner, .. } => {
            if contains_group(inner) {
                Err(aggregates_error())
            } else {
                Err(VerifyError::UnsupportedFragment(
                    "BIND (deterministic Extend is phase 3 of the fragment-extension program — expression estate not yet bound; fail-closed)".into(),
                ))
            }
        }
        GraphPattern::Group { .. } => Err(aggregates_error()),
        GraphPattern::Service { .. } => Err(VerifyError::UnsupportedFragment(
            "SERVICE (federation excluded by the fragment principle P2)".into(),
        )),
        // Covers feature-gated variants the workspace enables on spargebra
        // (e.g. sep-0006 LATERAL): everything unknown is outside the fragment.
        other => Err(VerifyError::UnsupportedFragment(format!("{other:?}"))),
    }
}

fn aggregates_error() -> VerifyError {
    VerifyError::UnsupportedFragment(
        "aggregates (an aggregate value is a completeness claim over the whole pattern — closed-world at pattern level; composed-pattern completeness is not proved)".into(),
    )
}

fn branch_cap_error(n: usize) -> VerifyError {
    VerifyError::UnsupportedFragment(format!(
        "union branch expansion exceeds the cap ({n} > {MAX_FRAGMENT_BRANCHES} branches)"
    ))
}

/// Whether an `Extend` chain bottoms out in a `Group` (the parse shape of
/// an aggregate SELECT expression).
fn contains_group(gp: &GraphPattern) -> bool {
    match gp {
        GraphPattern::Group { .. } => true,
        GraphPattern::Extend { inner, .. } => contains_group(inner),
        _ => false,
    }
}

/// Distributes a join over two branch sets (`(A1|A2) ⨝ (B1|B2)` =
/// `A1B1 | A1B2 | A2B1 | A2B2`), preserving query-text order within each
/// combined branch. Membership-preserving: a solution of the join is a
/// solution of some pair of witnessing branches.
fn join_branches(
    left: Vec<FragmentBranch>,
    right: Vec<FragmentBranch>,
) -> Result<Vec<FragmentBranch>, VerifyError> {
    let n = left.len().saturating_mul(right.len());
    if n > MAX_FRAGMENT_BRANCHES {
        return Err(branch_cap_error(n));
    }
    let mut out = Vec::with_capacity(n);
    for l in &left {
        for r in &right {
            let mut b = l.clone();
            b.patterns.extend(r.patterns.iter().cloned());
            b.path_reach.extend(r.path_reach.iter().cloned());
            b.filters.extend(r.filters.iter().cloned());
            b.values.extend(r.values.iter().cloned());
            out.push(b);
        }
    }
    Ok(out)
}

/// Rewrites one property-path pattern into branch structure (design record
/// §4 table): predicate → triple pattern; inverse → endpoint swap; sequence
/// → fresh non-projected intermediate; alternative → UNION branches;
/// `?`/`*`/`+` over an atomic step → a [`PathReach`] obligation; negated
/// property sets → rejected (deferred).
fn path_branches(
    subject: SlotPattern,
    path: &PropertyPathExpression,
    object: SlotPattern,
    fresh: &mut FreshNames,
) -> Result<Vec<FragmentBranch>, VerifyError> {
    use PropertyPathExpression as P;
    match path {
        P::NamedNode(p) => Ok(vec![FragmentBranch {
            patterns: vec![PatternKey {
                slots: [
                    subject,
                    SlotPattern::Term(oxrdf::Term::NamedNode(p.clone())),
                    object,
                ],
            }],
            ..FragmentBranch::default()
        }]),
        // `^p` from s to o is exactly p from o to s (composes under every
        // other form).
        P::Reverse(inner) => path_branches(object, inner, subject, fresh),
        // `p1/p2`: the SPARQL 1.1 translation — a fresh non-projected
        // intermediate joins the halves.
        P::Sequence(a, b) => {
            let mid = SlotPattern::Var(format!("!path:{}", fresh.next()));
            let l = path_branches(subject, a, mid.clone(), fresh)?;
            let r = path_branches(mid, b, object, fresh)?;
            join_branches(l, r)
        }
        // `p1|p2` desugars to UNION (branch attribution like any UNION).
        P::Alternative(a, b) => {
            let mut out = path_branches(subject.clone(), a, object.clone(), fresh)?;
            out.extend(path_branches(subject, b, object, fresh)?);
            if out.len() > MAX_FRAGMENT_BRANCHES {
                return Err(branch_cap_error(out.len()));
            }
            Ok(out)
        }
        P::ZeroOrOne(inner) => path_reach_branch(subject, inner, object, PathClosure::ZeroOrOne),
        P::ZeroOrMore(inner) => path_reach_branch(subject, inner, object, PathClosure::ZeroOrMore),
        P::OneOrMore(inner) => path_reach_branch(subject, inner, object, PathClosure::OneOrMore),
        P::NegatedPropertySet(_) => Err(VerifyError::UnsupportedFragment(
            "negated property set (monotone but deferred — design record §4: lands after the path_reach family)".into(),
        )),
    }
}

/// A closure obligation over an ATOMIC step: the inner path must normalize
/// to a single predicate IRI (inverses peel by swapping endpoints:
/// `(^p)+` from s to o is `p+` from o to s). Anything else — sequences,
/// alternatives, nested closures, negated sets — is outside the
/// `path_reach_d{k}` statement family (its chains carry ONE predicate) and
/// is rejected fail-closed.
fn path_reach_branch(
    subject: SlotPattern,
    inner: &PropertyPathExpression,
    object: SlotPattern,
    closure: PathClosure,
) -> Result<Vec<FragmentBranch>, VerifyError> {
    use PropertyPathExpression as P;
    match inner {
        P::NamedNode(p) => Ok(vec![FragmentBranch {
            path_reach: vec![PathReach {
                subject,
                predicate: p.clone(),
                object,
                closure,
            }],
            ..FragmentBranch::default()
        }]),
        P::Reverse(rev) => path_reach_branch(object, rev, subject, closure),
        other => Err(VerifyError::UnsupportedFragment(format!(
            "closure over a non-atomic path ({other} under ?/*/+) — the path_reach_d{{k}} statement family proves chains of a single predicate only"
        ))),
    }
}

/// Renames a subquery's non-projected variables apart (`!subq{n}:{name}`)
/// across the subquery's branch structure: inner projection is existential
/// quantification, so an inner-only variable must never be conflated with
/// an identically-named outer variable (conflation would strengthen the
/// derived statement — membership-safe, but it would reject honest
/// manifests and misstate the query). Names already in the reserved `!`
/// namespace are globally unique per extraction and are kept.
fn rename_subquery_apart(
    branches: Vec<FragmentBranch>,
    projected: &[Variable],
    fresh: &mut FreshNames,
) -> Vec<FragmentBranch> {
    let ord = fresh.next();
    let keep: BTreeSet<&str> = projected.iter().map(Variable::as_str).collect();
    let rename = |name: &str| -> Option<String> {
        if keep.contains(name) || name.starts_with('!') {
            None
        } else {
            Some(format!("!subq{ord}:{name}"))
        }
    };
    let rename_slot = |slot: &mut SlotPattern| {
        if let SlotPattern::Var(v) = slot {
            if let Some(new) = rename(v) {
                *v = new;
            }
        }
    };
    let mut out = branches;
    for b in &mut out {
        for key in &mut b.patterns {
            for slot in &mut key.slots {
                rename_slot(slot);
            }
        }
        for pr in &mut b.path_reach {
            rename_slot(&mut pr.subject);
            rename_slot(&mut pr.object);
        }
        for f in &mut b.filters {
            if let Some(new) = rename(&f.variable) {
                f.variable = new;
            }
        }
        for vb in &mut b.values {
            for v in &mut vb.variables {
                if let Some(new) = rename(v) {
                    *v = new;
                }
            }
        }
    }
    out
}

/// Converts a VALUES block: rows are PUBLIC constants of the query text;
/// `UNDEF` cells stay `None` (wildcards). Triple terms are rejected — the
/// committed leaf encoding has no triple-term lane (record §3.3, re-entry
/// belongs to sq-1s2.1).
fn values_block(
    variables: &[Variable],
    bindings: &[Vec<Option<GroundTerm>>],
) -> Result<ValuesBlock, VerifyError> {
    let vars: Vec<String> = variables.iter().map(|v| v.as_str().to_string()).collect();
    let mut rows = Vec::with_capacity(bindings.len());
    for row in bindings {
        // The grammar fixes each row's arity to the variable list.
        debug_assert_eq!(row.len(), vars.len());
        let mut cells = Vec::with_capacity(row.len());
        for cell in row {
            cells.push(match cell {
                None => None,
                Some(GroundTerm::NamedNode(n)) => Some(oxrdf::Term::NamedNode(n.clone())),
                Some(GroundTerm::Literal(l)) => Some(oxrdf::Term::Literal(l.clone())),
                Some(other) => {
                    return Err(VerifyError::UnsupportedFragment(format!(
                        "triple term in VALUES ({other} — the committed leaf encoding has no triple-term lane; record §3.3 / sq-1s2.1)"
                    )))
                }
            });
        }
        rows.push(cells);
    }
    Ok(ValuesBlock { variables: vars, rows })
}

/// Extended-gate triple-pattern conversion: like [`triple_pattern_key`] but
/// a blank node is accepted as the non-distinguished variable it denotes
/// (renamed into the deterministic `!bnode:{n}` namespace — the parser
/// itself flattens `p1/p2` and bare `^p` syntax into BGPs with ANONYMOUS
/// bnode intermediates whose labels are random per parse, so labels must
/// never reach derived names).
fn triple_pattern_key_ext(
    tp: &TriplePattern,
    fresh: &mut FreshNames,
) -> Result<PatternKey, VerifyError> {
    let subject = term_slot_ext(&tp.subject, fresh)?;
    let predicate = match &tp.predicate {
        NamedNodePattern::NamedNode(n) => SlotPattern::Term(oxrdf::Term::NamedNode(n.clone())),
        NamedNodePattern::Variable(v) => SlotPattern::Var(v.as_str().to_string()),
    };
    let object = term_slot_ext(&tp.object, fresh)?;
    Ok(PatternKey { slots: [subject, predicate, object] })
}

fn term_slot_ext(t: &TermPattern, fresh: &mut FreshNames) -> Result<SlotPattern, VerifyError> {
    match t {
        TermPattern::NamedNode(n) => Ok(SlotPattern::Term(oxrdf::Term::NamedNode(n.clone()))),
        TermPattern::Literal(l) => Ok(SlotPattern::Term(oxrdf::Term::Literal(l.clone()))),
        TermPattern::Variable(v) => Ok(SlotPattern::Var(v.as_str().to_string())),
        TermPattern::BlankNode(b) => Ok(SlotPattern::Var(fresh.bnode_var(b.as_str()))),
        other => Err(VerifyError::UnsupportedFragment(format!(
            "triple pattern term {other} (no committed leaf encoding lane)"
        ))),
    }
}

/// The [`PatternKey`] view of a [`PathReach`] obligation used for join-edge
/// analysis: its endpoints join other patterns exactly like scan-slot
/// variables (the chain interior is covered separately by
/// [`BranchObligations::path_link_non_bnode`]).
fn path_pseudo_key(p: &PathReach) -> PatternKey {
    PatternKey {
        slots: [
            p.subject.clone(),
            SlotPattern::Term(oxrdf::Term::NamedNode(p.predicate.clone())),
            p.object.clone(),
        ],
    }
}

/// Computes one branch's Q6 obligations under the manifest's public
/// attributions: `scan_attributions[i]` attributes `branch.patterns[i]`,
/// `path_attributions[i]` attributes `branch.path_reach[i]`. Extends the
/// stage-1 rule ([`cross_graph_join_obligations`]) to path-rewritten
/// patterns and path obligations UNCHANGED in the safe (coarser) direction:
/// every join edge whose two obligations admit any cross-graph pair needs a
/// non-bnode obligation, and every multi-graph path additionally needs its
/// chain links non-bnode (see [`BranchObligations`]).
pub fn branch_obligations(
    branch: &FragmentBranch,
    scan_attributions: &[BTreeSet<usize>],
    path_attributions: &[BTreeSet<usize>],
) -> Result<BranchObligations, VerifyError> {
    if branch.patterns.len() != scan_attributions.len()
        || branch.path_reach.len() != path_attributions.len()
    {
        return Err(VerifyError::ArityMismatch {
            patterns: branch.patterns.len() + branch.path_reach.len(),
            attributions: scan_attributions.len() + path_attributions.len(),
        });
    }
    let mut keys = branch.patterns.clone();
    keys.extend(branch.path_reach.iter().map(path_pseudo_key));
    let attrs: Vec<BTreeSet<usize>> = scan_attributions
        .iter()
        .chain(path_attributions.iter())
        .cloned()
        .collect();
    let join_edges = cross_graph_join_obligations(&keys, &attrs)?;
    let path_link_non_bnode = path_attributions
        .iter()
        .enumerate()
        .filter(|(_, a)| a.len() > 1)
        .map(|(i, _)| i)
        .collect();
    Ok(BranchObligations { join_edges, path_link_non_bnode })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(sets: &[&[usize]]) -> Vec<BTreeSet<usize>> {
        sets.iter().map(|s| s.iter().copied().collect()).collect()
    }

    #[test]
    fn patterns_re_derived_in_query_order() {
        let keys = fragment_patterns(
            "SELECT ?n WHERE { ?p <http://ex/worksAt> ?org . ?org <http://ex/name> ?n . FILTER(?n != \"x\") }",
        )
        .unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].slots[2], SlotPattern::Var("org".into()));
        assert_eq!(keys[1].slots[0], SlotPattern::Var("org".into()));
        assert!(matches!(&keys[0].slots[1], SlotPattern::Term(t) if t.to_string().contains("worksAt")));
    }

    #[test]
    fn outside_fragment_fails_closed() {
        for (q, what) in [
            ("SELECT * WHERE { ?s ?p ?o OPTIONAL { ?o ?q ?r } }", "OPTIONAL"),
            ("SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }", "GRAPH"),
            ("SELECT * WHERE { { ?s ?p ?o } UNION { ?o ?p ?s } }", "UNION"),
            // One-or-more survives as an algebra-level Path…
            ("SELECT * WHERE { ?s <http://ex/a>+ ?o }", "property path"),
            // …while a fixed-length sequence path is flattened by the parser
            // into a BGP with an intermediate blank node — caught by the
            // bnode-in-pattern rejection (still fail-closed).
            ("SELECT * WHERE { ?s <http://ex/a>/<http://ex/b> ?o }", "blank node"),
            ("SELECT * WHERE { ?s ?p ?o BIND(1 AS ?x) }", "BIND"),
            (
                "SELECT * WHERE { ?s ?p ?o FILTER NOT EXISTS { ?o ?q ?r } }",
                "EXISTS",
            ),
            ("SELECT * WHERE { _:b ?p ?o }", "blank node"),
        ] {
            let err = fragment_patterns(q).unwrap_err();
            match err {
                VerifyError::UnsupportedFragment(msg) => {
                    assert!(msg.contains(what), "{q}: expected {what}, got {msg}")
                }
                other => panic!("{q}: expected UnsupportedFragment, got {other:?}"),
            }
        }
        assert!(matches!(
            fragment_patterns("SELECT WHERE"),
            Err(VerifyError::Parse(_))
        ));
    }

    /// sq-p2hci: a dataset clause RE-DEFINES the dataset the query is
    /// evaluated over, but the stage-1 statement is fixed — "these solutions
    /// are in the answer over the COMMITTED union". A consumer reading
    /// `SELECT * FROM <g1> …` off the manifest would read the verdict as a
    /// claim about `g1` alone, while the proof binds scans attributed to any
    /// committed graph. The gate must fail closed on the clause instead of
    /// silently ignoring it (the wave-1 gate already does).
    #[test]
    fn stage1_gate_rejects_dataset_clauses() {
        for q in [
            "SELECT * FROM <http://ex/g1> WHERE { ?s <http://ex/p> ?o }",
            "SELECT * FROM NAMED <http://ex/g1> WHERE { ?s <http://ex/p> ?o }",
            "SELECT * FROM <http://ex/g1> FROM NAMED <http://ex/g2> WHERE { ?s <http://ex/p> ?o }",
            "ASK FROM <http://ex/g1> WHERE { ?s <http://ex/p> ?o }",
        ] {
            for (entry, got) in [
                ("fragment_patterns", fragment_patterns(q).err()),
                ("fragment_filters", fragment_filters(q).err()),
                ("fragment_expr_trees", fragment_expr_trees(q).err()),
                ("fragment_query", fragment_query(q).err()),
                ("recheck", recheck(q, &attrs(&[&[0]]), &[]).err()),
            ] {
                match got {
                    Some(VerifyError::UnsupportedFragment(msg)) => assert!(
                        msg.contains("FROM / FROM NAMED"),
                        "{entry}({q}): expected the dataset-clause reason, got `{msg}`"
                    ),
                    other => panic!("{entry}({q}): expected UnsupportedFragment, got {other:?}"),
                }
            }
        }
        // A dataset-free query of the same shape still passes every entry
        // point — the rejection is the clause, not the shape.
        let q = "SELECT * WHERE { ?s <http://ex/p> ?o }";
        assert_eq!(fragment_patterns(q).unwrap().len(), 1);
        assert!(fragment_filters(q).unwrap().is_empty());
        assert!(fragment_expr_trees(q).unwrap().is_empty());
        assert!(fragment_query(q).is_ok());
        assert!(recheck(q, &attrs(&[&[0]]), &[]).unwrap().is_empty());
    }

    #[test]
    fn same_single_graph_join_needs_no_obligation() {
        let keys = fragment_patterns(
            "SELECT * WHERE { ?s <http://ex/p> ?x . ?x <http://ex/q> ?o }",
        )
        .unwrap();
        let edges = cross_graph_join_obligations(&keys, &attrs(&[&[0], &[0]])).unwrap();
        assert!(edges.is_empty(), "within-graph joins may bind bnodes");
    }

    #[test]
    fn cross_graph_join_requires_obligation() {
        let keys = fragment_patterns(
            "SELECT * WHERE { ?s <http://ex/p> ?x . ?x <http://ex/q> ?o }",
        )
        .unwrap();
        // Disjoint attributions.
        let edges = cross_graph_join_obligations(&keys, &attrs(&[&[0], &[1]])).unwrap();
        assert_eq!(
            edges,
            vec![JoinEdge { variable: "x".into(), patterns: (0, 1) }]
        );
        // Overlapping-but-plural attributions are still cross-capable
        // (coarser than the prover guard, by design).
        let edges = cross_graph_join_obligations(&keys, &attrs(&[&[0], &[0, 1]])).unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn recheck_rejects_undeclared_edge_and_accepts_declared() {
        let q = "SELECT * WHERE { ?s <http://ex/p> ?x . ?x <http://ex/q> ?o }";
        let a = attrs(&[&[0], &[1]]);
        let err = recheck(q, &a, &[]).unwrap_err();
        assert!(matches!(err, VerifyError::MissingObligation(_)));
        let edge = JoinEdge { variable: "x".into(), patterns: (0, 1) };
        let required = recheck(q, &a, std::slice::from_ref(&edge)).unwrap();
        assert_eq!(required, vec![edge]);
    }

    #[test]
    fn malformed_manifests_rejected() {
        let keys =
            fragment_patterns("SELECT * WHERE { ?s <http://ex/p> ?x . ?x <http://ex/q> ?o }")
                .unwrap();
        assert_eq!(
            cross_graph_join_obligations(&keys, &attrs(&[&[0]])),
            Err(VerifyError::ArityMismatch { patterns: 2, attributions: 1 })
        );
        assert_eq!(
            cross_graph_join_obligations(&keys, &attrs(&[&[0], &[]])),
            Err(VerifyError::EmptyAttribution { pattern: 1 })
        );
    }

    // [OPUS-4.8] FILTER query-binding extraction tests (audit #5/#6/#10).

    fn xint(v: u64) -> String {
        format!(
            "\"{v}\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        )
    }

    #[test]
    fn extracts_integer_filter_in_both_operand_orders() {
        let q = format!(
            "SELECT ?s ?age WHERE {{ ?s <http://ex/age> ?age FILTER(?age >= {}) }}",
            xint(18)
        );
        let fs = fragment_filters(&q).unwrap();
        assert_eq!(
            fs,
            vec![QueryFilter { variable: "age".into(), op: FilterCmp::Ge, bound: 18 }]
        );
        // `const op ?var` flips to `?var op' const`.
        let q2 = format!(
            "SELECT ?s ?age WHERE {{ ?s <http://ex/age> ?age FILTER({} <= ?age) }}",
            xint(18)
        );
        let fs2 = fragment_filters(&q2).unwrap();
        assert_eq!(
            fs2,
            vec![QueryFilter { variable: "age".into(), op: FilterCmp::Ge, bound: 18 }],
            "18 <= ?age must normalize to ?age >= 18"
        );
    }

    #[test]
    fn flattens_conjoined_filters() {
        let q = format!(
            "SELECT ?s ?age WHERE {{ ?s <http://ex/age> ?age FILTER(?age >= {} && ?age < {}) }}",
            xint(18),
            xint(65)
        );
        let fs = fragment_filters(&q).unwrap();
        assert_eq!(
            fs,
            vec![
                QueryFilter { variable: "age".into(), op: FilterCmp::Ge, bound: 18 },
                QueryFilter { variable: "age".into(), op: FilterCmp::Lt, bound: 65 },
            ]
        );
    }

    // [OPUS-4.8] roborev codex 2207: SPARQL `!=` binds to `FilterCmp::Ne`.
    // spargebra has no `NotEqual` node — `?v != c` parses to `Not(Equal(..))`.
    #[test]
    fn extracts_not_equal_filter_in_both_operand_orders() {
        // `?var != const` → Ne.
        let q = format!(
            "SELECT ?s ?age WHERE {{ ?s <http://ex/age> ?age FILTER(?age != {}) }}",
            xint(18)
        );
        let fs = fragment_filters(&q).unwrap();
        assert_eq!(
            fs,
            vec![QueryFilter { variable: "age".into(), op: FilterCmp::Ne, bound: 18 }],
            "?age != 18 must bind to Ne"
        );
        // `const != ?var` → Ne (Ne is symmetric, so the flip is a no-op).
        let q2 = format!(
            "SELECT ?s ?age WHERE {{ ?s <http://ex/age> ?age FILTER({} != ?age) }}",
            xint(18)
        );
        let fs2 = fragment_filters(&q2).unwrap();
        assert_eq!(
            fs2,
            vec![QueryFilter { variable: "age".into(), op: FilterCmp::Ne, bound: 18 }],
            "18 != ?age must also normalize to ?age != 18 (Ne)"
        );
    }

    #[test]
    fn not_equal_flattens_with_conjoined_comparisons() {
        // `!=` is a first-class conjunct alongside ordered comparisons.
        let q = format!(
            "SELECT ?s ?age WHERE {{ ?s <http://ex/age> ?age FILTER(?age >= {} && ?age != {}) }}",
            xint(18),
            xint(21)
        );
        let fs = fragment_filters(&q).unwrap();
        assert_eq!(
            fs,
            vec![
                QueryFilter { variable: "age".into(), op: FilterCmp::Ge, bound: 18 },
                QueryFilter { variable: "age".into(), op: FilterCmp::Ne, bound: 21 },
            ]
        );
    }

    #[test]
    fn unbindable_not_equal_filters_fail_closed() {
        // `!=` must NOT loosen the bindable fragment: a non-integer, var-var,
        // or non-canonical `!=` operand still fails closed, exactly like the
        // ordered comparisons. A general `!(...)` over a non-equality is also
        // rejected.
        for q in [
            // var vs var `!=` → Not(Equal(?a,?b)), unbindable
            "SELECT ?s WHERE { ?s <http://ex/a> ?a . ?s <http://ex/b> ?b FILTER(?a != ?b) }",
            // plain string (no xsd:integer datatype)
            "SELECT ?s WHERE { ?s <http://ex/h> ?h FILTER(?h != \"18\") }",
            // float literal
            "SELECT ?s WHERE { ?s <http://ex/h> ?h FILTER(?h != \"1.5\"^^<http://www.w3.org/2001/XMLSchema#double>) }",
            // non-canonical leading-zero integer
            "SELECT ?s WHERE { ?s <http://ex/age> ?a FILTER(?a != \"018\"^^<http://www.w3.org/2001/XMLSchema#integer>) }",
            // negation of a non-equality (e.g. `!(?a > c)`) is NOT a bindable Ne
            "SELECT ?s WHERE { ?s <http://ex/age> ?a FILTER(!(?a > \"18\"^^<http://www.w3.org/2001/XMLSchema#integer>)) }",
        ] {
            assert!(
                matches!(fragment_filters(q), Err(VerifyError::UnsupportedFragment(_))),
                "must fail closed: {q}"
            );
        }
    }

    #[test]
    fn no_filter_yields_empty() {
        let fs = fragment_filters("SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }").unwrap();
        assert!(fs.is_empty());
    }

    #[test]
    fn unbindable_filters_fail_closed() {
        // Non-integer / arithmetic / var-var / disjunction / non-canonical:
        // each is OUTSIDE the bindable fragment and must REJECT (never be
        // silently disclosed unproven — audit #10).
        for q in [
            // float literal
            "SELECT ?s WHERE { ?s <http://ex/h> ?h FILTER(?h >= \"1.5\"^^<http://www.w3.org/2001/XMLSchema#double>) }",
            // plain string (no xsd:integer datatype)
            "SELECT ?s WHERE { ?s <http://ex/h> ?h FILTER(?h >= \"18\") }",
            // var vs var
            "SELECT ?s WHERE { ?s <http://ex/a> ?a . ?s <http://ex/b> ?b FILTER(?a >= ?b) }",
            // disjunction (not a flat AND of comparisons)
            "SELECT ?s WHERE { ?s <http://ex/age> ?a FILTER(?a >= \"18\"^^<http://www.w3.org/2001/XMLSchema#integer> || ?a < \"0\"^^<http://www.w3.org/2001/XMLSchema#integer>) }",
            // arithmetic operand
            "SELECT ?s WHERE { ?s <http://ex/age> ?a FILTER(?a + \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> >= \"18\"^^<http://www.w3.org/2001/XMLSchema#integer>) }",
            // non-canonical leading-zero integer
            "SELECT ?s WHERE { ?s <http://ex/age> ?a FILTER(?a >= \"018\"^^<http://www.w3.org/2001/XMLSchema#integer>) }",
        ] {
            assert!(
                matches!(fragment_filters(q), Err(VerifyError::UnsupportedFragment(_))),
                "must fail closed: {q}"
            );
        }
    }

    // =====================================================================
    // Wave-1 extended fragment gate (sq-3kd2g.3): two-sided table tests.
    // The REJECT side is the security surface — every OUT construct of the
    // design record's §3 table must fail closed with its documented reason.
    // =====================================================================

    fn var(name: &str) -> SlotPattern {
        SlotPattern::Var(name.into())
    }

    fn pred(iri: &str) -> SlotPattern {
        SlotPattern::Term(oxrdf::Term::NamedNode(oxrdf::NamedNode::new(iri).unwrap()))
    }

    #[test]
    fn wave1_closures_map_to_path_reach_family() {
        for (q, closure) in [
            ("SELECT * WHERE { ?s <http://ex/a>+ ?o }", PathClosure::OneOrMore),
            ("SELECT * WHERE { ?s <http://ex/a>* ?o }", PathClosure::ZeroOrMore),
            ("SELECT * WHERE { ?s <http://ex/a>? ?o }", PathClosure::ZeroOrOne),
        ] {
            let fq = fragment_query(q).unwrap();
            assert!(!fq.ask);
            assert_eq!(fq.branches.len(), 1, "{q}");
            let b = &fq.branches[0];
            assert!(b.patterns.is_empty() && b.filters.is_empty() && b.values.is_empty());
            assert_eq!(
                b.path_reach,
                vec![PathReach {
                    subject: var("s"),
                    predicate: oxrdf::NamedNode::new("http://ex/a").unwrap(),
                    object: var("o"),
                    closure,
                }],
                "{q}"
            );
        }
        // The statement-family parameters are pinned per closure kind.
        assert_eq!(PathClosure::OneOrMore.min_len(), 1);
        assert_eq!(PathClosure::ZeroOrMore.min_len(), 0);
        assert_eq!(PathClosure::ZeroOrOne.min_len(), 0);
        assert_eq!(PathClosure::ZeroOrOne.fixed_k(), Some(1));
        assert_eq!(PathClosure::ZeroOrMore.fixed_k(), None);
        assert_eq!(PathClosure::OneOrMore.fixed_k(), None);
    }

    #[test]
    fn wave1_inverse_closure_swaps_endpoints() {
        // `(^a)+` from ?s to ?o is `a+` from ?o to ?s.
        let fq = fragment_query("SELECT * WHERE { ?s (^<http://ex/a>)+ ?o }").unwrap();
        assert_eq!(fq.branches.len(), 1);
        assert_eq!(
            fq.branches[0].path_reach,
            vec![PathReach {
                subject: var("o"),
                predicate: oxrdf::NamedNode::new("http://ex/a").unwrap(),
                object: var("s"),
                closure: PathClosure::OneOrMore,
            }]
        );
    }

    #[test]
    fn wave1_alternative_desugars_to_union_branches() {
        let fq =
            fragment_query("SELECT * WHERE { ?s <http://ex/a>|<http://ex/b> ?o }").unwrap();
        assert_eq!(fq.branches.len(), 2);
        for (branch, iri) in fq.branches.iter().zip(["http://ex/a", "http://ex/b"]) {
            assert_eq!(
                branch.patterns,
                vec![PatternKey { slots: [var("s"), pred(iri), var("o")] }]
            );
            assert!(branch.path_reach.is_empty());
        }
    }

    #[test]
    fn wave1_sequence_path_uses_deterministic_intermediates() {
        // The parser flattens `a/b` into a BGP with an ANONYMOUS bnode whose
        // label is random per parse; the gate must rename it into the
        // reserved deterministic namespace so prover and verifier parses
        // agree.
        let q = "SELECT * WHERE { ?s <http://ex/a>/<http://ex/b> ?o }";
        let fq = fragment_query(q).unwrap();
        assert_eq!(fq.branches.len(), 1);
        let pats = &fq.branches[0].patterns;
        assert_eq!(pats.len(), 2);
        assert_eq!(pats[0].slots[0], var("s"));
        assert_eq!(pats[0].slots[1], pred("http://ex/a"));
        assert_eq!(pats[0].slots[2], var("!bnode:0"));
        assert_eq!(pats[1].slots[0], var("!bnode:0"));
        assert_eq!(pats[1].slots[1], pred("http://ex/b"));
        assert_eq!(pats[1].slots[2], var("o"));
        // Two independent parses derive the identical structure.
        assert_eq!(fq, fragment_query(q).unwrap());
    }

    #[test]
    fn wave1_sequence_inside_closure_body_rejected_but_plain_reverse_ok() {
        // `^(a/b)` flattens in the parser (no closure) — accepted as two
        // patterns with swapped endpoints.
        let fq =
            fragment_query("SELECT * WHERE { ?s ^(<http://ex/a>/<http://ex/b>) ?o }").unwrap();
        assert_eq!(fq.branches[0].patterns.len(), 2);
        assert_eq!(fq.branches[0].patterns[0].slots[0], var("o"));
        assert_eq!(fq.branches[0].patterns[1].slots[2], var("s"));
    }

    #[test]
    fn wave1_union_distributes_over_join() {
        let fq = fragment_query(
            "SELECT * WHERE { ?x <http://ex/p> ?y . { { ?y <http://ex/a> ?z } UNION { ?y <http://ex/b> ?z } } }",
        )
        .unwrap();
        assert_eq!(fq.branches.len(), 2);
        for (branch, iri) in fq.branches.iter().zip(["http://ex/a", "http://ex/b"]) {
            assert_eq!(branch.patterns.len(), 2, "join distributes into each branch");
            assert_eq!(branch.patterns[0].slots[1], pred("http://ex/p"));
            assert_eq!(branch.patterns[1].slots[1], pred(iri));
        }
    }

    #[test]
    fn wave1_values_rows_re_derived_with_undef_wildcards() {
        let fq = fragment_query(
            "SELECT * WHERE { VALUES (?a ?b) { (<http://ex/1> UNDEF) (\"x\" \"y\") } ?s <http://ex/p> ?a }",
        )
        .unwrap();
        assert_eq!(fq.branches.len(), 1);
        let b = &fq.branches[0];
        assert_eq!(b.patterns.len(), 1);
        assert_eq!(b.values.len(), 1);
        let vb = &b.values[0];
        assert_eq!(vb.variables, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(vb.rows.len(), 2);
        assert_eq!(
            vb.rows[0],
            vec![
                Some(oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://ex/1").unwrap())),
                None
            ]
        );
        assert!(vb.rows[1].iter().all(Option::is_some));
    }

    #[test]
    fn wave1_subquery_renames_inner_vars_apart() {
        // Outer ?x is shared (projected by the subquery); inner ?y is
        // existential and must NOT be conflated with any outer ?y.
        let q = "SELECT * WHERE { { SELECT ?x WHERE { ?x <http://ex/p> ?y } } ?x <http://ex/q> ?y }";
        let fq = fragment_query(q).unwrap();
        assert_eq!(fq.branches.len(), 1);
        let pats = &fq.branches[0].patterns;
        assert_eq!(pats.len(), 2);
        assert_eq!(pats[0].slots[0], var("x"), "projected subquery var kept");
        assert_eq!(pats[0].slots[2], var("!subq0:y"), "inner-only var renamed apart");
        assert_eq!(pats[1].slots[2], var("y"), "outer ?y untouched");
        // Determinism across parses.
        assert_eq!(fq, fragment_query(q).unwrap());
    }

    #[test]
    fn wave1_subquery_renames_filters_and_paths() {
        let q = format!(
            "SELECT * WHERE {{ {{ SELECT ?x WHERE {{ ?x <http://ex/a>+ ?y FILTER(?y >= {}) }} }} }}",
            xint(18)
        );
        let fq = fragment_query(&q).unwrap();
        let b = &fq.branches[0];
        assert_eq!(b.path_reach.len(), 1);
        assert_eq!(b.path_reach[0].subject, var("x"));
        assert_eq!(b.path_reach[0].object, var("!subq0:y"));
        assert_eq!(
            b.filters,
            vec![QueryFilter { variable: "!subq0:y".into(), op: FilterCmp::Ge, bound: 18 }]
        );
    }

    #[test]
    fn wave1_filter_distributes_into_union_branches() {
        let q = format!(
            "SELECT * WHERE {{ {{ {{ ?s <http://ex/a> ?age }} UNION {{ ?s <http://ex/b> ?age }} }} FILTER(?age >= {}) }}",
            xint(18)
        );
        let fq = fragment_query(&q).unwrap();
        assert_eq!(fq.branches.len(), 2);
        for b in &fq.branches {
            assert_eq!(
                b.filters,
                vec![QueryFilter { variable: "age".into(), op: FilterCmp::Ge, bound: 18 }],
                "a FILTER above a UNION constrains every branch"
            );
        }
    }

    #[test]
    fn wave1_ask_and_top_level_modifiers_accepted() {
        let fq = fragment_query("ASK { ?s <http://ex/a>* ?o }").unwrap();
        assert!(fq.ask);
        assert!(fq.projected.is_empty());
        assert_eq!(fq.branches[0].path_reach[0].closure, PathClosure::ZeroOrMore);

        let fq = fragment_query(
            "SELECT DISTINCT ?s WHERE { ?s <http://ex/p> ?o } LIMIT 5 OFFSET 2",
        )
        .unwrap();
        assert_eq!(fq.projected, vec!["s".to_string()]);
        assert_eq!(fq.branches[0].patterns.len(), 1);
    }

    #[test]
    fn wave1_user_bnodes_are_nondistinguished_variables() {
        // A query bnode denotes a non-distinguished variable; same label =
        // same variable, deterministically renamed.
        let fq = fragment_query(
            "SELECT * WHERE { _:b <http://ex/p> ?o . _:b <http://ex/q> ?o2 }",
        )
        .unwrap();
        let pats = &fq.branches[0].patterns;
        assert_eq!(pats[0].slots[0], var("!bnode:0"));
        assert_eq!(pats[1].slots[0], var("!bnode:0"), "shared label, shared variable");
    }

    #[test]
    fn wave1_branch_obligations_cover_paths_coarsely() {
        let fq = fragment_query(
            "SELECT * WHERE { ?s <http://ex/p> ?x . ?x <http://ex/a>+ ?o }",
        )
        .unwrap();
        let b = &fq.branches[0];
        assert_eq!((b.patterns.len(), b.path_reach.len()), (1, 1));

        // Single shared graph: no obligations (within-graph bnode identity
        // is exact — stage-1 rule unchanged).
        let ob = branch_obligations(b, &attrs(&[&[0]]), &attrs(&[&[0]])).unwrap();
        assert!(ob.join_edges.is_empty() && ob.path_link_non_bnode.is_empty());

        // Multi-graph path attribution: the ?x endpoint join edge is
        // required (pseudo-pattern index = patterns.len() + path index) AND
        // the chain links must be non-bnode.
        let ob = branch_obligations(b, &attrs(&[&[0]]), &attrs(&[&[0, 1]])).unwrap();
        assert_eq!(
            ob.join_edges,
            vec![JoinEdge { variable: "x".into(), patterns: (0, 1) }]
        );
        assert_eq!(ob.path_link_non_bnode, vec![0]);

        // Arity is checked per obligation list (split mismatches with a
        // matching total are still rejected).
        assert_eq!(
            branch_obligations(b, &attrs(&[&[0], &[0]]), &attrs(&[])),
            Err(VerifyError::ArityMismatch { patterns: 2, attributions: 2 })
        );
        // Empty attribution sets are rejected (inherited from the stage-1
        // rule; the path obligation occupies the combined index space).
        assert_eq!(
            branch_obligations(b, &attrs(&[&[0]]), &attrs(&[&[]])),
            Err(VerifyError::EmptyAttribution { pattern: 1 })
        );
    }

    #[test]
    fn wave1_join_carries_right_side_obligations() {
        // Obligations on the RIGHT side of a join (filters, VALUES,
        // patterns) must survive the distribution — dropping any of them
        // would under-constrain the derived statement.
        let q = format!(
            "SELECT * WHERE {{ ?s <http://ex/p> ?x . {{ ?x <http://ex/q> ?y FILTER(?y >= {}) VALUES ?y {{ {} }} }} }}",
            xint(18),
            xint(21)
        );
        let fq = fragment_query(&q).unwrap();
        assert_eq!(fq.branches.len(), 1);
        let b = &fq.branches[0];
        assert_eq!(b.patterns.len(), 2);
        assert_eq!(
            b.filters,
            vec![QueryFilter { variable: "y".into(), op: FilterCmp::Ge, bound: 18 }]
        );
        assert_eq!(b.values.len(), 1);
        assert_eq!(b.values[0].variables, vec!["y".to_string()]);
    }

    #[test]
    fn wave1_out_constructs_fail_closed() {
        // The REJECT side of the two-sided table: every OUT construct of
        // the design record §3/§4 tables, with its documented reason. This
        // list is the security surface — the gate must never accept a query
        // whose verified statement the circuit family cannot express.
        for (q, why) in [
            // Non-monotone operators (closed-world claims).
            ("SELECT * WHERE { ?s ?p ?o OPTIONAL { ?o ?q ?r } }", "OPTIONAL"),
            ("SELECT * WHERE { ?s ?p ?o MINUS { ?o ?q ?r } }", "MINUS"),
            ("SELECT * WHERE { ?s ?p ?o FILTER NOT EXISTS { ?o ?q ?r } }", "EXISTS"),
            // Positive EXISTS is monotone but DEFERRED (1.2 semantics pin).
            ("SELECT * WHERE { ?s ?p ?o FILTER EXISTS { ?o ?q ?r } }", "EXISTS"),
            // Privacy-model / federation exclusions.
            ("SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }", "GRAPH"),
            ("SELECT * WHERE { ?s ?p ?o SERVICE <http://ex/ep> { ?a ?b ?c } }", "SERVICE"),
            ("SELECT * FROM <http://ex/g> WHERE { ?s ?p ?o }", "FROM / FROM NAMED"),
            // Later phases / deferred forms stay out for wave 1.
            ("SELECT * WHERE { ?s ?p ?o BIND(1 AS ?x) }", "BIND"),
            ("SELECT * WHERE { ?s !(<http://ex/a>) ?o }", "negated property set"),
            // Completeness-claiming forms.
            ("SELECT (COUNT(?s) AS ?c) WHERE { ?s ?p ?o }", "aggregates"),
            ("SELECT ?o WHERE { ?s ?p ?o } GROUP BY ?o", "aggregates"),
            ("SELECT * WHERE { ?s ?p ?o } ORDER BY ?o", "ORDER BY"),
            // Result forms outside the membership property.
            ("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", "CONSTRUCT"),
            ("DESCRIBE <http://ex/x>", "DESCRIBE"),
            // Subquery composed of NON-fragment operators.
            (
                "SELECT * WHERE { { SELECT ?x WHERE { ?x <http://ex/p> ?y } LIMIT 3 } }",
                "LIMIT / OFFSET inside a subquery",
            ),
            (
                "SELECT * WHERE { { SELECT ?x WHERE { ?x <http://ex/p> ?y } ORDER BY ?x } }",
                "ORDER BY",
            ),
            // Closures the path_reach family cannot express (chains carry
            // ONE predicate).
            ("SELECT * WHERE { ?s (<http://ex/a>/<http://ex/b>)+ ?o }", "non-atomic"),
            ("SELECT * WHERE { ?s (<http://ex/a>|<http://ex/b>)* ?o }", "non-atomic"),
            ("SELECT * WHERE { ?s (<http://ex/a>+)+ ?o }", "non-atomic"),
            // Feature-gated / unknown algebra nodes (sep-0006 LATERAL).
            ("SELECT * WHERE { ?s <http://ex/p> ?o LATERAL { ?o <http://ex/q> ?r } }", "Lateral"),
            // Triple terms have no committed leaf encoding lane.
            (
                "SELECT * WHERE { VALUES ?t { <<(<http://ex/s> <http://ex/p> <http://ex/o>)>> } ?t <http://ex/q> ?z }",
                "triple term",
            ),
            // FILTERs outside the bindable fragment reject the whole query
            // (never silently disclosed unproven).
            (
                "SELECT * WHERE { ?s <http://ex/h> ?h FILTER(?h >= \"1.5\"^^<http://www.w3.org/2001/XMLSchema#double>) }",
                "FILTER",
            ),
            (
                "SELECT * WHERE { ?s <http://ex/a> ?a . ?s <http://ex/b> ?b FILTER(?a >= ?b) }",
                "FILTER",
            ),
        ] {
            match fragment_query(q) {
                Err(VerifyError::UnsupportedFragment(msg)) => {
                    assert!(msg.contains(why), "{q}: expected reason `{why}`, got `{msg}`")
                }
                other => panic!("{q}: expected UnsupportedFragment({why}), got {other:?}"),
            }
        }
        assert!(matches!(fragment_query("SELECT WHERE"), Err(VerifyError::Parse(_))));
    }

    #[test]
    fn wave1_branch_expansion_cap_fails_closed() {
        // 65 path alternatives expand past MAX_FRAGMENT_BRANCHES.
        let alts = (0..=MAX_FRAGMENT_BRANCHES)
            .map(|i| format!("<http://ex/p{i}>"))
            .collect::<Vec<_>>()
            .join("|");
        let q = format!("SELECT * WHERE {{ ?s ({alts}) ?o }}");
        match fragment_query(&q) {
            Err(VerifyError::UnsupportedFragment(msg)) => {
                assert!(msg.contains("exceeds the cap"), "got `{msg}`")
            }
            other => panic!("expected the branch cap, got {other:?}"),
        }
        // Join-side multiplication is capped too: 8 × 9 alternatives = 72.
        let alt8 = (0..8).map(|i| format!("<http://ex/a{i}>")).collect::<Vec<_>>().join("|");
        let alt9 = (0..9).map(|i| format!("<http://ex/b{i}>")).collect::<Vec<_>>().join("|");
        let q = format!("SELECT * WHERE {{ ?s ({alt8}) ?m . ?m ({alt9}) ?o }}");
        assert!(
            matches!(fragment_query(&q), Err(VerifyError::UnsupportedFragment(m)) if m.contains("exceeds the cap"))
        );
    }

    #[test]
    fn wave1_stage1_gate_is_unchanged() {
        // The wave-1 gate does NOT loosen the stage-1 entry points: the
        // stage-1 compose verifier's circuit dispatch cannot express the
        // extensions, so `fragment_patterns` must still reject them.
        for q in [
            "SELECT * WHERE { ?s <http://ex/a>+ ?o }",
            "SELECT * WHERE { { ?s <http://ex/a> ?o } UNION { ?s <http://ex/b> ?o } }",
            "SELECT * WHERE { VALUES ?a { <http://ex/1> } ?s <http://ex/p> ?a }",
        ] {
            assert!(
                matches!(fragment_patterns(q), Err(VerifyError::UnsupportedFragment(_))),
                "stage-1 gate must still reject: {q}"
            );
            assert!(fragment_query(q).is_ok(), "wave-1 gate accepts: {q}");
        }
    }

    #[test]
    fn variable_slots_maps_var_positions() {
        let keys = fragment_patterns(
            "SELECT ?s ?age WHERE { ?s <http://ex/age> ?age . ?s <http://ex/sal> ?sal }",
        )
        .unwrap();
        let vs = variable_slots(&keys);
        // ?s at (0,0) and (1,0); ?age at (0,2); ?sal at (1,2).
        assert!(vs.contains(&("s".into(), 0, 0)));
        assert!(vs.contains(&("s".into(), 1, 0)));
        assert!(vs.contains(&("age".into(), 0, 2)));
        assert!(vs.contains(&("sal".into(), 1, 2)));
    }

    // =====================================================================
    // [OPUS-4.8] sq-3kd2g.8: query-side FILTER/BIND expression-tree
    // extraction. Two-sided table: every §5.1 IN form produces its EXACT
    // expected tree; every OUT form fails closed. The REJECT side is the
    // security surface — the extractor must never emit an approximated tree
    // for a construct outside the §5.1 IN estate.
    // =====================================================================

    fn cint(v: u64) -> ExprTree {
        ExprTree::Const(oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            v.to_string(),
            oxrdf::NamedNode::new(XSD_INTEGER).unwrap(),
        )))
    }
    fn cstr(s: &str) -> ExprTree {
        ExprTree::Const(oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal(s)))
    }
    fn evar(n: &str) -> ExprTree {
        ExprTree::Var(n.into())
    }
    fn en(op: ExprOp, args: Vec<ExprTree>) -> ExprTree {
        ExprTree::Node { op, args }
    }

    /// Extract the single FILTER obligation's tree for `FILTER(<filter>)`.
    fn filter_tree(filter: &str) -> ExprTree {
        let q = format!("ASK {{ ?a <http://ex/p> ?b . ?b <http://ex/q> ?c FILTER({filter}) }}");
        let obs = fragment_expr_trees(&q).unwrap_or_else(|e| panic!("`{filter}`: {e:?}"));
        assert_eq!(obs.len(), 1, "one FILTER obligation for `{filter}`");
        match obs.into_iter().next().unwrap() {
            ExprObligation::Filter(t) => t,
            other => panic!("expected a Filter obligation, got {other:?}"),
        }
    }

    fn rejects_expr(q: &str) {
        match fragment_expr_trees(q) {
            Err(VerifyError::UnsupportedFragment(_)) => {}
            other => panic!("expected UnsupportedFragment for `{q}`, got {other:?}"),
        }
    }

    fn rejects_filter(filter: &str) {
        let q = format!("ASK {{ ?a <http://ex/p> ?b . ?b <http://ex/q> ?c FILTER({filter}) }}");
        rejects_expr(&q);
    }

    #[test]
    fn expr_in_forms_extract_to_exact_trees() {
        let xi = |v: u64| format!("\"{v}\"^^<{XSD_INTEGER}>");
        let cases: Vec<(String, ExprTree)> = vec![
            // Logical AND of two comparisons across lanes.
            (
                format!("?a >= {} && ?b < {}", xi(18), xi(5)),
                en(
                    ExprOp::And,
                    vec![
                        en(ExprOp::Ge, vec![evar("a"), cint(18)]),
                        en(ExprOp::Lt, vec![evar("b"), cint(5)]),
                    ],
                ),
            ),
            // OR + logical NOT (structural, not folded to `!=`).
            (
                format!("?a = {} || !(?b = {})", xi(1), xi(2)),
                en(
                    ExprOp::Or,
                    vec![
                        en(ExprOp::Eq, vec![evar("a"), cint(1)]),
                        en(ExprOp::Not, vec![en(ExprOp::Eq, vec![evar("b"), cint(2)])]),
                    ],
                ),
            ),
            // `!=` stays `Not(Eq)` — no dedicated Ne tag.
            (
                format!("?a != {}", xi(3)),
                en(ExprOp::Not, vec![en(ExprOp::Eq, vec![evar("a"), cint(3)])]),
            ),
            // Operand order is preserved verbatim (const on the left).
            (
                format!("{} <= ?a", xi(7)),
                en(ExprOp::Le, vec![cint(7), evar("a")]),
            ),
            // sameTerm.
            (
                "sameTerm(?a, ?b)".into(),
                en(ExprOp::SameTerm, vec![evar("a"), evar("b")]),
            ),
            // IN constant list.
            (
                format!("?a IN ({}, {})", xi(1), xi(2)),
                en(ExprOp::In, vec![evar("a"), cint(1), cint(2)]),
            ),
            // IN constant list with an IRI item.
            (
                format!("?a IN (<http://ex/x>, {})", xi(2)),
                en(
                    ExprOp::In,
                    vec![
                        evar("a"),
                        ExprTree::Const(oxrdf::Term::NamedNode(
                            oxrdf::NamedNode::new("http://ex/x").unwrap(),
                        )),
                        cint(2),
                    ],
                ),
            ),
            // NOT IN stays `Not(In)`.
            (
                format!("?a NOT IN ({})", xi(1)),
                en(ExprOp::Not, vec![en(ExprOp::In, vec![evar("a"), cint(1)])]),
            ),
            // IF ternary nested in an equality.
            (
                format!("IF(?a > {}, {}, {}) = ?b", xi(0), xi(1), xi(2)),
                en(
                    ExprOp::Eq,
                    vec![
                        en(
                            ExprOp::If,
                            vec![en(ExprOp::Gt, vec![evar("a"), cint(0)]), cint(1), cint(2)],
                        ),
                        evar("b"),
                    ],
                ),
            ),
            // COALESCE.
            (
                format!("COALESCE(?a, {}) > {}", xi(0), xi(5)),
                en(
                    ExprOp::Gt,
                    vec![en(ExprOp::Coalesce, vec![evar("a"), cint(0)]), cint(5)],
                ),
            ),
            // Static BOUND — a leaf recording only the variable.
            ("BOUND(?a)".into(), ExprTree::Bound("a".into())),
            // Term accessors.
            ("isIRI(?a)".into(), en(ExprOp::IsIri, vec![evar("a")])),
            (
                "STR(?a) = \"x\"".into(),
                en(ExprOp::Eq, vec![en(ExprOp::Str, vec![evar("a")]), cstr("x")]),
            ),
            // String functions.
            (
                "CONTAINS(?a, \"ab\")".into(),
                en(ExprOp::Contains, vec![evar("a"), cstr("ab")]),
            ),
            (
                format!("STRLEN(?a) > {}", xi(3)),
                en(ExprOp::Gt, vec![en(ExprOp::StrLen, vec![evar("a")]), cint(3)]),
            ),
            // Numeric functions + arithmetic.
            (
                format!("ABS(?a + {}) < {}", xi(1), xi(5)),
                en(
                    ExprOp::Lt,
                    vec![
                        en(ExprOp::Abs, vec![en(ExprOp::Add, vec![evar("a"), cint(1)])]),
                        cint(5),
                    ],
                ),
            ),
            // Date component accessor.
            (
                format!("YEAR(?a) >= {}", xi(2000)),
                en(ExprOp::Ge, vec![en(ExprOp::Year, vec![evar("a")]), cint(2000)]),
            ),
            // Bounded REGEX (literal + anchored).
            (
                "REGEX(?a, \"^abc$\")".into(),
                en(ExprOp::Regex, vec![evar("a"), cstr("^abc$")]),
            ),
            // Bounded REGEX with an EMPTY flags argument normalizes to 2-arg.
            (
                "REGEX(?a, \"abc\", \"\")".into(),
                en(ExprOp::Regex, vec![evar("a"), cstr("abc")]),
            ),
            // An explicit `xsd:string` typed pattern is the same accepted
            // string category as a simple literal (oxrdf normalizes them).
            (
                format!("REGEX(?a, \"abc\"^^<{XSD_STRING}>)"),
                en(ExprOp::Regex, vec![evar("a"), cstr("abc")]),
            ),
            // Bounded REPLACE (literal pattern + constant replacement) nested
            // in an equality.
            (
                "REPLACE(?a, \"ab\", \"cd\") = \"x\"".into(),
                en(
                    ExprOp::Eq,
                    vec![
                        en(ExprOp::Replace, vec![evar("a"), cstr("ab"), cstr("cd")]),
                        cstr("x"),
                    ],
                ),
            ),
        ];
        for (filter, expected) in cases {
            assert_eq!(filter_tree(&filter), expected, "tree mismatch for `{filter}`");
            // Re-derivation from an independent parse is byte-identical.
            assert_eq!(filter_tree(&filter), filter_tree(&filter), "`{filter}` not deterministic");
        }
    }

    #[test]
    fn expr_bind_target_is_extracted() {
        let q = "SELECT * WHERE { ?s <http://ex/p> ?a BIND(UCASE(?a) AS ?u) }";
        let obs = fragment_expr_trees(q).unwrap();
        assert_eq!(
            obs,
            vec![ExprObligation::Bind {
                target: "u".into(),
                expr: en(ExprOp::UCase, vec![evar("a")]),
            }]
        );
    }

    #[test]
    fn expr_obligations_are_in_evaluation_order() {
        // BIND(?u), BIND(?n over ?u), FILTER(?n): spargebra nests each later
        // step OUTSIDE the earlier one (`Filter { Extend(n) { Extend(u) {
        // Bgp }}}`), so a pre-order emission would reverse the chain. The
        // extractor must emit evaluation order — each BIND target before
        // every obligation that consumes it.
        let q = format!(
            "SELECT * WHERE {{ ?s <http://ex/p> ?a \
             BIND(UCASE(?a) AS ?u) \
             BIND(STRLEN(?u) AS ?n) \
             FILTER(?n > \"3\"^^<{XSD_INTEGER}>) }}"
        );
        let obs = fragment_expr_trees(&q).unwrap();
        assert_eq!(
            obs,
            vec![
                ExprObligation::Bind {
                    target: "u".into(),
                    expr: en(ExprOp::UCase, vec![evar("a")]),
                },
                ExprObligation::Bind {
                    target: "n".into(),
                    expr: en(ExprOp::StrLen, vec![evar("u")]),
                },
                ExprObligation::Filter(en(ExprOp::Gt, vec![evar("n"), cint(3)])),
            ]
        );
    }

    #[test]
    fn expr_leaf_vars_covers_var_and_bound_leaves() {
        // `(?a + 1 > ?b) && BOUND(?c)` references a, b, c at its leaves; a
        // constant contributes nothing.
        let tree = filter_tree(&format!(
            "?a + \"1\"^^<{XSD_INTEGER}> > ?b && BOUND(?c)"
        ));
        let vars = expr_leaf_vars(&tree);
        assert_eq!(
            vars,
            ["a", "b", "c"].iter().map(|s| s.to_string()).collect()
        );
    }

    #[test]
    fn expr_out_forms_fail_closed() {
        // EXISTS / NOT EXISTS anywhere in a FILTER/BIND expression.
        rejects_expr("ASK { ?a <http://ex/p> ?b FILTER EXISTS { ?a <http://ex/q> ?c } }");
        rejects_expr("ASK { ?a <http://ex/p> ?b FILTER NOT EXISTS { ?a <http://ex/q> ?c } }");
        rejects_filter("?a > \"0\"^^<http://www.w3.org/2001/XMLSchema#integer> && EXISTS { ?a <http://ex/q> ?c }");

        // Aggregates-in-expression (a HAVING over an aggregate Group).
        rejects_expr(
            "SELECT ?s WHERE { ?s <http://ex/p> ?o } GROUP BY ?s HAVING(COUNT(?o) > \"1\"^^<http://www.w3.org/2001/XMLSchema#integer>)",
        );

        // Non-deterministic builtins (§5.1 OUT — nothing provable).
        rejects_filter("RAND() < ?b");
        rejects_filter("?a < NOW()");
        rejects_filter("?a = UUID()");
        rejects_filter("?a = STRUUID()");
        rejects_filter("isBlank(BNODE())");

        // IN with a non-constant RHS item (§5.1: constant list only).
        rejects_filter("?a IN (?b)"); // variable item
        rejects_filter(
            "?a IN (\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>, ?c + \"1\"^^<http://www.w3.org/2001/XMLSchema#integer>)",
        ); // computed item

        // REGEX/REPLACE constant operands must be STRING literals — a typed
        // non-string literal or a language-tagged string fails closed.
        rejects_filter("REGEX(?a, 123)"); // numeric pattern
        rejects_filter("REGEX(?a, \"abc\"@en)"); // language-tagged pattern
        rejects_filter("REGEX(?a, \"abc\", true)"); // non-string flags
        rejects_filter("REPLACE(?a, \"ab\", 12) = \"x\""); // non-string replacement

        // Full REGEX beyond the bounded literal/anchored subset.
        rejects_filter("REGEX(?a, \"a.c\")"); // metacharacter `.`
        rejects_filter("REGEX(?a, \"[a-z]+\")"); // char class + quantifier
        rejects_filter("REGEX(?a, ?b)"); // non-constant pattern
        rejects_filter("REGEX(?a, \"abc\", \"i\")"); // non-empty flags
        rejects_filter("REGEX(?a, \"a^b\")"); // interior anchor
        rejects_filter("REPLACE(?a, \"a.b\", \"c\") = \"x\""); // metachar pattern
        rejects_filter("REPLACE(?a, \"ab\", ?b) = \"x\""); // non-constant replacement

        // Term constructors (deferred) and estate-gap functions.
        rejects_filter("?a = IRI(\"http://ex/z\")");
        rejects_filter("langMatches(LANG(?a), \"en\")");
        rejects_filter("?a = SHA256(\"x\")");
        rejects_filter("TZ(?a) = \"Z\"");

        // A structurally-unsupported spine (UNION) also fails closed here.
        rejects_expr("ASK { { ?a <http://ex/p> ?b } UNION { ?a <http://ex/q> ?b } }");
    }
}
