//! Constraint-component evaluation: walks each targeted shape's focus nodes and
//! emits [`ValidationResult`]s via direct graph scans (no SPARQL round-trip).

use crate::model::{
    sh, Component, ComponentDef, ComponentMeta, PreparedComponentValidator, ShapesModel, Target,
};
use crate::path::{Path, PathIds};
use crate::report::{ShapeDiagnostic, ValidationResult};
use crate::sparql::{ComponentResultFields, PreparedValidator};
use crate::view::GraphView;
use oxrdf::{Literal, Term};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{is_inline, is_literal_id, split_lang_dir, Id, TermParts};
use sparq_core::Graph;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::rc::Rc;

const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
/// The datatype of every dictionary inline id ([FABLE-5] sq-7d3dj.33.4).
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
/// [OPUS-4.8] (sq-0mjfd) The RDF-1.2 reification predicate: a reifier `?r` reifies
/// a triple-term via `?r rdf:reifies <<( s p o )>>`. Used by `sh:reifierShape`.
const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
thread_local! {
    /// [OPUS-4.8] (sq-7d3dj.33.1) Test-only: when set, `prime_sparql_batch` skips
    /// priming so `validate` takes the per-focus path — lets the in-crate
    /// differential test diff the batched report against the per-focus one.
    static FORCE_NO_BATCH: Cell<bool> = const { Cell::new(false) };
    /// [FABLE-5] (sq-7d3dj.33.4) Test-only: when set, `validate_shape_at` never
    /// resolves a focus-node id, so value nodes are computed by the Term-level
    /// path walk and every constraint takes the Term-level route — lets the
    /// in-crate differential tests diff the id-fast-path report against the
    /// original Term-level one.
    static FORCE_NO_IDFAST: Cell<bool> = const { Cell::new(false) };
    /// [FABLE-5] (sq-7d3dj.33.4) Test-only: counts (shape, focus) validations that
    /// took the id-level value-node walk, so the differential tests can assert
    /// the fast path actually FIRED (non-vacuity — with a never-firing fast path
    /// the report diff would be trivially, meaninglessly equal).
    static IDFAST_WALKS: Cell<u64> = const { Cell::new(0) };
    /// [FABLE-5] (sq-j9hls) Test-only: counts the id-level checks taken by the
    /// EXTENDED surfaces (sh:class / comparand paths / sh:closed / focus-target
    /// enumeration / sh:hasValue / sh:in / sh:uniqueLang) — the same non-vacuity
    /// role `IDFAST_WALKS` plays for the value-node walk.
    static IDFAST_EXT: Cell<u64> = const { Cell::new(0) };
}

/// [FABLE-5] (sq-j9hls) Ticks the extended-surface non-vacuity counter (no-op
/// outside tests). Called once per id-level route taken by an extended arm.
#[cfg(test)]
fn tick_idfast_ext() {
    IDFAST_EXT.with(|c| c.set(c.get() + 1));
}
#[cfg(not(test))]
#[inline]
fn tick_idfast_ext() {}

/// [FABLE-5] (sq-j9hls) Test-only: the number of extended-surface id-level checks
/// taken on this thread (see `IDFAST_EXT`).
#[cfg(test)]
pub(crate) fn idfast_ext_checks() -> u64 {
    IDFAST_EXT.with(Cell::get)
}

/// [FABLE-5] (sq-7d3dj.33.4) Test-only: the number of id-level value-node walks
/// taken on this thread (see `IDFAST_WALKS`).
#[cfg(test)]
pub(crate) fn idfast_walks() -> u64 {
    IDFAST_WALKS.with(Cell::get)
}

/// [OPUS-4.8] (sq-7d3dj.33.1) Test-only helper: run `f` (typically a `validate`
/// closure) with `sh:sparql` focus-node batching forced OFF, restoring the prior
/// state afterwards. Used by the batched-vs-per-focus report-equivalence test.
#[cfg(test)]
pub(crate) fn with_sparql_batching_disabled<R>(f: impl FnOnce() -> R) -> R {
    let prev = FORCE_NO_BATCH.with(Cell::get);
    FORCE_NO_BATCH.with(|c| c.set(true));
    let r = f();
    FORCE_NO_BATCH.with(|c| c.set(prev));
    r
}

/// [FABLE-5] (sq-7d3dj.33.4) Test-only helper: run `f` with the id-level
/// core-constraint fast path forced OFF (Term-level walk everywhere), restoring
/// the prior state afterwards. Used by the id-fastpath report-equivalence tests.
#[cfg(test)]
pub(crate) fn with_id_fastpath_disabled<R>(f: impl FnOnce() -> R) -> R {
    let prev = FORCE_NO_IDFAST.with(Cell::get);
    FORCE_NO_IDFAST.with(|c| c.set(true));
    let r = f();
    FORCE_NO_IDFAST.with(|c| c.set(prev));
    r
}

/// Runs every targeted shape over its focus nodes. Results are deliberately NOT
/// deduplicated: the SHACL test suite expects one result per constraint-component
/// OCCURRENCE (e.g. two sh:class violations on one value) and per traversal route
/// (a nested shape reached through two parents reports twice). Duplicate focus
/// nodes from overlapping targets ARE collapsed (in `focus_nodes`).
pub(crate) fn validate_graph(
    data: &Graph,
    shapes: &ShapesModel,
) -> (Vec<ValidationResult>, Vec<ShapeDiagnostic>) {
    let mut v = Validator::new(data, shapes);
    let mut out = Vec::new();
    for &sid in &shapes.targeted {
        // [OPUS-4.8] (sq-7d3dj.33.1) Enumerate this shape's focus nodes ONCE, then
        // batch-evaluate its `sh:sparql` constraints over the whole focus set in a
        // single query each (instead of re-running the full query per focus node —
        // the O(N_focus × full-query) quadratic root cause). `validate_shape` then
        // drains the primed per-focus results; a non-batchable constraint or a
        // nested/data-graph focus falls back to the per-focus path unchanged.
        let foci = v.focus_nodes(sid);
        v.prime_sparql_batch(sid, &foci);
        // [SONNET-4.6] (sq-7d3dj.33.5) If the shape has a SPARQL node expression
        // (`sh:values [ sh:select … ]` / `[ sh:sparqlExpr … ]`), batch-evaluate it
        // over ALL focus nodes in one query execution and cache the result map so
        // `validate_shape` drains it per focus instead of re-running per focus node.
        if let Some(idx) = v.shapes.shapes[sid].value_expr {
            let data = v.data.graph();
            let map = v.shapes.select_exprs[idx].eval_batch(data, &foci);
            v.node_expr_cache = Some((sid, map));
        }
        for focus in &foci {
            v.validate_shape(sid, focus, &mut out);
        }
        v.sparql_batch.clear();
        v.node_expr_cache = None;
    }
    // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:shape` data-graph targets: a triple
    // `?n sh:shape ?S` in the DATA graph makes `?n` a focus node of shape `?S`.
    // Unlike the shapes-graph targets above this is discovered per-data-graph, so
    // it is handled here rather than as a per-shape `Target`.
    v.validate_data_shape_targets(shapes, &mut out);
    (out, v.diagnostics)
}

/// [OPUS-4.8] (sq-rnkdh) The number of DISTINCT focus nodes the model's targets
/// select over `data` — the deterministic target-selection statistic exposed as
/// [`crate::count_focus_nodes`]. Built on the same [`Validator::focus_nodes`]
/// enumeration the real validation walk uses (plus the `sh:shape` data-graph
/// targets), so it stays in lock-step with validation, including the SHACL-1.2
/// data-dependent targets (`sh:targetWhere` / SPARQL-valued `sh:targetNode`).
pub(crate) fn count_focus_nodes(data: &Graph, shapes: &ShapesModel) -> usize {
    let mut v = Validator::new(data, shapes);
    let mut all: Vec<Term> = Vec::new();
    for &sid in &shapes.targeted {
        all.extend(v.focus_nodes(sid));
    }
    // `sh:shape` data-graph targets contribute their linked subjects as focus nodes
    // (only when the linked shape is in the model — matching the validation walk).
    for [focus, _, shape_node] in v.data.triples(None, Some(&sh("shape")), None) {
        if shapes.by_node(&shape_node).is_some() {
            all.push(focus);
        }
    }
    crate::view::dedup(all).len()
}

/// [OPUS-4.8] (sq-d1dw, `shacl-af`) True iff `node` conforms to the shape `sid`
/// over `data` — i.e. validating `node` against that shape (as a standalone
/// focus node, ignoring the shape's own targets) produces no results. The
/// SHACL-AF rules module uses this to evaluate `sh:condition` gating: a rule
/// fires for a focus node only when it conforms to every condition shape. Reuses
/// the full constraint validator (Core + sh:sparql + nested shapes), so a
/// condition may be any shape. Gated to the feature so it adds nothing when off.
#[cfg(feature = "shacl-af")]
pub(crate) fn conforms_node(data: &Graph, shapes: &ShapesModel, sid: usize, node: &Term) -> bool {
    let mut v = Validator::new(data, shapes);
    v.conforms(node, sid)
}

/// [FABLE-5] (sq-7d3dj.33.4) The value nodes of one (shape, focus) validation, in
/// either representation. The id form comes from the compiled-path walk and lets
/// the id-capable constraint arms check values without materialising terms;
/// [`ValueNodes::materialize`] converts in place (order-preserving) the first
/// time a Term-level arm needs `&[Term]`. Both routes materialise data-graph ids
/// through the same dictionary, so the representations are interchangeable
/// term-for-term.
enum ValueNodes {
    Terms(Vec<Term>),
    Ids(Vec<Id>),
}

impl ValueNodes {
    /// The Term form, converting from ids on first use (then cached in place).
    fn materialize(&mut self, g: &GraphView) -> &[Term] {
        if let ValueNodes::Ids(ids) = self {
            let terms = ids.iter().map(|&id| g.term_of(id)).collect();
            *self = ValueNodes::Terms(terms);
        }
        match self {
            ValueNodes::Terms(v) => v,
            ValueNodes::Ids(_) => unreachable!("converted to Terms above"),
        }
    }
}

/// [FABLE-5] (sq-j9hls) The id-level `rdfs:subClassOf` closure of one class: the
/// class id plus every descendant present in the data dictionary. `order` keeps
/// the BFS discovery order (the `sh:targetClass` instance enumeration must match
/// the Term-level order exactly); `set` answers the per-value instance check.
/// Both are empty when the class term is absent from the dictionary (no data
/// triple can mention it, so it has no instances — exactly the Term-level
/// outcome).
struct ClassClosure {
    order: Vec<Id>,
    set: FxHashSet<Id>,
}

struct Validator<'a> {
    data: GraphView<'a>,
    shapes: &'a ShapesModel,
    /// (focus, shape) pairs currently being validated — the recursion guard for
    /// cyclic sh:node / sh:property references (re-entry counts as conforming).
    stack: Vec<(Term, usize)>,
    /// `(focus, shape) → bool` conformance memo, indexed by shape id. Only
    /// CONTEXT-FREE results are stored — see [`conforms`](Validator::conforms)
    /// for the cycle rule that keeps this sound under the recursion guard.
    memo: Vec<FxHashMap<Term, bool>>,
    /// The lowest stack depth the recursion guard re-entered since the current
    /// `conforms` frame was pushed (`usize::MAX` = none). A frame may be
    /// memoised only if no re-entry pointed BELOW it.
    min_reentry: usize,
    /// Compiled `sh:pattern` regexes, keyed by `source\0flags`. `Rc`-shared —
    /// handing a CLONE of a `regex::Regex` to each focus node gives every clone a
    /// fresh, EMPTY cache pool, so each `is_match` rebuilt the lazy-DFA cache from
    /// scratch (the profiled `Pool::get_slow → create_cache` hot spot on the
    /// `datatype_range` workload); sharing one `Regex` through an `Rc` keeps its
    /// warm cache pool across all focus nodes. [FABLE-5] (sq-7d3dj.33.4)
    regexes: FxHashMap<String, Option<Rc<regex::Regex>>>,
    /// [OPUS-4.8] (sq-vg3y) Per-shape target-node cache for `sh:uniqueValuesFor`
    /// (the constraint needs every target node of the shape, but is evaluated once
    /// per focus node — compute the target scan once).
    uvf_targets: FxHashMap<usize, Vec<Term>>,
    /// [OPUS-4.8] (sq-lz99x) Non-fatal author-time diagnostics for constraints
    /// that were SKIPPED because they could not be evaluated (currently: an
    /// uncompilable `sh:pattern` regex). De-duplicated per (shape, pattern, flags)
    /// so a skip is reported once, not once per value/focus node.
    diagnostics: Vec<ShapeDiagnostic>,
    /// [OPUS-4.8] (sq-pb0wm) The per-constraint-statement RDF-1.2 reified-annotation
    /// override active for the component CURRENTLY being evaluated, set by
    /// `validate_shape` around each `eval_component` call. `result()` reads it to
    /// apply a per-occurrence `sh:message` / `sh:severity` (SHACL 1.2). `None` when
    /// the current constraint carries no annotation (the common case).
    active_meta: Option<ComponentMeta>,
    /// [OPUS-4.8] (sq-7d3dj.33.1) Batched `sh:sparql` results, keyed by
    /// `(shape id, sparql-constraint index)` → (focus node → its results). Primed
    /// by [`Validator::prime_sparql_batch`] once per targeted shape (over ALL its
    /// focus nodes in one execution) and drained per focus by
    /// [`Validator::eval_sparql`], replacing the O(N_focus)-query per-focus loop.
    /// A `(sid, idx)` absent here (a non-batchable constraint, or a shape validated
    /// outside the top-level targeted loop) falls back to the per-focus path.
    sparql_batch: FxHashMap<(usize, usize), FxHashMap<Term, Vec<ValidationResult>>>,
    /// [SONNET-4.6] (sq-7d3dj.33.5) Pre-computed node-expression value-node map.
    /// Set to `Some((sid, map))` by [`validate_graph`] before iterating over a
    /// shape's focus nodes when the shape carries `sh:values [ sh:select … ]` /
    /// `[ sh:sparqlExpr … ]`, then cleared to `None` after the loop. This allows
    /// [`validate_shape`] to drain from the pre-computed map instead of re-running
    /// the SPARQL query per focus node (the O(N_focus) root cause).
    node_expr_cache: Option<(usize, FxHashMap<Term, Vec<Term>>)>,
    /// [FABLE-5] (sq-7d3dj.33.4) Per-shape compiled id-level paths (index = shape
    /// id; `None` = the shape has no `sh:path`). Compiled once per validation run
    /// so the per-focus walk never re-hashes a predicate IRI string.
    compiled_paths: Vec<Option<PathIds>>,
    /// [FABLE-5] (sq-7d3dj.33.4) Id-keyed layer over [`Self::memo`]: `(focus id,
    /// shape) → bool` conformance, so the hot `sh:node` route (`conforms_id`)
    /// resolves a memo hit from a u32 hash without materialising the focus term.
    /// An entry is stored ONLY when the Term-keyed memo stored one (the same
    /// context-free rule — see [`Validator::conforms`]).
    memo_ids: Vec<FxHashMap<Id, bool>>,
    /// [FABLE-5] (sq-j9hls) Predicate ids of `rdf:type` / `rdfs:subClassOf` in
    /// the data dictionary (`None` = absent → no typed instances / no subclass
    /// edges), resolved once so the id-level class-instance checks and the
    /// `sh:targetClass` enumeration never re-hash the two IRIs.
    rdf_type_id: Option<Id>,
    subclass_of_id: Option<Id>,
    /// [FABLE-5] (sq-j9hls) Per-class id-level subclass closures, cached per
    /// class TERM (a shape set names few distinct classes; each closure is built
    /// once per validation run and shared by every focus/value check against it).
    class_closures: FxHashMap<Term, Rc<ClassClosure>>,
    /// [FABLE-5] (sq-j9hls) Compiled id-level COMPARAND paths (`sh:equals` /
    /// `sh:disjoint` / `sh:lessThan(OrEquals)` / `sh:subsetOf`), indexed by shape
    /// id and keyed by component index — compiled once per validation run exactly
    /// like [`Self::compiled_paths`], so the per-focus comparand walk never
    /// re-hashes a predicate IRI string.
    compiled_comparands: Vec<FxHashMap<usize, PathIds>>,
    /// [FABLE-5] (sq-j9hls) Dictionary ids of shapes-graph constants
    /// (`sh:hasValue` / `sh:in` members), memoised per term (`None` = absent
    /// from the data dictionary — such a constant can exactly-equal no data id).
    term_ids: FxHashMap<Term, Option<Id>>,
}

