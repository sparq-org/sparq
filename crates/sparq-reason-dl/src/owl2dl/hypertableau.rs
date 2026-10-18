//! The hypertableau calculus over [DL-clauses](super::clausify) (Motik, Shearer and
//! Horrocks, JAIR 36, 2009, §3).
//!
//! The state is a completion graph: *root* nodes (the ABox individuals, or the test
//! individual of a satisfiability test) and *blockable* nodes created by the `≥`-rule, each
//! with a label of atomic concepts (at-least concepts are atoms too), role edges and
//! inequalities. The rules:
//!
//! - **Hyp**: a clause whose body matches at a central node `x` and whose head does not hold
//!   adds its single head atom, or opens a branch over its disjuncts; an empty head is a
//!   clash.
//! - **≥**: `≥n R.A(x)` on a node that is not blocked, without `n` pairwise-unequal
//!   `R`-neighbours in `A`, creates `n` fresh successors.
//! - **≈**: `s ≈ t` merges the later node into the earlier one (a blockable node into a root),
//!   pruning the merged node's descendants; `s ≉ s` is a clash.
//! - **o**: a node labelled with the nominal `{a}` is merged into `a`'s node (the ABox
//!   asserts `{a}(a)`). These merges are queued and done before anything else, so none is
//!   pending when a branch point is created.
//!
//! **Nominals and the NI-rule.** A nominal can leave an edge between an individual and a
//! blockable node that is not its child. A complete, clash-free graph is reported
//! satisfiable only when it provably unravels into a model despite such edges (see
//! [`Tableau::unravels`]). Where it does not because an at-most restriction `≤n R.A` at the
//! individual `s` counts the blockable node `w`, the **NI-rule** applies: `w` is one of at
//! most `n` elements, so a branch point chooses which, `i ≤ n`, and merges `w` into a root
//! introduced for (`s`, the restriction, `i`) with the restriction's roles from `s` and its
//! filler. A clause takes part when its head is one at-most restriction and disjuncts at
//! `s` ([`NiShape`]): those disjuncts are further alternatives, since the restriction holds
//! at `s` unless one of them does. Otherwise the branch is closed and, if no branch gives a
//! model, the run returns [`Stop::Unconfirmed`], never a verdict. When a plain attempt runs
//! out or cannot confirm its model, [`Tableau::run`] starts over with the rule also applied
//! as HermiT applies it, where the at-most restriction at `s` would merge `w` with another
//! neighbour (`eager_ni`); the first choice among slots not introduced yet is the only one
//! tried, since they are interchangeable.
//!
//! **Termination.** HermiT's NI-rule (Motik, Shearer and Horrocks, JAIR 36, 2009) fires on
//! an annotated equality: an at-most restriction at a root forcing two nodes to merge. This
//! one fires there in the eager attempt and at completion, on an edge [`Tableau::unravels`]
//! refuses. The roots it introduces can introduce further roots, so the JAIR termination
//! argument does not carry over as it stands. Termination is backed by the [`Budget`]: every rule application and
//! every new node counts against it, and running out is [`Stop::Exhausted`]
//! (`Unknown(ResourceBudget)`), never a verdict.
//!
//! Blocking is pairwise anywhere blocking over full labels, recomputed whenever the `≥`-rule
//! runs, which makes the calculus terminate in the presence of inverse roles and number
//! restrictions. Super-role facts are materialised when a role fact is added.
//!
//! Every fact carries the set of branch points it depends on; a clash backjumps to the
//! latest branch point it depends on (dependency-directed backtracking), and state is
//! restored from an undo trail. Work is bounded by a deterministic step count; running out
//! is reported as [`Stop::Exhausted`], never as a verdict.
//!
//! **Data.** Data property edges end in *concrete* nodes: a separate sort that is never
//! blocked, never the central node of a clause and has no successors. A concrete node's
//! label is a set of data ranges, and `≉` between concrete nodes means their values differ
//! (from the `≥`-rule, from `DisjointDataProperties`, and between the asserted values of an
//! individual). The **datatype check** runs whenever the concrete part changed, before any
//! branching, expansion or completion. Over each connected component of the `≉` graph it
//! asks whether every node can get a value in all its ranges with `≉`-neighbours apart:
//!
//! 1. a node whose ranges have no common value is a clash (a node with a `OneOf` range has
//!    its candidate values enumerated and tested with [`data_ranges::contains`]);
//! 2. a node with more values than `≉`-neighbours left can always be given a value last, so
//!    it is peeled off, repeatedly;
//! 3. what is left is solved exactly: the enumerated nodes by backtracking, and the rest
//!    must form cliques of nodes with identical labels, decided by
//!    [`data_ranges::conjunction_satisfiable`] with the clique size (and the values of
//!    their enumerated neighbours excluded).
//!
//! A clash depends on the labels and inequalities of the nodes it involves. Anything else,
//! or any `Unknown` from the oracle, leaves the branch *undecided*: it is closed like a
//! clash so the search goes on, and a run whose every branch closed returns
//! [`Stop::Undecided`] instead of "unsatisfiable" when one of them was undecided. Only a
//! clash-free, complete branch whose datatype check succeeded is reported satisfiable.

use super::clausify::{Atom, AtomKind, Clause, Clauses, Head, Key, Role, Var};
use super::data_ranges::{self, DataRangeExpr, Tri};
use super::datatypes::Value;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::Cell;
use std::rc::Rc;

pub(crate) type Node = u32;

/// The branch points a fact depends on, sorted ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dep(Option<Rc<[u32]>>);

/// A pseudo branch point no branch has: the dependency of a branch closed without a
/// clash (undecided or unconfirmed). Backtracking never removes it, so whatever is derived
/// on the alternatives after such a close keeps it, and is never read as forced.
const UNCERTAIN: u32 = u32::MAX;

impl Dep {
    fn single(b: u32) -> Dep {
        Dep(Some(Rc::from(vec![b])))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    fn items(&self) -> &[u32] {
        self.0.as_deref().unwrap_or(&[])
    }

    fn contains(&self, b: u32) -> bool {
        self.items().binary_search(&b).is_ok()
    }

    fn union(&self, other: &Dep) -> Dep {
        match (&self.0, &other.0) {
            (None, _) => other.clone(),
            (_, None) => self.clone(),
            (Some(a), Some(b)) if Rc::ptr_eq(a, b) => self.clone(),
            (Some(a), Some(b)) => {
                let mut out = Vec::with_capacity(a.len() + b.len());
                let (mut i, mut j) = (0, 0);
                while i < a.len() || j < b.len() {
                    let next = match (a.get(i), b.get(j)) {
                        (Some(&x), Some(&y)) if x == y => {
                            i += 1;
                            j += 1;
                            x
                        }
                        (Some(&x), Some(&y)) if x < y => {
                            i += 1;
                            x
                        }
                        (Some(_), Some(&y)) => {
                            j += 1;
                            y
                        }
                        (Some(&x), None) => {
                            i += 1;
                            x
                        }
                        (None, Some(&y)) => {
                            j += 1;
                            y
                        }
                        (None, None) => unreachable!(),
                    };
                    out.push(next);
                }
                Dep(Some(Rc::from(out)))
            }
        }
    }

    fn without(&self, b: u32) -> Dep {
        let rest: Vec<u32> = self.items().iter().copied().filter(|&x| x != b).collect();
        if rest.is_empty() {
            Dep(None)
        } else {
            Dep(Some(Rc::from(rest)))
        }
    }
}

/// Deterministic work bounds of one tableau run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Rule applications (facts added, nodes created, branches opened).
    pub max_steps: u64,
    /// Nodes of the completion graph.
    pub max_nodes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            max_steps: 2_000_000,
            max_nodes: 100_000,
        }
    }
}

/// Why a run gave no verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The run ran out of its [`Budget`].
    Exhausted,
    /// Every branch closed, but at least one only because its datatype check was
    /// undecided. Carries the axioms (indices into the ontology) of the data ranges
    /// involved in the first undecided check.
    Undecided(Vec<u32>),
    /// Every branch closed, but at least one only because its completion graph could not
    /// be confirmed to be a model: nominals connect a copied or blocked part of the graph
    /// to a node whose number restrictions the copies could break, where the NI-rule does
    /// not apply (the clause is not an [`NiShape`], or none counts the node), or two
    /// individuals might share a key's data value that is not fixed.
    Unconfirmed,
}

/// A clause's neighbour variable: its role atoms, its concept atoms, and whether it binds
/// only nodes after the previous variable's.
type YVar = (Vec<Role>, Vec<Atom>, bool);

/// Where the NI-rule applies: a root, a blockable neighbour, the at-most clause counting
/// it (an index into `ClauseIndex::ni`), and what that match depends on.
type NiMatch = (Node, Node, u32, Dep);

/// Where the clauses applicable at a central node are found.
#[derive(Debug, Default)]
pub(crate) struct ClauseIndex {
    /// Clauses whose first `A(x)` body atom is the atom.
    by_x_atom: Vec<Vec<u32>>,
    /// Clauses without `A(x)` body atoms, by their first role (or self) body atom.
    by_role: FxHashMap<Role, Vec<u32>>,
    /// Clauses with an empty body.
    unconditional: Vec<u32>,
    /// Per clause, per neighbour variable: its role atoms, its concept atoms, and whether it
    /// binds only nodes after the previous variable's.
    ys: Vec<Vec<YVar>>,
    /// The clauses with two or more neighbour variables (number restrictions).
    multi_y: Vec<u32>,
    /// The clauses the NI-rule applies to, in clause order, with their shape.
    ni: Vec<(u32, NiShape)>,
}

/// An at-most restriction `≤n R.A` inside a clause: `body → ≤n R.A ∨ D₁(x) ∨ …`, where the
/// body may also bind neighbour variables of its own (context) that no head mentions.
#[derive(Debug, Clone)]
struct NiShape {
    /// The restriction's `n + 1` interchangeable neighbour variables (same roles and atoms).
    group: Vec<u16>,
    /// The other neighbour variables, bound by the body only.
    context: Vec<u16>,
    /// The head disjuncts that are not equalities of the group: concepts or a self-loop at
    /// the central node.
    extra: Vec<Head>,
}

impl NiShape {
    /// The shape of `cl`, when it has exactly one at-most group and nothing else in its
    /// head but disjuncts at the central node.
    fn of(cl: &Clause, ys: &[YVar]) -> Option<NiShape> {
        // A group starts at `v - 1` for every ordered `v` whose predecessor is not ordered.
        let starts: Vec<u16> = cl
            .ordered
            .iter()
            .filter(|&&v| !cl.ordered.contains(&(v - 1)))
            .map(|&v| v - 1)
            .collect();
        let [start] = starts[..] else {
            return None;
        };
        let mut group = vec![start];
        while cl.ordered.contains(&(group[group.len() - 1] + 1)) {
            group.push(group[group.len() - 1] + 1);
        }
        let (roles, atoms, _) = &ys[start as usize];
        if group
            .iter()
            .any(|&v| ys[v as usize].0 != *roles || ys[v as usize].1 != *atoms)
        {
            return None;
        }
        let mut extra = Vec::new();
        for h in &cl.head {
            match *h {
                Head::Eq(Var::Y(a), Var::Y(b)) if group.contains(&a) && group.contains(&b) => {}
                Head::Concept(_, Var::X) | Head::SelfLoop(_) => extra.push(h.clone()),
                _ => return None,
            }
        }
        let context = (0..cl.n_y).filter(|v| !group.contains(v)).collect();
        Some(NiShape {
            group,
            context,
            extra,
        })
    }
}

