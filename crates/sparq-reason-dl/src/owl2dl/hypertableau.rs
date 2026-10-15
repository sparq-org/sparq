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
//! **Nominals without the NI-rule.** A nominal can leave an edge between an individual and a
//! blockable node that is not its child. A complete, clash-free graph is reported
//! satisfiable only when it provably unravels into a model despite such edges (see
//! [`Tableau::unravels`]); otherwise the branch is closed and, if no branch gives a model,
//! the run returns [`Stop::Unconfirmed`], never a verdict.
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

use super::clausify::{Atom, AtomKind, Clauses, Head, Key, Role, Var};
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
    /// to a node whose number restrictions the copies could break (the case the NI-rule of
    /// SHOIQ handles, which this calculus does not implement), or two individuals might
    /// share a key's data value that is not fixed.
    Unconfirmed,
}

/// A clause's neighbour variable: its role atoms, its concept atoms, and whether it binds
/// only nodes after the previous variable's.
type YVar = (Vec<Role>, Vec<Atom>, bool);

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

#[derive(Debug)]
enum Undo {
    Concept(Node, Atom),
    Role(u32, Node, Node),
    Ineq(Node, Node),
    NewNode,
    Deactivate(Node),
    Merge(Node),
    PendingPush,
    PendingPop(Instance),
}

#[derive(Debug)]
struct Branch {
    mark: usize,
    inst: Instance,
    alt: usize,
    acc: Dep,
}

/// One tableau run over a fixed clause set.
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
}

/// What the key rule did.
enum KeyStep {
    /// It merged two individuals (or the budget ran out).
    Merged,
    /// Nothing to merge, but some pair might share an unfixed data value.
    Unsure,
    Done,
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
    /// built, `Ok(false)` when every branch closed on a clash.
    pub(crate) fn run(&mut self) -> Result<bool, Stop> {
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
            if !unravels {
                self.close_unconfirmed();
                continue;
            }
            return Ok(true);
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
        // A named individual's canonical node stands for it only given the merges on the way.
        let mut named: Vec<(Node, Dep)> = self
            .named
            .iter()
            .map(|&n| self.canonical_dep(n))
            .filter(|&(n, _)| self.nodes[n as usize].active)
            .collect();
        named.sort_unstable_by_key(|&(n, _)| n);
        named.dedup_by_key(|&mut (n, _)| n);
        let mut unsure = false;
        for key in &self.c.keys {
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
            for (i, (x, dx)) in members.iter().enumerate() {
                for (y, dy) in &members[i + 1..] {
                    // A pair scans the named individuals once per object property.
                    if self.charge(1 + key.roles.len() * named.len()) {
                        // `run` reports the exhaustion before using anything.
                        return KeyStep::Merged;
                    }
                    match self.key_shared(key, *x, *y, &named) {
                        Shared::No => {}
                        Shared::Maybe => unsure = true,
                        Shared::Yes(d) => {
                            let d = d.union(dx).union(dy);
                            self.merge(*x, *y, &d);
                            return KeyStep::Merged;
                        }
                    }
                }
            }
        }
        if unsure {
            KeyStep::Unsure
        } else {
            KeyStep::Done
        }
    }

