//! **Opt-in derivation explanations** (`explain` feature): the [`ProofTree`] a
//! `Materialized*Graph::why(triple)` call returns, with JSON and human-readable renderings.
//!
//! # Shape — flat, id-based, deterministic (ZK-witness-friendly)
//!
//! A proof is a flat `Vec` of [`ProofNode`]s in **premises-before-conclusion** order: every
//! node's premise indices are strictly smaller than its own index, the **root (the explained
//! triple) is the last node**, and shared sub-proofs appear once (the tree is really a DAG —
//! one node per distinct fact). A verifier can therefore validate the whole proof in one
//! linear scan, which is exactly the access pattern a zero-knowledge derivation circuit
//! wants for its witness (see `research/zkp-query-proofs-plan.md` §inference): no engine
//! ids leak — conclusions are self-contained N-Triples-style term strings — and the same
//! input always yields the same node order (all internal choice points iterate sorted).
//!
//! # Semantics — a witness, not all witnesses
//!
//! `why()` returns **one** valid derivation of the triple from the current asserted base
//! (the first in the deterministic search order), not an enumeration of every derivation.
//! Leaves are `asserted` facts (or the few `axiom-*` tautologies, e.g. the XSD numeric-tower
//! `rdfs:subClassOf` axioms); every internal node names the rule that fired — W3C spec rule
//! ids where they exist (`cax-sco`, `prp-trp`, `rdfs9`, …), documented engine rules
//! otherwise (`inv-dom`, `sym-dif`), `n3-rule-<i>` for user-supplied N3 rules.

use rustc_hash::FxHashMap;

/// One step of a [`ProofTree`].
///
/// `rule == "asserted"` marks a leaf: the conclusion is an explicitly asserted base triple
/// (`premises` empty). `axiom-*` rules are premise-free tautologies. Every other node is an
/// application of the named inference rule to the `premises` nodes, in the rule's
/// spec-table premise order (schema/TBox premises first, data premises after).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofNode {
    /// The derived (or asserted) fact, as self-contained N-Triples/N3-style term strings.
    pub conclusion: [String; 3],
    /// Rule identifier (see the module docs for the vocabulary).
    pub rule: String,
    /// Indices of the premise nodes — each strictly less than this node's own index.
    pub premises: Vec<u32>,
}

/// A derivation of one triple from asserted facts: a flat DAG of [`ProofNode`]s whose last
/// node is the explained triple. See the module docs for the shape guarantees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofTree {
    nodes: Vec<ProofNode>,
}

/// Caps for proof construction ([`MaterializedGraph::why_with`](crate::MaterializedGraph::why_with)
/// and siblings): construction aborts (returns `None`) rather than exceed them.
#[derive(Clone, Copy, Debug)]
pub struct ExplainOpts {
    /// Maximum recursion depth while expanding premises (default 128).
    pub max_depth: usize,
    /// Maximum number of nodes in the proof (default 65 536).
    pub max_nodes: usize,
}

impl Default for ExplainOpts {
    fn default() -> Self {
        ExplainOpts { max_depth: 128, max_nodes: 65_536 }
    }
}

impl ProofTree {
    /// The proof steps, premises-before-conclusion; the last node is the explained triple.
    pub fn nodes(&self) -> &[ProofNode] {
        &self.nodes
    }

    /// Index of the root node (the explained triple) — always the last node.
    pub fn root(&self) -> u32 {
        (self.nodes.len() - 1) as u32
    }

    /// The explained triple.
    pub fn conclusion(&self) -> &[String; 3] {
        &self.nodes[self.nodes.len() - 1].conclusion
    }

    /// JSON rendering (hand-built, house style — no serde):
    /// `{"root":R,"nodes":[{"id":0,"conclusion":[s,p,o],"rule":"…","premises":[…]},…]}`.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(self.nodes.len() * 96);
        out.push_str("{\"root\":");
        out.push_str(&self.root().to_string());
        out.push_str(",\"nodes\":[");
        for (i, n) in self.nodes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"id\":");
            out.push_str(&i.to_string());
            out.push_str(",\"conclusion\":[");
            for (k, t) in n.conclusion.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                out.push('"');
                json_escape_into(t, &mut out);
                out.push('"');
            }
            out.push_str("],\"rule\":\"");
            json_escape_into(&n.rule, &mut out);
            out.push_str("\",\"premises\":[");
            for (k, p) in n.premises.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                out.push_str(&p.to_string());
            }
            out.push_str("]}");
        }
        out.push_str("]}");
        out
    }

    /// Human-readable indented rendering, root first. Shared sub-proofs are expanded once;
    /// later references print as `… (see #n)`.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let mut expanded = vec![false; self.nodes.len()];
        self.write_text(self.root() as usize, 0, &mut expanded, &mut out);
        out
    }

    fn write_text(&self, ix: usize, depth: usize, expanded: &mut [bool], out: &mut String) {
        let n = &self.nodes[ix];
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(&format!(
            "#{ix} {} {} {}  [{}]",
            n.conclusion[0], n.conclusion[1], n.conclusion[2], n.rule
        ));
        if expanded[ix] {
            out.push_str(" (see above)\n");
            return;
        }
        expanded[ix] = true;
        out.push('\n');
        for &p in &n.premises {
            self.write_text(p as usize, depth + 1, expanded, out);
        }
    }
}

