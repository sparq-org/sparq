// [GPT-6] Bounded published-SPARQL-1.1 substitution for MINUS inside EXISTS.
use super::{Bindings, Expression, Graph, GraphPattern, LocalVocab, Term, TermPattern};
use rustc_hash::FxHashMap;
use spargebra::term::Variable;

/// Selects the domain-sensitive branch without crossing unresolved scopes.
pub(super) fn required(pattern: &GraphPattern, captures: &FxHashMap<Variable, Term>) -> bool {
    let mut pending = vec![pattern];
    let mut minus = false;
    let mut structural_capture = false;
    let mut non_ground_capture = false;
    while let Some(p) = pending.pop() {
        match p {
            GraphPattern::Bgp { patterns } => {
                if patterns.iter().any(|p| {
                    matches!(p.subject, TermPattern::Triple(_))
                        || matches!(p.object, TermPattern::Triple(_))
                }) {
                    return false;
                }
                p.on_in_scope_variable(|v| {
                    if let Some(term) = captures.get(v) {
                        structural_capture = true;
                        non_ground_capture |=
                            !matches!(term, Term::NamedNode(_) | Term::Literal(_));
                    }
                });
            }
            GraphPattern::Minus { left, right } => {
                minus = true;
                pending.extend([left.as_ref(), right.as_ref()]);
            }
            GraphPattern::Join { left, right } | GraphPattern::Union { left, right } => {
                pending.extend([left.as_ref(), right.as_ref()]);
            }
            GraphPattern::Filter { expr, inner } => {
                if !expression_admitted(expr, captures) {
                    return false;
                }
                pending.push(inner);
            }
            // Project, VALUES, Extend and nested EXISTS have separate scope or
            // variable-only-position problems. Paths and graph switches also
            // remain outside this first domain repair.
            _ => return false,
        }
    }
    // Keep undefined blank substitution and unsupported algebra on the existing
    // native practical path. The proved profile excludes those combinations.
    minus && structural_capture && !non_ground_capture
}

fn expression_admitted(expr: &Expression, captures: &FxHashMap<Variable, Term>) -> bool {
    use Expression as E;
    let mut pending = vec![expr];
    while let Some(e) = pending.pop() {
        match e {
            E::Exists(_) => return false,
            E::Bound(v) if captures.contains_key(v) => return false,
            E::Variable(v)
                if captures
                    .get(v)
                    .is_some_and(|term| !matches!(term, Term::NamedNode(_) | Term::Literal(_))) =>
            {
                return false;
            }
            E::NamedNode(_) | E::Literal(_) | E::Variable(_) | E::Bound(_) => {}
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
            | E::Divide(a, b) => pending.extend([a.as_ref(), b.as_ref()]),
            E::UnaryPlus(e) | E::UnaryMinus(e) | E::Not(e) => pending.push(e),
            E::If(a, b, c) => pending.extend([a.as_ref(), b.as_ref(), c.as_ref()]),
            E::In(e, args) => {
                pending.push(e);
                pending.extend(args.iter());
            }
            E::Coalesce(args) | E::FunctionCall(_, args) => pending.extend(args.iter()),
        }
    }
    true
}

/// Filters exact captured terms, then removes their solution-domain columns.
pub(super) fn restrict(graph: &Graph, local: &LocalVocab, mut bindings: Bindings) -> Bindings {
    let captured: Vec<_> = bindings
        .vars
        .iter()
        .enumerate()
        .filter_map(|(column, variable)| {
            local
                .correlation
                .get(variable)
                .map(|term| (column, graph.id_of(term)))
        })
        .collect();
    if captured.is_empty() {
        return bindings;
    }
    // This branch contains only BGP leaves: each variable in a leaf's domain
    // is bound. UNION introduces UNBOUND only after each branch was restricted.
    bindings.rows.retain(|row| {
        captured
            .iter()
            .all(|&(column, expected)| expected == Some(row[column]))
    });
    let remaining: Vec<_> = bindings
        .vars
        .iter()
        .filter(|v| !local.correlation.contains_key(*v))
        .cloned()
        .collect();
    // Projection preserves multiplicities and repairs sorted-column metadata.
    super::project_bindings(bindings, &remaining)
}