    /// Whether `x` and `y` share a value on every property of `key`, and on what.
    fn key_shared(&self, key: &Key, x: Node, y: Node, named: &[(Node, Dep)]) -> Shared {
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
        for &d in &key.data {
            let (vx, ux) = self.data_values(d, x);
            let (vy, uy) = self.data_values(d, y);
            let common = vx
                .iter()
                .find_map(|(v, da)| vy.iter().find(|(w, _)| w == v).map(|(_, db)| da.union(db)));
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
    fn data_values(&self, d: Role, x: Node) -> (Vec<(Value, Dep)>, bool) {
        let mut fixed = Vec::new();
        let mut unfixed = false;
        for (&n, rs) in &self.nodes[x as usize].adj {
            let node = &self.nodes[n as usize];
            if !node.active || !node.concrete || !rs.contains(&d) {
                continue;
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

    fn has_role(&self, r: Role, a: Node, b: Node) -> bool {
        self.nodes[a as usize]
            .adj
            .get(&b)
            .is_some_and(|rs| rs.contains(&r))
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

    fn prune_descendants(&mut self, t: Node) {
        let len = self.nodes.len();
        self.charge(len);
        let mut desc = vec![false; len];
        for n in (t as usize + 1)..len {
            let node = &self.nodes[n];
            if !node.active || node.root {
                continue;
            }
            if let Some(p) = node.parent {
                if p == t || desc[p as usize] {
                    desc[n] = true;
                }
            }
        }
        for (n, &is_desc) in desc.iter().enumerate() {
            if is_desc {
                self.nodes[n].active = false;
                self.trail.push(Undo::Deactivate(n as Node));
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
                1 => {
                    let h = head[0].clone();
                    self.apply(&inst, &h, &inst.deps.clone());
                }
                _ => {
                    self.pending.push(inst);
                    self.trail.push(Undo::PendingPush);
                }
            }
        }
    }

    fn match_clause(&self, ci: u32, x: Node, found: &mut Vec<Instance>) {
        let cl = &self.c.clauses[ci as usize];
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
        let level = self.branches.len() as u32;
        self.steps += 1;
        let h = self.c.clauses[inst.clause as usize].head[0].clone();
        let d = inst.deps.union(&Dep::single(level));
        self.branches.push(Branch {
            mark: self.trail.len(),
            inst: inst.clone(),
            alt: 0,
            acc: Dep::default(),
        });
        self.apply(&inst, &h, &d);
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
            let inst = b.inst.clone();
            let alt = b.alt;
            let n_alts = self.c.clauses[inst.clause as usize].head.len();
            let h = self.c.clauses[inst.clause as usize].head[alt].clone();
            let d = if alt + 1 < n_alts {
                inst.deps.union(&Dep::single(level))
            } else {
                let acc = b.acc.clone();
                self.branches.pop();
                inst.deps.union(&acc)
            };
            self.apply(&inst, &h, &d);
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

    /// Per node: whether it is blocked (directly or through its parent), and whether it
    /// blocks some node.
    fn blocking(&self) -> (Vec<bool>, Vec<bool>) {
        let len = self.nodes.len();
        let mut blocked = vec![false; len];
        let mut blocker = vec![false; len];
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
                }
            }
        }
        (blocked, blocker)
    }

    /// Whether a complete, clash-free graph is known to unravel into a model.
    ///
    /// Without nominals every edge is a tree edge (parent to child) or joins two roots, and
    /// pairwise blocking unravels the graph: each blocked node is replaced by a copy of its
    /// blocker's subtree. A nominal merges a blockable node into a root, which leaves an edge
    /// between a root and a blockable node that is not its child. Such an edge is copied with
    /// the subtree it hangs from, and the model is still one as long as
    ///
    /// - no blocked node has one (it is dropped by the unravelling, together with what it
    ///   gave the root),
    /// - no edge joins two blockable nodes outside the tree (an at-most restriction at a
    ///   root merged nodes of different subtrees), and
    /// - where the blockable end is copied, no number restriction at the root can match two
    ///   of its copies.
    ///
    /// Otherwise the answer is unconfirmed (SHOIQ needs the NI-rule here). A graph with no
    /// blocked node is a finite model as it stands.
    fn unravels(&self) -> bool {
        let (blocked, blocker) = self.blocking();
        if !blocked.contains(&true) {
            return true;
        }
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
                    return false;
                }
                if !node.root {
                    continue;
                }
                let w = v as usize;
                if blocked[w] || (copied[w] && self.counts_copies(u as Node, v)) {
                    return false;
                }
            }
        }
        true
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
            if !cl.body_x.iter().all(|a| sn.label.contains_key(a)) {
                continue;
            }
            let fits = self.index.ys[ci as usize]
                .iter()
                .filter(|(roles, atoms, _)| {
                    roles.iter().all(|r| rs.contains(r))
                        && atoms.iter().all(|a| wn.label.contains_key(a))
                })
                .count();
            if fits >= 2 {
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
        let mut cands: Vec<Node> = self.nodes[x as usize]
            .adj
            .iter()
            .filter(|(&n, rs)| {
                self.nodes[n as usize].active
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