/// Escape `s` into `out` per JSON string semantics (mirrors sparq-engine's house escaper).
fn json_escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

/// Incremental [`ProofTree`] builder shared by the RDFS / OWL / N3 provers. Engines memoize
/// their own fact→node mapping (typed by their fact representation); the builder owns the
/// node vector and enforces the caps + the premises-before-conclusion invariant.
pub(crate) struct ProofBuilder {
    nodes: Vec<ProofNode>,
    pub(crate) opts: ExplainOpts,
}

impl ProofBuilder {
    pub(crate) fn new(opts: ExplainOpts) -> ProofBuilder {
        ProofBuilder { nodes: Vec::new(), opts }
    }

    /// Append a node; `None` when the node cap is hit. Premise indices must already exist
    /// (debug-asserted), which makes the flat order premises-before-conclusion by
    /// construction.
    pub(crate) fn push(
        &mut self,
        conclusion: [String; 3],
        rule: &str,
        premises: Vec<u32>,
    ) -> Option<u32> {
        if self.nodes.len() >= self.opts.max_nodes {
            return None;
        }
        debug_assert!(premises.iter().all(|&p| (p as usize) < self.nodes.len()));
        self.nodes.push(ProofNode { conclusion, rule: rule.to_string(), premises });
        Some((self.nodes.len() - 1) as u32)
    }

    /// Finish with `root` as the explained triple. The root must be the LAST node (the
    /// provers' memoized recursion guarantees it; debug-asserted here).
    pub(crate) fn finish(self, root: u32) -> ProofTree {
        debug_assert_eq!(root as usize, self.nodes.len() - 1, "root must be the last node");
        ProofTree { nodes: self.nodes }
    }
}

/// Render a dictionary id as a self-contained N-Triples-style term string.
pub(crate) fn id_term_string(dict: &sparq_core::dict::Dict, id: sparq_core::dict::Id) -> String {
    dict.term(id).to_string()
}

/// Render an id triple via [`id_term_string`].
pub(crate) fn id_triple_strings(
    dict: &sparq_core::dict::Dict,
    t: [sparq_core::dict::Id; 3],
) -> [String; 3] {
    [id_term_string(dict, t[0]), id_term_string(dict, t[1]), id_term_string(dict, t[2])]
}

/// Sorted copy of a slice (the provers sort every choice point for determinism).
pub(crate) fn sorted<T: Ord + Copy>(xs: &[T]) -> Vec<T> {
    let mut v = xs.to_vec();
    v.sort_unstable();
    v
}

/// A deterministic shortest path `from → to` over a directed graph given as sorted
/// adjacency (`succ(node) -> Vec<successor>` — the caller sorts); BFS, first-found parent
/// wins. Returns the node sequence `[from, …, to]`; `None` if unreachable. `from == to`
/// yields `[from]`.
pub(crate) fn bfs_path<N, F>(from: N, to: N, mut succ: F) -> Option<Vec<N>>
where
    N: Copy + Eq + std::hash::Hash,
    F: FnMut(N) -> Vec<N>,
{
    if from == to {
        return Some(vec![from]);
    }
    let mut parent: FxHashMap<N, N> = FxHashMap::default();
    let mut queue: std::collections::VecDeque<N> = std::collections::VecDeque::new();
    queue.push_back(from);
    parent.insert(from, from);
    while let Some(n) = queue.pop_front() {
        for m in succ(n) {
            if parent.contains_key(&m) {
                continue;
            }
            parent.insert(m, n);
            if m == to {
                let mut path = vec![m];
                let mut cur = n;
                while cur != from {
                    path.push(cur);
                    cur = parent[&cur];
                }
                path.push(from);
                path.reverse();
                return Some(path);
            }
            queue.push_back(m);
        }
    }
    None
}