impl<'a> Validator<'a> {
    /// [FABLE-5] (sq-7d3dj.33.4) The one construction site (was triplicated across
    /// `validate_graph` / `count_focus_nodes` / `conforms_node`): builds the empty
    /// caches and eagerly compiles every shape's `sh:path` to its id-level form
    /// (cheap — shapes graphs are small; the data graph never mutates under `'a`).
    fn new(data: &'a Graph, shapes: &'a ShapesModel) -> Self {
        let view = GraphView::new(data);
        let compiled_paths = shapes
            .shapes
            .iter()
            .map(|s| s.path.as_ref().map(|p| p.compile(&view)))
            .collect();
        // [FABLE-5] (sq-j9hls) Comparand paths, compiled under the same cheap
        // eager rule as `compiled_paths` (shapes graphs are small).
        let compiled_comparands = shapes
            .shapes
            .iter()
            .map(|s| {
                let mut m = FxHashMap::default();
                for (i, c) in s.components.iter().enumerate() {
                    if let Component::Equals(p)
                    | Component::Disjoint(p)
                    | Component::LessThan(p)
                    | Component::LessThanOrEquals(p)
                    | Component::SubsetOf(p) = c
                    {
                        m.insert(i, p.compile(&view));
                    }
                }
                m
            })
            .collect();
        Validator {
            data: view,
            shapes,
            stack: Vec::new(),
            memo: vec![FxHashMap::default(); shapes.shapes.len()],
            min_reentry: usize::MAX,
            regexes: FxHashMap::default(),
            uvf_targets: FxHashMap::default(),
            diagnostics: Vec::new(),
            active_meta: None,
            sparql_batch: FxHashMap::default(),
            node_expr_cache: None,
            compiled_paths,
            memo_ids: vec![FxHashMap::default(); shapes.shapes.len()],
            rdf_type_id: view.pred_id(crate::view::RDF_TYPE),
            subclass_of_id: view.pred_id(crate::view::RDFS_SUBCLASS_OF),
            class_closures: FxHashMap::default(),
            compiled_comparands,
            term_ids: FxHashMap::default(),
        }
    }

    /// The focus nodes selected by shape `sid`'s targets, deduplicated. Takes `sid`
    /// (not a `&Shape`) and `&mut self` because the SHACL-1.2 `sh:targetWhere`
    /// target evaluates conformance through the validator (which mutates the
    /// conformance memo/stack), and the SPARQL-valued target runs a query.
    fn focus_nodes(&mut self, sid: usize) -> Vec<Term> {
        let mut out = Vec::new();
        // Read the model through the validator's own `&'a` reference (independent
        // of the `&mut self` per-target work below) — no `targets.clone()` needed.
        // [FABLE-5] (sq-7d3dj.33.4)
        let shapes: &'a ShapesModel = self.shapes;
        for t in &shapes.shapes[sid].targets {
            match t {
                Target::Node(n) => out.push(n.clone()),
                Target::Class(c) | Target::ImplicitClass(c) => {
                    let instances = self.instances_of_fast(c);
                    out.extend(instances);
                }
                Target::SubjectsOf(p) => out.extend(self.data.subjects_of(p)),
                Target::ObjectsOf(p) => out.extend(self.data.objects_of(p)),
                // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:targetWhere [ <shape> ]`:
                // every data-graph node that CONFORMS to the inline (object) shape.
                Target::Where(inner) => out.extend(self.target_where_nodes(*inner)),
                // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:targetNode [ sh:select … ]`:
                // the output nodes of the SPARQL node expression.
                Target::Sparql(idx) => out.extend(self.target_sparql_nodes(*idx)),
            }
        }
        crate::view::dedup(out)
    }

    /// [OPUS-4.8] (sq-rnkdh) The `sh:targetWhere` focus nodes: every node that
    /// appears in the data graph (as a subject OR object of any triple) and
    /// CONFORMS to the inline shape `inner`. This mirrors the SHACL-1.2 target
    /// definition ("all nodes from the data graph that conform to the given
    /// shape"); a node that appears only as a predicate is not a focus candidate.
    fn target_where_nodes(&mut self, inner: usize) -> Vec<Term> {
        let mut candidates: Vec<Term> = Vec::new();
        let mut seen: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
        for [s, _, o] in self.data.triples(None, None, None) {
            for n in [s, o] {
                if seen.insert(n.clone()) {
                    candidates.push(n);
                }
            }
        }
        candidates
            .into_iter()
            .filter(|n| self.conforms(n, inner))
            .collect()
    }

    /// [OPUS-4.8] (sq-rnkdh) The output nodes of a SPARQL-valued target node
    /// expression (`sh:targetNode [ sh:select … ]`): the node expression is run
    /// with no specific `$this` (there is no focus node yet — a target query
    /// SELECTs the focus nodes), so `$this` is left unbound and the bindings of the
    /// first result variable are the focus nodes.
    fn target_sparql_nodes(&self, idx: usize) -> Vec<Term> {
        self.shapes.select_exprs[idx].eval_target(self.data.graph())
    }

    /// [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:shape` data-graph targets: for every
    /// triple `?n sh:shape ?S` in the DATA graph, `?n` is validated against the
    /// shape `?S` (resolved in the shapes model — a shape not present in the model
    /// is skipped). This is the data-driven dual of `sh:targetNode`: the link lives
    /// in the data graph, so it is discovered here (not as a per-shape `Target`).
    fn validate_data_shape_targets(
        &mut self,
        shapes: &ShapesModel,
        out: &mut Vec<ValidationResult>,
    ) {
        for [focus, _, shape_node] in self.data.triples(None, Some(&sh("shape")), None) {
            if let Some(sid) = shapes.by_node(&shape_node) {
                self.validate_shape(sid, &focus, out);
            }
        }
    }

    /// True iff `node` produces no results against `shape` (conformance
    /// checking), memoised per `(focus, shape)` pair — shared shapes reached
    /// through several routes (sh:and members, a sh:node target re-visited per
    /// value, …) validate once.
    ///
    /// # Memoisation under the recursion guard
    ///
    /// Re-entry of an in-progress `(node, shape)` pair counts as conforming
    /// (recursion is undefined in SHACL), which makes a frame's result depend
    /// on the *stack it ran under* whenever a re-entry escaped below it. The
    /// rule that keeps the memo sound: track the LOWEST stack depth any guard
    /// hit pointed at (`min_reentry`) and store a frame's result only when no
    /// re-entry pointed below that frame. A frame that closes its own cycle
    /// (re-entry at exactly its depth) is still memoisable — re-running it
    /// from an empty stack reproduces the same self-referential assumption.
    fn conforms(&mut self, node: &Term, sid: usize) -> bool {
        self.conforms_at(node, None, sid)
    }

    /// [FABLE-5] (sq-7d3dj.33.4) [`Self::conforms`] with an optional focus-id hint
    /// forwarded to [`Self::validate_shape_at`] (see there); the recursion-guard /
    /// memo logic is unchanged.
    fn conforms_at(&mut self, node: &Term, node_id: Option<Option<Id>>, sid: usize) -> bool {
        if let Some(i) = self.stack.iter().position(|(n, s)| n == node && *s == sid) {
            // Recursive re-entry: treat as conforming, and record how deep the
            // cycle reaches so the frames above it skip the memo.
            self.min_reentry = self.min_reentry.min(i);
            return true;
        }
        if let Some(&cached) = self.memo[sid].get(node) {
            return cached;
        }
        let depth = self.stack.len();
        let saved = std::mem::replace(&mut self.min_reentry, usize::MAX);
        self.stack.push((node.clone(), sid));
        let mut tmp = Vec::new();
        self.validate_shape_at(sid, node, node_id, &mut tmp);
        self.stack.pop();
        let ok = tmp.is_empty();
        if self.min_reentry >= depth {
            // No re-entry escaped below this frame: the result is context-free.
            self.memo[sid].insert(node.clone(), ok);
        }
        // Propagate cycle reach to the enclosing frame.
        self.min_reentry = saved.min(self.min_reentry);
        ok
    }

    /// [FABLE-5] (sq-7d3dj.33.4) Id-keyed [`Self::conforms`] — the hot `sh:node`
    /// route. A [`Self::memo_ids`] hit answers from a u32 hash with NO term
    /// materialisation; a miss materialises the term once, runs the Term-level
    /// `conforms` (which owns the recursion guard + memo rule), and mirrors the
    /// verdict into `memo_ids` ONLY when the Term-keyed memo stored it — the same
    /// context-free rule, so the two memos never disagree.
    fn conforms_id(&mut self, id: Id, sid: usize) -> bool {
        if let Some(&cached) = self.memo_ids[sid].get(&id) {
            return cached;
        }
        let term = self.data.term_of(id);
        let ok = self.conforms_at(&term, Some(Some(id)), sid);
        if self.memo[sid].contains_key(&term) {
            self.memo_ids[sid].insert(id, ok);
        }
        ok
    }

    // ---- extended id-level surfaces ([FABLE-5] sq-j9hls) -------------------------

    /// The cached id-level subclass closure of `class` (see [`ClassClosure`]).
    /// BFS over `subjects_ids(rdfs:subClassOf, c)` — the same scans, in the same
    /// order, as the Term-level [`GraphView::subclass_closure`], so the closure
    /// contains exactly the ids of that walk's terms in the same discovery order.
    fn class_closure(&mut self, class: &Term) -> Rc<ClassClosure> {
        if let Some(c) = self.class_closures.get(class) {
            return Rc::clone(c);
        }
        let mut order = Vec::new();
        let mut set = FxHashSet::default();
        if let Some(cid) = self.data.id_of(class) {
            order.push(cid);
            set.insert(cid);
            if let Some(sub_p) = self.subclass_of_id {
                let mut i = 0;
                while i < order.len() {
                    let c = order[i];
                    i += 1;
                    for sub in self.data.subjects_ids(sub_p, c) {
                        if set.insert(sub) {
                            order.push(sub);
                        }
                    }
                }
            }
        }
        let rc = Rc::new(ClassClosure { order, set });
        self.class_closures.insert(class.clone(), Rc::clone(&rc));
        rc
    }

    /// Id twin of the [`GraphView::is_instance_of`] decision: `v` is a SHACL
    /// instance iff one of its `rdf:type` ids is in the class's subclass-closure
    /// set — the same reachability relation the Term-level upward superclass walk
    /// decides, answered set-side (closure cached per class). A literal / absent
    /// / type-less `v` has no `rdf:type` triples, so it is not an instance —
    /// exactly the Term-level outcome.
    fn instance_in_closure(&self, v: Id, closure: &ClassClosure) -> bool {
        if closure.set.is_empty() {
            return false;
        }
        let Some(tp) = self.rdf_type_id else {
            return false;
        };
        self.data
            .objects_ids(v, tp)
            .iter()
            .any(|t| closure.set.contains(t))
    }

    /// Id-level `sh:targetClass` focus enumeration — the twin of
    /// [`GraphView::instances_of`]: same BFS class order, same per-class subject
    /// scan order, same first-seen dedup, materialising each distinct instance
    /// exactly once (the Term route materialised every scanned row).
    fn instances_of_fast(&mut self, class: &Term) -> Vec<Term> {
        #[cfg(test)]
        if FORCE_NO_IDFAST.with(Cell::get) {
            return self.data.instances_of(class);
        }
        let Some(tp) = self.rdf_type_id else {
            // No rdf:type triple exists, so no class has instances.
            return Vec::new();
        };
        tick_idfast_ext();
        let closure = self.class_closure(class);
        let mut seen: FxHashSet<Id> = FxHashSet::default();
        let mut out = Vec::new();
        for &c in &closure.order {
            for s in self.data.subjects_ids(tp, c) {
                if seen.insert(s) {
                    out.push(self.data.term_of(s));
                }
            }
        }
        out
    }

    /// The dictionary id of value node `v` under a RESOLVED focus id: the focus's
    /// own id when `v` IS the focus (the no-path node-shape case — no re-hash),
    /// else one dictionary hash. `None` = absent from the data graph (the term
    /// can match no triple — every graph-scan decision is vacuously empty, which
    /// is exactly what the Term-level scans decide for it).
    fn value_id(&self, focus: &Term, fid: Id, v: &Term) -> Option<Id> {
        if v == focus {
            Some(fid)
        } else {
            self.data.id_of(v)
        }
    }

    /// The comparand path's value-node ids for focus id `fid`, via the per-run
    /// compiled comparand of `(sid, comp_idx)` (present for every comparand
    /// component — compiled eagerly in [`Self::new`]).
    fn comparand_values_ids(&self, sid: usize, comp_idx: usize, fid: Id) -> Vec<Id> {
        self.compiled_comparands[sid]
            .get(&comp_idx)
            .expect("comparand path compiled in Validator::new")
            .values_ids(&self.data, fid)
    }

    /// The comparand path's value nodes as TERMS: the id-level walk +
    /// materialisation when the focus id is resolved (both routes materialise
    /// through the same dictionary and the compiled walk mirrors `Path::values`
    /// order exactly, so the vectors are identical), else the Term-level walk.
    fn comparand_terms(
        &self,
        sid: usize,
        comp_idx: usize,
        fid: Option<Id>,
        p: &Path,
        focus: &Term,
    ) -> Vec<Term> {
        match fid {
            Some(f) => {
                tick_idfast_ext();
                self.comparand_values_ids(sid, comp_idx, f)
                    .into_iter()
                    .map(|id| self.data.term_of(id))
                    .collect()
            }
            None => p.values(&self.data, focus),
        }
    }

    /// The memoised dictionary id of a shapes-graph constant term.
    fn term_id(&mut self, t: &Term) -> Option<Id> {
        if let Some(&id) = self.term_ids.get(t) {
            return id;
        }
        let id = self.data.id_of(t);
        self.term_ids.insert(t.clone(), id);
        id
    }

    /// Id twin of the `sh:hasValue` membership check: does any id in `ids`
    /// denote a term SHACL-equal ([`equal_value`]) to `t`? Exact term equality
    /// is id equality (the dictionary is injective; a `t` absent from the
    /// dictionary can exactly-equal no data id); value equality (SPARQL `=`
    /// families) only relates LITERALS, so only literal ids materialise — and
    /// only when `t` is itself a literal.
    fn equal_value_ids(&mut self, ids: &[Id], t: &Term) -> bool {
        if let Some(tid) = self.term_id(t) {
            if ids.contains(&tid) {
                return true;
            }
        }
        if !matches!(t, Term::Literal(_)) {
            return false;
        }
        ids.iter().any(|&v| {
            is_literal_id(&self.data.graph().dict, v)
                && matches!(cmp_terms(&self.data.term_of(v), t), Some(Ordering::Equal))
        })
    }

