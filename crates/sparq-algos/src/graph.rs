//! The directed adjacency view that every algorithm in this crate runs over.
//!
//! [`NodeGraph`] projects a [`sparq_core::Graph`] onto a *node graph*: subjects and
//! objects of triples become **nodes**, and each triple `(s, p, o)` contributes a
//! directed edge `s → o` (the predicate `p` is dropped — these are *topology* algorithms;
//! see the crate docs for predicate filtering). The projection is built once, in a single
//! pass over `Graph::iter_ids`, and is keyed by **dense, contiguous node indices**
//! (`0..n`) so the algorithms can use flat `Vec`s rather than hash maps on the hot path.
//! Each node index maps back to its sparq-core dictionary [`Id`] (and hence to an
//! [`oxrdf::Term`]) via [`NodeGraph::dict_id`] / [`NodeGraph::term`].
//!
//! Only **IRI and blank-node** terms become nodes by default: an RDF *literal* in the
//! object position is data, not a graph vertex, and including literals would let, e.g.,
//! every entity with `foaf:age 30` share a "neighbour". Pass [`NodeFilter::All`] to
//! [`NodeGraph::build_with`] to include literal objects as nodes too.

use rustc_hash::FxHashMap;
use sparq_core::dict::{self, Id};
use sparq_core::Graph;

use oxrdf::Term;

/// Which object terms are promoted to graph nodes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NodeFilter {
    /// Only IRIs and blank nodes are nodes; a triple whose object is a literal
    /// contributes **no** edge, while its subject remains an isolated node unless it has
    /// another entity-valued edge. This is the default — analytics over the *entity*
    /// graph, not its data values.
    #[default]
    EntitiesOnly,
    /// Every distinct subject/object term — including literals — is a node. A literal
    /// then becomes a sink that incoming edges point at (it never has out-edges, as a
    /// literal is never a subject in RDF).
    All,
}

/// A directed, predicate-erased view of a [`sparq_core::Graph`], keyed by dense node
/// indices `0..n`. Built once via [`NodeGraph::build`] / [`NodeGraph::build_with`];
/// immutable thereafter. Holds no reference to the source graph — only the resolved
/// dictionary ids, so it outlives the borrow used to build it.
#[derive(Clone, Debug)]
pub struct NodeGraph {
    /// `dict_ids[i]` is the sparq-core dictionary id of node `i`.
    dict_ids: Vec<Id>,
    /// CSR-style edge layout: `out_targets[out_offsets[i]..out_offsets[i + 1]]` are the
    /// node indices node `i` points at (with multiplicity collapsed — see below).
    out_offsets: Vec<u32>,
    out_targets: Vec<u32>,
    /// Reverse adjacency, same layout: who points AT node `i`.
    in_offsets: Vec<u32>,
    in_targets: Vec<u32>,
}

impl NodeGraph {
    /// Builds the node graph with the default [`NodeFilter::EntitiesOnly`].
    pub fn build(graph: &Graph) -> NodeGraph {
        NodeGraph::build_with(graph, NodeFilter::EntitiesOnly)
    }

    /// Builds the node graph, choosing which object terms become nodes.
    ///
    /// One pass collects the distinct node ids and the edge list; a second groups the
    /// edges into the CSR layout. Parallel edges between the same ordered pair are
    /// **collapsed** (an unweighted simple-digraph view): `a → b` asserted by three
    /// predicates is one edge, so PageRank and degree count it once. Self-loops
    /// (`s == o`) are kept — they are meaningful for PageRank's stationary mass.
    ///
    /// # Determinism
    ///
    /// Node indices `0..n` are assigned in **canonical ascending-term order** (the
    /// `oxrdf::Term` total order), NOT in graph traversal / dictionary-id order. This makes
    /// the view — and hence every algorithm over it — REPRODUCIBLE regardless of how the
    /// source graph's dictionary ids were assigned. That matters because `sparq-core`'s
    /// parallel sharded dictionary merge (the default `parallel` feature) assigns ids in a
    /// thread-count-dependent order, so `iter_ids` (id-sorted) visits nodes differently on a
    /// 2-core vs an 8-core host. Order-sensitive heuristics such as
    /// [`label_propagation`](crate::label_propagation) would otherwise return a
    /// host-dependent partition for the same graph; canonicalising the node order here pins a
    /// single answer. sq-lqty.
    pub fn build_with(graph: &Graph, filter: NodeFilter) -> NodeGraph {
        // Map sparq-core dict id -> provisional (first-seen) node index. The final indices
        // are re-derived below in canonical term order, so this first pass only needs to
        // dedupe nodes and edges; its index values are temporary.
        let mut index_of: FxHashMap<Id, u32> = FxHashMap::default();
        let mut dict_ids: Vec<Id> = Vec::new();
        // Deduplicated directed edges as (src_index, dst_index) in PROVISIONAL index space.
        let mut edge_set: rustc_hash::FxHashSet<(u32, u32)> = rustc_hash::FxHashSet::default();

        let intern = |id: Id, index_of: &mut FxHashMap<Id, u32>, dict_ids: &mut Vec<Id>| -> u32 {
            if let Some(&ix) = index_of.get(&id) {
                ix
            } else {
                let ix = dict_ids.len() as u32;
                index_of.insert(id, ix);
                dict_ids.push(id);
                ix
            }
        };

        for [s, _p, o] in graph.iter_ids() {
            // The subject of an RDF triple is always an IRI or blank node, so it is
            // always a node, even when the object is excluded. This also preserves
            // data-only entities as isolated nodes in the topology projection.
            let si = intern(s, &mut index_of, &mut dict_ids);
            if filter == NodeFilter::EntitiesOnly && is_literal(graph, o) {
                continue;
            }
            let oi = intern(o, &mut index_of, &mut dict_ids);
            edge_set.insert((si, oi));
        }

        let n = dict_ids.len();

        // Canonicalise: assign final node index 0..n in ascending canonical-term order, so
        // the whole view is independent of the (possibly thread-count-dependent) dictionary-id
        // order. The sort key is the term's canonical string form (`Term`'s `Display`, the
        // N-Triples lexical form — a stable total order; `Term` itself is not `Ord`). Keys are
        // distinct because dictionary terms are distinct. `perm[old_index]` is the node's final
        // index; `dict_ids` is reordered to match.
        let keys: Vec<String> = dict_ids
            .iter()
            .map(|&id| graph.dict.term(id).to_string())
            .collect();
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_unstable_by(|&p, &q| keys[p as usize].cmp(&keys[q as usize]));
        let mut perm = vec![0u32; n];
        for (new_ix, &old_ix) in order.iter().enumerate() {
            perm[old_ix as usize] = new_ix as u32;
        }
        let dict_ids: Vec<Id> = order.iter().map(|&old| dict_ids[old as usize]).collect();
        let mut edges: Vec<(u32, u32)> = edge_set
            .into_iter()
            .map(|(s, d)| (perm[s as usize], perm[d as usize]))
            .collect();

        // Build forward CSR (sorted by src).
        edges.sort_unstable();
        let (out_offsets, out_targets) = build_csr(n, &edges, |&(s, d)| (s, d));

        // Build reverse CSR (sorted by dst).
        edges.sort_unstable_by_key(|&(s, d)| (d, s));
        let (in_offsets, in_targets) = build_csr(n, &edges, |&(s, d)| (d, s));

        NodeGraph {
            dict_ids,
            out_offsets,
            out_targets,
            in_offsets,
            in_targets,
        }
    }