/// Build a [`ProofTree`] for `target` from a [`reason_n3_proof`](crate::reason_n3_proof)
/// run (the id-level batch N3 entry point): `steps` is that call's derivation record.
/// A fact with no derivation step of its own is treated as an asserted input (`steps`
/// covers every NEWLY-derived triple, so inputs are exactly the step-less facts).
/// Returns `None` if `target` has no step and so is not derived (explain it as asserted
/// yourself), or if a cap is exceeded.
pub fn n3_proof_tree(
    dict: &sparq_core::dict::Dict,
    steps: &[crate::n3::ProofStep],
    target: [sparq_core::dict::Id; 3],
    opts: ExplainOpts,
) -> Option<ProofTree> {
    type Id3 = [sparq_core::dict::Id; 3];
    let mut step_map: FxHashMap<Id3, &crate::n3::ProofStep> = FxHashMap::default();
    for s in steps {
        step_map.entry(s.conclusion).or_insert(s);
    }
    if !step_map.contains_key(&target) {
        return None;
    }
    struct P<'a> {
        dict: &'a sparq_core::dict::Dict,
        step_map: FxHashMap<[sparq_core::dict::Id; 3], &'a crate::n3::ProofStep>,
        b: ProofBuilder,
        memo: FxHashMap<[sparq_core::dict::Id; 3], u32>,
        stack: rustc_hash::FxHashSet<[sparq_core::dict::Id; 3]>,
    }
    impl P<'_> {
        fn prove(&mut self, t: [sparq_core::dict::Id; 3], depth: usize) -> Option<u32> {
            if let Some(&ix) = self.memo.get(&t) {
                return Some(ix);
            }
            if depth > self.b.opts.max_depth || !self.stack.insert(t) {
                return None;
            }
            let r = (|| {
                let Some(&step) = self.step_map.get(&t) else {
                    let ix = self.b.push(id_triple_strings(self.dict, t), "asserted", vec![])?;
                    self.memo.insert(t, ix);
                    return Some(ix);
                };
                let mut prem = Vec::with_capacity(step.premises.len());
                for &p in &step.premises {
                    prem.push(self.prove(p, depth + 1)?);
                }
                let ix = self.b.push(
                    id_triple_strings(self.dict, t),
                    &format!("n3-rule-{}", step.rule),
                    prem,
                )?;
                self.memo.insert(t, ix);
                Some(ix)
            })();
            self.stack.remove(&t);
            r
        }
    }
    let mut p = P {
        dict,
        step_map,
        b: ProofBuilder::new(opts),
        memo: FxHashMap::default(),
        stack: rustc_hash::FxHashSet::default(),
    };
    let root = p.prove(target, 0)?;
    Some(p.b.finish(root))
}

// [OPUS-4.8] Test module kept LAST in the file (clippy::items_after_test_module):
// `n3_proof_tree` above is `explain`-feature-gated, so it is only compiled with
// `--features explain` — which the default-feature CI clippy never builds. When this
// opt-in feature IS compiled, an item after `mod tests` trips the lint; keeping the
// tests at the end satisfies it under both feature sets.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_text_render() {
        let mut b = ProofBuilder::new(ExplainOpts::default());
        let leaf = b
            .push(
                ["<http://ex/Dog>".into(), "<sc>".into(), "<http://ex/Animal>".into()],
                "asserted",
                vec![],
            )
            .unwrap();
        let leaf2 = b
            .push(["<http://ex/rex>".into(), "<ty>".into(), "<http://ex/Dog>".into()], "asserted", vec![])
            .unwrap();
        let root = b
            .push(
                ["<http://ex/rex>".into(), "<ty>".into(), "<http://ex/Animal>".into()],
                "rdfs9",
                vec![leaf, leaf2],
            )
            .unwrap();
        let tree = b.finish(root);
        assert_eq!(tree.root(), 2);
        let json = tree.to_json();
        assert_eq!(
            json,
            "{\"root\":2,\"nodes\":[\
             {\"id\":0,\"conclusion\":[\"<http://ex/Dog>\",\"<sc>\",\"<http://ex/Animal>\"],\"rule\":\"asserted\",\"premises\":[]},\
             {\"id\":1,\"conclusion\":[\"<http://ex/rex>\",\"<ty>\",\"<http://ex/Dog>\"],\"rule\":\"asserted\",\"premises\":[]},\
             {\"id\":2,\"conclusion\":[\"<http://ex/rex>\",\"<ty>\",\"<http://ex/Animal>\"],\"rule\":\"rdfs9\",\"premises\":[0,1]}]}"
        );
        let text = tree.to_text();
        assert!(text.starts_with("#2 <http://ex/rex> <ty> <http://ex/Animal>  [rdfs9]\n"));
        assert!(text.contains("\n  #0 <http://ex/Dog> <sc> <http://ex/Animal>  [asserted]\n"));
    }

    #[test]
    fn json_escapes() {
        let mut b = ProofBuilder::new(ExplainOpts::default());
        let root = b
            .push(["\"a\\b\"".into(), "<p>".into(), "\"x\ny\"".into()], "asserted", vec![])
            .unwrap();
        let json = b.finish(root).to_json();
        assert!(json.contains("\\\"a\\\\b\\\""));
        assert!(json.contains("x\\ny"));
    }

    #[test]
    fn node_cap_respected() {
        let mut b = ProofBuilder::new(ExplainOpts { max_depth: 8, max_nodes: 1 });
        assert!(b.push(["<a>".into(), "<b>".into(), "<c>".into()], "asserted", vec![]).is_some());
        assert!(b.push(["<d>".into(), "<e>".into(), "<f>".into()], "asserted", vec![]).is_none());
    }

    #[test]
    fn bfs_shortest_and_deterministic() {
        // 0→1→3, 0→2→3 — sorted successors mean the 1-path is found first.
        let succ = |n: u32| match n {
            0 => vec![1, 2],
            1 | 2 => vec![3],
            _ => vec![],
        };
        assert_eq!(bfs_path(0u32, 3, succ), Some(vec![0, 1, 3]));
        assert_eq!(bfs_path(0u32, 0, succ), Some(vec![0]));
        assert_eq!(bfs_path(3u32, 0, succ), None);
    }
}
