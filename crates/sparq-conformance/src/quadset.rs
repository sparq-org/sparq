//! [FABLE-5] (sq-tonhr.2) QUAD-SET plumbing shared by the W3C line-syntax suites
//! (`line_syntax`) and the parser differential harness (`differential`): render parsed
//! quads to a comparable string form, extract the quad set of a loaded
//! [`sparq_core::Graph`] dataset, and compare two quad sets for identity under a
//! blank-node bijection (the outcome-SET identity the epic's zero-regression guarantee
//! is stated in — never line-by-line, since blank-node labels differ across parsers and
//! across chunked/serial runs of the SAME parser).

use oxrdf::{GraphName, NamedOrBlankNode, Quad, Term};
use rustc_hash::FxHashMap;

/// Render an oxrdf term in its N-Triples `Display` form — the SAME shape
/// `sparq_core::Dict::term(id).to_string()` produces, so the two sides of a
/// differential compare directly (mirrors `turtle_suite::term_to_string`).
pub fn term_str(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        other => other.to_string(),
    }
}

fn nob_str(n: &NamedOrBlankNode) -> String {
    match n {
        NamedOrBlankNode::NamedNode(n) => format!("<{}>", n.as_str()),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

/// Render an oxrdf quad as `[s, p, o, g]` strings; the default graph is the empty
/// string (a graph name is otherwise `<iri>` or `_:label`).
pub fn quad_strings(q: &Quad) -> [String; 4] {
    let g = match &q.graph_name {
        GraphName::DefaultGraph => String::new(),
        GraphName::NamedNode(n) => format!("<{}>", n.as_str()),
        GraphName::BlankNode(b) => format!("_:{}", b.as_str()),
    };
    [
        nob_str(&q.subject),
        format!("<{}>", q.predicate.as_str()),
        term_str(&q.object),
        g,
    ]
}

/// Extract the full quad set of a loaded dataset [`sparq_core::Graph`]: the default
/// graph's triples with graph name `""` plus every named graph's triples under its
/// rendered name — the sparq side of a candidate-vs-incumbent quad-set differential.
pub fn dataset_quads(g: &sparq_core::Graph) -> Vec<[String; 4]> {
    let mut out = Vec::new();
    let mut push_graph = |sub: &sparq_core::Graph, gname: &str| {
        for [s, p, o] in sub.iter_ids() {
            out.push([
                sub.dict.term(s).to_string(),
                sub.dict.term(p).to_string(),
                sub.dict.term(o).to_string(),
                gname.to_string(),
            ]);
        }
    };
    push_graph(g, "");
    g.for_named_graphs_with_prefix("", |name, sub| {
        push_graph(sub, &term_str(name));
    });
    out
}

/// Result of a quad-set comparison.
#[derive(Debug, PartialEq, Eq)]
pub enum SetCompare {
    /// Identical as sets under one blank-node bijection.
    Equal,
    /// Provably different. Carries a bounded sample of each side's unmatched quads
    /// (after the exact-string set diff — the human-readable divergence witness).
    Different {
        /// Quads (rendered) present on side A but not matched on side B.
        only_a: Vec<[String; 4]>,
        /// Quads (rendered) present on side B but not matched on side A.
        only_b: Vec<[String; 4]>,
    },
    /// The blank-node isomorphism search exhausted its step budget — neither equality
    /// nor difference is PROVEN. Reported separately, never counted as agreement or
    /// divergence (honesty over convenience).
    Unverified,
}

/// Compare two quad SETS for identity under a blank-node bijection.
///
/// SET semantics, not multiset: an RDF graph/dataset is a SET of triples/quads
/// (duplicate statements in a document denote the same quad), and sparq's `Graph`
/// stores a set while a raw streaming parser emits one quad per statement — so each
/// side is deduplicated (post-render, exact-string) before comparison. A parser that
/// merely emits duplicate copies of a quad is therefore NOT a divergence.
///
/// Fast path: exact-string set equality (labelled blank nodes usually carry the
/// SAME label through both parsers, so most agreeing inputs never reach the search).
/// Slow path: cancel the exactly-equal quads, then run a budgeted backtracking
/// bijection search over the remainder (the `turtle_suite` isomorphism generalised to
/// 4 positions — graph names can be blank nodes too).
pub fn compare_quad_sets(a: &[[String; 4]], b: &[[String; 4]]) -> SetCompare {
    // Set fast path (sort + dedup = the post-render set).
    let mut sa = a.to_vec();
    let mut sb = b.to_vec();
    sa.sort();
    sa.dedup();
    sb.sort();
    sb.dedup();
    if sa == sb {
        return SetCompare::Equal;
    }
    // Cancel exact matches; only the remainder needs the bijection search. NOTE: this
    // cancellation is exact-string, which can in principle strand a valid bijection
    // that maps label X to label Y while X also appears verbatim on both sides — so a
    // failed search on the REMAINDER falls back to a search over the FULL sets before
    // declaring a difference (bounded by the same budget).
    let (ra, rb) = cancel_exact(&sa, &sb);
    if ra.len() != rb.len() {
        return SetCompare::Different {
            only_a: sample(&ra),
            only_b: sample(&rb),
        };
    }
    match iso4(&ra, &rb) {
        Some(true) => SetCompare::Equal,
        Some(false) => match iso4(&sa, &sb) {
            Some(true) => SetCompare::Equal,
            Some(false) => SetCompare::Different {
                only_a: sample(&ra),
                only_b: sample(&rb),
            },
            None => SetCompare::Unverified,
        },
        None => SetCompare::Unverified,
    }
}

fn sample(quads: &[[String; 4]]) -> Vec<[String; 4]> {
    quads.iter().take(8).cloned().collect()
}

fn cancel_exact(sa: &[[String; 4]], sb: &[[String; 4]]) -> (Vec<[String; 4]>, Vec<[String; 4]>) {
    // Both inputs are sorted + deduped: a two-pointer set difference.
    let (mut i, mut j) = (0usize, 0usize);
    let (mut ra, mut rb) = (Vec::new(), Vec::new());
    while i < sa.len() && j < sb.len() {
        match sa[i].cmp(&sb[j]) {
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                ra.push(sa[i].clone());
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                rb.push(sb[j].clone());
                j += 1;
            }
        }
    }
    ra.extend_from_slice(&sa[i..]);
    rb.extend_from_slice(&sb[j..]);
    (ra, rb)
}

fn is_bnode(s: &str) -> bool {
    s.starts_with("_:")
}

/// Budgeted blank-node bijection search over 4-position rows. `Some(true)` =
/// isomorphic, `Some(false)` = proven not isomorphic, `None` = budget exhausted.
fn iso4(a: &[[String; 4]], b: &[[String; 4]]) -> Option<bool> {
    if a.len() != b.len() {
        return Some(false);
    }
    let mut fwd: FxHashMap<String, String> = FxHashMap::default();
    let mut rev: FxHashMap<String, String> = FxHashMap::default();
    let mut trail: Vec<String> = Vec::new();
    let mut used = vec![false; b.len()];
    let mut steps = 0usize;
    let mut exhausted = false;
    let found = iso_search(
        0,
        a,
        b,
        &mut used,
        &mut fwd,
        &mut rev,
        &mut trail,
        &mut steps,
        &mut exhausted,
    );
    if exhausted && !found {
        return None;
    }
    Some(found)
}

#[allow(clippy::too_many_arguments)]
fn iso_search(
    i: usize,
    a: &[[String; 4]],
    b: &[[String; 4]],
    used: &mut [bool],
    fwd: &mut FxHashMap<String, String>,
    rev: &mut FxHashMap<String, String>,
    trail: &mut Vec<String>,
    steps: &mut usize,
    exhausted: &mut bool,
) -> bool {
    const BUDGET: usize = 2_000_000;
    if i == a.len() {
        return true;
    }
    for j in 0..b.len() {
        if used[j] {
            continue;
        }
        *steps += 1;
        if *steps > BUDGET {
            *exhausted = true;
            return false;
        }
        let mark = trail.len();
        if (0..4).all(|k| pos_match(&a[i][k], &b[j][k], fwd, rev, trail)) {
            used[j] = true;
            if iso_search(i + 1, a, b, used, fwd, rev, trail, steps, exhausted) {
                return true;
            }
            used[j] = false;
        }
        while trail.len() > mark {
            let x = trail.pop().unwrap();
            if let Some(y) = fwd.remove(&x) {
                rev.remove(&y);
            }
        }
        if *exhausted {
            return false;
        }
    }
    false
}

fn pos_match(
    x: &str,
    y: &str,
    fwd: &mut FxHashMap<String, String>,
    rev: &mut FxHashMap<String, String>,
    trail: &mut Vec<String>,
) -> bool {
    match (is_bnode(x), is_bnode(y)) {
        (true, true) => match (fwd.get(x), rev.get(y)) {
            (Some(mx), Some(my)) => mx == y && my == x,
            (None, None) => {
                fwd.insert(x.to_string(), y.to_string());
                rev.insert(y.to_string(), x.to_string());
                trail.push(x.to_string());
                true
            }
            _ => false,
        },
        (false, false) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str, p: &str, o: &str, g: &str) -> [String; 4] {
        [s.to_string(), p.to_string(), o.to_string(), g.to_string()]
    }

    #[test]
    fn equal_under_bnode_renaming_including_graph_names() {
        let a = vec![q("_:x", "<p>", "\"1\"", "_:gx"), q("_:x", "<p>", "<o>", "")];
        let b = vec![q("_:y", "<p>", "<o>", ""), q("_:y", "<p>", "\"1\"", "_:gy")];
        assert_eq!(compare_quad_sets(&a, &b), SetCompare::Equal);
    }

    #[test]
    fn different_quad_detected_with_witness() {
        let a = vec![q("<s>", "<p>", "\"1\"", ""), q("<s>", "<p>", "\"2\"", "")];
        let b = vec![q("<s>", "<p>", "\"1\"", ""), q("<s>", "<p>", "\"3\"", "")];
        match compare_quad_sets(&a, &b) {
            SetCompare::Different { only_a, only_b } => {
                assert_eq!(only_a, vec![q("<s>", "<p>", "\"2\"", "")]);
                assert_eq!(only_b, vec![q("<s>", "<p>", "\"3\"", "")]);
            }
            other => panic!("expected Different, got {other:?}"),
        }
    }

    #[test]
    fn bnode_vs_iri_never_matches() {
        let a = vec![q("_:x", "<p>", "<o>", "")];
        let b = vec![q("<s>", "<p>", "<o>", "")];
        assert!(matches!(
            compare_quad_sets(&a, &b),
            SetCompare::Different { .. }
        ));
    }

    #[test]
    fn dataset_quads_covers_default_and_named_graphs() {
        let g = sparq_core::Graph::load_dataset(
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n\
             <http://ex/s> <http://ex/p> \"v\" <http://ex/g> .\n",
            "nquads",
        )
        .unwrap();
        let mut quads = dataset_quads(&g);
        quads.sort();
        assert_eq!(
            quads,
            vec![
                q("<http://ex/s>", "<http://ex/p>", "\"v\"", "<http://ex/g>"),
                q("<http://ex/s>", "<http://ex/p>", "<http://ex/o>", ""),
            ]
        );
    }
}