impl ClauseIndex {
    pub(crate) fn new(c: &Clauses) -> ClauseIndex {
        let mut idx = ClauseIndex {
            by_x_atom: vec![Vec::new(); c.atoms.len()],
            ..ClauseIndex::default()
        };
        for (i, cl) in c.clauses.iter().enumerate() {
            let i = i as u32;
            if let Some(&a) = cl.body_x.first() {
                idx.by_x_atom[a as usize].push(i);
            } else if let Some(&(r, _)) = cl.body_roles.first() {
                idx.by_role.entry(r).or_default().push(i);
            } else if let Some(&r) = cl.body_self.first() {
                idx.by_role.entry(r).or_default().push(i);
            } else {
                idx.unconditional.push(i);
            }
            let mut ys = vec![(Vec::new(), Vec::new(), false); cl.n_y as usize];
            for &y in &cl.ordered {
                ys[y as usize].2 = true;
            }
            for &(r, y) in &cl.body_roles {
                ys[y as usize].0.push(r);
            }
            for &(a, y) in &cl.body_y {
                ys[y as usize].1.push(a);
            }
            idx.ys.push(ys);
            if cl.n_y >= 2 {
                idx.multi_y.push(i);
                if let Some(shape) = NiShape::of(cl, &idx.ys[i as usize]) {
                    idx.ni.push((i, shape));
                }
            }
        }
        idx
    }
}

#[derive(Clone, Debug)]
struct NodeData {
    parent: Option<Node>,
    root: bool,
    /// A data value (the end of a data property edge).
    concrete: bool,
    active: bool,
    label: FxHashMap<Atom, Dep>,
    /// `adj[n]` = the role expressions `r` with `r(self, n)`.
    adj: FxHashMap<Node, Vec<Role>>,
}

#[derive(Clone, Debug)]
struct Instance {
    clause: u32,
    /// `x`, then `y₀ … yₖ`.
    nodes: Vec<Node>,
    deps: Dep,
}

#[derive(Clone, Debug)]
enum Undo {
    Concept(Node, Atom),
    Role(u32, Node, Node),
    Ineq(Node, Node),
    NewNode,
    Deactivate(Node),
    Merge(Node),
    PendingPush,
    PendingPop(Instance),
    NiNode(Node, u32, u32),
}

/// What a branch point chooses between.
#[derive(Clone, Debug)]
enum Choice {
    /// The disjuncts of a clause's head.
    Clause(Instance),
    /// The NI-rule: which of root `s`'s at most `n` neighbours that clause `ClauseIndex::ni[ni]`
    /// (an at-most restriction) counts blockable node `w` is, or which of the clause's other
    /// disjuncts holds at `s` instead; `deps` is what the match depends on.
    Ni {
        s: Node,
        w: Node,
        ni: u32,
        deps: Dep,
    },
}

#[derive(Clone, Debug)]
struct Branch {
    mark: usize,
    choice: Choice,
    alt: usize,
    acc: Dep,
}

/// One tableau run over a fixed clause set.
#[derive(Clone)]
pub(crate) struct Tableau<'c> {
    c: &'c Clauses,
    index: &'c ClauseIndex,
    nodes: Vec<NodeData>,
    roles: FxHashMap<(u32, Node, Node), Dep>,
    ineq: FxHashMap<(Node, Node), Dep>,
    /// `t → s` for every merged node `t`, with what the merge depends on.
    merges: FxHashMap<Node, (Node, Dep)>,
    trail: Vec<Undo>,
    pending: Vec<Instance>,
    branches: Vec<Branch>,
    dirty: Vec<Node>,
    in_dirty: Vec<bool>,
    clash: Option<Dep>,
    forbidden: Option<(Node, Atom)>,
    steps: u64,
    /// Work inside a single rule application (clause matching, clique search), counted
    /// against the same step budget so one application cannot run unbounded.
    probes: Cell<u64>,
    budget: Budget,
    /// `≉` between concrete nodes, as adjacency.
    cineq: FxHashMap<Node, Vec<Node>>,
    /// Concrete nodes changed since the last datatype check.
    touched: Vec<Node>,
    /// Every concrete node must be rechecked (after backtracking).
    recheck_all: bool,
    /// The data labels of the first undecided datatype check, if any.
    undecided: Option<Vec<Atom>>,
    /// `conjunction_satisfiable` answers, by (label, count, excluded values).
    count_memo: FxHashMap<(Vec<Atom>, usize, Vec<Value>), Tri>,
    /// `contains` answers, by (data atom, value).
    contains_memo: FxHashMap<(Atom, Value), Tri>,
    /// Nodes that gained a nominal atom `{a}` and may have to be merged into `a`'s node.
    nominal_todo: Vec<Node>,
    /// A complete, clash-free branch was closed because it could not be confirmed to
    /// be a model ([`Stop::Unconfirmed`]).
    unconfirmed: bool,
    /// The nodes of the named individuals.
    named: Vec<Node>,
    /// The roots the NI-rule introduced, by (root, at-most clause, index).
    ni_nodes: FxHashMap<(Node, u32, u32), Node>,
    /// Whether the NI-rule also applies when an at-most restriction merges (see
    /// [`Tableau::run`]).
    eager_ni: bool,
    /// Whether an at-most filler's superset atom is chosen only where the restriction counts
    /// ([`Clause::scoped`]).
    scoped: bool,
}

/// What the key rule did.
enum KeyStep {
    /// It merged individuals (or the budget ran out).
    Merged,
    /// Nothing to merge, but some pair might share an unfixed data value.
    Unsure,
    Done,
}

/// A node's values on one data property: those fixed to a literal (with what that
/// depends on), and whether some value is not fixed.
type DataValues = (Vec<(Value, Dep)>, bool);

/// A key's members indexed by their values on its first data property.
#[derive(Default)]
struct KeyIndex<'v> {
    by_value: FxHashMap<&'v Value, Vec<usize>>,
    unfixed: Vec<usize>,
}

/// Whether two individuals share a key's values.
enum Shared {
    No,
    Maybe,
    Yes(Dep),
}

/// The outcome of the datatype check of one component.
enum DataOutcome {
    Sat,
    Clash(Dep),
    Undecided(Dep, Vec<Atom>),
}

/// A concrete node as the datatype check sees it.
struct CNode {
    node: Node,
    /// Its data atoms, sorted.
    atoms: Vec<Atom>,
    /// The union of their dependencies.
    dep: Dep,
    /// Its candidate values, when one of its ranges is a `OneOf`.
    cands: Option<Vec<Value>>,
}