    /// Id twin of the per-value `sh:in` membership check (`any(equal_value)`),
    /// under the same exact-vs-value split as [`Self::equal_value_ids`].
    fn in_list_id(&mut self, v: Id, list: &[Term]) -> bool {
        if list.iter().any(|m| self.term_id(m) == Some(v)) {
            return true;
        }
        if !is_literal_id(&self.data.graph().dict, v)
            || !list.iter().any(|m| matches!(m, Term::Literal(_)))
        {
            return false;
        }
        let vt = self.data.term_of(v);
        list.iter()
            .any(|m| matches!(cmp_terms(&vt, m), Some(Ordering::Equal)))
    }

    /// Id-level `sh:closed` scan of one value node: walks the value's
    /// `(predicate, object)` id pairs and reports every predicate outside
    /// `allowed`, materialising terms only for VIOLATING pairs. A data predicate
    /// is always in the dictionary, so string membership in the Term-level
    /// allowed set ⟺ id membership here; the non-IRI-predicate skip mirrors the
    /// Term-level arm (an id in `allowed` is the id of an IRI, so a non-IRI
    /// predicate is skipped on both routes regardless of check order).
    fn closed_scan_id(
        &mut self,
        sid: usize,
        focus: &Term,
        v: Id,
        allowed: &FxHashSet<Id>,
        out: &mut Vec<ValidationResult>,
    ) {
        for (pid, oid) in self.data.predicate_objects_ids(v) {
            if allowed.contains(&pid) {
                continue;
            }
            let p_str = match self.data.term_of(pid) {
                Term::NamedNode(n) => n.as_str().to_string(),
                _ => continue,
            };
            let mut r = self.result(
                sid,
                focus,
                Some(self.data.term_of(oid)),
                "ClosedConstraintComponent",
                format!("Predicate <{p_str}> is not allowed (closed shape)"),
            );
            r.path = Some(Path::Predicate(p_str));
            out.push(r);
        }
    }

    /// [OPUS-4.8] (sq-f8gu) Validate `node` against shape `sid` and RETURN the
    /// results produced (rather than the `conforms` boolean), under the same
    /// recursion guard. Used to collect `sh:memberShape` per-member `sh:detail`
    /// sub-results. Re-entry of an in-progress `(node, shape)` pair yields no
    /// results (the cycle is treated as conforming, exactly as `conforms` does),
    /// so a cyclic member never recurses unboundedly.
    fn member_details(&mut self, node: &Term, sid: usize) -> Vec<ValidationResult> {
        if self.stack.iter().any(|(n, s)| n == node && *s == sid) {
            return Vec::new();
        }
        self.stack.push((node.clone(), sid));
        let mut tmp = Vec::new();
        self.validate_shape(sid, node, &mut tmp);
        self.stack.pop();
        tmp
    }

    fn validate_shape(&mut self, sid: usize, focus: &Term, out: &mut Vec<ValidationResult>) {
        self.validate_shape_at(sid, focus, None, out);
    }

    /// [FABLE-5] (sq-7d3dj.33.4) [`Self::validate_shape`] with an optional focus-id
    /// hint: `Some(fid)` means the caller already resolved `focus` against the data
    /// dictionary (`fid = None` ⇒ absent). With a resolved id, value nodes are
    /// computed by the compiled id-level path walk and the id-capable constraint
    /// arms in [`Self::eval_component`] skip Term materialisation entirely; when
    /// the focus is absent from the data graph (or the hint says so), everything
    /// falls back to the original Term-level route unchanged.
    fn validate_shape_at(
        &mut self,
        sid: usize,
        focus: &Term,
        focus_id: Option<Option<Id>>,
        out: &mut Vec<ValidationResult>,
    ) {
        // Read the model through the validator's own `&'a` reference — independent
        // of the `&mut self` below, so the previous per-focus-node deep clones of
        // `components` / `component_meta` are gone. [FABLE-5] (sq-7d3dj.33.4)
        let shapes: &'a ShapesModel = self.shapes;
        let shape = &shapes.shapes[sid];
        if shape.deactivated {
            return;
        }
        // [FABLE-5] (sq-1jemy) Make nested shape evaluation TRANSPARENT to the
        // caller's per-statement annotation override. Every recursing composite
        // (`sh:node` / `sh:not` / `sh:and|or|xone` / qualified / member /
        // reifier-shape / `sh:property`) re-enters this function via
        // `conforms*()` / `validate_shape*()`, and the component loop below
        // resets `self.active_meta` per component — which used to CLOBBER the
        // outer occurrence's `{| sh:message |}` / `{| sh:severity |}` override
        // to `None` before the composite arm's `result()` read it (a silent
        // no-op). Saving the caller's meta here (take: the nested frame starts
        // clean, so an outer override can never leak INTO nested results — those
        // carry the nested shape's own metas, the reading recorded in
        // research/shacl12-conformance-gap.md §6) and restoring it on exit lets
        // the override survive the recursion for the composite's OWN result.
        let caller_meta = self.active_meta.take();
        #[cfg(test)]
        let focus_id = if FORCE_NO_IDFAST.with(Cell::get) {
            Some(None) // pretend the focus is unresolved → full Term-level route
        } else {
            focus_id
        };
        // The focus node's dictionary id, resolved AT MOST ONCE per (shape, focus)
        // validation (the Term-level route paid one dictionary hash per graph
        // lookup instead). `None` = absent from the data graph.
        let fid: Option<Id> = match focus_id {
            Some(f) => f,
            None => self.data.id_of(focus),
        };
        // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 SPARQL-valued value nodes: when the shape
        // carries `sh:values [ sh:select … ]` / `[ sh:sparqlExpr … ]`, the value
        // nodes are COMPUTED by the node expression (`$this` = focus node) instead
        // of derived by traversing `sh:path`. The reported `sh:resultPath` is still
        // the shape's path (set in `result`), so only the value-node SET changes.
        let mut values: ValueNodes = match shape.value_expr {
            Some(idx) => {
                // [SONNET-4.6] (sq-7d3dj.33.5) Use the pre-computed batch map when
                // available (set by `validate_graph` before the focused shape loop);
                // fall back to per-focus eval for nested/data-graph shapes.
                let cached = self.node_expr_cache.as_ref().and_then(|(csid, map)| {
                    if *csid == sid {
                        map.get(focus).cloned()
                    } else {
                        None
                    }
                });
                ValueNodes::Terms(crate::view::dedup(cached.unwrap_or_else(|| {
                    shapes.select_exprs[idx].eval(self.data.graph(), focus)
                })))
            }
            None => match &shape.path {
                Some(path) => match (&self.compiled_paths[sid], fid) {
                    // Id fast path: walk the compiled path over ids.
                    (Some(pids), Some(f)) => {
                        #[cfg(test)]
                        IDFAST_WALKS.with(|c| c.set(c.get() + 1));
                        ValueNodes::Ids(pids.values_ids(&self.data, f))
                    }
                    _ => ValueNodes::Terms(path.values(&self.data, focus)),
                },
                // The focus node itself. Kept as the caller's EXACT term (never
                // round-tripped through the dictionary), so a focus originating
                // outside the data graph is reported verbatim.
                None => ValueNodes::Terms(vec![focus.clone()]),
            },
        };
        // [OPUS-4.8] (sq-pb0wm) Per-constraint-statement RDF-1.2 reified-annotation
        // overrides, PARALLEL to `components`. A `{| sh:deactivated true |}` on a
        // single constraint statement suppresses ONLY that occurrence (the shape
        // keeps validating its other constraints); a `{| sh:message … |}` /
        // `{| sh:severity … |}` overrides the message / severity for ONLY that
        // occurrence's results. `result()` reads `self.active_meta` (set here) to
        // apply the message/severity override; deactivation is handled by skipping
        // the component outright.
        for (i, comp) in shape.components.iter().enumerate() {
            // Index access with a default fallback: a component WITHOUT a parallel
            // meta entry (should never happen — the vectors are kept index-aligned)
            // is treated as un-annotated rather than silently skipped (a `zip` would
            // truncate the longer vector and lose the trailing component). [OPUS-4.8] (sq-pb0wm)
            let meta = shape.component_meta.get(i);
            if meta.is_some_and(ComponentMeta::is_deactivated) {
                continue; // this constraint occurrence is deactivated (sq-pb0wm)
            }
            // Set the active per-statement override so `result()` (used by every
            // Core component arm) can apply a per-occurrence message/severity.
            self.active_meta = match meta {
                Some(m) if !m.is_empty() => Some(m.clone()),
                _ => None,
            };
            self.eval_component(sid, focus, fid, i, &mut values, comp, out);
            self.active_meta = None;
        }
        // [FABLE-5] (sq-1jemy) Hand the caller's per-statement override back
        // (see the take() above): the composite arm that recursed into this
        // frame reads it in `result()` after we return.
        self.active_meta = caller_meta;
    }

    /// A result owned by shape `sid` for `focus`. [OPUS-4.8] (sq-pb0wm) When a
    /// per-constraint-statement override is active (`self.active_meta`, set by
    /// `validate_shape` for the component being evaluated), its `sh:message` /
    /// `sh:severity` REPLACE the shape-level message / severity for this result —
    /// the SHACL 1.2 reified-annotation semantics applying to one occurrence only.
    fn result(
        &self,
        sid: usize,
        focus: &Term,
        value: Option<Term>,
        component: &str,
        message: String,
    ) -> ValidationResult {
        let shape = &self.shapes.shapes[sid];
        let (severity, messages) = match &self.active_meta {
            Some(meta) => (
                meta.severity.clone().unwrap_or_else(|| shape.severity.clone()),
                if meta.messages.is_empty() {
                    shape.messages.clone()
                } else {
                    meta.messages.clone()
                },
            ),
            None => (shape.severity.clone(), shape.messages.clone()),
        };
        ValidationResult {
            focus_node: focus.clone(),
            path: shape.path.clone(),
            value,
            source_shape: shape.node.clone(),
            // Core components have no `sh:SPARQLConstraint` node (sq-mue75).
            source_constraint: None,
            source_component: sh(component),
            severity,
            messages,
            default_message: message,
            details: Vec::new(),
        }
    }

    /// [OPUS-4.8] (sq-0mjfd) The reifiers of the asserted triple
    /// `(subject, predicate, object)` — the subjects of
    /// `?r rdf:reifies <<(subject predicate object)>>` (the RDF-1.2
    /// reified-annotation `{| … |}` form; the parser stores the triple-term as a
    /// regular `rdf:reifies` object). Returns the reifier nodes (each an
    /// IRI/blank node validated against `sh:reifierShape`). Empty when `subject`
    /// is not an IRI/blank node, or no reifier exists.
    fn reifiers_of(&self, subject: &Term, predicate: &str, object: &Term) -> Vec<Term> {
        use oxrdf::{NamedNode, NamedOrBlankNode, Triple};
        let subj: NamedOrBlankNode = match subject {
            Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
            Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
            _ => return Vec::new(),
        };
        let triple_term = Term::Triple(Box::new(Triple::new(
            subj,
            NamedNode::new_unchecked(predicate),
            object.clone(),
        )));
        self.data
            .triples(None, Some(RDF_REIFIES), Some(&triple_term))
            .into_iter()
            .map(|[s, _, _]| s)
            .collect()
    }