    /// Number of nodes.
    #[inline]
    pub fn len(&self) -> usize {
        self.dict_ids.len()
    }

    /// Whether the node graph is empty (no nodes).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dict_ids.is_empty()
    }

    /// Total number of (deduplicated, directed) edges.
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.out_targets.len()
    }

    /// The sparq-core dictionary [`Id`] of node `i`.
    #[inline]
    pub fn dict_id(&self, i: usize) -> Id {
        self.dict_ids[i]
    }

    /// The dense node index of a dictionary id, or `None` if it is not a node.
    pub fn index_of(&self, id: Id) -> Option<usize> {
        self.dict_ids.iter().position(|&d| d == id)
    }

    /// Resolves node `i` back to its [`oxrdf::Term`] via the source graph's dictionary.
    /// `graph` must be the same graph the view was built from (the ids are its dict ids).
    #[inline]
    pub fn term(&self, graph: &Graph, i: usize) -> Term {
        graph.dict.term(self.dict_ids[i])
    }

    /// Out-neighbours (targets of out-edges) of node `i`.
    #[inline]
    pub fn out_neighbors(&self, i: usize) -> &[u32] {
        let lo = self.out_offsets[i] as usize;
        let hi = self.out_offsets[i + 1] as usize;
        &self.out_targets[lo..hi]
    }

    /// In-neighbours (sources of in-edges) of node `i`.
    #[inline]
    pub fn in_neighbors(&self, i: usize) -> &[u32] {
        let lo = self.in_offsets[i] as usize;
        let hi = self.in_offsets[i + 1] as usize;
        &self.in_targets[lo..hi]
    }

    /// Out-degree of node `i` (number of distinct out-neighbours).
    #[inline]
    pub fn out_degree(&self, i: usize) -> usize {
        self.out_neighbors(i).len()
    }

    /// In-degree of node `i` (number of distinct in-neighbours).
    #[inline]
    pub fn in_degree(&self, i: usize) -> usize {
        self.in_neighbors(i).len()
    }
}

/// `true` if dict id `o` resolves to a literal term in `graph`. Inline ids encode
/// integer/date/etc. literals, so they are always literals; real ids resolve via the dict.
fn is_literal(graph: &Graph, o: Id) -> bool {
    if dict::is_inline(o) {
        return true;
    }
    matches!(graph.dict.term(o), Term::Literal(_))
}

/// Groups a `(key, value)` edge list (already sorted by `key`) into CSR
/// `(offsets, values)`. `key` selects which endpoint is the row.
fn build_csr(
    n: usize,
    edges: &[(u32, u32)],
    key: impl Fn(&(u32, u32)) -> (u32, u32),
) -> (Vec<u32>, Vec<u32>) {
    let mut offsets = vec![0u32; n + 1];
    for e in edges {
        let (row, _) = key(e);
        offsets[row as usize + 1] += 1;
    }
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let mut targets = vec![0u32; edges.len()];
    let mut cursor = offsets.clone();
    for e in edges {
        let (row, val) = key(e);
        let pos = cursor[row as usize] as usize;
        targets[pos] = val;
        cursor[row as usize] += 1;
    }
    (offsets, targets)
}