impl<'c> Tableau<'c> {
    pub(crate) fn new(c: &'c Clauses, index: &'c ClauseIndex, budget: Budget) -> Tableau<'c> {
        Tableau {
            c,
            index,
            nodes: Vec::new(),
            roles: FxHashMap::default(),
            ineq: FxHashMap::default(),
            merges: FxHashMap::default(),
            trail: Vec::new(),
            pending: Vec::new(),
            branches: Vec::new(),
            dirty: Vec::new(),
            in_dirty: Vec::new(),
            clash: None,
            forbidden: None,
            steps: 0,
            probes: Cell::new(0),
            budget,
            cineq: FxHashMap::default(),
            touched: Vec::new(),
            recheck_all: false,
            undecided: None,
            count_memo: FxHashMap::default(),
            contains_memo: FxHashMap::default(),
            nominal_todo: Vec::new(),
            unconfirmed: false,
            named: Vec::new(),
            ni_nodes: FxHashMap::default(),
            eager_ni: false,
            scoped: false,
        }
    }

    /// Creates a root node.
    pub(crate) fn add_root(&mut self) -> Node {
        self.new_node(None)
    }

    /// Makes deriving `atom` at `node` a clash (the `¬B` of a subsumption test).
    pub(crate) fn forbid(&mut self, node: Node, atom: Atom) {
        self.forbidden = Some((node, atom));
    }

    pub(crate) fn assert_concept(&mut self, n: Node, a: Atom) {
        self.add_concept(n, a, &Dep::default());
    }

    pub(crate) fn assert_role(&mut self, r: Role, a: Node, b: Node) {
        self.add_role(r, a, b, &Dep::default());
    }

    pub(crate) fn assert_different(&mut self, a: Node, b: Node) {
        self.add_ineq(a, b, &Dep::default());
    }

    pub(crate) fn assert_same(&mut self, a: Node, b: Node) {
        let (a, b) = (self.canonical(a), self.canonical(b));
        self.merge(a, b, &Dep::default());
    }

    /// Asserts the data property facts `r(a, v)`, given as (role, individual, the
    /// `OneOf(v)` atom, v). Each individual gets one concrete node per distinct value, and
    /// its nodes for different values are unequal.
    pub(crate) fn assert_data(&mut self, facts: &[(Role, Node, Atom, Value)]) {
        let mut nodes: FxHashMap<(Node, &Value), Node> = FxHashMap::default();
        let mut by_ind: FxHashMap<Node, Vec<Node>> = FxHashMap::default();
        for (r, a, atom, v) in facts {
            if self.over_budget() || self.nodes.len() > self.budget.max_nodes {
                return;
            }
            let a = self.canonical(*a);
            let y = match nodes.get(&(a, v)) {
                Some(&y) => y,
                None => {
                    let y = self.new_node_of(None, true);
                    self.add_concept(y, *atom, &Dep::default());
                    nodes.insert((a, v), y);
                    by_ind.entry(a).or_default().push(y);
                    y
                }
            };
            self.add_role(*r, a, y, &Dep::default());
        }
        let mut inds: Vec<Node> = by_ind.keys().copied().collect();
        inds.sort_unstable();
        // The pairwise inequalities are quadratic in an individual's values: charge them
        // before making them (`run` reports the exhaustion before using anything).
        let pairs = inds
            .iter()
            .map(|a| by_ind[a].len().saturating_mul(by_ind[a].len()) / 2)
            .fold(0usize, usize::saturating_add);
        if self.charge(pairs) || self.nodes.len() > self.budget.max_nodes {
            return;
        }
        for a in inds {
            let ys = by_ind[&a].clone();
            for (i, &y) in ys.iter().enumerate() {
                for &z in &ys[i + 1..] {
                    self.add_ineq(y, z, &Dep::default());
                }
            }
        }
    }

    /// After a satisfiable run, the distinct labels of the model's elements: the active
    /// abstract nodes not blocked (a directly blocked node has its blocker's label), each
    /// sorted.
    pub(crate) fn element_labels(&self) -> Vec<Vec<Atom>> {
        let blocked = self.blocked();
        let mut out: Vec<Vec<Atom>> = (0..self.nodes.len())
            .filter(|&n| {
                let node = &self.nodes[n];
                node.active && !node.concrete && !blocked[n]
            })
            .map(|n| {
                let mut l: Vec<Atom> = self.nodes[n].label.keys().copied().collect();
                l.sort_unstable();
                l
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The label of a root node after a satisfiable run: each atom, and whether it holds
    /// without depending on any branch choice.
    pub(crate) fn root_label(&self, n: Node) -> Vec<(Atom, bool)> {
        // A root a nominal merged into another node has that node's label; its facts hold
        // of `n` only given the merges on the way.
        let (s, dm) = self.canonical_dep(n);
        let mut out: Vec<(Atom, bool)> = self.nodes[s as usize]
            .label
            .iter()
            .map(|(&a, d)| (a, dm.is_empty() && d.is_empty()))
            .collect();
        out.sort_unstable();
        out
    }

    /// The node `n` was (transitively) merged into, or `n` itself.
    fn canonical(&self, mut n: Node) -> Node {
        while let Some((s, _)) = self.merges.get(&n) {
            n = *s;
        }
        n
    }

    /// [`canonical`](Self::canonical), with the branch points the merges on the way depend
    /// on: a fact found at the canonical node holds of `n` only given those merges.
    fn canonical_dep(&self, mut n: Node) -> (Node, Dep) {
        let mut d = Dep::default();
        while let Some((s, dd)) = self.merges.get(&n) {
            d = d.union(dd);
            n = *s;
        }
        (n, d)
    }

    /// Runs the calculus to completion: `Ok(true)` when a clash-free complete graph was
    /// built, `Ok(false)` when every branch closed on a clash. With nominals and at-most
    /// restrictions, a first attempt on a third of the budget merges the neighbours an
    /// at-most restriction at a root counts as usual; when it runs out or cannot confirm its
    /// model, the run starts over with the NI-rule applied to those merges too (`eager_ni`,
    /// as HermiT does), on the rest of the budget. The eager rule settles graphs whose
    /// merges at a root would otherwise keep joining parts of the tree, but slows some that
    /// the plain attempt decides. The eager attempt also chooses an at-most filler's
    /// superset atom only where the restriction counts ([`Clause::scoped`]).
    pub(crate) fn run(&mut self) -> Result<bool, Stop> {
        let (full, done) = (self.budget.max_steps, self.work());
        // A budget already spent (by loading the ABox, say) is left to the run to report;
        // the plain attempt's limit stays within the budget.
        if self.index.ni.is_empty() || self.eager_ni || done >= full {
            return self.run_inner();
        }
        let start = self.clone();
        self.budget.max_steps = done + (full - done) / 3;
        let r = self.run_inner();
        self.budget.max_steps = full;
        if !matches!(r, Err(Stop::Exhausted | Stop::Unconfirmed)) {
            return r;
        }
        let spent = self.work() - start.work();
        *self = start;
        self.eager_ni = true;
        self.scoped = true;
        self.probes.set(self.probes.get() + spent);
        self.run_inner()
    }

    fn run_inner(&mut self) -> Result<bool, Stop> {
        loop {
            if self.over_budget() || self.nodes.len() > self.budget.max_nodes {
                return Err(Stop::Exhausted);
            }
            if let Some(d) = self.clash.take() {
                if !self.backtrack(d) {
                    return match self.undecided.take() {
                        Some(atoms) => Err(Stop::Undecided(self.data_axioms(&atoms))),
                        None if self.unconfirmed => Err(Stop::Unconfirmed),
                        None => Ok(false),
                    };
                }
                continue;
            }
            if let Some(n) = self.nominal_todo.pop() {
                self.merge_nominal(n);
                continue;
            }
            if let Some(x) = self.dirty.pop() {
                self.in_dirty[x as usize] = false;
                if self.nodes[x as usize].active {
                    self.match_node(x);
                }
                continue;
            }
            if self.recheck_all || !self.touched.is_empty() {
                self.datatype_check();
                continue;
            }
            if let Some(inst) = self.pending.pop() {
                self.trail.push(Undo::PendingPop(inst.clone()));
                if self.valid(&inst) && !self.satisfied(&inst) {
                    self.branch(inst);
                }
                continue;
            }
            if self.expand() {
                continue;
            }
            match self.apply_keys() {
                KeyStep::Merged => continue,
                KeyStep::Unsure => {
                    self.close_unconfirmed();
                    continue;
                }
                KeyStep::Done => {}
            }
            let unravels = self.unravels();
            if self.over_budget() {
                // The confirmation itself ran out: no verdict.
                return Err(Stop::Exhausted);
            }
            match unravels {
                Ok(()) => return Ok(true),
                Err(Some((s, w, ni, deps))) => {
                    self.open(Choice::Ni { s, w, ni, deps });
                }
                Err(None) => self.close_unconfirmed(),
            }
        }
    }

    /// Closes a complete branch that is not a verdict either way, on every choice made so
    /// far (chronological backtracking), so the search looks for a branch that is.
    fn close_unconfirmed(&mut self) {
        self.unconfirmed = true;
        let mut all: Vec<u32> = (0..self.branches.len() as u32).collect();
        all.push(UNCERTAIN);
        self.set_clash(Dep(Some(Rc::from(all))));
    }

    /// Marks `n` as a named individual's node (keys apply to named individuals only).
    pub(crate) fn mark_named(&mut self, n: Node) {
        self.named.push(n);
    }

    /// The key rule, on a complete graph: two named individuals in a key's class that share
    /// a named value on every object property and a value on every data property of the key
    /// are merged. A data value is known only when its node is fixed to one literal; a pair
    /// that might share an unfixed value is [`KeyStep::Unsure`].
    fn apply_keys(&mut self) -> KeyStep {
        if self.c.keys.is_empty() {
            return KeyStep::Done;
        }
        // A named individual's canonical node stands for it only given the merges on the way
        // (one unit per individual and per merge followed).
        let mut named: Vec<(Node, Dep)> = Vec::with_capacity(self.named.len());
        for &n in &self.named {
            let (mut n, mut d) = (n, Dep::default());
            if self.probe() {
                return KeyStep::Merged;
            }
            while let Some((s, ds)) = self.merges.get(&n) {
                if self.probe() {
                    return KeyStep::Merged;
                }
                d = d.union(ds);
                n = *s;
            }
            if self.nodes[n as usize].active {
                named.push((n, d));
            }
        }
        // Sorting, `n log n` units, charged before it runs.
        let m = named.len();
        if self.charge(m.saturating_mul(m.max(2).ilog2() as usize)) {
            return KeyStep::Merged;
        }
        named.sort_unstable_by_key(|&(n, _)| n);
        named.dedup_by_key(|&mut (n, _)| n);
        let (mut unsure, mut merged) = (false, false);
        for key in &self.c.keys {
            // The member scan, one unit per named individual, charged before it runs.
            if self.charge(named.len()) {
                // `run` reports the exhaustion before using anything.
                return KeyStep::Merged;
            }
            let members: Vec<(Node, Dep)> = named
                .iter()
                .filter_map(|(n, dn)| match key.class {
                    None => Some((*n, dn.clone())),
                    Some(a) => self.nodes[*n as usize]
                        .label
                        .get(&a)
                        .map(|d| (*n, d.union(dn))),
                })
                .collect();
            // Each member's values on the key's data properties, read once per key (one unit
            // per entry, besides the scans).
            let mut values: Vec<Vec<DataValues>> = Vec::with_capacity(members.len());
            for &(x, _) in &members {
                let mut row = Vec::with_capacity(key.data.len());
                for &d in &key.data {
                    if self.probe() {
                        return KeyStep::Merged;
                    }
                    row.push(self.data_values(d, x));
                    if self.over_budget() {
                        return KeyStep::Merged;
                    }
                }
                values.push(row);
            }
            let n = members.len();
            let index = self.key_index(&values);
            if self.over_budget() {
                return KeyStep::Merged;
            }
            // Pairs in order, built one `i` at a time. Every identified pair is merged in the
            // same round: the values read stay true of a survivor, and a pair with a member
            // merged away waits for the next round, which rereads the values.
            for i in 0..n {
                if !self.nodes[members[i].0 as usize].active {
                    continue;
                }
                let partners: Box<dyn Iterator<Item = usize>> = match &index {
                    Some(index) => Box::new(self.key_partners(index, &values, i).into_iter()),
                    None => Box::new(i + 1..n),
                };
                if self.over_budget() {
                    return KeyStep::Merged;
                }
                for j in partners {
                    let ((x, dx), (y, dy)) = (&members[i], &members[j]);
                    // A pair scans the named individuals once per object property.
                    if self.charge(1 + key.roles.len() * named.len()) {
                        return KeyStep::Merged;
                    }
                    if !self.nodes[*x as usize].active || !self.nodes[*y as usize].active {
                        continue;
                    }
                    let shared = self.key_shared(key, *x, *y, &named, &values[i], &values[j]);
                    if self.over_budget() {
                        // The value comparisons ran out of budget part way.
                        return KeyStep::Merged;
                    }
                    match shared {
                        Shared::No => {}
                        Shared::Maybe => unsure = true,
                        Shared::Yes(d) => {
                            let d = d.union(dx).union(dy);
                            self.merge(*x, *y, &d);
                            merged = true;
                            if self.clash.is_some() {
                                return KeyStep::Merged;
                            }
                        }
                    }
                }
            }
        }
        if merged {
            KeyStep::Merged
        } else if unsure {
            KeyStep::Unsure
        } else {
            KeyStep::Done
        }
    }

    /// The members by each fixed value of a key's first data property (ascending), and those
    /// with a value not fixed there (ascending). `None` for a key without data properties,
    /// whose every pair is a candidate. One unit per value indexed.
    fn key_index<'v>(&self, values: &'v [Vec<DataValues>]) -> Option<KeyIndex<'v>> {
        if values.first().is_none_or(|v| v.is_empty()) {
            return None;
        }
        let mut index = KeyIndex::default();
        for (i, v) in values.iter().enumerate() {
            let (fixed, open) = &v[0];
            if self.charge(fixed.len()) {
                break;
            }
            for (value, _) in fixed {
                // Once per member, though several neighbours may carry the value.
                let ms = index.by_value.entry(value).or_default();
                if ms.last() != Some(&i) {
                    ms.push(i);
                }
            }
            if *open {
                index.unfixed.push(i);
            }
        }
        Some(index)
    }

    /// The members `j > i`, ascending and once each, that might share a value with member
    /// `i` on the key's first data property: those sharing a fixed value, and those with an
    /// unfixed value on one side and any value on the other. Others share no value there,
    /// so [`key_shared`](Self::key_shared) would answer [`Shared::No`]. One unit per
    /// value of `i` and per partner generated (charged before it is), and `m log m` for sorting `m` of them; empty
    /// once the budget is spent.
    fn key_partners(
        &self,
        index: &KeyIndex<'_>,
        values: &[Vec<DataValues>],
        i: usize,
    ) -> Vec<usize> {
        let mut js = Vec::new();
        let (fixed, open) = &values[i][0];
        let mut seen: FxHashSet<&Value> = FxHashSet::default();
        for (value, _) in fixed {
            // Each distinct value once (a merged individual keeps its duplicates' values).
            if self.probe() {
                return Vec::new();
            }
            if !seen.insert(value) {
                continue;
            }
            let ms = index.by_value.get(value).map_or(&[][..], Vec::as_slice);
            for &j in &ms[ms.partition_point(|&j| j <= i)..] {
                if self.probe() {
                    return Vec::new();
                }
                js.push(j);
            }
        }
        if *open {
            // An unfixed value may equal any value, fixed or not, on the other side.
            for (j, v) in values.iter().enumerate().skip(i + 1) {
                if self.probe() {
                    return Vec::new();
                }
                if v[0].1 || !v[0].0.is_empty() {
                    js.push(j);
                }
            }
        } else if !fixed.is_empty() {
            let us = &index.unfixed[index.unfixed.partition_point(|&j| j <= i)..];
            for &j in us {
                if self.probe() {
                    return Vec::new();
                }
                js.push(j);
            }
        }
        let m = js.len();
        if self.charge(m.saturating_mul(m.max(2).ilog2() as usize)) {
            return Vec::new();
        }
        js.sort_unstable();
        js.dedup();
        js
    }

    /// Whether `x` and `y` share a value on every property of `key`, and on what.
    fn key_shared(
        &self,
        key: &Key,
        x: Node,
        y: Node,
        named: &[(Node, Dep)],
        values_x: &[DataValues],
        values_y: &[DataValues],
    ) -> Shared {
        let mut deps = Dep::default();
        for &r in &key.roles {
            let z = named
                .iter()
                .find(|&&(z, _)| self.has_role(r, x, z) && self.has_role(r, y, z));
            match z {
                Some((z, dz)) => {
                    deps = deps.union(
                        &self
                            .role_dep(r, x, *z)
                            .union(&self.role_dep(r, y, *z))
                            .union(dz),
                    )
                }
                None => return Shared::No,
            }
        }
        let mut maybe = false;
        for ((vx, ux), (vy, uy)) in values_x.iter().zip(values_y) {
            let (ux, uy) = (*ux, *uy);
            // Comparing the two value lists through a hash map of one of them, and one unit
            // for the property itself (its lists may be empty).
            if self.charge(1 + vx.len() + vy.len()) {
                return Shared::Maybe;
            }
            let by_value: FxHashMap<&Value, &Dep> = vx.iter().map(|(v, da)| (v, da)).collect();
            let common = vy
                .iter()
                .find_map(|(w, db)| by_value.get(w).map(|da| da.union(db)));
            match common {
                Some(dv) => deps = deps.union(&dv),
                // An unfixed value on one side may equal any value on the other.
                None if (ux && (uy || !vy.is_empty())) || (uy && !vx.is_empty()) => maybe = true,
                None => return Shared::No,
            }
        }
        if maybe {
            Shared::Maybe
        } else {
            Shared::Yes(deps)
        }
    }

    /// The `d`-values of `x`: those of concrete neighbours fixed to one literal (with what
    /// that depends on), and whether some neighbour's value is not fixed.
    fn data_values(&self, d: Role, x: Node) -> DataValues {
        let mut fixed = Vec::new();
        let mut unfixed = false;
        for &n in self.nodes[x as usize].adj.keys() {
            let node = &self.nodes[n as usize];
            // One unit per neighbour, and one per label entry of a matching one.
            if self.probe() {
                break;
            }
            if !node.active || !node.concrete || !self.has_role(d, x, n) {
                continue;
            }
            if self.charge(node.label.len()) {
                break;
            }
            let one = node
                .label
                .iter()
                .find_map(|(&a, da)| match self.c.atoms[a as usize] {
                    AtomKind::Data(i) => match &self.c.data[i as usize] {
                        DataRangeExpr::OneOf(vs) if vs.len() == 1 => {
                            Some((vs[0].clone(), da.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                });
            match one {
                Some((v, da)) => fixed.push((v, da.union(&self.role_dep(d, x, n)))),
                None => unfixed = true,
            }
        }
        (fixed, unfixed)
    }

    fn over_budget(&self) -> bool {
        self.steps.saturating_add(self.probes.get()) > self.budget.max_steps
    }

    /// Counts one unit of inner work; `true` when the budget is spent and the caller should
    /// stop (`run` then reports [`Exhausted`] before using any partial result).
    fn probe(&self) -> bool {
        self.charge(1)
    }

    /// Counts `n` units of inner work; `true` when the budget is spent.
    fn charge(&self, n: usize) -> bool {
        self.probes.set(self.probes.get().saturating_add(n as u64));
        self.over_budget()
    }

    /// The work done so far (steps plus inner work), in budget units.
    pub(crate) fn work(&self) -> u64 {
        self.steps.saturating_add(self.probes.get())
    }

    // ---- facts ------------------------------------------------------------------------

    fn new_node(&mut self, parent: Option<Node>) -> Node {
        self.new_node_of(parent, false)
    }

    fn new_node_of(&mut self, parent: Option<Node>, concrete: bool) -> Node {
        let n = self.nodes.len() as Node;
        self.nodes.push(NodeData {
            parent,
            root: parent.is_none(),
            concrete,
            active: true,
            label: FxHashMap::default(),
            adj: FxHashMap::default(),
        });
        self.in_dirty.push(false);
        self.trail.push(Undo::NewNode);
        self.steps += 1;
        self.mark(n);
        if concrete {
            self.touched.push(n);
        }
        n
    }

    fn mark(&mut self, n: Node) {
        if !self.in_dirty[n as usize] {
            self.in_dirty[n as usize] = true;
            self.dirty.push(n);
        }
    }

    fn mark_with_neighbours(&mut self, n: Node) {
        self.mark(n);
        let nbs: Vec<Node> = self.nodes[n as usize].adj.keys().copied().collect();
        for m in nbs {
            if self.nodes[m as usize].active {
                self.mark(m);
            }
        }
    }

    fn set_clash(&mut self, d: Dep) {
        if self.clash.is_none() {
            self.clash = Some(d);
        }
    }

    fn add_concept(&mut self, n: Node, a: Atom, d: &Dep) {
        let node = &mut self.nodes[n as usize];
        if node.label.contains_key(&a) {
            return;
        }
        node.label.insert(a, d.clone());
        if node.concrete {
            self.touched.push(n);
        }
        self.trail.push(Undo::Concept(n, a));
        self.steps += 1;
        if let Some((f, fa)) = self.forbidden {
            let (cf, df) = self.canonical_dep(f);
            if fa == a && cf == n {
                self.set_clash(d.union(&df));
            }
        }
        if matches!(self.c.atoms[a as usize], AtomKind::Nominal(_)) {
            self.nominal_todo.push(n);
        }
        self.mark_with_neighbours(n);
    }

    /// The nominal rule: a node labelled `{a}` is `a`'s node, so it is merged into it.
    fn merge_nominal(&mut self, n: Node) {
        if !self.nodes[n as usize].active {
            // Merged away: its labels moved to the node it was merged into, which is queued
            // in turn if it gained a nominal.
            return;
        }
        let mut found: Option<(Node, Dep)> = None;
        for (&a, d) in &self.nodes[n as usize].label {
            if let AtomKind::Nominal(i) = self.c.atoms[a as usize] {
                let (s, ds) = self.canonical_dep(i as Node);
                if s != n {
                    found = Some((s, d.union(&ds)));
                    break;
                }
            }
        }
        self.charge(self.nodes[n as usize].label.len());
        if let Some((s, d)) = found {
            self.merge(n, s, &d);
            // `n` may have survived (a root with a smaller id) and carry another nominal.
            if self.nodes[n as usize].active {
                self.nominal_todo.push(n);
            }
        }
    }

    /// Whether `a` has an `r`-edge to `b`: one lookup, however many roles the edge carries.
    fn has_role(&self, r: Role, a: Node, b: Node) -> bool {
        let key = if r.inv { (r.idx, b, a) } else { (r.idx, a, b) };
        self.roles.contains_key(&key)
    }

    fn role_dep(&self, r: Role, a: Node, b: Node) -> Dep {
        let key = if r.inv { (r.idx, b, a) } else { (r.idx, a, b) };
        self.roles.get(&key).cloned().unwrap_or_default()
    }

    fn add_role(&mut self, r: Role, a: Node, b: Node, d: &Dep) {
        let supers = self.c.supers(r).to_vec();
        for s in supers {
            self.add_base_role(s, a, b, d);
        }
    }

    fn add_base_role(&mut self, r: Role, a: Node, b: Node, d: &Dep) {
        let (from, to) = if r.inv { (b, a) } else { (a, b) };
        let key = (r.idx, from, to);
        if self.roles.contains_key(&key) {
            return;
        }
        self.roles.insert(key, d.clone());
        let fwd = Role {
            idx: r.idx,
            inv: false,
        };
        self.nodes[from as usize]
            .adj
            .entry(to)
            .or_default()
            .push(fwd);
        self.nodes[to as usize]
            .adj
            .entry(from)
            .or_default()
            .push(fwd.inverse());
        self.trail.push(Undo::Role(r.idx, from, to));
        self.steps += 1;
        self.mark(from);
        self.mark(to);
    }

    fn add_ineq(&mut self, a: Node, b: Node, d: &Dep) {
        if a == b {
            self.set_clash(d.clone());
            return;
        }
        let key = (a.min(b), a.max(b));
        if self.ineq.contains_key(&key) {
            return;
        }
        self.ineq.insert(key, d.clone());
        if self.nodes[a as usize].concrete {
            self.cineq.entry(a).or_default().push(b);
            self.cineq.entry(b).or_default().push(a);
            self.touched.push(a);
            self.touched.push(b);
        }
        self.trail.push(Undo::Ineq(key.0, key.1));
        self.steps += 1;
    }

    fn merge(&mut self, a: Node, b: Node, d: &Dep) {
        if a == b {
            return;
        }
        let (ra, rb) = (self.nodes[a as usize].root, self.nodes[b as usize].root);
        let (s, t) = match (ra, rb) {
            (true, false) => (a, b),
            (false, true) => (b, a),
            _ => (a.min(b), a.max(b)),
        };
        if let Some(di) = self.ineq.get(&(s.min(t), s.max(t))) {
            let clash = d.union(di);
            self.set_clash(clash);
            return;
        }
        if let Some((f, fa)) = self.forbidden {
            // The forbidden node becomes `s`: a forbidden atom `s` already has is a clash.
            let (cf, df) = self.canonical_dep(f);
            if cf == t {
                if let Some(ds) = self.nodes[s as usize].label.get(&fa) {
                    let clash = d.union(ds).union(&df);
                    self.set_clash(clash);
                    return;
                }
            }
        }
        self.prune_descendants(t);
        let label: Vec<(Atom, Dep)> = self.nodes[t as usize]
            .label
            .iter()
            .map(|(&a, dd)| (a, dd.union(d)))
            .collect();
        let edges: Vec<(Node, Vec<Role>)> = self.nodes[t as usize]
            .adj
            .iter()
            .map(|(&n, rs)| (n, rs.clone()))
            .collect();
        self.charge(self.ineq.len());
        let ineqs: Vec<(Node, Dep)> = self
            .ineq
            .iter()
            .filter_map(|(&(x, y), dd)| {
                if x == t {
                    Some((y, dd.union(d)))
                } else if y == t {
                    Some((x, dd.union(d)))
                } else {
                    None
                }
            })
            .collect();
        self.nodes[t as usize].active = false;
        self.trail.push(Undo::Deactivate(t));
        self.merges.insert(t, (s, d.clone()));
        self.trail.push(Undo::Merge(t));
        for (a, dd) in label {
            self.add_concept(s, a, &dd);
        }
        for (n, rs) in edges {
            if n != t && !self.nodes[n as usize].active {
                continue;
            }
            let target = if n == t { s } else { n };
            for r in rs {
                let dd = self.role_dep(r, t, n).union(d);
                self.add_base_role(r, s, target, &dd);
            }
        }
        for (n, dd) in ineqs {
            if self.nodes[n as usize].active {
                self.add_ineq(s, n, &dd);
            }
        }
        self.mark_with_neighbours(s);
    }

    /// Deactivates `t`'s descendants. A child is always adjacent to its parent (the edge
    /// that created it), so the walk follows edges: work proportional to the subtree.
    fn prune_descendants(&mut self, t: Node) {
        let mut stack = vec![t];
        while let Some(u) = stack.pop() {
            let adj = &self.nodes[u as usize].adj;
            self.charge(adj.len());
            let children: Vec<Node> = adj
                .keys()
                .copied()
                .filter(|&n| {
                    let node = &self.nodes[n as usize];
                    node.active && !node.root && node.parent == Some(u)
                })
                .collect();
            for n in children {
                self.nodes[n as usize].active = false;
                self.trail.push(Undo::Deactivate(n));
                stack.push(n);
            }
        }
    }

    // ---- Hyp-rule ---------------------------------------------------------------------

    fn candidates(&self, x: Node) -> Vec<u32> {
        let node = &self.nodes[x as usize];
        let mut out: Vec<u32> = self.index.unconditional.clone();
        for &a in node.label.keys() {
            out.extend_from_slice(&self.index.by_x_atom[a as usize]);
        }
        self.charge(out.len());
        if !self.index.by_role.is_empty() {
            for (&n, rs) in &node.adj {
                if !self.nodes[n as usize].active {
                    continue;
                }
                for r in rs {
                    if let Some(cs) = self.index.by_role.get(r) {
                        out.extend_from_slice(cs);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn match_node(&mut self, x: Node) {
        // Every clause is about an abstract central node (data facts are read from it).
        if self.nodes[x as usize].concrete {
            return;
        }
        let mut found = Vec::new();
        for ci in self.candidates(x) {
            self.match_clause(ci, x, &mut found);
        }
        for inst in found {
            if self.clash.is_some() {
                return;
            }
            if !self.valid(&inst) || self.satisfied(&inst) {
                continue;
            }
            let head = &self.c.clauses[inst.clause as usize].head;
            match head.len() {
                0 => self.set_clash(inst.deps.clone()),
                1 => match self.ni_trigger(&inst) {
                    Some(choice) => self.open(choice),
                    None => {
                        let h = head[0].clone();
                        self.apply(&inst, &h, &inst.deps.clone());
                    }
                },
                _ => {
                    self.pending.push(inst);
                    self.trail.push(Undo::PendingPush);
                }
            }
        }
    }

    fn match_clause(&self, ci: u32, x: Node, found: &mut Vec<Instance>) {
        let cl = &self.c.clauses[ci as usize];
        if cl.scoped.is_some_and(|scoped| scoped != self.scoped) {
            return;
        }
        let node = &self.nodes[x as usize];
        let mut deps = Dep::default();
        for a in &cl.body_x {
            match node.label.get(a) {
                Some(d) => deps = deps.union(d),
                None => return,
            }
        }
        for &r in &cl.body_self {
            if !self.has_role(r, x, x) {
                return;
            }
            deps = deps.union(&self.role_dep(r, x, x));
        }
        let mut nodes = vec![x];
        self.match_ys(ci, 0, &mut nodes, deps, found);
    }

    fn match_ys(
        &self,
        ci: u32,
        y: usize,
        nodes: &mut Vec<Node>,
        deps: Dep,
        found: &mut Vec<Instance>,
    ) {
        let ys = &self.index.ys[ci as usize];
        if y == ys.len() {
            found.push(Instance {
                clause: ci,
                nodes: nodes.clone(),
                deps,
            });
            return;
        }
        let x = nodes[0];
        let (roles, atoms, ordered) = &ys[y];
        let after = if *ordered { Some(nodes[y]) } else { None };
        let first = roles[0];
        let mut cands: Vec<Node> = self.nodes[x as usize]
            .adj
            .iter()
            .filter(|(&n, rs)| self.nodes[n as usize].active && rs.contains(&first))
            .map(|(&n, _)| n)
            .collect();
        cands.sort_unstable();
        'cand: for n in cands {
            if after.is_some_and(|p| n <= p) {
                continue;
            }
            if self.probe() {
                return;
            }
            let mut d = deps.clone();
            for &r in roles {
                if !self.has_role(r, x, n) {
                    continue 'cand;
                }
                d = d.union(&self.role_dep(r, x, n));
            }
            for a in atoms {
                match self.nodes[n as usize].label.get(a) {
                    Some(da) => d = d.union(da),
                    None => continue 'cand,
                }
            }
            nodes.push(n);
            self.match_ys(ci, y + 1, nodes, d, found);
            nodes.pop();
        }
    }

    fn valid(&self, inst: &Instance) -> bool {
        inst.nodes.iter().all(|&n| self.nodes[n as usize].active)
    }

    fn var(inst: &Instance, v: Var) -> Node {
        match v {
            Var::X => inst.nodes[0],
            Var::Y(i) => inst.nodes[1 + i as usize],
        }
    }

    fn holds(&self, inst: &Instance, h: &Head) -> bool {
        match *h {
            Head::Concept(a, v) => self.nodes[Self::var(inst, v) as usize]
                .label
                .contains_key(&a),
            Head::Eq(u, v) => Self::var(inst, u) == Self::var(inst, v),
            Head::SelfLoop(r) => {
                let x = inst.nodes[0];
                self.has_role(r, x, x)
            }
            Head::Ineq(u, v) => {
                let (a, b) = (Self::var(inst, u), Self::var(inst, v));
                self.ineq.contains_key(&(a.min(b), a.max(b)))
            }
        }
    }

    fn satisfied(&self, inst: &Instance) -> bool {
        self.c.clauses[inst.clause as usize]
            .head
            .iter()
            .any(|h| self.holds(inst, h))
    }

    fn apply(&mut self, inst: &Instance, h: &Head, d: &Dep) {
        match *h {
            Head::Concept(a, v) => self.add_concept(Self::var(inst, v), a, d),
            Head::Eq(u, v) => self.merge(Self::var(inst, u), Self::var(inst, v), d),
            Head::SelfLoop(r) => {
                let x = inst.nodes[0];
                self.add_role(r, x, x, d);
            }
            Head::Ineq(u, v) => self.add_ineq(Self::var(inst, u), Self::var(inst, v), d),
        }
    }

    // ---- branching --------------------------------------------------------------------

    fn branch(&mut self, inst: Instance) {
        let choice = self.ni_trigger(&inst).unwrap_or(Choice::Clause(inst));
        self.open(choice);
    }

    /// The NI-rule where an at-most clause applies at root `s` to a blockable node `w` that
    /// is not `s`'s child: instead of merging `w` with another neighbour, `w` becomes one
    /// of the restriction's at most `n` named neighbours (or another disjunct holds at `s`).
    fn ni_trigger(&self, inst: &Instance) -> Option<Choice> {
        if !self.eager_ni {
            return None;
        }
        let k = self
            .index
            .ni
            .binary_search_by_key(&inst.clause, |(c, _)| *c)
            .ok()?;
        let s = inst.nodes[0];
        if !self.nodes[s as usize].root {
            return None;
        }
        let w = self.index.ni[k]
            .1
            .group
            .iter()
            .map(|&v| inst.nodes[1 + v as usize])
            .find(|&w| {
                let n = &self.nodes[w as usize];
                !n.root && !n.concrete && n.parent != Some(s)
            })?;
        // The choice depends on the restriction holding at `s` and counting `w`, not on the
        // other neighbours it counted.
        let (ci, shape) = &self.index.ni[k];
        let (cl, ys) = (&self.c.clauses[*ci as usize], &self.index.ys[*ci as usize]);
        let sn = &self.nodes[s as usize];
        let mut deps = Dep::default();
        for a in &cl.body_x {
            deps = deps.union(sn.label.get(a)?);
        }
        for &r in &cl.body_self {
            deps = deps.union(&self.role_dep(r, s, s));
        }
        for &v in shape.context.iter().chain(&shape.group[..1]) {
            let u = if shape.group[0] == v {
                w
            } else {
                inst.nodes[1 + v as usize]
            };
            let (roles, atoms, _) = &ys[v as usize];
            for &r in roles {
                deps = deps.union(&self.role_dep(r, s, u));
            }
            for a in atoms {
                deps = deps.union(self.nodes[u as usize].label.get(a)?);
            }
        }
        Some(Choice::Ni {
            s,
            w,
            ni: k as u32,
            deps,
        })
    }

    /// Opens a branch point over `choice` and takes its first alternative (a single
    /// alternative is taken without one).
    fn open(&mut self, choice: Choice) {
        let level = self.branches.len() as u32;
        self.steps += 1;
        if self.alternatives(&choice) == 1 {
            let d = Self::choice_deps(&choice).clone();
            self.choose(&choice, 0, &d);
            return;
        }
        let d = Self::choice_deps(&choice).union(&Dep::single(level));
        self.branches.push(Branch {
            mark: self.trail.len(),
            choice: choice.clone(),
            alt: 0,
            acc: Dep::default(),
        });
        self.choose(&choice, 0, &d);
    }

    fn alternatives(&self, choice: &Choice) -> usize {
        match choice {
            Choice::Clause(inst) => self.c.clauses[inst.clause as usize].head.len(),
            // `≤n R.A` has `n + 1` neighbour variables.
            Choice::Ni { ni, .. } => {
                let shape = &self.index.ni[*ni as usize].1;
                shape.group.len() - 1 + shape.extra.len()
            }
        }
    }

    fn choice_deps(choice: &Choice) -> &Dep {
        match choice {
            Choice::Clause(inst) => &inst.deps,
            Choice::Ni { deps, .. } => deps,
        }
    }

    fn choose(&mut self, choice: &Choice, alt: usize, d: &Dep) {
        match choice {
            Choice::Clause(inst) => {
                let h = self.c.clauses[inst.clause as usize].head[alt].clone();
                self.apply(inst, &h, d);
            }
            &Choice::Ni { s, w, ni, .. } => {
                let shape = &self.index.ni[ni as usize].1;
                let n = shape.group.len() - 1;
                // Past the restriction's `n` alternatives, a disjunct at `s` holds instead.
                match alt.checked_sub(n).map(|e| shape.extra[e].clone()) {
                    Some(Head::Concept(a, _)) => self.add_concept(s, a, d),
                    Some(Head::SelfLoop(r)) => self.add_role(r, s, s, d),
                    _ => self.ni_merge(s, w, ni, alt as u32, d),
                }
            }
        }
    }

    fn backtrack(&mut self, clash: Dep) -> bool {
        loop {
            let Some(b) = self.branches.last() else {
                return false;
            };
            let level = (self.branches.len() - 1) as u32;
            let mark = b.mark;
            self.undo_to(mark);
            if !clash.contains(level) {
                self.branches.pop();
                continue;
            }
            let b = self.branches.last_mut().expect("checked above");
            b.acc = b.acc.union(&clash.without(level));
            b.alt += 1;
            let choice = b.choice.clone();
            let alt = b.alt;
            let n_alts = self.alternatives(&choice);
            let d = if alt + 1 < n_alts {
                Self::choice_deps(&choice).union(&Dep::single(level))
            } else {
                let acc = self.branches.pop().expect("checked above").acc;
                Self::choice_deps(&choice).union(&acc)
            };
            self.choose(&choice, alt, &d);
            return true;
        }
    }

    fn undo_to(&mut self, mark: usize) {
        while self.trail.len() > mark {
            match self.trail.pop().expect("len > mark") {
                Undo::Concept(n, a) => {
                    self.nodes[n as usize].label.remove(&a);
                }
                Undo::Role(idx, from, to) => {
                    self.roles.remove(&(idx, from, to));
                    let fwd = Role { idx, inv: false };
                    remove_role(&mut self.nodes[from as usize].adj, to, fwd);
                    remove_role(&mut self.nodes[to as usize].adj, from, fwd.inverse());
                }
                Undo::Ineq(a, b) => {
                    self.ineq.remove(&(a, b));
                    if self.nodes[a as usize].concrete {
                        for (x, y) in [(a, b), (b, a)] {
                            if let Some(ns) = self.cineq.get_mut(&x) {
                                if let Some(p) = ns.iter().rposition(|&n| n == y) {
                                    ns.remove(p);
                                }
                                if ns.is_empty() {
                                    self.cineq.remove(&x);
                                }
                            }
                        }
                    }
                }
                Undo::NewNode => {
                    self.nodes.pop();
                    self.in_dirty.pop();
                }
                Undo::Deactivate(n) => self.nodes[n as usize].active = true,
                Undo::Merge(t) => {
                    self.merges.remove(&t);
                }
                Undo::PendingPush => {
                    self.pending.pop();
                }
                Undo::PendingPop(inst) => self.pending.push(inst),
                Undo::NiNode(s, clause, i) => {
                    self.ni_nodes.remove(&(s, clause, i));
                }
            }
        }
        for n in self.dirty.drain(..) {
            if let Some(f) = self.in_dirty.get_mut(n as usize) {
                *f = false;
            }
        }
        self.touched.clear();
        // Nominal merges are done before any branch point is created, so a queued one is
        // about a fact the undo just removed.
        self.nominal_todo.clear();
        self.recheck_all = true;
    }

    // ---- ≥-rule and blocking -----------------------------------------------------------

    fn blocked(&self) -> Vec<bool> {
        self.blocking().0
    }

    /// Per node: whether it is blocked (directly or through its parent), whether it blocks
    /// some node, and the node blocking it directly.
    fn blocking(&self) -> (Vec<bool>, Vec<bool>, Vec<Option<Node>>) {
        let len = self.nodes.len();
        let mut blocked = vec![false; len];
        let mut blocker = vec![false; len];
        let mut blocked_by = vec![None; len];
        let mut seen: FxHashMap<Vec<u32>, Node> = FxHashMap::default();
        for n in 0..len {
            let node = &self.nodes[n];
            // Concrete nodes are never blocked and never block.
            if !node.active || node.root || node.concrete {
                continue;
            }
            let p = node.parent.expect("blockable nodes have a parent") as usize;
            if blocked[p] {
                blocked[n] = true;
                continue;
            }
            let sig = self.signature(n as Node, p as Node);
            // Building and hashing the signature is work proportional to its length.
            self.charge(sig.len());
            match seen.entry(sig) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(n as Node);
                }
                std::collections::hash_map::Entry::Occupied(e) => {
                    blocked[n] = true;
                    blocker[*e.get() as usize] = true;
                    blocked_by[n] = Some(*e.get());
                }
            }
        }
        (blocked, blocker, blocked_by)
    }

    /// Whether a complete, clash-free graph is known to unravel into a model.
    ///
    /// Without nominals every edge is a tree edge (parent to child) or joins two roots, and
    /// pairwise blocking unravels the graph: each blocked node is replaced by a copy of its
    /// blocker's subtree. A nominal merges a blockable node into a root, which leaves an edge
    /// between a root and a blockable node that is not its child. Such an edge is copied with
    /// the subtree it hangs from. A child of a root is blocked only by a child of the same
    /// root ([`signature`](Self::signature)), so the copy hung from the root in its place
    /// has the blocker's edges to roots and no more. The model is still one as long as
    ///
    /// - a blocked node's edge is kept when its direct blocker has the same edge to the same
    ///   root (the copy of the blocker that takes its place gives the root the same
    ///   neighbour: the two have the same label) and no number restriction at the root
    ///   counts the blocker's copies; any other blocked node's edge is dropped with the
    ///   node, and the root must keep its at-least restrictions without it (dropping a
    ///   neighbour can break nothing else at the root),
    /// - no edge joins two blockable nodes outside the tree (an at-most restriction at a
    ///   root merged nodes of different subtrees), and
    /// - where the blockable end is copied, no number restriction at the root can match two
    ///   of its copies.
    ///
    /// Otherwise the answer is the NI-rule's match where an at-most restriction at the root
    /// counts the blockable end ([`ni_clause`](Self::ni_clause)), else unconfirmed
    /// (`Err(None)`). A graph with no blocked node is a finite model as it stands.
    fn unravels(&self) -> Result<(), Option<NiMatch>> {
        let (blocked, blocker, blocked_by) = self.blocking();
        if !blocked.contains(&true) {
            return Ok(());
        }
        let mut unconfirmed = false;
        // (root, blocked neighbour) edges the unravelling drops.
        let mut dropped: Vec<(Node, Node)> = Vec::new();
        let len = self.nodes.len();
        // Copied: a blocker or inside a blocker's subtree (parents have smaller ids).
        let mut copied = vec![false; len];
        for n in 0..len {
            let node = &self.nodes[n];
            if !node.active || node.root {
                continue;
            }
            copied[n] = blocker[n] || node.parent.is_some_and(|p| copied[p as usize]);
        }
        for (u, node) in self.nodes.iter().enumerate() {
            if !node.active || node.concrete {
                continue;
            }
            self.charge(node.adj.len());
            for &v in node.adj.keys() {
                let vn = &self.nodes[v as usize];
                if v as usize == u || !vn.active {
                    continue;
                }
                let tree = node.parent == Some(v) || vn.parent == Some(u as Node);
                if tree || (node.root && vn.root) {
                    continue;
                }
                // Only a root's edge to a blockable node is understood (each edge is seen
                // from both ends; take it from the root's).
                if vn.concrete || !(node.root || vn.root) {
                    return Err(None);
                }
                if !node.root {
                    continue;
                }
                let w = v as usize;
                if blocked[w] && !self.same_edge(u as Node, v, blocked_by[w]) {
                    // The unravelling drops the edge: the root must not need it.
                    dropped.push((u as Node, v));
                    continue;
                }
                let counted = if blocked[w] {
                    blocked_by[w].is_some_and(|b| self.counts_copies(u as Node, b))
                } else {
                    copied[w] && self.counts_copies(u as Node, v)
                };
                if counted {
                    // An at-most restriction at the root that counts the node makes it one of
                    // the root's few neighbours: the NI-rule names them.
                    match self.ni_clause(u as Node, v) {
                        Some((clause, deps)) => return Err(Some((u as Node, v, clause, deps))),
                        None if self.index.ni.is_empty() => return Err(None),
                        None => unconfirmed = true,
                    }
                }
            }
        }
        if unconfirmed || !self.keeps_at_least(&mut dropped) {
            Err(None)
        } else {
            Ok(())
        }
    }

    /// Whether every root keeps its at-least restrictions without the blocked neighbours
    /// the unravelling drops (`(root, neighbour)` pairs). Dropping a neighbour removes body
    /// matches of the root's clauses only, so the at-least restrictions are all it can
    /// break.
    fn keeps_at_least(&self, dropped: &mut [(Node, Node)]) -> bool {
        dropped.sort_unstable();
        for group in dropped.chunk_by(|a, b| a.0 == b.0) {
            let s = group[0].0;
            let gone: Vec<Node> = group.iter().map(|&(_, w)| w).collect();
            let mut atoms: Vec<Atom> = self.nodes[s as usize]
                .label
                .keys()
                .copied()
                .filter(|&a| matches!(self.c.atoms[a as usize], AtomKind::AtLeast { .. }))
                .collect();
            atoms.sort_unstable();
            for a in atoms {
                let AtomKind::AtLeast { n: k, role, filler } = self.c.atoms[a as usize] else {
                    unreachable!("filtered");
                };
                if !self.at_least_holds_without(s, k, role, filler, &gone) {
                    return false;
                }
            }
        }
        true
    }

    /// A clause with an at-most restriction `≤n R.A` (see [`NiShape`]) whose body holds at
    /// root `s`, whose other disjuncts do not hold there yet, and whose restriction's
    /// variables match `w`, with what that depends on. Unless another disjunct comes to hold
    /// at `s`, the restriction holds there, so `w` is one of at most `n` elements.
    fn ni_clause(&self, s: Node, w: Node) -> Option<(u32, Dep)> {
        let (sn, wn) = (&self.nodes[s as usize], &self.nodes[w as usize]);
        let rs = sn.adj.get(&w)?;
        'clause: for (k, (ci, shape)) in self.index.ni.iter().enumerate() {
            if self.probe() {
                return None;
            }
            let cl = &self.c.clauses[*ci as usize];
            // A clause of the other attempt is sound to apply but costs branches here.
            if cl.scoped.is_some_and(|scoped| scoped != self.scoped) {
                continue;
            }
            let ys = &self.index.ys[*ci as usize];
            let (roles, atoms, _) = &ys[shape.group[0] as usize];
            if !roles.iter().all(|r| rs.contains(r))
                || !atoms.iter().all(|a| wn.label.contains_key(a))
                || shape.extra.iter().any(|h| match *h {
                    Head::Concept(a, _) => sn.label.contains_key(&a),
                    Head::SelfLoop(r) => self.has_role(r, s, s),
                    _ => false,
                })
            {
                continue;
            }
            let mut d = Dep::default();
            for a in &cl.body_x {
                match sn.label.get(a) {
                    Some(da) => d = d.union(da),
                    None => continue 'clause,
                }
            }
            for &r in &cl.body_self {
                if !self.has_role(r, s, s) {
                    continue 'clause;
                }
                d = d.union(&self.role_dep(r, s, s));
            }
            for &v in &shape.context {
                match self.context_match(s, &ys[v as usize]) {
                    Some(dv) => d = d.union(&dv),
                    None => continue 'clause,
                }
            }
            for &r in roles {
                d = d.union(&self.role_dep(r, s, w));
            }
            for a in atoms {
                d = d.union(&wn.label[a]);
            }
            return Some((k as u32, d));
        }
        None
    }

    /// A neighbour of `s` that body-only variable `y` binds, with what the binding depends on.
    fn context_match(&self, s: Node, (roles, atoms, _): &YVar) -> Option<Dep> {
        let sn = &self.nodes[s as usize];
        for (&u, rs) in &sn.adj {
            self.charge(1);
            let un = &self.nodes[u as usize];
            if roles.iter().all(|r| rs.contains(r))
                && atoms.iter().all(|a| un.label.contains_key(a))
            {
                let mut d = Dep::default();
                for &r in roles {
                    d = d.union(&self.role_dep(r, s, u));
                }
                for a in atoms {
                    d = d.union(&un.label[a]);
                }
                return Some(d);
            }
        }
        None
    }

    /// The NI-rule's `i`-th alternative: `w` is the `i`-th of root `s`'s neighbours that
    /// at-most clause `ClauseIndex::ni[ni]` counts, a root introduced for it (once per
    /// branch) with the restriction's roles from `s` and its filler.
    fn ni_merge(&mut self, s: Node, w: Node, ni: u32, i: u32, d: &Dep) {
        let (clause, ref shape) = self.index.ni[ni as usize];
        let v = shape.group[0];
        // Slots not introduced yet are interchangeable: only the first of them is tried, the
        // others fail as the alternatives before them did.
        let fresh = |k: u32| !self.ni_nodes.contains_key(&(s, clause, k));
        if i > 0 && fresh(i) && fresh(i - 1) {
            self.set_clash(d.clone());
            return;
        }
        let (z, dz) = match self.ni_nodes.get(&(s, clause, i)) {
            Some(&z) => self.canonical_dep(z),
            None => {
                let z = self.new_node(None);
                self.ni_nodes.insert((s, clause, i), z);
                self.trail.push(Undo::NiNode(s, clause, i));
                (z, Dep::default())
            }
        };
        let d = d.union(&dz);
        let (roles, atoms, _) = self.index.ys[clause as usize][v as usize].clone();
        for r in roles {
            self.add_role(r, s, z, &d);
        }
        for a in atoms {
            self.add_concept(z, a, &d);
        }
        self.merge(w, z, &d);
    }

    /// Whether blocked node `w`'s edge to root `s` survives the unravelling: `w` is blocked
    /// directly by a node with the same roles to `s`, whose copy takes `w`'s place (one
    /// unit per role compared).
    fn same_edge(&self, s: Node, w: Node, by: Option<Node>) -> bool {
        let Some(b) = by else {
            return false;
        };
        let (rw, rb) = (
            self.nodes[w as usize].adj.get(&s),
            self.nodes[b as usize].adj.get(&s),
        );
        let (Some(rw), Some(rb)) = (rw, rb) else {
            return false;
        };
        self.charge(rw.len() + rb.len());
        rw.len() == rb.len() && rw.iter().all(|r| rb.contains(r))
    }

    /// Whether a clause with several neighbour variables (a number restriction) can apply
    /// at root `s` with two of its variables bound to copies of `w`.
    fn counts_copies(&self, s: Node, w: Node) -> bool {
        let (sn, wn) = (&self.nodes[s as usize], &self.nodes[w as usize]);
        let Some(rs) = sn.adj.get(&w) else {
            return false;
        };
        for &ci in &self.index.multi_y {
            self.probe();
            let cl = &self.c.clauses[ci as usize];
            // A clause inactive in this attempt counts nothing, nor does one whose body fails
            // at `s` or whose head already holds there.
            if cl.scoped.is_some_and(|scoped| scoped != self.scoped)
                || !cl.body_x.iter().all(|a| sn.label.contains_key(a))
                || cl.head.iter().any(|h| match *h {
                    Head::Concept(a, Var::X) => sn.label.contains_key(&a),
                    Head::SelfLoop(r) => self.has_role(r, s, s),
                    _ => false,
                })
            {
                continue;
            }
            let ys = &self.index.ys[ci as usize];
            let fit = |(roles, atoms, _): &YVar| {
                roles.iter().all(|r| rs.contains(r))
                    && atoms.iter().all(|a| wn.label.contains_key(a))
            };
            // The variables `w` cannot bind need a neighbour of `s` (copies have the labels
            // and edges of the nodes they copy).
            if ys.iter().filter(|y| fit(y)).count() >= 2
                && ys
                    .iter()
                    .all(|y| fit(y) || self.context_match(s, y).is_some())
            {
                return true;
            }
        }
        false
    }

    fn signature(&self, n: Node, p: Node) -> Vec<u32> {
        let mut sig: Vec<u32> = Vec::new();
        let push_label = |sig: &mut Vec<u32>, m: Node| {
            let mut l: Vec<u32> = self.nodes[m as usize].label.keys().copied().collect();
            l.sort_unstable();
            sig.extend(l);
            sig.push(u32::MAX);
        };
        push_label(&mut sig, n);
        push_label(&mut sig, p);
        let mut rs: Vec<u32> = self.nodes[p as usize]
            .adj
            .get(&n)
            .map(|rs| rs.iter().map(|r| 2 * r.idx + u32::from(r.inv)).collect())
            .unwrap_or_default();
        rs.sort_unstable();
        sig.extend(rs);
        // A root parent itself: the copy of a blocker hangs from the blocked node's parent,
        // so a blocker under another root, or joined to this one off the tree, would gain or
        // merge an edge to a root.
        if self.nodes[p as usize].root {
            sig.push(u32::MAX);
            sig.push(p);
        }
        sig
    }

    fn expand(&mut self) -> bool {
        let blocked = self.blocked();
        if self.over_budget() {
            // `run` reports the exhaustion before using anything.
            return true;
        }
        let mut todo: Vec<(Node, Atom, u32, Role, Option<Atom>, Dep)> = Vec::new();
        for (n, node) in self.nodes.iter().enumerate() {
            if !node.active || blocked[n] {
                continue;
            }
            let mut atoms: Vec<(&Atom, &Dep)> = node
                .label
                .iter()
                .filter(|(a, _)| matches!(self.c.atoms[**a as usize], AtomKind::AtLeast { .. }))
                .collect();
            atoms.sort_unstable_by_key(|(a, _)| **a);
            for (&a, d) in atoms {
                let AtomKind::AtLeast { n: k, role, filler } = self.c.atoms[a as usize] else {
                    unreachable!("filtered");
                };
                if !self.at_least_holds(n as Node, k, role, filler) {
                    todo.push((n as Node, a, k, role, filler, d.clone()));
                }
            }
        }
        if todo.is_empty() {
            return false;
        }
        for (x, _, k, role, filler, d) in todo {
            if !self.nodes[x as usize].active || self.at_least_holds(x, k, role, filler) {
                continue;
            }
            let concrete = self.c.is_data(role);
            let ys: Vec<Node> = (0..k)
                .map(|_| self.new_node_of(Some(x), concrete))
                .collect();
            for &y in &ys {
                self.add_role(role, x, y, &d);
                if let Some(f) = filler {
                    self.add_concept(y, f, &d);
                }
            }
            for (i, &a) in ys.iter().enumerate() {
                for &b in &ys[i + 1..] {
                    self.add_ineq(a, b, &d);
                }
            }
        }
        true
    }

    fn at_least_holds(&self, x: Node, k: u32, r: Role, filler: Option<Atom>) -> bool {
        self.at_least_holds_without(x, k, r, filler, &[])
    }

    /// [`at_least_holds`](Self::at_least_holds) with the neighbours `gone` left out.
    fn at_least_holds_without(
        &self,
        x: Node,
        k: u32,
        r: Role,
        filler: Option<Atom>,
        gone: &[Node],
    ) -> bool {
        let mut cands: Vec<Node> = self.nodes[x as usize]
            .adj
            .iter()
            .filter(|(&n, rs)| {
                self.nodes[n as usize].active
                    && !gone.contains(&n)
                    && rs.contains(&r)
                    && filler.is_none_or(|f| self.nodes[n as usize].label.contains_key(&f))
            })
            .map(|(&n, _)| n)
            .collect();
        if (cands.len() as u64) < u64::from(k) {
            return false;
        }
        if k <= 1 {
            return true;
        }
        cands.sort_unstable();
        let mut chosen = Vec::with_capacity(k as usize);
        self.clique(&cands, 0, k as usize, &mut chosen)
    }

    /// `true` when `k` of `cands[from..]` together with `chosen` are pairwise unequal.
    fn clique(&self, cands: &[Node], from: usize, k: usize, chosen: &mut Vec<Node>) -> bool {
        if chosen.len() == k {
            return true;
        }
        if cands.len() - from < k - chosen.len() {
            return false;
        }
        for i in from..cands.len() {
            if self.probe() {
                return false;
            }
            let c = cands[i];
            if chosen
                .iter()
                .all(|&o| self.ineq.contains_key(&(o.min(c), o.max(c))))
            {
                chosen.push(c);
                if self.clique(cands, i + 1, k, chosen) {
                    return true;
                }
                chosen.pop();
            }
        }
        false
    }

    // ---- datatype check ----------------------------------------------------------------

    /// The axioms of the data ranges among `atoms`, sorted and de-duplicated.
    fn data_axioms(&self, atoms: &[Atom]) -> Vec<u32> {
        let mut out: Vec<u32> = atoms
            .iter()
            .filter_map(|&a| match self.c.atoms[a as usize] {
                AtomKind::Data(i) => Some(self.c.data_source[i as usize]),
                _ => None,
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    fn expr(&self, a: Atom) -> &DataRangeExpr {
        match self.c.atoms[a as usize] {
            AtomKind::Data(i) => &self.c.data[i as usize],
            _ => unreachable!("concrete nodes carry data atoms only"),
        }
    }

    /// Whether `k` pairwise different values outside `excluded` are in every range of
    /// `atoms`.
    fn count(&mut self, atoms: &[Atom], k: usize, excluded: &[Value]) -> Tri {
        let key = (atoms.to_vec(), k, excluded.to_vec());
        if let Some(&t) = self.count_memo.get(&key) {
            self.charge(1);
            return t;
        }
        let exprs: Vec<DataRangeExpr> = atoms.iter().map(|&a| self.expr(a).clone()).collect();
        self.charge(8 * (atoms.len() + excluded.len() + 1));
        let t = data_ranges::conjunction_satisfiable(&exprs, &[], k, excluded);
        self.count_memo.insert(key, t);
        t
    }

    fn contains(&mut self, a: Atom, v: &Value) -> Tri {
        if let Some(&t) = self.contains_memo.get(&(a, v.clone())) {
            self.charge(1);
            return t;
        }
        self.charge(8);
        let t = data_ranges::contains(self.expr(a), v);
        self.contains_memo.insert((a, v.clone()), t);
        t
    }

    /// The datatype check over the components of the concrete nodes changed since the last
    /// check (all of them after backtracking). Sets a clash on the first component without
    /// values; otherwise, on an undecided component, closes the branch as undecided.
    fn datatype_check(&mut self) {
        let seeds: Vec<Node> = if self.recheck_all {
            (0..self.nodes.len() as Node)
                .filter(|&n| self.nodes[n as usize].concrete)
                .collect()
        } else {
            std::mem::take(&mut self.touched)
        };
        self.touched.clear();
        self.recheck_all = false;
        let mut seen: FxHashSet<Node> = FxHashSet::default();
        let mut undecided: Option<(Dep, Vec<Atom>)> = None;
        for seed in seeds {
            if !self.nodes[seed as usize].active || !seen.insert(seed) {
                continue;
            }
            let mut comp = vec![seed];
            let mut i = 0;
            while i < comp.len() {
                let n = comp[i];
                i += 1;
                for &m in self.cineq.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
                    if self.nodes[m as usize].active && seen.insert(m) {
                        comp.push(m);
                    }
                }
            }
            self.charge(comp.len());
            match self.solve_component(&comp) {
                DataOutcome::Sat => {}
                DataOutcome::Clash(d) => {
                    self.set_clash(d);
                    return;
                }
                DataOutcome::Undecided(d, atoms) => {
                    undecided.get_or_insert((d, atoms));
                }
            }
            if self.over_budget() {
                // `run` reports the exhaustion before using anything.
                return;
            }
        }
        if let Some((d, atoms)) = undecided {
            self.undecided.get_or_insert(atoms);
            self.set_clash(d.union(&Dep::single(UNCERTAIN)));
        }
    }

    fn cnode(&mut self, n: Node) -> Result<CNode, DataOutcome> {
        let mut atoms: Vec<Atom> = self.nodes[n as usize].label.keys().copied().collect();
        atoms.sort_unstable();
        let mut dep = Dep::default();
        for a in &atoms {
            dep = dep.union(&self.nodes[n as usize].label[a]);
        }
        let smallest = atoms
            .iter()
            .filter_map(|&a| match self.expr(a) {
                DataRangeExpr::OneOf(vs) => Some(vs),
                _ => None,
            })
            .min_by_key(|vs| vs.len())
            .cloned();
        let cands = match smallest {
            None => None,
            Some(mut vs) => {
                vs.sort();
                vs.dedup();
                let mut keep = Vec::with_capacity(vs.len());
                'value: for v in vs {
                    for &a in &atoms {
                        match self.contains(a, &v) {
                            Tri::Yes => {}
                            Tri::No => continue 'value,
                            Tri::Unknown => {
                                return Err(DataOutcome::Undecided(dep.clone(), atoms.clone()))
                            }
                        }
                    }
                    keep.push(v);
                }
                Some(keep)
            }
        };
        let empty = match &cands {
            Some(c) => c.is_empty(),
            None => match self.count(&atoms, 1, &[]) {
                Tri::Yes => false,
                Tri::No => true,
                Tri::Unknown => return Err(DataOutcome::Undecided(dep, atoms)),
            },
        };
        if empty {
            return Err(DataOutcome::Clash(dep));
        }
        Ok(CNode {
            node: n,
            atoms,
            dep,
            cands,
        })
    }

    /// The datatype check of one connected component of the `≉` graph.
    fn solve_component(&mut self, comp: &[Node]) -> DataOutcome {
        let mut cs = Vec::with_capacity(comp.len());
        for &n in comp {
            match self.cnode(n) {
                Ok(c) => cs.push(c),
                Err(out) => return out,
            }
        }
        if cs.len() == 1 {
            return DataOutcome::Sat;
        }
        let pos: FxHashMap<Node, usize> = cs.iter().enumerate().map(|(i, c)| (c.node, i)).collect();
        let adj: Vec<Vec<usize>> = cs
            .iter()
            .map(|c| {
                let mut v: Vec<usize> = self
                    .cineq
                    .get(&c.node)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|m| pos.get(m).copied())
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            })
            .collect();
        // Peel nodes with more values than live neighbours: they get a value last.
        let mut alive = vec![true; cs.len()];
        loop {
            let mut changed = false;
            for i in 0..cs.len() {
                if !alive[i] {
                    continue;
                }
                let deg = adj[i].iter().filter(|&&j| alive[j]).count();
                let big = match &cs[i].cands {
                    Some(c) => c.len() > deg,
                    None => {
                        let atoms = cs[i].atoms.clone();
                        self.count(&atoms, deg + 1, &[]) == Tri::Yes
                    }
                };
                if big {
                    alive[i] = false;
                    changed = true;
                }
            }
            if !changed || self.over_budget() {
                break;
            }
        }
        let rest: Vec<usize> = (0..cs.len()).filter(|&i| alive[i]).collect();
        if rest.is_empty() || self.over_budget() {
            return DataOutcome::Sat;
        }
        // The dependencies of what is left: its labels and the inequalities among it.
        let mut dep = Dep::default();
        let mut atoms: Vec<Atom> = Vec::new();
        for &i in &rest {
            dep = dep.union(&cs[i].dep);
            atoms.extend_from_slice(&cs[i].atoms);
            for &j in &adj[i] {
                if alive[j] && i < j {
                    let (a, b) = (cs[i].node, cs[j].node);
                    if let Some(d) = self.ineq.get(&(a.min(b), a.max(b))) {
                        dep = dep.union(d);
                    }
                }
            }
        }
        let enumerated: Vec<usize> = rest
            .iter()
            .copied()
            .filter(|&i| cs[i].cands.is_some())
            .collect();
        let open: Vec<usize> = rest
            .iter()
            .copied()
            .filter(|&i| cs[i].cands.is_none())
            .collect();
        let mut assigned: Vec<Option<Value>> = vec![None; cs.len()];
        let mut any_undecided = false;
        let sat = self.assign(
            &cs,
            &adj,
            &enumerated,
            0,
            &mut assigned,
            &open,
            &mut any_undecided,
        );
        if sat {
            DataOutcome::Sat
        } else if any_undecided {
            atoms.sort_unstable();
            atoms.dedup();
            DataOutcome::Undecided(dep, atoms)
        } else {
            DataOutcome::Clash(dep)
        }
    }

    /// Backtracking over the enumerated nodes `enumerated[k..]`; at each full assignment the
    /// open nodes are checked. `true` when some assignment works.
    #[allow(clippy::too_many_arguments)]
    fn assign(
        &mut self,
        cs: &[CNode],
        adj: &[Vec<usize>],
        enumerated: &[usize],
        k: usize,
        assigned: &mut Vec<Option<Value>>,
        open: &[usize],
        any_undecided: &mut bool,
    ) -> bool {
        if self.probe() {
            // Over budget: `run` reports the exhaustion before using the answer.
            return true;
        }
        let Some(&i) = enumerated.get(k) else {
            return match self.open_nodes(cs, adj, assigned, open) {
                Tri::Yes => true,
                Tri::No => false,
                Tri::Unknown => {
                    *any_undecided = true;
                    false
                }
            };
        };
        let cands = cs[i].cands.as_ref().expect("enumerated");
        for v in cands {
            if adj[i].iter().any(|&j| assigned[j].as_ref() == Some(v)) {
                continue;
            }
            assigned[i] = Some(v.clone());
            let ok = self.assign(cs, adj, enumerated, k + 1, assigned, open, any_undecided);
            assigned[i] = None;
            if ok {
                return true;
            }
        }
        false
    }

    /// Whether the open (not enumerated) nodes get values, given the values `assigned` to
    /// the enumerated ones: peel again with those values excluded, then every remaining
    /// component must be a clique of identical labels and exclusions.
    fn open_nodes(
        &mut self,
        cs: &[CNode],
        adj: &[Vec<usize>],
        assigned: &[Option<Value>],
        open: &[usize],
    ) -> Tri {
        if open.is_empty() {
            return Tri::Yes;
        }
        let is_open: FxHashSet<usize> = open.iter().copied().collect();
        let excluded: FxHashMap<usize, Vec<Value>> = open
            .iter()
            .map(|&i| {
                let mut vs: Vec<Value> =
                    adj[i].iter().filter_map(|&j| assigned[j].clone()).collect();
                vs.sort();
                vs.dedup();
                (i, vs)
            })
            .collect();
        let mut alive: FxHashSet<usize> = is_open.clone();
        loop {
            let mut peeled = Vec::new();
            let mut order: Vec<usize> = alive.iter().copied().collect();
            order.sort_unstable();
            for i in order {
                let deg = adj[i].iter().filter(|j| alive.contains(j)).count();
                if self.count(&cs[i].atoms, deg + 1, &excluded[&i]) == Tri::Yes {
                    peeled.push(i);
                }
            }
            if peeled.is_empty() || self.over_budget() {
                break;
            }
            for i in peeled {
                alive.remove(&i);
            }
        }
        let mut left: Vec<usize> = alive.iter().copied().collect();
        left.sort_unstable();
        let mut done: FxHashSet<usize> = FxHashSet::default();
        let mut answer = Tri::Yes;
        for &start in &left {
            if !done.insert(start) {
                continue;
            }
            let mut clique = vec![start];
            let mut i = 0;
            while i < clique.len() {
                for &j in &adj[clique[i]] {
                    if alive.contains(&j) && done.insert(j) {
                        clique.push(j);
                    }
                }
                i += 1;
            }
            let first = clique[0];
            let uniform = clique.iter().all(|&a| {
                cs[a].atoms == cs[first].atoms
                    && excluded[&a] == excluded[&first]
                    && clique
                        .iter()
                        .all(|&b| a == b || adj[a].binary_search(&b).is_ok())
            });
            if !uniform {
                answer = Tri::Unknown;
                continue;
            }
            match self.count(&cs[first].atoms, clique.len(), &excluded[&first]) {
                Tri::Yes => {}
                Tri::No => return Tri::No,
                Tri::Unknown => answer = Tri::Unknown,
            }
        }
        answer
    }
}

fn remove_role(adj: &mut FxHashMap<Node, Vec<Role>>, n: Node, r: Role) {
    if let Some(rs) = adj.get_mut(&n) {
        if let Some(pos) = rs.iter().rposition(|&x| x == r) {
            rs.remove(pos);
        }
        if rs.is_empty() {
            adj.remove(&n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::clausify::clausify;
    use super::super::model::Individual;
    use super::*;
    use crate::owl2dl::extract;
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;

    /// The NI-rule names a root's neighbours only for an at-most clause of the running
    /// attempt. `≤1 r.(≥2 s.D)` at `o` defines a superset atom of its filler twice: for
    /// the eager attempt where the restriction counts (`∃r⁻.⊤ ⊓ ≥2 s.D`), for the plain one
    /// at every node. Both definitions are at-most clauses `≤1 s.D`, the eager one first.
    /// With an `r`-predecessor of `o` both match an `s`-neighbour of `o`; the plain attempt
    /// must take its own.
    #[test]
    fn the_ni_rule_skips_an_at_most_of_the_other_attempt() {
        let body = r#"
            @prefix : <http://ex/> . @prefix owl: <http://www.w3.org/2002/07/owl#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            :D a owl:Class . :r a owl:ObjectProperty . :s a owl:ObjectProperty .
            :q a owl:NamedIndividual ; :r :o .
            :o a owl:NamedIndividual , [ a owl:Restriction ; owl:onProperty :r ;
                owl:maxQualifiedCardinality "1"^^xsd:nonNegativeInteger ; owl:onClass
                [ a owl:Restriction ; owl:onProperty :s ;
                  owl:minQualifiedCardinality "2"^^xsd:nonNegativeInteger ; owl:onClass :D ] ] .
        "#;
        let (d, triples) = Graph::parse_to_triples(body, "turtle").unwrap();
        let onto = extract(&d, &triples).unwrap();
        let c = clausify(&d, &onto, Budget::default().max_steps).unwrap();
        let index = ClauseIndex::new(&c);
        let id = |frag: &str| {
            d.lookup(&Term::NamedNode(NamedNode::new_unchecked(format!(
                "http://ex/{frag}"
            ))))
        };
        let class = c.classes.iter().find(|&&(i, _)| i == id("D")).unwrap().1;
        let s = Role {
            idx: c.roles.iter().position(|&i| i == id("s")).unwrap() as u32,
            inv: false,
        };
        let scope = |k: usize| c.clauses[index.ni[k].0 as usize].scoped;
        let eager = (0..index.ni.len()).position(|k| scope(k) == Some(true));
        let plain = (0..index.ni.len()).position(|k| scope(k) == Some(false));
        assert!(eager < plain, "{eager:?} {plain:?}");
        let mut t = Tableau::new(&c, &index, Budget::default());
        for _ in &c.abox.individuals {
            t.add_root();
        }
        for &(a, n, _) in &c.abox.concepts {
            t.assert_concept(n, a);
        }
        for &(r, a, b, _) in &c.abox.roles {
            t.assert_role(r, a, b);
        }
        // `w` hangs from another root and joins `o` off the tree.
        let o = c
            .abox
            .individuals
            .iter()
            .position(|i| matches!(i, Individual::Named(n) if *n == id("o")))
            .unwrap() as Node;
        let p = t.add_root();
        let w = t.new_node(Some(p));
        t.add_role(s, o, w, &Dep::default());
        t.add_concept(w, class, &Dep::default());
        let (k, _) = t.ni_clause(o, w).expect("an at-most at o counts w");
        assert_eq!(Some(k as usize), plain);
    }

    /// A blocked child of a root is replaced by a copy of its blocker hung from that root.
    /// A blocker under another root that also joins this one off the tree would give its
    /// copy two edges to the same root collapsed into one, so only children of the same
    /// root block each other.
    #[test]
    fn children_of_different_roots_do_not_block_each_other() {
        let body = r#"
            @prefix : <http://ex/> . @prefix owl: <http://www.w3.org/2002/07/owl#> .
            :D a owl:Class . :s a owl:ObjectProperty .
            :D owl:equivalentClass [ a owl:Restriction ; owl:onProperty :s ; owl:someValuesFrom :D ] .
        "#;
        let (d, triples) = Graph::parse_to_triples(body, "turtle").unwrap();
        let onto = extract(&d, &triples).unwrap();
        let c = clausify(&d, &onto, Budget::default().max_steps).unwrap();
        let index = ClauseIndex::new(&c);
        let mut t = Tableau::new(&c, &index, Budget::default());
        let s = Role { idx: 0, inv: false };
        let (p, q) = (t.add_root(), t.add_root());
        let b = t.new_node(Some(q));
        t.add_role(s, q, b, &Dep::default());
        t.add_role(s, p, b, &Dep::default());
        let w = t.new_node(Some(p));
        t.add_role(s, p, w, &Dep::default());
        assert!(!t.blocked()[w as usize]);
        let v = t.new_node(Some(p));
        t.add_role(s, p, v, &Dep::default());
        assert!(t.blocked()[v as usize], "a sibling still blocks");
    }
}