    /// `fid`/`comp_idx` ([FABLE-5] sq-j9hls): the resolved focus id (`None` ⇒
    /// every extended fast route is OFF — the Term-level original runs) and this
    /// component's index in the shape (the compiled-comparand key).
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn eval_component(
        &mut self,
        sid: usize,
        focus: &Term,
        fid: Option<Id>,
        comp_idx: usize,
        values: &mut ValueNodes,
        comp: &Component,
        out: &mut Vec<ValidationResult>,
    ) {
        let g = self.data;
        // ---- id-level fast arms ([FABLE-5] sq-7d3dj.33.4, extended sq-j9hls) --
        // While the value nodes are still ids (from the compiled-path walk), the
        // hot Core components check them without materialising a single term —
        // only a VIOLATING value materialises, for its report entry. Every arm
        // mirrors its Term-level twin below decision-for-decision over the same
        // dictionary records (differential-tested: `idfast_` tests in lib.rs).
        // Any other component materialises the values once and takes the
        // unchanged Term-level route.
        if let (ValueNodes::Ids(ids), Some(fid)) = (&*values, fid) {
            match comp {
                Component::MinCount(n) => {
                    if (ids.len() as u64) < *n {
                        out.push(self.result(
                            sid,
                            focus,
                            None,
                            "MinCountConstraintComponent",
                            format!("Fewer than {n} values"),
                        ));
                    }
                    return;
                }
                Component::MaxCount(n) => {
                    if (ids.len() as u64) > *n {
                        out.push(self.result(
                            sid,
                            focus,
                            None,
                            "MaxCountConstraintComponent",
                            format!("More than {n} values"),
                        ));
                    }
                    return;
                }
                Component::Datatype(dts) => {
                    for &v in ids {
                        if !datatype_ok_id(&g, v, dts) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "DatatypeConstraintComponent",
                                iri_set_message("datatype", dts),
                            ));
                        }
                    }
                    return;
                }
                Component::NodeKind(kinds) => {
                    for &v in ids {
                        if !kinds.iter().any(|kind| node_kind_matches_id(&g, v, kind)) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "NodeKindConstraintComponent",
                                iri_set_message("node kind", kinds),
                            ));
                        }
                    }
                    return;
                }
                Component::MinLength(n) => {
                    for &v in ids {
                        let ok = string_repr_id(&g, v)
                            .map(|s| s.chars().count() as u64 >= *n)
                            .unwrap_or(false);
                        if !ok {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "MinLengthConstraintComponent",
                                format!("Value is shorter than {n} characters"),
                            ));
                        }
                    }
                    return;
                }
                Component::MaxLength(n) => {
                    for &v in ids {
                        let ok = string_repr_id(&g, v)
                            .map(|s| s.chars().count() as u64 <= *n)
                            .unwrap_or(false);
                        if !ok {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "MaxLengthConstraintComponent",
                                format!("Value is longer than {n} characters"),
                            ));
                        }
                    }
                    return;
                }
                Component::Pattern { source, flags } => {
                    match self.regex_for(source, flags.as_deref()) {
                        None => self.note_pattern_skipped(sid, source, flags.as_deref()),
                        Some(re) => {
                            for &v in ids {
                                let ok = match string_repr_id(&g, v) {
                                    Some(s) => re.is_match(&s),
                                    None => false,
                                };
                                if !ok {
                                    out.push(self.result(
                                        sid,
                                        focus,
                                        Some(g.term_of(v)),
                                        "PatternConstraintComponent",
                                        format!("Value does not match pattern \"{source}\""),
                                    ));
                                }
                            }
                        }
                    }
                    return;
                }
                Component::Node(inner) => {
                    for &v in ids {
                        if !self.conforms_id(v, *inner) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "NodeConstraintComponent",
                                "Value does not conform to the node shape".into(),
                            ));
                        }
                    }
                    return;
                }
                // ---- extended arms ([FABLE-5] sq-j9hls) ----------------------
                Component::Class(cls) => {
                    tick_idfast_ext();
                    let closure = self.class_closure(cls);
                    for &v in ids {
                        if !self.instance_in_closure(v, &closure) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "ClassConstraintComponent",
                                format!("Value does not have class {cls}"),
                            ));
                        }
                    }
                    return;
                }
                Component::ClassIn(classes) => {
                    tick_idfast_ext();
                    let closures: Vec<Rc<ClassClosure>> =
                        classes.iter().map(|c| self.class_closure(c)).collect();
                    for &v in ids {
                        if !closures.iter().any(|c| self.instance_in_closure(v, c)) {
                            let joined = classes
                                .iter()
                                .map(|c| c.to_string())
                                .collect::<Vec<_>>()
                                .join(" ");
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "ClassConstraintComponent",
                                format!("Value does not have any of the allowed classes {joined}"),
                            ));
                        }
                    }
                    return;
                }
                // sh:equals — both membership directions are exact-term checks,
                // which over dictionary ids is id equality; only violating ids
                // materialise. Result order mirrors the Term arm: path values
                // missing from the comparand first, then comparand values
                // missing from the path.
                Component::Equals(p) => {
                    tick_idfast_ext();
                    let others = self.comparand_values_ids(sid, comp_idx, fid);
                    for &v in ids {
                        if !others.contains(&v) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "EqualsConstraintComponent",
                                format!("Value missing from {}", p.to_turtle()),
                            ));
                        }
                    }
                    for &o in &others {
                        if !ids.contains(&o) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(o)),
                                "EqualsConstraintComponent",
                                format!("Value of {} missing from the path values", p.to_turtle()),
                            ));
                        }
                    }
                    return;
                }
                Component::Disjoint(p) => {
                    tick_idfast_ext();
                    let others = self.comparand_values_ids(sid, comp_idx, fid);
                    for &v in ids {
                        if others.contains(&v) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "DisjointConstraintComponent",
                                format!("Value shared with {}", p.to_turtle()),
                            ));
                        }
                    }
                    return;
                }
                Component::SubsetOf(p) => {
                    tick_idfast_ext();
                    let others = self.comparand_values_ids(sid, comp_idx, fid);
                    for &v in ids {
                        if !others.contains(&v) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "SubsetOfConstraintComponent",
                                format!("Value not in the value set of {}", p.to_turtle()),
                            ));
                        }
                    }
                    return;
                }
                Component::HasValue(t) => {
                    tick_idfast_ext();
                    if !self.equal_value_ids(ids, t) {
                        out.push(self.result(
                            sid,
                            focus,
                            None,
                            "HasValueConstraintComponent",
                            format!("Missing required value {t}"),
                        ));
                    }
                    return;
                }
                Component::In(list) => {
                    tick_idfast_ext();
                    for &v in ids {
                        if !self.in_list_id(v, list) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(g.term_of(v)),
                                "InConstraintComponent",
                                "Value is not in the enumeration".into(),
                            ));
                        }
                    }
                    return;
                }
                // sh:uniqueLang — the language key comes straight from the
                // stored slot: `split_lang_dir` agrees with the materialised
                // `Literal::language()`/`direction()` pair by construction (the
                // dict documents both paths validate the `--dir` suffix
                // identically), so the keys — and the sorted duplicate list —
                // match the Term arm byte-for-byte with ZERO materialisation.
                Component::UniqueLang => {
                    tick_idfast_ext();
                    let mut counts: FxHashMap<String, usize> = FxHashMap::default();
                    for &v in ids {
                        if is_inline(v) {
                            continue; // inline xsd:integer — no language tag
                        }
                        if let TermParts::Lit {
                            lang: Some(slot), ..
                        } = g.graph().dict.term_parts(v)
                        {
                            let (tag, dir) = split_lang_dir(slot);
                            let key = match dir {
                                Some(d) => format!("{}--{}", tag.to_lowercase(), d),
                                None => tag.to_lowercase(),
                            };
                            *counts.entry(key).or_insert(0) += 1;
                        }
                    }
                    let mut langs: Vec<&String> = counts
                        .iter()
                        .filter(|(_, &c)| c > 1)
                        .map(|(l, _)| l)
                        .collect();
                    langs.sort();
                    for lang in langs {
                        out.push(self.result(
                            sid,
                            focus,
                            None,
                            "UniqueLangConstraintComponent",
                            format!("Language \"{}\" is used more than once", lang),
                        ));
                    }
                    return;
                }
                _ => {} // no id twin — materialise and take the Term-level route
            }
        }
        let values: &[Term] = values.materialize(&g);
        match comp {
            Component::Class(cls) => {
                // [FABLE-5] (sq-j9hls) With a resolved focus id, decide via the
                // cached id-level closure (Term-form values arise for node shapes
                // — the value IS the focus — and for `sh:values` expressions).
                // Both routes resolve `v` through the same dictionary hash, so
                // the per-value instance verdicts are identical.
                let closure = fid.map(|_| self.class_closure(cls));
                if closure.is_some() {
                    tick_idfast_ext();
                }
                for v in values {
                    let ok = match (&closure, fid) {
                        (Some(c), Some(f)) => self
                            .value_id(focus, f, v)
                            .is_some_and(|vid| self.instance_in_closure(vid, c)),
                        _ => self.data.is_instance_of(v, cls),
                    };
                    if !ok {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "ClassConstraintComponent",
                            format!("Value does not have class {cls}"),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-sx15d) `sh:class ( ex:A ex:B )` (SHACL 1.2): a value
            // conforms iff it is a SHACL instance of ANY listed class (the
            // subclass-aware `is_instance_of` is reused per listed class).
            Component::ClassIn(classes) => {
                // [FABLE-5] (sq-j9hls) Same id-level closure route as `sh:class`.
                let closures: Option<Vec<Rc<ClassClosure>>> = fid.map(|_| {
                    tick_idfast_ext();
                    classes.iter().map(|c| self.class_closure(c)).collect()
                });
                for v in values {
                    let ok = match (&closures, fid) {
                        (Some(cs), Some(f)) => self.value_id(focus, f, v).is_some_and(|vid| {
                            cs.iter().any(|c| self.instance_in_closure(vid, c))
                        }),
                        _ => classes.iter().any(|cls| self.data.is_instance_of(v, cls)),
                    };
                    if !ok {
                        let joined = classes
                            .iter()
                            .map(|c| c.to_string())
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "ClassConstraintComponent",
                            format!("Value does not have any of the allowed classes {joined}"),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-vg3y) A value conforms iff it is a literal whose
            // (well-formed) datatype is ANY of the allowed datatypes — the single
            // IRI is a singleton set, the SHACL-1.2 list form is the disjunction.
            Component::Datatype(dts) => {
                for v in values {
                    let ok = match v {
                        Term::Literal(l) => {
                            dts.iter().any(|dt| l.datatype().as_str() == dt) && well_formed(l)
                        }
                        _ => false,
                    };
                    if !ok {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "DatatypeConstraintComponent",
                            iri_set_message("datatype", dts),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-vg3y) A value conforms iff its node kind matches ANY of
            // the allowed kinds (single IRI = singleton; SHACL-1.2 list = disjunction).
            Component::NodeKind(kinds) => {
                for v in values {
                    let ok = kinds.iter().any(|kind| node_kind_matches(v, kind));
                    if !ok {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "NodeKindConstraintComponent",
                            iri_set_message("node kind", kinds),
                        ));
                    }
                }
            }
            Component::MinCount(n) => {
                if (values.len() as u64) < *n {
                    out.push(self.result(
                        sid,
                        focus,
                        None,
                        "MinCountConstraintComponent",
                        format!("Fewer than {n} values"),
                    ));
                }
            }
            Component::MaxCount(n) => {
                if (values.len() as u64) > *n {
                    out.push(self.result(
                        sid,
                        focus,
                        None,
                        "MaxCountConstraintComponent",
                        format!("More than {n} values"),
                    ));
                }
            }
            Component::MinExclusive(b) => self.range(
                sid,
                focus,
                values,
                b,
                &[Ordering::Greater],
                "MinExclusiveConstraintComponent",
                out,
            ),
            Component::MinInclusive(b) => self.range(
                sid,
                focus,
                values,
                b,
                &[Ordering::Greater, Ordering::Equal],
                "MinInclusiveConstraintComponent",
                out,
            ),
            Component::MaxExclusive(b) => self.range(
                sid,
                focus,
                values,
                b,
                &[Ordering::Less],
                "MaxExclusiveConstraintComponent",
                out,
            ),
            Component::MaxInclusive(b) => self.range(
                sid,
                focus,
                values,
                b,
                &[Ordering::Less, Ordering::Equal],
                "MaxInclusiveConstraintComponent",
                out,
            ),
            Component::MinLength(n) => {
                for v in values {
                    let ok = string_repr(v)
                        .map(|s| s.chars().count() as u64 >= *n)
                        .unwrap_or(false);
                    if !ok {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "MinLengthConstraintComponent",
                            format!("Value is shorter than {n} characters"),
                        ));
                    }
                }
            }
            Component::MaxLength(n) => {
                for v in values {
                    let ok = string_repr(v)
                        .map(|s| s.chars().count() as u64 <= *n)
                        .unwrap_or(false);
                    if !ok {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "MaxLengthConstraintComponent",
                            format!("Value is longer than {n} characters"),
                        ));
                    }
                }
            }
            Component::Pattern { source, flags } => {
                // [OPUS-4.8] (sq-lz99x) An uncompilable `sh:pattern` (e.g. an
                // unbalanced bracket, or a `(?!...)` lookahead the Rust `regex`
                // crate does not support — neither does the XML Schema regex
                // flavour the W3C spec ties `sh:pattern` to) yields no regex.
                // SKIP the constraint (the crate's lenient ill-formed-shape
                // policy) and record a diagnostic, instead of fail-closed
                // flagging EVERY value as a violation.
                match self.regex_for(source, flags.as_deref()) {
                    None => self.note_pattern_skipped(sid, source, flags.as_deref()),
                    Some(re) => {
                        for v in values {
                            let ok = match string_repr(v) {
                                Some(s) => re.is_match(&s),
                                None => false,
                            };
                            if !ok {
                                out.push(self.result(
                                    sid,
                                    focus,
                                    Some(v.clone()),
                                    "PatternConstraintComponent",
                                    format!("Value does not match pattern \"{source}\""),
                                ));
                            }
                        }
                    }
                }
            }
            Component::LanguageIn(tags) => {
                for v in values {
                    let ok = match v {
                        Term::Literal(l) => match l.language() {
                            Some(lang) => tags.iter().any(|t| lang_matches(lang, t)),
                            None => false,
                        },
                        _ => false,
                    };
                    if !ok {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "LanguageInConstraintComponent",
                            "Value language is not in the allowed list".into(),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-0mjfd) `sh:uniqueLang true` — no language may repeat
            // across the value set. The key includes the BASE DIRECTION (RDF-1.2
            // `rdf:dirLangString`, `@ar--ltr`): `"X"@ar`, `"Y"@ar--ltr` and
            // `"Z"@ar--rtl` are THREE distinct keys, so a value set with one of each
            // conforms, while two `@ar--ltr` values collide (uniqueLang-003).
            Component::UniqueLang => {
                let mut counts: FxHashMap<String, usize> = FxHashMap::default();
                for v in values {
                    if let Term::Literal(l) = v {
                        if let Some(lang) = l.language() {
                            // [OPUS-4.8] positional format args (rust/unused-variable CodeQL FP).
                            let key = match l.direction() {
                                Some(dir) => format!("{}--{}", lang.to_lowercase(), dir),
                                None => lang.to_lowercase(),
                            };
                            *counts.entry(key).or_insert(0) += 1;
                        }
                    }
                }
                let mut langs: Vec<&String> = counts
                    .iter()
                    .filter(|(_, &c)| c > 1)
                    .map(|(l, _)| l)
                    .collect();
                langs.sort();
                for lang in langs {
                    out.push(self.result(
                        sid,
                        focus,
                        None,
                        "UniqueLangConstraintComponent",
                        format!("Language \"{}\" is used more than once", lang),
                    ));
                }
            }
            // [OPUS-4.8] (sq-sx15d) `sh:equals` — the comparand is a SHACL property
            // PATH (SHACL 1.2; a bare predicate IRI is a trivial path). The value
            // sets must be equal: one result per path value absent from the
            // comparand set, and one per comparand value absent from the path set.
            Component::Equals(p) => {
                let others = self.comparand_terms(sid, comp_idx, fid, p, focus);
                for v in values {
                    if !others.contains(v) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "EqualsConstraintComponent",
                            format!("Value missing from {}", p.to_turtle()),
                        ));
                    }
                }
                for o in &others {
                    if !values.contains(o) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(o.clone()),
                            "EqualsConstraintComponent",
                            format!("Value of {} missing from the path values", p.to_turtle()),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-sx15d) `sh:disjoint` — the comparand is a PATH (SHACL
            // 1.2). A path value shared with the comparand value set is a result.
            Component::Disjoint(p) => {
                let others = self.comparand_terms(sid, comp_idx, fid, p, focus);
                for v in values {
                    if others.contains(v) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "DisjointConstraintComponent",
                            format!("Value shared with {}", p.to_turtle()),
                        ));
                    }
                }
            }
            // One result per FAILING PAIR (value, other) — the suite expects e.g.
            // `1 < "a"` and `1 < "b"` to report `sh:value 1` twice.
            // [OPUS-4.8] (sq-sx15d) the comparand is a PATH (SHACL 1.2).
            Component::LessThan(p) => {
                // [FABLE-5] (sq-j9hls) The pair comparison needs terms either
                // way, so only the comparand WALK is id-level here.
                let others = self.comparand_terms(sid, comp_idx, fid, p, focus);
                for v in values {
                    for o in &others {
                        if !matches!(cmp_terms(v, o), Some(Ordering::Less)) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(v.clone()),
                                "LessThanConstraintComponent",
                                format!("Value is not less than {o} (via {})", p.to_turtle()),
                            ));
                        }
                    }
                }
            }
            Component::LessThanOrEquals(p) => {
                let others = self.comparand_terms(sid, comp_idx, fid, p, focus);
                for v in values {
                    for o in &others {
                        if !matches!(cmp_terms(v, o), Some(Ordering::Less | Ordering::Equal)) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(v.clone()),
                                "LessThanOrEqualsConstraintComponent",
                                format!(
                                    "Value is not less than or equal to {o} (via {})",
                                    p.to_turtle()
                                ),
                            ));
                        }
                    }
                }
            }
            // [OPUS-4.8] (sq-sx15d) `sh:subsetOf` (SHACL 1.2) — the value set of the
            // shape's path must be a SUBSET of the comparand path's value set. One
            // result per path value absent from the comparand set.
            Component::SubsetOf(p) => {
                let others = self.comparand_terms(sid, comp_idx, fid, p, focus);
                for v in values {
                    if !others.contains(v) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "SubsetOfConstraintComponent",
                            format!("Value not in the value set of {}", p.to_turtle()),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-sx15d) `sh:someValue` (SHACL 1.2) — EXISTENTIAL: at
            // least one value node must conform to the nested shape. One result on
            // the focus/path (NO sh:value) when NONE conform. An empty value set
            // also fails (no value can satisfy the existential).
            Component::SomeValue(inner) => {
                if !values.iter().any(|v| self.conforms(v, *inner)) {
                    out.push(self.result(
                        sid,
                        focus,
                        None,
                        "SomeValueConstraintComponent",
                        "No value node conforms to the sh:someValue shape".into(),
                    ));
                }
            }
            // [OPUS-4.8] (sq-sx15d) `sh:singleLine true` (SHACL 1.2) — each string
            // value must contain no line-break character (LF / CR / FF / VT). A
            // non-string value contributes its lexical form; a blank node has none
            // and conforms vacuously (no string to break).
            Component::SingleLine => {
                for v in values {
                    if let Some(s) = string_repr(v) {
                        if s.chars().any(is_line_break) {
                            out.push(self.result(
                                sid,
                                focus,
                                Some(v.clone()),
                                "SingleLineConstraintComponent",
                                "Value contains a line break".into(),
                            ));
                        }
                    }
                }
            }
            // [OPUS-4.8] (sq-sx15d) `sh:rootClass` (SHACL 1.2) — each value node must
            // be the named class or a transitive `rdfs:subClassOf`-descendant of it
            // (reuses the `sh:class` subclass closure).
            Component::RootClass(cls) => {
                let closure = self.data.subclass_closure(cls);
                for v in values {
                    if !closure.contains(v) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "RootClassConstraintComponent",
                            format!("Value is not {cls} or a subclass of it"),
                        ));
                    }
                }
            }
            Component::Not(inner) => {
                for v in values {
                    if self.conforms(v, *inner) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "NotConstraintComponent",
                            "Value conforms to the negated shape".into(),
                        ));
                    }
                }
            }
            Component::And(ids) => {
                for v in values {
                    if !ids.iter().all(|&s| self.conforms(v, s)) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "AndConstraintComponent",
                            "Value does not conform to every member shape".into(),
                        ));
                    }
                }
            }
            Component::Or(ids) => {
                for v in values {
                    if !ids.iter().any(|&s| self.conforms(v, s)) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "OrConstraintComponent",
                            "Value does not conform to any member shape".into(),
                        ));
                    }
                }
            }
            Component::Xone(ids) => {
                for v in values {
                    let n = ids.iter().filter(|&&s| self.conforms(v, s)).count();
                    if n != 1 {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "XoneConstraintComponent",
                            format!("Value conforms to {n} member shapes, not exactly one"),
                        ));
                    }
                }
            }
            Component::Node(inner) => {
                for v in values {
                    if !self.conforms(v, *inner) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "NodeConstraintComponent",
                            "Value does not conform to the node shape".into(),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-0mjfd) `sh:reifierShape` (SHACL 1.2): for each value
            // `v`, the reifiers of the asserted triple `(focus, predicate, v)` must
            // conform to the referenced shape; `required` additionally demands a
            // reifier exist. The predicate is the shape's path when it is a single
            // predicate (the only path form for which a reified triple is defined).
            Component::ReifierShape {
                shape: inner,
                required,
            } => {
                let predicate = match &self.shapes.shapes[sid].path {
                    Some(Path::Predicate(p)) => Some(p.clone()),
                    _ => None,
                };
                for v in values {
                    let reifiers = match &predicate {
                        Some(p) => self.reifiers_of(focus, p, v),
                        None => Vec::new(),
                    };
                    let conforms = if reifiers.is_empty() {
                        // No reifier: conforms unless reification is required.
                        !*required
                    } else {
                        reifiers.iter().all(|r| self.conforms(r, *inner))
                    };
                    if !conforms {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "ReifierShapeConstraintComponent",
                            "A reifier of the value's triple does not conform to sh:reifierShape"
                                .into(),
                        ));
                    }
                }
            }
            Component::Property(child) => {
                for v in values {
                    // [OPUS-4.8] Guard the direct validate_shape recursion against cyclic
                    // sh:property references (e.g. A sh:property B / B sh:property A): without the
                    // (focus, shape) re-entry check this overflows the stack. Re-entry into a
                    // (value, child) pair already on the stack counts as conforming (SHACL leaves
                    // recursion undefined and the validator treats a cycle as satisfied), so we
                    // skip it — emitting no further results — and record the cycle reach so any
                    // enclosing `conforms` frame skips its memo. Mirrors `conforms`'s guard. 1616.
                    if let Some(i) = self.stack.iter().position(|(n, s)| n == v && *s == *child) {
                        self.min_reentry = self.min_reentry.min(i);
                        continue;
                    }
                    self.stack.push((v.clone(), *child));
                    self.validate_shape(*child, v, out);
                    self.stack.pop();
                }
            }
            Component::Qualified {
                shape,
                min,
                max,
                disjoint,
                siblings,
            } => {
                let count = values
                    .iter()
                    .filter(|v| {
                        self.conforms(v, *shape)
                            && (!disjoint || !siblings.iter().any(|&s| self.conforms(v, s)))
                    })
                    .count() as u64;
                if let Some(min) = min {
                    if count < *min {
                        // [SONNET-4.6] (sq-7os4t) A Qualified component merges
                        // several parameter statements. Select the annotation on
                        // the statement that caused this particular result.
                        let meta = self.shapes.shapes[sid]
                            .qualified_min_meta
                            .get(comp_idx)
                            .cloned()
                            .unwrap_or_default();
                        if !meta.is_deactivated() {
                            let component_meta = self.active_meta.replace(meta);
                            out.push(self.result(
                                sid,
                                focus,
                                None,
                                "QualifiedMinCountConstraintComponent",
                                format!("Fewer than {min} values conform to the qualified shape"),
                            ));
                            self.active_meta = component_meta;
                        }
                    }
                }
                if let Some(max) = max {
                    if count > *max {
                        let meta = self.shapes.shapes[sid]
                            .qualified_max_meta
                            .get(comp_idx)
                            .cloned()
                            .unwrap_or_default();
                        if !meta.is_deactivated() {
                            let component_meta = self.active_meta.replace(meta);
                            out.push(self.result(
                                sid,
                                focus,
                                None,
                                "QualifiedMaxCountConstraintComponent",
                                format!("More than {max} values conform to the qualified shape"),
                            ));
                            self.active_meta = component_meta;
                        }
                    }
                }
            }
            Component::Closed { ignored, by_types } => {
                let ignored_preds: Vec<String> = ignored
                    .iter()
                    .filter_map(|t| match t {
                        Term::NamedNode(n) => Some(n.as_str().to_string()),
                        _ => None,
                    })
                    .collect();
                // `sh:closed true`: the allowed set is fixed (this shape's
                // sh:property/sh:path predicates), so compute it once. `sh:closed
                // sh:ByTypes`: the allowed set is recomputed PER value node from its
                // rdf:type(s) (SHACL-1.2 §4.8.1 collectProperties), so defer it.
                // [OPUS-4.8] (sq-vg3y) added the ByTypes branch.
                let fixed_allowed: Option<Vec<String>> = if *by_types {
                    None
                } else {
                    let mut allowed = ignored_preds.clone();
                    for &child in &self.shapes.shapes[sid].property_children {
                        if let Some(Path::Predicate(p)) = &self.shapes.shapes[child].path {
                            allowed.push(p.clone());
                        }
                    }
                    Some(allowed)
                };
                // [FABLE-5] (sq-j9hls) With a resolved focus id and a FIXED
                // allowed set, scan each value's (predicate, object) pairs at
                // the id level — an allowed IRI absent from the data dictionary
                // can never appear as a data predicate, so id-set membership ⟺
                // the Term-level string membership; a value absent from the
                // dictionary has no triples (no violations) on both routes. The
                // per-value `sh:ByTypes` allowed set stays on the Term route.
                if let (Some(f), Some(a)) = (fid, &fixed_allowed) {
                    tick_idfast_ext();
                    let allowed_ids: FxHashSet<Id> =
                        a.iter().filter_map(|p| g.pred_id(p)).collect();
                    for v in values {
                        if let Some(vid) = self.value_id(focus, f, v) {
                            self.closed_scan_id(sid, focus, vid, &allowed_ids, out);
                        }
                    }
                    return;
                }
                for v in values {
                    let allowed = match &fixed_allowed {
                        Some(a) => a.clone(),
                        None => self.close_by_types_allowed(sid, v, &ignored_preds),
                    };
                    for (p, o) in self.data.predicate_objects(v) {
                        let p_str = match &p {
                            Term::NamedNode(n) => n.as_str().to_string(),
                            _ => continue,
                        };
                        if !allowed.contains(&p_str) {
                            let mut r = self.result(
                                sid,
                                focus,
                                Some(o),
                                "ClosedConstraintComponent",
                                format!("Predicate <{p_str}> is not allowed (closed shape)"),
                            );
                            r.path = Some(Path::Predicate(p_str));
                            out.push(r);
                        }
                    }
                }
            }
            Component::HasValue(t) => {
                if !values.iter().any(|v| equal_value(v, t)) {
                    out.push(self.result(
                        sid,
                        focus,
                        None,
                        "HasValueConstraintComponent",
                        format!("Missing required value {t}"),
                    ));
                }
            }
            Component::In(list) => {
                for v in values {
                    if !list.iter().any(|m| equal_value(v, m)) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "InConstraintComponent",
                            "Value is not in the enumeration".into(),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-vg3y) `sh:memberShape` (SHACL-1.2): each value node must
            // be a well-formed SHACL list whose members all conform to `member`. A
            // non-list (or member non-conformance) is ONE top-level result on the
            // value node. (sq-f8gu) That top-level result carries one `sh:detail`
            // sub-result PER non-conforming member — the actual results of
            // validating that member against the member shape. `sh:detail` is
            // non-normative (the W3C suite compares only top-level result fields),
            // so a non-list value (no members to validate) carries no details.
            Component::MemberShape(member) => {
                for v in values {
                    let members = self.data.list_strict(v);
                    // Collect per-member sub-results before any top-level push, so
                    // the borrow of `self` for `member_details` is released first.
                    let details: Vec<ValidationResult> = match &members {
                        None => Vec::new(), // not a SHACL list — no members
                        Some(ms) => ms
                            .iter()
                            .flat_map(|m| self.member_details(m, *member))
                            .collect(),
                    };
                    // Violation iff the value is not a well-formed list, or any
                    // member produced a (detail) result.
                    if members.is_none() || !details.is_empty() {
                        let mut r = self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "MemberShapeConstraintComponent",
                            "List value is not a well-formed SHACL list, or a member \
                             does not conform to the member shape"
                                .into(),
                        );
                        r.details = details;
                        out.push(r);
                    }
                }
            }
            // [OPUS-4.8] (sq-vg3y) `sh:uniqueMembers true` (SHACL-1.2): each value
            // node must be a SHACL list whose members are pairwise distinct. A
            // non-list, or a list with duplicate members, is one result on v.
            // (sq-f8gu) A duplicate-bearing list carries one `sh:detail` sub-result
            // per DUPLICATED member (each duplicated value reported once via
            // `sh:value`); a non-list value carries no details.
            Component::UniqueMembers => {
                for v in values {
                    let members = self.data.list_strict(v);
                    let duplicates: Vec<Term> = match &members {
                        None => Vec::new(),
                        Some(ms) => duplicate_members(ms),
                    };
                    if members.is_none() || !duplicates.is_empty() {
                        let mut r = self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "UniqueMembersConstraintComponent",
                            "List value is not a well-formed SHACL list, or has \
                             duplicate members"
                                .into(),
                        );
                        r.details = duplicates
                            .into_iter()
                            .map(|d| {
                                let mut detail = self.result(
                                    sid,
                                    focus,
                                    Some(d),
                                    "UniqueMembersConstraintComponent",
                                    "List member is not unique".into(),
                                );
                                // The detail's path is the member, not the parent
                                // shape's path — clear it to avoid a misleading
                                // sh:resultPath on the sub-result.
                                detail.path = None;
                                detail
                            })
                            .collect();
                        out.push(r);
                    }
                }
            }
            // [OPUS-4.8] (sq-vg3y) `sh:maxListLength` / `sh:minListLength` (SHACL-1.2):
            // each value node must be a SHACL list with at most / at least N members.
            Component::MaxListLength(n) => {
                for v in values {
                    let violates = match self.data.list_strict(v) {
                        None => true,
                        Some(members) => members.len() as u64 > *n,
                    };
                    if violates {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "MaxListLengthConstraintComponent",
                            format!("List value is not a SHACL list with at most {n} members"),
                        ));
                    }
                }
            }
            Component::MinListLength(n) => {
                for v in values {
                    let violates = match self.data.list_strict(v) {
                        None => true,
                        Some(members) => (members.len() as u64) < *n,
                    };
                    if violates {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "MinListLengthConstraintComponent",
                            format!("List value is not a SHACL list with at least {n} members"),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-vg3y) `sh:uniqueValuesFor` (SHACL-1.2): the values of the
            // listed properties of a value node must be unique across ALL target
            // nodes of the shape. For each value node V that shares the exact same
            // value-set (for every listed property) with another target node Other,
            // one result with Other as sh:value. No result if V has no values for
            // ANY of the properties.
            Component::UniqueValuesFor(props) => {
                self.eval_unique_values_for(sid, focus, values, props, out)
            }
            // [OPUS-4.8] sh:sparql — SPARQL-based constraints (SHACL §5.2). The
            // sh:select runs against the data graph with $this pre-bound to the
            // focus node; every returned solution is one violation. Note this
            // evaluates over the FOCUS NODE (not the path value nodes): a sh:sparql
            // on a property shape sees `values` only through the query (e.g. by
            // re-traversing sh:path inside the SELECT), which matches the spec —
            // the constraint is responsible for its own value selection.
            Component::Sparql(idx) => self.eval_sparql(sid, focus, *idx, out),
            // [OPUS-4.8] SHACL §6: a SPARQL-based constraint COMPONENT that
            // activated on this shape. ASK validators run per value node
            // (ASK=false → violation); SELECT validators run per focus node
            // (each solution → violation). Parameters are pre-bound as $paramName.
            Component::CustomSparql {
                component,
                args,
                path_validator,
            } => self.eval_custom_component(
                sid,
                focus,
                values,
                *component,
                args,
                path_validator.map(|i| &self.shapes.path_validators[i]),
                out,
            ),
            // [OPUS-4.8] (sq-mk9n, `shacl-af`) `sh:expression`
            // (`sh:ExpressionConstraintComponent`): a value node is violated when
            // the node expression does NOT evaluate to `{ true }` for that value.
            #[cfg(feature = "shacl-af")]
            Component::Expression(idx) => {
                let data = self.data.graph();
                let expr = &self.shapes.expressions[*idx];
                for v in values {
                    let result = crate::rules::eval_parsed_expr(data, self.shapes, expr, v);
                    if !crate::rules::is_true_result(&result) {
                        out.push(self.result(
                            sid,
                            focus,
                            Some(v.clone()),
                            "ExpressionConstraintComponent",
                            "Value does not satisfy the sh:expression".to_string(),
                        ));
                    }
                }
            }
            // [OPUS-4.8] (sq-3w6n, `shacl-af`) `sh:nodeByExpression`
            // (`sh:NodeByExpressionConstraintComponent`): for each value node `v`,
            // the node expression is evaluated against `v` as focus to a set of
            // node-shape terms; `v` is violated when it does NOT conform to one of
            // them. (Per SHACL-AF, `sh:node` is the special case of a constant IRI
            // node-shape expression.) Result-set evaluation borrows the model
            // immutably, so the (value → shape ids) work is collected up front and
            // the recursion-safe `conforms` (which needs `&mut self`) runs after.
            #[cfg(feature = "shacl-af")]
            Component::NodeByExpression(idx) => {
                let mut checks: Vec<(Term, Vec<usize>)> = Vec::with_capacity(values.len());
                {
                    let data = self.data.graph();
                    let expr = &self.shapes.expressions[*idx];
                    for v in values {
                        let shape_terms =
                            crate::rules::eval_parsed_expr(data, self.shapes, expr, v);
                        // Resolve each computed shape term to a parsed shape id. An
                        // unparsed term (no such shape) is skipped — lenient, per
                        // this crate's policy.
                        let sids: Vec<usize> = shape_terms
                            .iter()
                            .filter_map(|s| self.shapes.by_node(s))
                            .collect();
                        checks.push((v.clone(), sids));
                    }
                }
                for (v, sids) in checks {
                    for s in sids {
                        if !self.conforms(&v, s) {
                            out.push(
                                self.result(
                                    sid,
                                    focus,
                                    Some(v.clone()),
                                    "NodeByExpressionConstraintComponent",
                                    "Value does not conform to the node shape computed by \
                                 sh:nodeByExpression"
                                        .to_string(),
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    /// [OPUS-4.8] (sq-7d3dj.33.1) Stamps one `sh:sparql` solution's per-result
    /// [`crate::sparql::ResultFields`] with the shape-owned fields (source
    /// shape/constraint/component, severity, messages). Shared by the per-focus
    /// [`Self::eval_sparql`] fallback and the batched [`Self::prime_sparql_batch`] so
    /// both stamp identically — the load-bearing report-equivalence invariant.
    fn stamp_sparql(
        &self,
        sid: usize,
        idx: usize,
        focus: &Term,
        fields: crate::sparql::ResultFields,
    ) -> ValidationResult {
        let shape = &self.shapes.shapes[sid];
        let constraint = &self.shapes.sparql[idx];
        // [OPUS-4.8] (sq-rnkdh) A constraint-level `sh:severity` on the
        // `sh:SPARQLConstraint` node OVERRIDES the shape's default severity (SHACL
        // 1.2 `sparql/node/sparql-001`); otherwise inherit the shape's severity.
        let severity = constraint
            .severity
            .clone()
            .unwrap_or_else(|| shape.severity.clone());
        ValidationResult {
            focus_node: focus.clone(),
            // A bound ?path overrides the shape's path; otherwise inherit the
            // (property-shape) path, if any.
            path: fields.path.or_else(|| shape.path.clone()),
            value: fields.value,
            source_shape: shape.node.clone(),
            // [OPUS-4.8] (sq-mue75) stamp the originating `sh:SPARQLConstraint` node
            // onto each result as `sh:sourceConstraint` (SHACL §5.2.2).
            source_constraint: Some(constraint.node.clone()),
            source_component: sh("SPARQLConstraintComponent"),
            severity,
            messages: shape.messages.clone(),
            default_message: fields.default_message,
            details: Vec::new(),
        }
    }

    /// [OPUS-4.8] (sq-7d3dj.33.1) Batch-evaluates shape `sid`'s `sh:sparql`
    /// constraints over ALL its focus nodes in ONE execution each, priming
    /// [`Self::sparql_batch`] for the per-focus drain in [`Self::eval_sparql`]. This
    /// replaces the O(N_focus × full-query) per-focus re-execution with O(1) queries
    /// (chunked at 10 000 foci) — the quadratic root cause fixed by this bead. A
    /// constraint that is NOT batchable (a top-level `LIMIT`/`OFFSET`, a `GROUP
    /// BY`/aggregate not keyed on `$this`, or `REDUCED` — see
    /// `crate::sparql::PreparedSparql::is_batchable`) is left unprimed and falls back
    /// to the per-focus path unchanged.
    fn prime_sparql_batch(&mut self, sid: usize, foci: &[Term]) {
        // [OPUS-4.8] (sq-7d3dj.33.1) Test-only knob so the in-crate differential test
        // can force the per-focus path and diff its report against the batched one.
        #[cfg(test)]
        if FORCE_NO_BATCH.with(Cell::get) {
            return;
        }
        // A single focus is as cheap per-focus (and needs no map); correctness holds
        // either way, so only prime when batching can actually amortise a query.
        if foci.len() < 2 {
            return;
        }
        let sparql_idxs: Vec<usize> = self.shapes.shapes[sid]
            .components
            .iter()
            .filter_map(|c| match c {
                Component::Sparql(i) => Some(*i),
                _ => None,
            })
            .collect();
        for idx in sparql_idxs {
            let constraint = &self.shapes.sparql[idx];
            if constraint.deactivated {
                continue;
            }
            let Some(prepared) = &constraint.prepared else {
                continue; // ill-formed sh:select — skipped (lenient, per crate policy)
            };
            if !prepared.is_batchable() {
                continue; // non-batchable shape → per-focus fallback (documented)
            }
            let map = prepared.evaluate_batch(
                self.data.graph(),
                foci,
                constraint,
                |focus, fields| self.stamp_sparql(sid, idx, focus, fields),
            );
            self.sparql_batch.insert((sid, idx), map);
        }
    }

    /// SHACL-SPARQL evaluation for one `sh:sparql` constraint occurrence.
    fn eval_sparql(
        &mut self,
        sid: usize,
        focus: &Term,
        idx: usize,
        out: &mut Vec<ValidationResult>,
    ) {
        // [OPUS-4.8] (sq-7d3dj.33.1) Fast path: drain the results this focus already
        // received when the shape's constraints were batch-evaluated (the common
        // top-level targeted case). Every focus covered by the batch has an entry —
        // even with zero violations — so a `Some(_)` here means "already computed",
        // never "not yet run"; we must not fall through and re-execute per-focus.
        if let Some(per_focus) = self.sparql_batch.get(&(sid, idx)) {
            if let Some(results) = per_focus.get(focus) {
                out.extend(results.iter().cloned());
                return;
            }
        }
        // Per-focus fallback: a non-batchable constraint, a focus outside the primed
        // set (a nested / `sh:shape` data-graph validation), or a single-focus shape.
        let constraint = &self.shapes.sparql[idx];
        if constraint.deactivated {
            return;
        }
        let Some(prepared) = &constraint.prepared else {
            return; // ill-formed sh:select — skipped (lenient, per crate policy)
        };
        let data = self.data.graph();
        prepared.evaluate(
            data,
            focus,
            constraint,
            |fields| self.stamp_sparql(sid, idx, focus, fields),
            out,
        );
    }

    /// [OPUS-4.8] SHACL §6 evaluation for one activated SPARQL-based constraint
    /// component occurrence. The validator (kind-specific or generic) runs with
    /// `$this`, the bound parameter VALUES (`$paramName`) and (on a property
    /// shape) `$PATH` pre-bound; ASK validators additionally run per VALUE NODE
    /// with `$value` pre-bound (ASK=false → one violation for that value),
    /// SELECT validators run once per focus node (§5.2 solution→result mapping).
    #[allow(clippy::too_many_arguments)]
    fn eval_custom_component(
        &mut self,
        sid: usize,
        focus: &Term,
        values: &[Term],
        cidx: usize,
        args: &[Option<Term>],
        path_validator: Option<&PreparedComponentValidator>,
        out: &mut Vec<ValidationResult>,
    ) {
        let comp: &ComponentDef = &self.shapes.components[cidx];
        let shape = &self.shapes.shapes[sid];
        let is_property_shape = shape.path.is_some();
        // [OPUS-4.8] `$PATH` pre-binding (SHACL §6.3): a property shape whose
        // chosen validator references `$PATH` carries a per-shape re-parsed
        // validator (built at model-build time, with the shape's property-path
        // expression substituted). Prefer it; otherwise use the shared one.
        let validator = match path_validator {
            Some(v) => v,
            None => match comp.validator_for(is_property_shape) {
                Some(v) => v,
                None => return, // no usable validator for this shape kind (lenient)
            },
        };
        let message = validator.message.clone();
        // [SONNET-4.6] (sq-ou3) The component's `sh:labelTemplate` (SHACL §6.1),
        // used ONLY as the last-resort message for the results below — after the
        // shape's `sh:message` and the validator's `sh:message`. It describes the
        // constraint in the component author's words, so it beats the generic
        // "does not satisfy constraint component <iri>" fallback. It never
        // influences whether the constraint fires.
        let label_template = comp.label_template().map(str::to_string);
        let component_iri = match &comp.node {
            Term::NamedNode(n) => n.as_str().to_string(),
            // A blank-node component has no IRI for sh:sourceConstraintComponent;
            // fall back to the generic SPARQL component IRI.
            _ => sh("SPARQLConstraintComponent"),
        };

        // Parameter pre-bindings, common to every value node / the SELECT run.
        // `$paramName` ← bound arg; absent optionals contribute no binding.
        let mut param_bindings: Vec<(String, Term)> = Vec::with_capacity(args.len());
        for (p, arg) in comp.parameters.iter().zip(args.iter()) {
            if let Some(term) = arg {
                param_bindings.push((p.var.clone(), term.clone()));
            }
        }
        // `$PATH` (SHACL §6.3): a property PATH, not a term, so it cannot ride the
        // VALUES table the `$this`/`$value`/`$paramName` bindings use. Instead the
        // chosen validator is RE-PARSED per property shape with the path's SPARQL
        // property-path form textually substituted (the `path_validator` above) —
        // the same approach the §5.2 `sh:sparql` path takes. `$this`/`$value`/the
        // parameters still pre-bind through VALUES below.
        let shape_path = shape.path.clone();
        let severity = shape.severity.clone();
        let shape_messages = shape.messages.clone();
        let shape_node = shape.node.clone();
        let data = self.data.graph();

        match &validator.prepared {
            PreparedValidator::Ask(query) => {
                // One ASK per value node; ASK=false (constraint not satisfied) is a
                // violation reported on that value (SHACL §6.3).
                for v in values {
                    let mut bindings: Vec<(&str, &Term)> = vec![("this", focus), ("value", v)];
                    for (name, term) in &param_bindings {
                        bindings.push((name.as_str(), term));
                    }
                    if crate::sparql::ask_violates(query, data, &bindings) {
                        let default_message = match (&message, &label_template) {
                            // {$param}/{$value}/{$this} substitution from the pre-bound terms.
                            (Some(tpl), _) => crate::sparql::substitute_bindings(tpl, &bindings),
                            // [SONNET-4.6] (sq-ou3) No validator message → render
                            // `sh:labelTemplate` over the SAME pre-bound terms, so a
                            // label may reference `{$value}` as well as parameters.
                            (None, Some(label)) => {
                                crate::sparql::substitute_bindings(label, &bindings)
                            }
                            (None, None) => format!(
                                "Value does not satisfy constraint component <{component_iri}>"
                            ),
                        };
                        out.push(ValidationResult {
                            focus_node: focus.clone(),
                            path: shape_path.clone(),
                            value: Some(v.clone()),
                            source_shape: shape_node.clone(),
                            // A SPARQL-based constraint COMPONENT has no single
                            // `sh:SPARQLConstraint` node (sq-mue75).
                            source_constraint: None,
                            source_component: component_iri.clone(),
                            severity: severity.clone(),
                            messages: shape_messages.clone(),
                            default_message,
                            details: Vec::new(),
                        });
                    }
                }
            }
            PreparedValidator::Select(query) => {
                // One SELECT per focus node; each solution is a violation.
                let mut bindings: Vec<(&str, &Term)> = vec![("this", focus)];
                for (name, term) in &param_bindings {
                    bindings.push((name.as_str(), term));
                }
                // [SONNET-4.6] (sq-ou3) Render the label's PARAMETER placeholders
                // here (parameters pre-bind via VALUES, so they are not
                // necessarily projected into the solution rows `select_validate`
                // sees); any remaining `{?var}` is then substituted per row there.
                let label = label_template
                    .as_deref()
                    .map(|tpl| crate::sparql::substitute_bindings(tpl, &bindings));
                crate::sparql::select_validate(
                    query,
                    data,
                    focus,
                    &bindings,
                    message.as_deref(),
                    label.as_deref(),
                    |fields: ComponentResultFields| ValidationResult {
                        focus_node: fields.focus,
                        path: fields.path.or_else(|| shape_path.clone()),
                        value: fields.value,
                        source_shape: shape_node.clone(),
                        // A SPARQL-based constraint COMPONENT has no single
                        // `sh:SPARQLConstraint` node (sq-mue75).
                        source_constraint: None,
                        source_component: component_iri.clone(),
                        severity: severity.clone(),
                        messages: shape_messages.clone(),
                        default_message: fields.default_message,
                        details: Vec::new(),
                    },
                    out,
                );
            }
        }
    }

    /// [OPUS-4.8] (sq-vg3y) The allowed-predicate set P for `sh:closed sh:ByTypes`
    /// at value node `v` (SHACL-1.2 §4.8.1 `collectProperties`): the IRI properties
    /// reachable via `sh:property/sh:path` from every shape that the value node's
    /// `rdf:type`s pull in — transitively through `rdfs:subClassOf`, inbound
    /// `sh:targetClass`, and `sh:node` — plus `rdf:type` and the shape's own
    /// `sh:ignoredProperties`. The shapes-graph traversal is DATA-INDEPENDENT, so
    /// it is precomputed once per shapes node into [`ShapesModel::by_types_closure`]
    /// at parse time; here we just union the closures for the shape's own node and
    /// for each `rdf:type` of the value node (read from the DATA graph).
    fn close_by_types_allowed(&self, sid: usize, v: &Term, ignored: &[String]) -> Vec<String> {
        const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        let mut allowed: rustc_hash::FxHashSet<String> = ignored.iter().cloned().collect();
        allowed.insert(RDF_TYPE.to_string());
        let mut add = |node: &Term| {
            if let Some(props) = self.shapes.by_types_closure(node) {
                allowed.extend(props.iter().cloned());
            }
        };
        // The shape's own node carries sh:closed and is the rdfs:Class in the
        // closed-003/004 tests, then every rdf:type of the value node.
        add(&self.shapes.shapes[sid].node);
        for ty in self.data.objects(v, RDF_TYPE) {
            add(&ty);
        }
        allowed.into_iter().collect()
    }

    /// [OPUS-4.8] (sq-vg3y) `sh:uniqueValuesFor` evaluation: for each value node V
    /// (with at least one value across the listed properties), report each OTHER
    /// target node of this shape that has the exact same per-property value sets as
    /// V (with that other node as `sh:value`, V's focus as `sh:focusNode`).
    fn eval_unique_values_for(
        &mut self,
        sid: usize,
        focus: &Term,
        values: &[Term],
        props: &[String],
        out: &mut Vec<ValidationResult>,
    ) {
        // The shape's target nodes (cached once per shape, reused across foci).
        let targets = self.unique_values_targets(sid);
        for v in values {
            let sig_v = value_signature(&self.data, v, props);
            if sig_v.iter().all(|s| s.is_empty()) {
                continue; // V has no values for any property — never a violation
            }
            for other in &targets {
                if other == v {
                    continue;
                }
                if value_signature(&self.data, other, props) == sig_v {
                    out.push(
                        self.result(
                            sid,
                            focus,
                            Some(other.clone()),
                            "UniqueValuesForConstraintComponent",
                            "Another target node has the same values for the unique \
                         properties"
                                .into(),
                        ),
                    );
                }
            }
        }
    }

    /// The (deduplicated) target nodes of shape `sid`, for `sh:uniqueValuesFor`.
    /// Memoised on first use so a shape's whole-target scan happens once even
    /// though the constraint is evaluated once per focus node.
    fn unique_values_targets(&mut self, sid: usize) -> Vec<Term> {
        if let Some(cached) = self.uvf_targets.get(&sid) {
            return cached.clone();
        }
        let targets = self.focus_nodes(sid);
        self.uvf_targets.insert(sid, targets.clone());
        targets
    }

    #[allow(clippy::too_many_arguments)] // one call site per range component; a struct would obscure it
    fn range(
        &mut self,
        sid: usize,
        focus: &Term,
        values: &[Term],
        bound: &Term,
        accept: &[Ordering],
        component: &str,
        out: &mut Vec<ValidationResult>,
    ) {
        for v in values {
            let ok = match cmp_terms(v, bound) {
                Some(ord) => accept.contains(&ord),
                None => false, // not comparable: constraint not satisfied
            };
            if !ok {
                out.push(self.result(
                    sid,
                    focus,
                    Some(v.clone()),
                    component,
                    format!("Value is out of range relative to {bound}"),
                ));
            }
        }
    }

    fn regex_for(&mut self, source: &str, flags: Option<&str>) -> Option<Rc<regex::Regex>> {
        let key = format!("{}\u{0}{}", source, flags.unwrap_or(""));
        self.regexes
            .entry(key)
            .or_insert_with(|| {
                regex::Regex::new(&compose_pattern(source, flags))
                    .ok()
                    .map(Rc::new)
            })
            .clone()
    }

    /// [OPUS-4.8] (sq-lz99x) Records a diagnostic for an uncompilable
    /// `sh:pattern` that was SKIPPED, de-duplicated per (shape, pattern, flags) so
    /// it is reported once regardless of how many focus nodes / values the shape
    /// has. The recorded message carries the `regex` crate's own compile error
    /// (recomputed here — only on the rare skip path), which names the unsupported
    /// construct (e.g. look-around).
    fn note_pattern_skipped(&mut self, sid: usize, source: &str, flags: Option<&str>) {
        let shape = self.shapes.shapes[sid].node.clone();
        let component = sh("PatternConstraintComponent");
        let needle = format!("\"{source}\"");
        let already = self.diagnostics.iter().any(|d| {
            d.source_shape == shape
                && d.source_component == component
                && d.message.contains(&needle)
        });
        if already {
            return;
        }
        let detail = match regex::Regex::new(&compose_pattern(source, flags)) {
            Err(e) => e.to_string(),
            // Unreachable: this is only called when `regex_for` returned None, but
            // be defensive rather than panic.
            Ok(_) => "the pattern did not compile".to_string(),
        };
        let flag_note = match flags {
            Some(f) if !f.is_empty() => format!(" (flags \"{f}\")"),
            _ => String::new(),
        };
        self.diagnostics.push(ShapeDiagnostic {
            source_shape: shape,
            source_component: component,
            message: format!(
                "sh:pattern \"{source}\"{flag_note} did not compile and was SKIPPED \
                 (the constraint reports no violations); the Rust regex engine has no \
                 lookahead/lookbehind, matching the XML Schema regex flavour the SHACL \
                 spec ties sh:pattern to. regex error: {detail}"
            ),
        });
    }
}

/// [OPUS-4.8] (sq-lz99x) Builds the final regex source for a `sh:pattern` /
/// `sh:flags` pair: the supported XPath-regex flags (`i`/`m`/`s`/`x`/`U`) are
/// turned into a leading inline `(?...)` group. Shared by [`Validator::regex_for`]
/// (compile + match) and [`Validator::note_pattern_skipped`] (recompute the error)
/// so both see byte-identical input.
///
/// [FABLE-5] (sq-8ro) XPath-regex divergences from the Rust `regex` crate are
/// translated here: the XPath F&O `q` flag (literal-pattern mode — only `i` keeps
/// its effect alongside it, mirroring the engine's `build_regex`), and the
/// XPath/XSD `\i` / `\I` / `\c` / `\C` name-character classes (see
/// [`translate_xpath_escapes`]).
fn compose_pattern(source: &str, flags: Option<&str>) -> String {
    let flags = flags.unwrap_or("");
    if flags.contains('q') {
        // Literal-pattern mode: every pattern character matches itself, which
        // also suppresses the other flags' metacharacters — per XPath, only `i`
        // is combinable with `q`.
        let escaped = regex::escape(source);
        return if flags.contains('i') {
            format!("(?i){}", escaped)
        } else {
            escaped
        };
    }
    let source = translate_xpath_escapes(source);
    let mut inline = String::new();
    for f in flags.chars() {
        if matches!(f, 'i' | 'm' | 's' | 'x' | 'U') {
            inline.push(f);
        }
    }
    if inline.is_empty() {
        source
    } else {
        format!("(?{inline}){source}")
    }
}

/// XML 1.0 `NameStartChar` as a Rust-regex character-class BODY (no brackets).
macro_rules! xml_name_start_char {
    () => {
        concat!(
            r":A-Z_a-z\x{C0}-\x{D6}\x{D8}-\x{F6}\x{F8}-\x{2FF}\x{370}-\x{37D}",
            r"\x{37F}-\x{1FFF}\x{200C}-\x{200D}\x{2070}-\x{218F}\x{2C00}-\x{2FEF}",
            r"\x{3001}-\x{D7FF}\x{F900}-\x{FDCF}\x{FDF0}-\x{FFFD}\x{10000}-\x{EFFFF}"
        )
    };
}
const XML_NAME_START_CHAR: &str = xml_name_start_char!();
/// XML 1.0 `NameChar` (`NameStartChar` plus `-` `.` digits #xB7 combining marks).
const XML_NAME_CHAR: &str = concat!(
    xml_name_start_char!(),
    r"\-.0-9\x{B7}\x{300}-\x{36F}\x{203F}-\x{2040}"
);

/// [FABLE-5] (sq-8ro) Translates the XPath/XSD multi-character escapes the Rust
/// `regex` crate does not recognise — `\i` (XML `NameStartChar`), `\c` (XML
/// `NameChar`) and their complements `\I` / `\C` — into bracketed character
/// classes. The replacement is a bracketed class even INSIDE `[...]`: the Rust
/// regex syntax accepts nested classes, so `[\c!]` becomes `[[…]!]` with the
/// correct union (and `[^\i]` the correct complement-of-union) semantics. Every
/// other escape pair passes through untouched, so `\\i` stays a literal
/// backslash + `i` and constructs the Rust engine genuinely lacks (look-around)
/// still land on the sq-lz99x skip-with-diagnostic path.
fn translate_xpath_escapes(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('i') => {
                out.push('[');
                out.push_str(XML_NAME_START_CHAR);
                out.push(']');
            }
            Some('I') => {
                out.push_str("[^");
                out.push_str(XML_NAME_START_CHAR);
                out.push(']');
            }
            Some('c') => {
                out.push('[');
                out.push_str(XML_NAME_CHAR);
                out.push(']');
            }
            Some('C') => {
                out.push_str("[^");
                out.push_str(XML_NAME_CHAR);
                out.push(']');
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            // A trailing lone backslash: pass through (an invalid regex either
            // way — it takes the skip-with-diagnostic path).
            None => out.push('\\'),
        }
    }
    out
}

/// [OPUS-4.8] (sq-f8gu) The DISTINCT members of `members` that appear more than
/// once (each duplicated value listed once, in first-seen order). Uses exact
/// `Term` equality — the same notion `view::dedup` uses for the `sh:uniqueMembers`
/// constraint itself — so the per-duplicate `sh:detail` sub-results agree exactly
/// with the top-level violation decision.
fn duplicate_members(members: &[Term]) -> Vec<Term> {
    let mut seen: rustc_hash::FxHashSet<&Term> = rustc_hash::FxHashSet::default();
    let mut reported: rustc_hash::FxHashSet<&Term> = rustc_hash::FxHashSet::default();
    let mut out = Vec::new();
    for m in members {
        if !seen.insert(m) && reported.insert(m) {
            out.push(m.clone());
        }
    }
    out
}

/// [OPUS-4.8] (sq-vg3y) True iff value node `v` matches the single `sh:nodeKind`
/// IRI `kind` (SHACL §4.6.1): an IRI matches sh:IRI / sh:BlankNodeOrIRI /
/// sh:IRIOrLiteral; a blank node matches sh:BlankNode / sh:BlankNodeOrIRI /
/// sh:BlankNodeOrLiteral; a literal matches sh:Literal / sh:BlankNodeOrLiteral /
/// sh:IRIOrLiteral. A list-form `sh:nodeKind` is the OR over its members.
fn node_kind_matches(v: &Term, kind: &str) -> bool {
    let local = kind.strip_prefix(crate::model::SH).unwrap_or(kind);
    match v {
        Term::NamedNode(_) => matches!(local, "IRI" | "BlankNodeOrIRI" | "IRIOrLiteral"),
        Term::BlankNode(_) => {
            matches!(local, "BlankNode" | "BlankNodeOrIRI" | "BlankNodeOrLiteral")
        }
        Term::Literal(_) => matches!(local, "Literal" | "BlankNodeOrLiteral" | "IRIOrLiteral"),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

/// [OPUS-4.8] (sq-vg3y) A default message for a set-valued `sh:datatype` /
/// `sh:nodeKind` constraint (`label` = "datatype" / "node kind"): the single IRI
/// reads naturally, the list form reads "any of <…> <…>".
fn iri_set_message(label: &str, iris: &[String]) -> String {
    match iris {
        [one] => format!("Value does not have {label} <{one}>"),
        many => {
            let joined = many
                .iter()
                .map(|i| format!("<{i}>"))
                .collect::<Vec<_>>()
                .join(" ");
            format!("Value does not have any of the allowed {label}s {joined}")
        }
    }
}

/// [OPUS-4.8] (sq-vg3y) The `sh:uniqueValuesFor` signature of a node: for each
/// listed property (IN ORDER), the SORTED, deduplicated set of its data-graph
/// values (rendered as terms). Two nodes share a key iff their signatures are
/// equal — exact term match, so `"04"^^xsd:byte` ≠ `"4"^^xsd:integer`.
fn value_signature(data: &GraphView, node: &Term, props: &[String]) -> Vec<Vec<String>> {
    props
        .iter()
        .map(|p| {
            let mut vs: Vec<String> = data
                .objects(node, p)
                .iter()
                .map(|t| t.to_string())
                .collect();
            vs.sort();
            vs.dedup();
            vs
        })
        .collect()
}

/// [OPUS-4.8] (sq-sx15d) A line-break character for `sh:singleLine`: line feed
/// (U+000A), carriage return (U+000D), form feed (U+000C) or vertical tab
/// (U+000B). The W3C `singleLine-001` test exercises all four.
fn is_line_break(c: char) -> bool {
    matches!(c, '\u{000A}' | '\u{000D}' | '\u{000C}' | '\u{000B}')
}

/// The string a value node contributes to sh:minLength/maxLength/pattern:
/// the IRI string or the literal's lexical form; blank nodes have none (fail).
fn string_repr(t: &Term) -> Option<String> {
    match t {
        Term::NamedNode(n) => Some(n.as_str().to_string()),
        Term::Literal(l) => Some(l.value().to_string()),
        _ => None,
    }
}

// ---- id-level twins of the value-check helpers ([FABLE-5] sq-7d3dj.33.4) ------
// Each mirrors its Term-level twin decision-for-decision over the dictionary's
// zero-copy record parts, so the id fast arms in `eval_component` report exactly
// what the Term-level arms would (differential-tested in `lib.rs::idfast_*`).

/// Id twin of the `Component::Datatype` value check: a literal whose
/// (well-formed) datatype is in the allowed set. An inline id IS a canonical
/// `xsd:integer` literal (always well-formed); a stored literal checks its
/// zero-copy record parts; IRIs / blank nodes / triple terms are not literals.
fn datatype_ok_id(g: &GraphView, id: Id, dts: &[String]) -> bool {
    if is_inline(id) {
        return dts.iter().any(|dt| dt == XSD_INTEGER);
    }
    match g.graph().dict.term_parts(id) {
        TermParts::Lit {
            value,
            datatype,
            lang,
        } => dts.iter().any(|dt| dt == datatype) && well_formed_parts(value, datatype, lang.is_some()),
        _ => false,
    }
}

/// Id twin of [`node_kind_matches`] (single `sh:nodeKind` IRI vs one value id).
fn node_kind_matches_id(g: &GraphView, id: Id, kind: &str) -> bool {
    let local = kind.strip_prefix(crate::model::SH).unwrap_or(kind);
    if is_inline(id) {
        // Inline ids are xsd:integer literals.
        return matches!(local, "Literal" | "BlankNodeOrLiteral" | "IRIOrLiteral");
    }
    match g.graph().dict.term_parts(id) {
        TermParts::Iri { .. } => matches!(local, "IRI" | "BlankNodeOrIRI" | "IRIOrLiteral"),
        TermParts::Lit { .. } => {
            matches!(local, "Literal" | "BlankNodeOrLiteral" | "IRIOrLiteral")
        }
        TermParts::Blank(_) => {
            matches!(local, "BlankNode" | "BlankNodeOrIRI" | "BlankNodeOrLiteral")
        }
        // A triple term matches no SHACL node kind — same as the Term-level
        // `node_kind_matches` catch-all.
        TermParts::Triple(_) => false,
    }
}

/// Id twin of [`string_repr`]: the string a `sh:pattern` / length constraint
/// checks — zero-copy for stored literals (the hot case) and for prefix-less
/// IRIs; blank nodes and triple terms have none (`None` ⇒ violation, matching
/// the Term-level arms).
fn string_repr_id<'g>(g: &GraphView<'g>, id: Id) -> Option<Cow<'g, str>> {
    if is_inline(id) {
        // Inline xsd:integer: its canonical lexical form (rare on this path).
        return match g.term_of(id) {
            Term::Literal(l) => Some(Cow::Owned(l.value().to_string())),
            _ => None,
        };
    }
    match g.graph().dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => Some(if prefix.is_empty() {
            Cow::Borrowed(suffix)
        } else {
            Cow::Owned(format!("{prefix}{suffix}"))
        }),
        TermParts::Lit { value, .. } => Some(Cow::Borrowed(value)),
        TermParts::Blank(_) | TermParts::Triple(_) => None,
    }
}

/// HTTP-style basic language-range match: exact (case-insensitive) or prefix
/// followed by `-` (so `en` matches `en-GB`); `*` matches any tag.
fn lang_matches(lang: &str, range: &str) -> bool {
    if range == "*" {
        return !lang.is_empty();
    }
    let lang = lang.to_ascii_lowercase();
    let range = range.to_ascii_lowercase();
    lang == range || (lang.starts_with(&range) && lang.as_bytes().get(range.len()) == Some(&b'-'))
}

/// SHACL "equal values" (sh:hasValue / sh:in): same term, or literals equal
/// under SPARQL `=` value semantics (numeric / boolean / date-time families).
fn equal_value(a: &Term, b: &Term) -> bool {
    if a == b {
        return true;
    }
    matches!(cmp_terms(a, b), Some(Ordering::Equal))
}

/// SPARQL-operator-style comparison of two terms; `None` when not comparable
/// (non-literals, or literals of incompatible / unordered types).
fn cmp_terms(a: &Term, b: &Term) -> Option<Ordering> {
    match (a, b) {
        (Term::Literal(la), Term::Literal(lb)) => cmp_literals(la, lb),
        _ => None,
    }
}

fn cmp_literals(a: &Literal, b: &Literal) -> Option<Ordering> {
    let (da, db) = (a.datatype(), b.datatype());
    let (da, db) = (da.as_str(), db.as_str());
    if is_numeric(da) && is_numeric(db) {
        let fa: f64 = a.value().parse().ok()?;
        let fb: f64 = b.value().parse().ok()?;
        return fa.partial_cmp(&fb);
    }
    if a.language().is_none()
        && b.language().is_none()
        && da == xsd("string")
        && db == xsd("string")
    {
        return Some(a.value().cmp(b.value()));
    }
    if da == xsd("boolean") && db == xsd("boolean") {
        let pa = parse_bool(a.value())?;
        let pb = parse_bool(b.value())?;
        return Some(pa.cmp(&pb));
    }
    if is_date_time(da) && is_date_time(db) {
        let (ta, tza) = timestamp(a.value(), da)?;
        let (tb, tzb) = timestamp(b.value(), db)?;
        if tza != tzb {
            // XSD's ±14h rule (XSD 1.1 pt.2 §3.3.7): an untimezoned value may
            // lie in any timezone from -14:00 to +14:00, so it denotes a 28h
            // window of instants around its as-if-UTC timestamp. Order against
            // a timezoned instant is DETERMINATE only when the whole window
            // falls strictly on one side; otherwise the pair is incomparable.
            const TZ_WINDOW_SECS: f64 = 14.0 * 3600.0;
            let (u, z) = if tza { (tb, ta) } else { (ta, tb) };
            let ord = if u + TZ_WINDOW_SECS < z {
                Ordering::Less
            } else if u - TZ_WINDOW_SECS > z {
                Ordering::Greater
            } else {
                return None;
            };
            return Some(if tza { ord.reverse() } else { ord });
        }
        return ta.partial_cmp(&tb);
    }
    None
}

fn xsd(local: &str) -> String {
    format!("{XSD}{local}")
}

fn is_numeric(dt: &str) -> bool {
    let Some(local) = dt.strip_prefix(XSD) else {
        return false;
    };
    matches!(
        local,
        "integer"
            | "decimal"
            | "double"
            | "float"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "positiveInteger"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
    )
}

fn is_date_time(dt: &str) -> bool {
    let Some(local) = dt.strip_prefix(XSD) else {
        return false;
    };
    matches!(local, "dateTime" | "date" | "time" | "dateTimeStamp")
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Seconds-since-epoch of an XSD date/time lexical value plus whether it
/// carries an explicit timezone (an untimezoned value is normalised AS IF UTC;
/// against a timezoned value it is ordered via the ±14h determinate window —
/// see `cmp_literals`).
fn timestamp(value: &str, dt: &str) -> Option<(f64, bool)> {
    let local = dt.strip_prefix(XSD)?;
    let v = value.trim();
    let (date_part, rest) = match local {
        "time" => ("2000-01-01", v),
        "date" => {
            // The date may carry a timezone suffix directly.
            let split = date_tz_split(v);
            (&v[..split], &v[split..])
        }
        _ => {
            let i = v.find('T')?;
            (&v[..i], &v[i + 1..])
        }
    };
    let (y, m, d) = parse_date(date_part)?;
    let (mut rest, mut secs) = (rest, 0.0f64);
    if local != "date" {
        let tz_at = rest
            .char_indices()
            .find(|(i, c)| *c == 'Z' || ((*c == '+' || *c == '-') && *i > 0))
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let time_part = &rest[..tz_at];
        rest = &rest[tz_at..];
        // [OPUS-4.8] xsd:dateTime / dateTimeStamp REQUIRE a time after the 'T'; the old code left
        // secs=0 for an empty time part, accepting "2024-01-01T" / "2024-01-01TZ". Reject it. 1616.
        if time_part.is_empty() {
            return None;
        }
        let mut it = time_part.split(':');
        let h: f64 = parse_time_field(it.next()?, 2)?;
        let mi: f64 = parse_time_field(it.next()?, 2)?;
        // Seconds may carry a fractional part (ss.sss…); validate the integral part shape.
        let s = parse_seconds_field(it.next().unwrap_or("00"))?;
        if it.next().is_some() {
            return None;
        }
        // [OPUS-4.8] Range-check the fields: hour 0..=24 (24 only as 24:00:00), minute/second
        // 0..=59. The old code accepted out-of-range values like 99:99:99. 1616.
        if h > 24.0 || mi > 59.0 || s >= 60.0 || (h == 24.0 && (mi != 0.0 || s != 0.0)) {
            return None;
        }
        secs = h * 3600.0 + mi * 60.0 + s;
    }
    let tz_offset_secs = parse_tz(rest)?;
    let has_tz = !rest.is_empty();
    // [OPUS-4.8] xsd:dateTimeStamp requires an explicit timezone; a timezone-less value is NOT in
    // its lexical space (this is the sole lexical difference from xsd:dateTime). See review 1616.
    if local == "dateTimeStamp" && !has_tz {
        return None;
    }
    Some((
        days_from_civil(y, m, d) as f64 * 86_400.0 + secs - tz_offset_secs,
        has_tz,
    ))
}

/// The byte index where a date lexical's optional timezone suffix starts.
fn date_tz_split(v: &str) -> usize {
    if v.ends_with('Z') {
        return v.len() - 1;
    }
    // ±hh:mm — but the leading sign of a negative year is not a timezone.
    if v.len() > 6 {
        let tail = &v[v.len() - 6..];
        if (tail.starts_with('+') || tail.starts_with('-')) && tail.as_bytes()[3] == b':' {
            return v.len() - 6;
        }
    }
    v.len()
}

fn parse_tz(s: &str) -> Option<f64> {
    if s.is_empty() || s == "Z" {
        return Some(0.0);
    }
    let (sign, rest) = match s.as_bytes()[0] {
        b'+' => (1.0, &s[1..]),
        b'-' => (-1.0, &s[1..]),
        _ => return None,
    };
    let mut it = rest.split(':');
    // [OPUS-4.8] Enforce the fixed ±hh:mm shape and the XSD timezone range (≤ ±14:00). The old
    // bare f64 parse accepted "+9:5", "+99:99", etc. 1616.
    let h = parse_time_field(it.next()?, 2)?;
    let m = parse_time_field(it.next()?, 2)?;
    if it.next().is_some() || h > 14.0 || m > 59.0 || (h == 14.0 && m != 0.0) {
        return None;
    }
    Some(sign * (h * 3600.0 + m * 60.0))
}

/// [OPUS-4.8] Parse a fixed-width all-digit time field (hh / mm) of exactly `width` digits.
/// Rejects signs, whitespace, and wrong widths the bare f64 parse would otherwise accept. 1616.
fn parse_time_field(s: &str, width: usize) -> Option<f64> {
    if s.len() != width || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// [OPUS-4.8] Parse the seconds field `ss` or `ss.sss…`: two leading digits, an optional fraction
/// of one-or-more digits. Returns the numeric value (range-checked by the caller). 1616.
fn parse_seconds_field(s: &str) -> Option<f64> {
    let (int_part, frac_ok) = match s.split_once('.') {
        Some((i, f)) => (i, !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit())),
        None => (s, true),
    };
    if !frac_ok || int_part.len() != 2 || !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn parse_date(s: &str) -> Option<(i64, u32, u32)> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mut it = s.split('-');
    let ys = it.next()?;
    let ms = it.next()?;
    let ds = it.next()?;
    // [OPUS-4.8] XSD lexical space is fixed-width: yyyy (≥4 digits), mm, dd, all ASCII digits.
    // The plain f64/u32 parse below accepts e.g. "+04" or whitespace; enforce the shape. 1616.
    if ys.len() < 4
        || !ys.bytes().all(|b| b.is_ascii_digit())
        || ms.len() != 2
        || !ms.bytes().all(|b| b.is_ascii_digit())
        || ds.len() != 2
        || !ds.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let y: i64 = ys.parse().ok()?;
    let m: u32 = ms.parse().ok()?;
    let d: u32 = ds.parse().ok()?;
    // [OPUS-4.8] Reject impossible calendar days (e.g. 2024-02-30, 2023-02-29, 2024-04-31): the
    // old check only rejected d>31, so February 30th and April 31st passed as well-formed. 1616.
    if it.next().is_some() || m == 0 || m > 12 || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    Some((if neg { -y } else { y }, m, d))
}

/// [OPUS-4.8] Days in a (proleptic-Gregorian) month, accounting for leap years. The `y` is the
/// signed calendar year as written (sign applied by the caller); leap rules are sign-symmetric for
/// the purpose of day-count here. See review 1616.
fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days since 1970-01-01 of a proleptic-Gregorian civil date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Is the literal's lexical form in the lexical space of its (known XSD)
/// datatype? Unknown datatypes are accepted (no lexical check possible).
fn well_formed(l: &Literal) -> bool {
    well_formed_parts(l.value(), l.datatype().as_str(), l.language().is_some())
}

/// [FABLE-5] (sq-7d3dj.33.4) The body of [`well_formed`] over raw literal parts,
/// so the id-level datatype arm can check a stored literal's zero-copy record
/// (`Dict::term_parts`) without constructing an `oxrdf::Literal`. `has_lang`
/// covers both a plain language tag and a `lang--dir` RDF 1.2 slot (only
/// presence matters here, exactly as `l.language().is_some()` did).
fn well_formed_parts(v: &str, dt: &str, has_lang: bool) -> bool {
    let Some(local) = dt.strip_prefix(XSD) else {
        return dt != RDF_LANG_STRING || has_lang;
    };
    match local {
        "integer" => parse_integer(v).is_some(),
        "long" => in_range(v, i64::MIN as i128, i64::MAX as i128),
        "int" => in_range(v, i32::MIN as i128, i32::MAX as i128),
        "short" => in_range(v, i16::MIN as i128, i16::MAX as i128),
        "byte" => in_range(v, i8::MIN as i128, i8::MAX as i128),
        "nonNegativeInteger" => parse_integer(v).map(|n| n >= 0).unwrap_or(false),
        "positiveInteger" => parse_integer(v).map(|n| n > 0).unwrap_or(false),
        "nonPositiveInteger" => parse_integer(v).map(|n| n <= 0).unwrap_or(false),
        "negativeInteger" => parse_integer(v).map(|n| n < 0).unwrap_or(false),
        "unsignedLong" => in_range(v, 0, u64::MAX as i128),
        "unsignedInt" => in_range(v, 0, u32::MAX as i128),
        "unsignedShort" => in_range(v, 0, u16::MAX as i128),
        "unsignedByte" => in_range(v, 0, u8::MAX as i128),
        "decimal" => is_decimal(v),
        "float" | "double" => is_float(v),
        "boolean" => parse_bool(v).is_some(),
        "dateTime" | "dateTimeStamp" | "date" | "time" => timestamp(v, dt).is_some(),
        _ => true,
    }
}

fn parse_integer(v: &str) -> Option<i128> {
    let body = v.strip_prefix(['+', '-']).unwrap_or(v);
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse().ok()
}

fn in_range(v: &str, lo: i128, hi: i128) -> bool {
    parse_integer(v)
        .map(|n| n >= lo && n <= hi)
        .unwrap_or(false)
}

fn is_decimal(v: &str) -> bool {
    let body = v.strip_prefix(['+', '-']).unwrap_or(v);
    if body.is_empty() {
        return false;
    }
    let mut dots = 0;
    let mut digits = 0;
    for b in body.bytes() {
        match b {
            b'.' => dots += 1,
            b'0'..=b'9' => digits += 1,
            _ => return false,
        }
    }
    dots <= 1 && digits > 0
}

fn is_float(v: &str) -> bool {
    if matches!(v, "NaN" | "INF" | "-INF" | "+INF") {
        return true;
    }
    let (mantissa, exp) = match v.find(['e', 'E']) {
        Some(i) => (&v[..i], Some(&v[i + 1..])),
        None => (v, None),
    };
    if !is_decimal(mantissa) {
        return false;
    }
    match exp {
        None => true,
        Some(e) => parse_integer(e).is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xsd_well_formedness() {
        let lit = |v: &str, dt: &str| {
            Literal::new_typed_literal(v, oxrdf::NamedNode::new(xsd(dt)).unwrap())
        };
        assert!(well_formed(&lit("42", "integer")));
        assert!(!well_formed(&lit("aaa", "integer")));
        assert!(well_formed(&lit("-3.14", "decimal")));
        assert!(!well_formed(&lit("3.1.4", "decimal")));
        assert!(well_formed(&lit("1.0e6", "double")));
        assert!(well_formed(&lit("INF", "float")));
        assert!(well_formed(&lit("2024-02-29", "date")));
        assert!(!well_formed(&lit("2024-13-01", "date")));
        assert!(well_formed(&lit("2024-02-29T12:00:00Z", "dateTime")));

        // [OPUS-4.8] Stricter XSD date/time lexical validation (review 1616):
        // impossible calendar days are rejected.
        assert!(
            !well_formed(&lit("2023-02-29", "date")),
            "Feb 29 in a non-leap year"
        );
        assert!(
            !well_formed(&lit("2024-02-30", "date")),
            "Feb 30 never exists"
        );
        assert!(
            !well_formed(&lit("2024-04-31", "date")),
            "April has 30 days"
        );
        assert!(well_formed(&lit("2024-04-30", "date")));
        assert!(
            well_formed(&lit("2000-02-29", "date")),
            "2000 is a leap year (÷400)"
        );
        assert!(
            !well_formed(&lit("1900-02-29", "date")),
            "1900 is not (÷100, ¬÷400)"
        );
        // dateTime requires a time component after 'T'.
        assert!(
            !well_formed(&lit("2024-01-01T", "dateTime")),
            "missing time part"
        );
        assert!(
            !well_formed(&lit("2024-01-01TZ", "dateTime")),
            "empty time part"
        );
        // Out-of-range hour/minute/second.
        assert!(
            !well_formed(&lit("2024-01-01T25:00:00", "dateTime")),
            "hour 25"
        );
        assert!(
            !well_formed(&lit("2024-01-01T12:60:00", "dateTime")),
            "minute 60"
        );
        assert!(
            !well_formed(&lit("2024-01-01T12:00:61", "dateTime")),
            "second 61"
        );
        assert!(
            well_formed(&lit("2024-01-01T24:00:00", "dateTime")),
            "24:00:00 is legal"
        );
        assert!(
            !well_formed(&lit("2024-01-01T24:00:01", "dateTime")),
            "24:00:01 is not"
        );
        assert!(
            well_formed(&lit("2024-01-01T12:30:45.5", "dateTime")),
            "fractional seconds"
        );
        // Out-of-range / malformed timezone.
        assert!(
            !well_formed(&lit("2024-01-01T12:00:00+15:00", "dateTime")),
            "tz > ±14:00"
        );
        assert!(well_formed(&lit("2024-01-01T12:00:00+14:00", "dateTime")));
        // dateTimeStamp requires an explicit timezone; dateTime does not.
        assert!(
            well_formed(&lit("2024-01-01T12:00:00", "dateTime")),
            "tz-less dateTime ok"
        );
        assert!(
            !well_formed(&lit("2024-01-01T12:00:00", "dateTimeStamp")),
            "tz-less dateTimeStamp is NOT in its lexical space"
        );
        assert!(well_formed(&lit("2024-01-01T12:00:00Z", "dateTimeStamp")));
    }

    #[test]
    fn date_time_ordering() {
        let cmp = |a: &str, b: &str, dt: &str| {
            timestamp(a, &xsd(dt))
                .unwrap()
                .0
                .partial_cmp(&timestamp(b, &xsd(dt)).unwrap().0)
                .unwrap()
        };
        assert_eq!(cmp("2024-01-01", "2024-01-02", "date"), Ordering::Less);
        assert_eq!(
            cmp(
                "2024-01-01T10:00:00+01:00",
                "2024-01-01T09:00:00Z",
                "dateTime"
            ),
            Ordering::Equal
        );
        // Mixed timezone presence: XSD's ±14h rule. An untimezoned value may
        // lie in any timezone from -14:00 to +14:00, so pairs closer than 14h
        // are INDETERMINATE (None); pairs farther apart are determinate.
        let lit = |v: &str| {
            Literal::new_typed_literal(v, oxrdf::NamedNode::new(xsd("dateTime")).unwrap())
        };
        assert_eq!(
            cmp_literals(
                &lit("2002-10-10T12:00:00-05:00"),
                &lit("2002-10-10T12:00:00")
            ),
            None,
            "inside the ±14h window -> indeterminate"
        );
        assert_eq!(
            cmp_literals(&lit("2000-01-12T12:00:00"), &lit("2000-01-14T12:00:00Z")),
            Some(Ordering::Less),
            "whole window below the instant -> determinate <"
        );
        assert_eq!(
            cmp_literals(&lit("2000-01-14T12:00:00Z"), &lit("2000-01-12T12:00:00")),
            Some(Ordering::Greater),
            "same pair, flipped operands"
        );
        assert_eq!(
            cmp_literals(&lit("2000-01-16T12:00:00"), &lit("2000-01-14T12:00:00Z")),
            Some(Ordering::Greater),
            "whole window above the instant -> determinate >"
        );
        assert_eq!(
            cmp_literals(&lit("2000-01-01T00:00:00"), &lit("2000-01-01T14:00:00Z")),
            None,
            "exactly 14h apart: the -14:00 reading coincides -> indeterminate"
        );
        assert_eq!(
            cmp_literals(&lit("2000-01-01T00:00:00"), &lit("2000-01-01T14:00:01Z")),
            Some(Ordering::Less),
            "one second past the window edge -> determinate <"
        );
    }
}
