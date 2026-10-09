//! Framing Algorithm (JSON-LD 1.1 Framing §3) — matching, value patterns, `@embed`.
//!
//! [FABLE-5] (sq-oy1f.29) The document-level **Framing** operation over the native
//! expanded-document pipeline (design record §3.1–3.2): the input is expanded
//! ([`crate::expand::expand`]), the frame is expanded in `frameExpansion` mode, the input's
//! node map is generated ([`crate::node_map::generate_node_map`]) and — unless
//! `frameDefault` — merged into an `@merged` graph, the recursive **frame matching**
//! algorithm selects and reshapes node objects, and the framed expanded output is compacted
//! against the frame's `@context` ([`crate::compact::compact_expanded`]). This replaces the
//! RDF-first framer's term-level matching with the spec's document-level matching:
//!
//! - **node patterns** — `@id` / `@type` / property duck-typing with `@requireAll`,
//!   wildcard (`{}`) and match-none (`[]`) forms;
//! - **value patterns** — `@value` / `@type` / `@language` alternative arrays over value
//!   objects (§2.2 Value Pattern Matching);
//! - **`@explicit` / `@default`** — explicit-inclusion pruning and `@default` fill (with
//!   `@omitDefault`), threaded through `@preserve` so compaction keeps the fills;
//! - **named-graph framing** — a matched subject that names a graph recurses into that
//!   graph under `@graph` (framing runs over the `@merged` graph by default, §4.1);
//! - **`@list` re-emit** — list values are re-framed entry-wise (node entries recurse,
//!   values copy);
//! - **blank-node `@embed`** — `@once` (default) / `@always` / `@never` embedding with
//!   circular-reference guards, plus `pruneBlankNodeIdentifiers` (1.1 default) removing
//!   `@id` from blank nodes referenced only once.
//!
//! Framing validation errors are raised as spec error codes: an out-of-range `@embed`
//! value raises `invalid @embed value`; a blank-node `@id`/`@type` pattern (or a
//! non-object frame) raises `invalid frame`.
//!
//! ## Documented fallbacks (design record §11)
//!
//! - **`@embed: @link`** (identity-preserving embedding) is treated as `@once` until a
//!   consumer needs output-tree object identity.
//! - **`@embed: @last`** (the JSON-LD 1.0 flag) embeds every occurrence, then a
//!   post-pass demotes all but the LAST embed to node references — behaviourally the
//!   reference processors' remove-embed mutation.
//!
//! Spec: <https://www.w3.org/TR/json-ld11-framing/>.

use crate::compact::compact_expanded;
use crate::error::{JsonLdError, JsonLdErrorCode as E};
use crate::expand::expand;
use crate::json::Json;
use crate::loader::DocumentLoader;
use crate::node_map::{generate_node_map, BlankNodeIssuer};
use crate::options::{JsonLdOptions, ProcessingMode};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Framing-only processing options (JSON-LD 1.1 Framing §4.1), kept separate from
/// [`JsonLdOptions`] (which already carries the frame-object flag defaults `embed` /
/// `explicit` / `omitDefault` / `requireAll`). Construct with [`FrameOptions::default`];
/// `None` fields resolve per the active [`ProcessingMode`].
///
/// [FABLE-5] (sq-oy1f.29)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameOptions {
    /// `omitGraph` — omit the `@graph` wrapper when the framed output holds one node.
    /// `None` (the default) resolves to the spec default: `true` under `json-ld-1.1`,
    /// `false` under `json-ld-1.0`.
    pub omit_graph: Option<bool>,
    /// `pruneBlankNodeIdentifiers` — remove `@id` from blank nodes referenced only once
    /// in the framed output. `None` (the default) resolves to the spec default: `true`
    /// under `json-ld-1.1`, `false` under `json-ld-1.0`.
    pub prune_blank_node_identifiers: Option<bool>,
    /// `frameDefault` — frame the default graph instead of the merged graph
    /// (default `false`: framing operates over `@merged`, §4.1 step 4).
    pub frame_default: bool,
}

impl FrameOptions {
    /// The effective `omitGraph` value under `mode`.
    fn omit_graph(&self, mode: ProcessingMode) -> bool {
        self.omit_graph
            .unwrap_or(mode == ProcessingMode::JsonLd11)
    }

    /// The effective `pruneBlankNodeIdentifiers` value under `mode`.
    fn prune(&self, mode: ProcessingMode) -> bool {
        self.prune_blank_node_identifiers
            .unwrap_or(mode == ProcessingMode::JsonLd11)
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// **Framing** (the `frame()` API operation, JSON-LD 1.1 Framing §4.1). Expands `input`
/// (via [`expand`]), expands `frame_doc` in `frameExpansion` mode, frames the expanded
/// input against the expanded frame (see [`frame_expanded`]), compacts the framed output
/// against the frame's `@context`, and applies the framing post-processing (`@preserve`
/// unwrap, `@null` → `null`, `omitGraph` shaping).
///
/// Remote `@context` / `@import` references are dereferenced only through `loader`
/// (deny-by-default via [`NoopLoader`](crate::loader::NoopLoader)). Returns the first spec
/// [`JsonLdError`] raised by expansion, context processing, compaction, or frame
/// validation (`invalid frame`, `invalid @embed value`).
///
/// [FABLE-5] (sq-oy1f.29)
pub fn frame(
    input: &Json,
    frame_doc: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    crate::context::budget::with_budget(|| frame_inner(input, frame_doc, options, frame_options, loader))
}

fn frame_inner(
    input: &Json,
    frame_doc: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    let framed = frame_match(input, frame_doc, options, frame_options, loader)?;
    let ctx_value = frame_doc.get("@context").cloned().unwrap_or_default();
    compact_framed(&framed, &ctx_value, options, frame_options, loader)
}

/// The matching half of [`frame`] (§4.1 steps 2–7): expands `input` and `frame_doc` and
/// returns the framed **expanded** output of [`frame_expanded`], `@preserve` fills
/// included, ready for [`compact_framed`].
pub fn frame_match(
    input: &Json,
    frame_doc: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    crate::context::budget::with_budget(|| {
        // §4.1 step 2: expand the input (ordinary mode).
        let mut in_opts = options.clone();
        in_opts.frame_expansion = false;
        let expanded_input = expand(input, &in_opts, loader)?;
        let input_bytes = json_bytes(&expanded_input);
        frame_match_inner(input_bytes, |frame_default| build_graph_maps(&expanded_input, frame_default), frame_doc, options, frame_options, loader)
    })
}

/// [`frame_match`] over the output of fromRdf, taken by value and not re-expanded.
/// fromRdf already emits one node object per subject, with each named graph's nodes
/// under its node's `@graph`, so the graph map is regrouped from it directly instead of
/// running node map generation, and its blank-node labels are kept.
pub fn frame_match_from_rdf(
    from_rdf: Json,
    frame_doc: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    crate::context::budget::with_budget(|| {
        let input_bytes = json_bytes(&from_rdf);
        frame_match_inner(input_bytes, |frame_default| graph_maps_from_rdf(from_rdf, frame_default), frame_doc, options, frame_options, loader)
    })
}

fn frame_match_inner(
    input_bytes: usize,
    graph_maps: impl FnOnce(bool) -> GraphMaps,
    frame_doc: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    // §4.1 step 3: expand the frame (frameExpansion).
    let expanded_frame = expanded_frame(frame_doc, options, loader)?;

    // §4.1 step 4: a top-level `@graph` entry in the frame DOCUMENT selects the
    // default graph instead of the merged graph (the suite's t0047 posture — the
    // reference processors read the unexpanded frame for this).
    let mut fopts = frame_options.clone();
    if frame_doc.get("@graph").is_some() {
        fopts.frame_default = true;
    }

    // §4.1 steps 4–7: frame the expanded input (returns the pruned expanded output).
    let maps = graph_maps(fopts.frame_default);
    frame_maps(&maps, input_bytes, &expanded_frame, options, &fopts)
}

thread_local! {
    /// The last frame expanded on this thread, its options and its expansion, so callers
    /// framing many documents with one frame expand it once.
    static LAST_FRAME: RefCell<Option<(Json, JsonLdOptions, Rc<Json>)>> = const { RefCell::new(None) };
}

/// `frame_doc` expanded in `frameExpansion` mode, reusing [`LAST_FRAME`] when it matches.
/// Only frames whose contexts cannot reach the loader are cached, since a loader may
/// resolve the same IRI differently between calls.
fn expanded_frame(frame_doc: &Json, options: &JsonLdOptions, loader: &dyn DocumentLoader) -> Result<Rc<Json>, JsonLdError> {
    let cacheable = contexts_self_contained(frame_doc);
    if cacheable {
        let hit = LAST_FRAME.with(|last| {
            last.borrow()
                .as_ref()
                .filter(|(f, o, _)| f == frame_doc && o == options)
                .map(|(_, _, e)| Rc::clone(e))
        });
        if let Some(expanded) = hit {
            return Ok(expanded);
        }
    }
    let mut fr_opts = options.clone();
    fr_opts.frame_expansion = true;
    let expanded = Rc::new(expand(frame_doc, &fr_opts, loader)?);
    if cacheable {
        LAST_FRAME.with(|last| {
            *last.borrow_mut() = Some((frame_doc.clone(), options.clone(), Rc::clone(&expanded)));
        });
    }
    Ok(expanded)
}

/// True iff every `@context` in `frame` is [`crate::compact::self_contained`].
fn contexts_self_contained(frame: &Json) -> bool {
    let mut stack = vec![frame];
    while let Some(j) = stack.pop() {
        match j {
            Json::Arr(items) => stack.extend(items),
            Json::Obj(members) => {
                for (k, v) in members {
                    if k == "@context" {
                        if !crate::compact::self_contained(v) {
                            return false;
                        }
                    } else {
                        stack.push(v);
                    }
                }
            }
            _ => {}
        }
    }
    true
}

/// The output half of [`frame`] (§4.1 step 8): compacts `framed` (from [`frame_match`])
/// against `context`, unwraps `@preserve` (with `@null` → `null`) and applies the
/// `omitGraph` shaping. The same match can be compacted against more than one context.
pub fn compact_framed(
    framed: &Json,
    context: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    let framed_len = match framed {
        Json::Arr(items) => items.len(),
        _ => 1,
    };
    let compacted = compact_expanded(framed, context, options, loader)?;
    let mut result = cleanup_preserve(compacted, options.compact_arrays);
    if !frame_options.omit_graph(options.processing_mode) {
        result = ensure_graph_envelope(result, context, framed_len);
    }
    Ok(result)
}

/// **Frame matching** over already-expanded documents (§4.1 steps 4–7): generates the
/// node map of `expanded_input`, adds the `@merged` graph (unless
/// [`FrameOptions::frame_default`]), runs the recursive frame-matching algorithm for
/// `expanded_frame` over every top-level subject, and applies
/// `pruneBlankNodeIdentifiers`. Returns the framed **expanded** output (an array of
/// framed node objects, `@default`-fill values wrapped under `@preserve`), the input
/// [`frame`] compacts for the caller-facing result.
///
/// `expanded_frame` must be the output of [`expand`] with
/// [`JsonLdOptions::frame_expansion`] set (an array holding one frame object; an empty
/// array is treated as the match-everything `{}` frame).
///
/// [FABLE-5] (sq-oy1f.29)
pub fn frame_expanded(
    expanded_input: &Json,
    expanded_frame: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
) -> Result<Json, JsonLdError> {
    // §4.1 step 4: the graph map (+ @merged unless frameDefault).
    let maps = build_graph_maps(expanded_input, frame_options.frame_default);
    frame_maps(&maps, json_bytes(expanded_input), expanded_frame, options, frame_options)
}

/// [`frame_expanded`] over the input's graph maps; `input_bytes` is the input's
/// [`json_bytes`], which sets the output bound.
fn frame_maps(
    maps: &GraphMaps,
    input_bytes: usize,
    expanded_frame: &Json,
    options: &JsonLdOptions,
    frame_options: &FrameOptions,
) -> Result<Json, JsonLdError> {
    // Normalise the expanded frame to a one-object array (an empty frame `{}` expands to
    // `[]`; re-materialise the match-everything object).
    let frame_arr = match expanded_frame {
        Json::Arr(items) if items.is_empty() => Cow::Owned(Json::Arr(vec![Json::obj()])),
        Json::Arr(_) => Cow::Borrowed(expanded_frame),
        other => Cow::Owned(Json::Arr(vec![other.clone()])),
    };
    let graph = if frame_options.frame_default {
        "@default"
    } else {
        "@merged"
    };

    let mut st = FState {
        options,
        graph: graph.to_string(),
        subject_stack: Vec::new(),
        unique_embeds: BTreeMap::new(),
        bnode_counts: BTreeMap::new(),
        last_ids: BTreeSet::new(),
        budget: Budget::new(output_bound(input_bytes, &frame_arr)),
        reverse: BTreeMap::new(),
        implicit_frames: Default::default(),
    };

    // §4.1 steps 5–6: match the frame over every subject of the active graph.
    let subjects: Vec<String> = maps
        .graphs
        .get(graph)
        .map(|g| g.keys().cloned().collect())
        .unwrap_or_default();
    let mut framed = Out::array(&mut st.budget)?;
    match_frame(maps, &mut st, &subjects, &frame_arr, &mut framed, None, false)?;
    let mut framed = framed.into_json();

    // `@embed: @last`: every occurrence embedded above; keep only the LAST embed of
    // each such id per top-level match (earlier ones demote to node references) —
    // the reference processors' remove-embed, without shared-pointer mutation.
    if !st.last_ids.is_empty() {
        if let Json::Arr(top) = &mut framed {
            for element in top {
                // Only ids embedded more than once in this element need a walk, and each
                // walk is charged to the output bound, so the pass stays bounded.
                let mut embeds = BTreeMap::new();
                count_all_embeds(element, &st.last_ids, &mut embeds);
                for (id, n) in embeds {
                    if n > 1 {
                        st.emit(json_bytes(element))?;
                        demote_all_but_last_embed(element, &id);
                    }
                }
            }
        }
    }

    // §4.1 step 7: prune blank-node identifiers referenced only once.
    if frame_options.prune(options.processing_mode) {
        let to_clear: BTreeSet<String> = st
            .bnode_counts
            .iter()
            .filter(|(_, n)| **n == 1)
            .map(|(id, _)| id.clone())
            .collect();
        if !to_clear.is_empty() {
            prune_bnode_ids(&mut framed, &to_clear);
        }
    }
    Ok(framed)
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

/// One graph's subjects: `@id` → node object, ordered (framing iterates subjects in
/// lexicographical order).
type Subjects = BTreeMap<String, Json>;

/// The immutable graph maps framing reads: graph name → its subjects (always contains
/// `@default`; contains `@merged` unless `frameDefault`).
struct GraphMaps {
    graphs: BTreeMap<String, Rc<Subjects>>,
}

/// The mutable framing state (§4.1 "framing state").
struct FState<'a> {
    options: &'a JsonLdOptions,
    /// The active graph name.
    graph: String,
    /// (graph, id) pairs currently being embedded — the circular-reference guard.
    subject_stack: Vec<(String, String)>,
    /// graph name → ids already embedded under it (`@once` bookkeeping).
    unique_embeds: BTreeMap<String, BTreeSet<String>>,
    /// ids embedded under an `@embed: @last` frame (each occurrence embeds; the
    /// post-pass keeps only the LAST embed per top-level match).
    last_ids: BTreeSet<String>,
    /// blank-node id → number of framed output objects/`@type` usages (for pruning).
    bnode_counts: BTreeMap<String, usize>,
    /// What this call may still allocate for its output and bookkeeping ([`Budget`]).
    budget: Budget,
    /// (graph, reverse property) → referenced id → referring subjects ([`FState::referrers`]).
    reverse: BTreeMap<(String, String), BTreeMap<String, Vec<String>>>,
    /// The implicit frames built so far, by flags ([`FState::implicit`]).
    implicit_frames: [Option<Rc<Json>>; 16],
}

/// Bytes a framing call may allocate for its output per byte of its input and frame,
/// beyond [`OUTPUT_FLOOR`]. `@once` embeds each subject once per top-level match, so
/// legitimate output can be quadratic in the input; `@always` over shared subjects can be
/// exponential.
const OUTPUT_PER_INPUT: usize = 32;
/// Bytes any framing call may allocate for its output, whatever the input's size.
const OUTPUT_FLOOR: usize = 1 << 24;
/// What one emitted object costs beyond its strings (the map, its first array and slack).
const NODE_BYTES: usize = 32;

/// The output bound of a framing call over `input` with `frame` (both expanded).
fn output_bound(input_bytes: usize, frame: &Json) -> usize {
    input_bytes
        .saturating_add(json_bytes(frame))
        .saturating_mul(OUTPUT_PER_INPUT)
        .saturating_add(OUTPUT_FLOOR)
}

impl FState<'_> {
    /// The implicit sub-frame carrying `flags` forward ([`implicit_frame`]), built once
    /// per distinct set of flags.
    fn implicit(&mut self, flags: &Flags) -> Result<Rc<Json>, JsonLdError> {
        let slot = flags.embed as usize * 4 + usize::from(flags.explicit) * 2 + usize::from(flags.require_all);
        if let Some(frame) = &self.implicit_frames[slot] {
            return Ok(Rc::clone(frame));
        }
        let frame = Rc::new(implicit_frame(flags));
        self.emit(json_bytes(&frame))?;
        self.implicit_frames[slot] = Some(Rc::clone(&frame));
        Ok(frame)
    }

    /// The subject ids of `graph`, in order.
    fn subject_ids(&mut self, graph: &Subjects) -> Result<Vec<String>, JsonLdError> {
        self.emit(graph.keys().map(|k| k.len() + 8).sum())?;
        Ok(graph.keys().cloned().collect())
    }

    /// The subjects of the active graph whose `property` references `id`, in order. The
    /// graph is scanned once per property and indexed, so `@reverse` framing stays linear.
    fn referrers(&mut self, maps: &GraphMaps, property: &str, id: &str) -> Result<Vec<String>, JsonLdError> {
        let key = (self.graph.clone(), property.to_string());
        if !self.reverse.contains_key(&key) {
            let mut index: BTreeMap<String, Vec<String>> = BTreeMap::new();
            if let Some(g) = maps.graphs.get(&self.graph) {
                self.emit(g.len() * 8)?;
                for (sid, node) in g.iter() {
                    for v in node.get(property).map(as_slice).unwrap_or_default() {
                        if let Some(target) = v.get("@id").and_then(Json::as_str) {
                            let list = index.entry(target.to_string()).or_default();
                            if list.last() != Some(sid) {
                                self.emit(sid.len() + target.len() + 16)?;
                                list.push(sid.clone());
                            }
                        }
                    }
                }
            }
            self.reverse.insert(key.clone(), index);
        }
        let found = self.reverse[&key].get(id).cloned().unwrap_or_default();
        self.emit(found.iter().map(|s| s.len() + 8).sum())?;
        Ok(found)
    }

    /// Charges `n` bytes of bookkeeping (frame copies, id lists) to the call's budget.
    fn emit(&mut self, n: usize) -> Result<(), JsonLdError> {
        self.budget.charge(n)
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

use output::{Budget, Out};

/// Framing's output and the budget that pays for it. [`Out`]'s contents are private to
/// this module, so [`match_frame`] and its helpers can build output only through the
/// constructors and mutators here, and each one charges the [`Budget`] before it
/// allocates: keys, copied values, nodes, lists, defaults and the arrays that hold them.
mod output {
    use super::{as_slice, json_bytes, output_member_mut, Json, JsonLdError, E, NODE_BYTES};

    /// What a framing call may still allocate. Charges fail with `context overflow` once
    /// it is spent, so the output (and the compaction and rendering that are linear in
    /// it) stays bounded by the input.
    pub(super) struct Budget {
        left: usize,
    }

    impl Budget {
        pub(super) fn new(left: usize) -> Self {
            Budget { left }
        }

        /// Charges `n` bytes before they are allocated.
        pub(super) fn charge(&mut self, n: usize) -> Result<(), JsonLdError> {
            match self.left.checked_sub(n) {
                Some(left) => {
                    self.left = left;
                    Ok(())
                }
                None => Err(JsonLdError::with_detail(
                    E::ContextOverflow,
                    "framing output exceeds its bound for this input",
                )),
            }
        }
    }

    /// A framed output value under construction.
    pub(super) struct Out(Json);

    impl Out {
        /// A new, empty array.
        pub(super) fn array(budget: &mut Budget) -> Result<Out, JsonLdError> {
            budget.charge(NODE_BYTES)?;
            Ok(Out(Json::Arr(Vec::new())))
        }

        /// A new node object `{"@id": id}`.
        pub(super) fn node(budget: &mut Budget, id: &str) -> Result<Out, JsonLdError> {
            budget.charge(NODE_BYTES + "@id".len() + id.len() + 16)?;
            Ok(Out(Json::Obj(vec![("@id".to_string(), Json::Str(id.to_string()))])))
        }

        /// A new, empty list object `{"@list": []}`.
        pub(super) fn list(budget: &mut Budget) -> Result<Out, JsonLdError> {
            budget.charge(2 * NODE_BYTES + "@list".len() + 8)?;
            Ok(Out(Json::Obj(vec![("@list".to_string(), Json::Arr(Vec::new()))])))
        }

        /// A copy of `value` (from the input or the frame).
        pub(super) fn copy(budget: &mut Budget, value: &Json) -> Result<Out, JsonLdError> {
            budget.charge(json_bytes(value))?;
            Ok(Out(value.clone()))
        }

        /// A `@default` fill: the frame's `default` values (or `@null`), wrapped in
        /// `@preserve` unless `direct` (a `@type` default, whose IRIs must still go through
        /// IRI compaction).
        pub(super) fn default_fill(budget: &mut Budget, default: Option<&Json>, direct: bool) -> Result<Out, JsonLdError> {
            budget.charge(default.map_or(16, json_bytes) + 3 * NODE_BYTES + "@preserve".len() + 8)?;
            let values = match default {
                Some(d) => Json::Arr(as_slice(d).to_vec()),
                None => Json::Arr(vec![Json::Str("@null".to_string())]),
            };
            Ok(Out(if direct {
                values
            } else {
                Json::Arr(vec![Json::Obj(vec![("@preserve".to_string(), values)])])
            }))
        }

        /// Whether this object has member `key`.
        pub(super) fn has(&self, key: &str) -> bool {
            self.0.get(key).is_some()
        }

        /// Sets member `key` of this object to `value`.
        pub(super) fn set(&mut self, budget: &mut Budget, key: &str, value: Out) -> Result<(), JsonLdError> {
            budget.charge(key.len() + 8)?;
            self.0.set(key, value.0);
            Ok(())
        }

        /// Adds `value` to this output (§4.2 "add output to parent"): pushed onto an
        /// array, or appended to the `property` array of an object, retaining duplicates
        /// (a `@list` legitimately repeats values).
        pub(super) fn append(&mut self, budget: &mut Budget, property: Option<&str>, value: Out) -> Result<(), JsonLdError> {
            match (&mut self.0, property) {
                (Json::Arr(items), _) => {
                    budget.charge(8)?;
                    items.push(value.0);
                }
                (Json::Obj(_), Some(key)) => {
                    if self.0.get(key).is_none() {
                        budget.charge(key.len() + 8 + NODE_BYTES)?;
                        self.0.set(key, Json::Arr(Vec::new()));
                    }
                    budget.charge(8)?;
                    if let Some(Json::Arr(items)) = output_member_mut(&mut self.0, key) {
                        items.push(value.0);
                    }
                }
                _ => {}
            }
            Ok(())
        }

        /// Adds one framed referrer to this node's `@reverse` map under `property`.
        pub(super) fn append_reverse(&mut self, budget: &mut Budget, property: &str, item: Out) -> Result<(), JsonLdError> {
            if self.0.get("@reverse").is_none() {
                budget.charge("@reverse".len() + 8 + NODE_BYTES)?;
                self.0.set("@reverse", Json::obj());
            }
            if let Some(rev) = output_member_mut(&mut self.0, "@reverse") {
                let mut map = Out(std::mem::take(rev));
                let added = map.append(budget, Some(property), item);
                *rev = map.0;
                added?;
            }
            Ok(())
        }

        /// The items of this array.
        pub(super) fn into_items(self) -> Vec<Out> {
            match self.0 {
                Json::Arr(items) => items.into_iter().map(Out).collect(),
                _ => Vec::new(),
            }
        }

        /// The finished output.
        pub(super) fn into_json(self) -> Json {
            self.0
        }
    }
}

/// An upper estimate of the bytes `json` occupies: every string and member name by its
/// length, plus [`NODE_BYTES`] per array or object and 8 per scalar.
fn json_bytes(json: &Json) -> usize {
    let mut size = 0usize;
    let mut stack = vec![json];
    while let Some(j) = stack.pop() {
        size = size.saturating_add(match j {
            Json::Arr(items) => {
                stack.extend(items);
                NODE_BYTES
            }
            Json::Obj(members) => {
                stack.extend(members.iter().map(|(_, v)| v));
                members.iter().fold(NODE_BYTES, |n, (k, _)| n.saturating_add(k.len() + 8))
            }
            Json::Str(s) | Json::Raw(s) => s.len() + 8,
        });
    }
    size
}

/// The per-frame flags (§4.1), each read from the frame object with the option default.
struct Flags {
    embed: Embed,
    explicit: bool,
    require_all: bool,
}

/// The resolved `@embed` disposition. `@link` resolves to [`Embed::Once`] (module doc:
/// documented fallback, design record §11). [`Embed::Last`] (the JSON-LD 1.0 flag)
/// embeds every occurrence and a post-pass demotes all but the last to references —
/// equivalent to the reference processors' remove-embed mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Embed {
    Always,
    Once,
    Never,
    Last,
}

/// Build the framing graph maps from the expanded input: clone the node map's graphs and
/// (unless `frame_default`) add the `@merged` graph (Merge Node Maps, JSON-LD 1.1 API
/// §7.3).
fn build_graph_maps(expanded_input: &Json, frame_default: bool) -> GraphMaps {
    let graphs = generate_node_map(expanded_input)
        .into_graphs()
        .map(|(name, nodes)| (name, nodes.into_iter().collect()))
        .collect();
    finish_graph_maps(graphs, frame_default)
}

/// The graph maps of fromRdf output ([`frame_match_from_rdf`]): what node map generation
/// builds from it. Blank nodes are relabelled in the order node map generation visits
/// them, and since fromRdf omits nodes that are only referenced, a node object
/// `{"@id": …}` is added for each reference to one, as node map generation does.
fn graph_maps_from_rdf(from_rdf: Json, frame_default: bool) -> GraphMaps {
    let mut graphs: BTreeMap<String, Subjects> = BTreeMap::new();
    graphs.insert("@default".to_string(), Subjects::new());
    let Json::Arr(mut nodes) = from_rdf else {
        return finish_graph_maps(graphs, frame_default);
    };
    let mut issuer = BlankNodeIssuer::new();
    for node in &mut nodes {
        relabel_node(node, &mut issuer);
    }
    for node in nodes {
        let Json::Obj(mut members) = node else { continue };
        let Some(id) = members.iter().find(|(k, _)| k == "@id").and_then(|(_, v)| v.as_str()).map(str::to_string) else {
            continue;
        };
        if let Some(at) = members.iter().position(|(k, _)| k == "@graph") {
            let (_, inner) = members.remove(at);
            let graph = graphs.entry(id.clone()).or_default();
            let Json::Arr(inner) = inner else { continue };
            for node in inner {
                if let Some(gid) = node.get("@id").and_then(Json::as_str) {
                    graph.insert(gid.to_string(), node);
                }
            }
        }
        graphs.get_mut("@default").expect("present").insert(id, Json::Obj(members));
    }
    for graph in graphs.values_mut() {
        let mut referenced = Vec::new();
        for node in graph.values() {
            let Json::Obj(members) = node else { continue };
            for (key, values) in members {
                if key.starts_with('@') {
                    continue;
                }
                for value in as_slice(values) {
                    let items = value.get("@list").map_or(std::slice::from_ref(value), as_slice);
                    for item in items {
                        if let Some(rid) = subject_reference_id(item) {
                            if !graph.contains_key(rid) {
                                referenced.push(rid.to_string());
                            }
                        }
                    }
                }
            }
        }
        for rid in referenced {
            graph.entry(rid.clone()).or_insert_with(|| Json::Obj(vec![("@id".to_string(), Json::Str(rid))]));
        }
    }
    finish_graph_maps(graphs, frame_default)
}

/// Relabels the blank nodes of one fromRdf node object in node map generation's order:
/// its `@id`, then its members in order (`@type` values, `@graph` nodes, and the node
/// references in property values and lists).
fn relabel_node(node: &mut Json, issuer: &mut BlankNodeIssuer) {
    fn relabel(label: &mut String, issuer: &mut BlankNodeIssuer) {
        if label.starts_with("_:") {
            *label = issuer.issue(Some(label));
        }
    }
    fn relabel_value(value: &mut Json, issuer: &mut BlankNodeIssuer) {
        let Json::Obj(members) = value else { return };
        for (key, v) in members {
            match (key.as_str(), v) {
                ("@id", Json::Str(id)) => relabel(id, issuer),
                ("@list", Json::Arr(items)) => items.iter_mut().for_each(|i| relabel_value(i, issuer)),
                _ => {}
            }
        }
    }
    let Json::Obj(members) = node else { return };
    if let Some((_, Json::Str(id))) = members.iter_mut().find(|(k, _)| k == "@id") {
        relabel(id, issuer);
    }
    for (key, value) in members {
        let Json::Arr(items) = value else { continue };
        match key.as_str() {
            "@type" => {
                for t in items {
                    if let Json::Str(t) = t {
                        relabel(t, issuer);
                    }
                }
            }
            "@graph" => items.iter_mut().for_each(|n| relabel_node(n, issuer)),
            k if k.starts_with('@') => {}
            _ => items.iter_mut().for_each(|v| relabel_value(v, issuer)),
        }
    }
}

/// [`GraphMaps`] over `graphs`, adding `@merged` unless `frame_default`.
fn finish_graph_maps(graphs: BTreeMap<String, Subjects>, frame_default: bool) -> GraphMaps {
    let mut graphs: BTreeMap<String, Rc<Subjects>> = graphs.into_iter().map(|(name, nodes)| (name, Rc::new(nodes))).collect();
    if !frame_default {
        // With only the default graph, merging changes nothing: share it.
        let merged = match graphs.len() {
            1 => Rc::clone(&graphs["@default"]),
            _ => Rc::new(merge_graphs(&graphs)),
        };
        graphs.insert("@merged".to_string(), merged);
    }
    GraphMaps { graphs }
}

/// **Merge Node Maps** (JSON-LD 1.1 API §7.3): fold every graph's node objects into one
/// `@merged` subject map — non-`@type` keywords copied, `@type` and ordinary property
/// values merged without duplicates.
fn merge_graphs(graphs: &BTreeMap<String, Rc<Subjects>>) -> Subjects {
    let mut merged = Subjects::new();
    for subjects in graphs.values() {
        for (id, node) in subjects.iter() {
            let Json::Obj(members) = node else { continue };
            let Some(entry) = merged.get_mut(id) else {
                // A subject's first node object: its values are already unique.
                merged.insert(id.clone(), node.clone());
                continue;
            };
            for (prop, value) in members {
                if prop.starts_with('@') && prop != "@type" {
                    if prop != "@id" {
                        entry.set(prop, value.clone());
                    }
                } else {
                    add_all_unique(entry, prop, as_slice(value));
                }
            }
        }
    }
    merged
}

/// Adds each of `values` to the array member `prop` of `obj` (creating it) unless an
/// equal value is already there, in time linear in the values.
fn add_all_unique(obj: &mut Json, prop: &str, values: &[Json]) {
    if obj.get(prop).is_none() {
        obj.set(prop, Json::Arr(Vec::new()));
    }
    let Some(Json::Arr(items)) = output_member_mut(obj, prop) else { return };
    let key = |v: &Json| {
        let mut s = String::new();
        v.write(&mut s);
        s
    };
    let mut seen: std::collections::HashSet<String> = items.iter().map(key).collect();
    for v in values {
        if seen.insert(key(v)) {
            items.push(v.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Frame matching (§4.2)
// ---------------------------------------------------------------------------

/// The recursive frame-matching algorithm (§4.2): validate `frame`, filter `subjects`
/// against it, and emit each match into `parent` (embedding, list re-emit, `@default`
/// fill, named-graph and `@reverse` recursion).
///
/// `embedded` is the framing state's *embedded flag*: `false` at the top level and in
/// graph / `@included` recursion, `true` when recursing into a property or list value.
/// A non-embedded match already embedded elsewhere in the graph is SKIPPED entirely
/// (not re-emitted as a reference) — the compartmentalisation the 1.1 algorithm uses
/// so a graph-level subject embedded inside a sibling does not reappear.
#[allow(clippy::too_many_arguments)]
fn match_frame(
    maps: &GraphMaps,
    st: &mut FState<'_>,
    subjects: &[String],
    frame: &Json,
    parent: &mut Out,
    property: Option<&str>,
    embedded: bool,
) -> Result<(), JsonLdError> {
    let _nested = crate::context::budget::nest()?;
    let frame_obj = validate_frame(frame)?;
    let flags = Flags {
        embed: frame_flag_embed(&frame_obj, st.options)?,
        explicit: frame_flag_bool(&frame_obj, "@explicit", st.options.explicit),
        require_all: frame_flag_bool(&frame_obj, "@requireAll", st.options.require_all),
    };

    let matches = filter_subjects(maps, st, subjects, &frame_obj, &flags)?;

    for id in matches {
        // Compartmentalise each top-level match: reset the unique-embeds map when there
        // is no active property (top level only).
        if property.is_none() {
            st.unique_embeds = BTreeMap::new();
        }
        st.unique_embeds.entry(st.graph.clone()).or_default();

        let Some(subject) = maps.graphs.get(&st.graph).and_then(|g| g.get(&id)) else {
            continue;
        };

        // A NON-embedded match (top level / graph level) already embedded elsewhere
        // in this graph is skipped entirely — it was already included in another
        // node object (the 1.1 embedded-flag rule).
        if !embedded
            && st
                .unique_embeds
                .get(&st.graph)
                .map(|e| e.contains(&id))
                .unwrap_or(false)
        {
            continue;
        }

        let mut output = Out::node(&mut st.budget, &id)?;
        if id.starts_with("_:") {
            *st.bnode_counts.entry(id.clone()).or_insert(0) += 1;
        }

        // For an EMBEDDED match, @never — or a circular reference — emits a bare
        // node reference; @once re-references an already-embedded subject.
        if embedded {
            let circular = st
                .subject_stack
                .iter()
                .any(|(g, s)| *g == st.graph && *s == id);
            if flags.embed == Embed::Never || circular {
                parent.append(&mut st.budget, property, output)?;
                continue;
            }
            if flags.embed == Embed::Once
                && st
                    .unique_embeds
                    .get(&st.graph)
                    .map(|e| e.contains(&id))
                    .unwrap_or(false)
            {
                parent.append(&mut st.budget, property, output)?;
                continue;
            }
        }
        st.unique_embeds
            .entry(st.graph.clone())
            .or_default()
            .insert(id.clone());
        if flags.embed == Embed::Last {
            st.last_ids.insert(id.clone());
        }

        st.subject_stack.push((st.graph.clone(), id.clone()));

        // Named-graph framing: a matched subject that names a graph recurses into it.
        if maps.graphs.contains_key(&id) {
            let empty = Json::obj();
            let (recurse, subframe) = match frame_obj.get("@graph") {
                None => (st.graph != "@merged", &empty),
                Some(gf) => {
                    let sf = first_of(gf).filter(|f| f.is_obj()).unwrap_or(&empty);
                    (id != "@merged" && id != "@default", sf)
                }
            };
            if recurse {
                let graph_subjects = match maps.graphs.get(&id) {
                    Some(g) => st.subject_ids(g)?,
                    None => Vec::new(),
                };
                let prev = std::mem::replace(&mut st.graph, id.clone());
                match_frame(
                    maps,
                    st,
                    &graph_subjects,
                    subframe,
                    &mut output,
                    Some("@graph"),
                    false,
                )?;
                st.graph = prev;
            }
        }

        // @included: recurse over the same subjects with the included sub-frame.
        if let Some(included) = frame_obj.get("@included") {
            match_frame(
                maps,
                st,
                subjects,
                included,
                &mut output,
                Some("@included"),
                false,
            )?;
        }

        // Iterate the subject's properties in lexicographical order.
        let Json::Obj(subject_members) = subject else {
            st.subject_stack.pop();
            continue;
        };
        st.emit(8 * subject_members.len())?;
        let mut props: Vec<&(String, Json)> = subject_members.iter().collect();
        props.sort_by(|a, b| a.0.cmp(&b.0));
        for (prop, objects) in props {
            if prop == "@id" {
                continue;
            }
            if prop.starts_with('@') {
                // Keywords copy verbatim; blank-node @type values count toward pruning.
                let copied = Out::copy(&mut st.budget, objects)?;
                output.set(&mut st.budget, prop, copied)?;
                if prop == "@type" {
                    for t in as_slice(objects) {
                        if let Some(s) = t.as_str() {
                            if s.starts_with("_:") {
                                *st.bnode_counts.entry(s.to_string()).or_insert(0) += 1;
                            }
                        }
                    }
                }
                continue;
            }
            // @explicit: only include properties present in the frame.
            let frame_prop = frame_obj.get(prop.as_str());
            if flags.explicit && frame_prop.is_none() {
                continue;
            }
            for o in as_slice(objects) {
                if is_list_object(o) {
                    // @list re-emit: node entries recurse with the list sub-frame,
                    // value entries copy verbatim.
                    let implicit;
                    let subframe = match frame_prop.and_then(first_of).and_then(|f0| f0.get("@list")) {
                        Some(f) => f,
                        None => {
                            implicit = st.implicit(&flags)?;
                            &*implicit
                        }
                    };
                    let mut list = Out::list(&mut st.budget)?;
                    for oo in o.get("@list").map(as_slice).unwrap_or_default() {
                        if let Some(oid) = subject_reference_id(oo) {
                            match_frame(
                                maps,
                                st,
                                &[oid.to_string()],
                                subframe,
                                &mut list,
                                Some("@list"),
                                true,
                            )?;
                        } else {
                            let copied = Out::copy(&mut st.budget, oo)?;
                            list.append(&mut st.budget, Some("@list"), copied)?;
                        }
                    }
                    output.append(&mut st.budget, Some(prop), list)?;
                } else if let Some(oid) = subject_reference_id(o) {
                    // Node reference: recurse with the property sub-frame (or the
                    // implicit frame inheriting the current flags).
                    let implicit;
                    let subframe = match frame_prop {
                        Some(f) => f,
                        None => {
                            implicit = st.implicit(&flags)?;
                            &*implicit
                        }
                    };
                    match_frame(
                        maps,
                        st,
                        &[oid.to_string()],
                        subframe,
                        &mut output,
                        Some(prop),
                        true,
                    )?;
                } else {
                    // A value: include iff it matches the value pattern (an absent
                    // pattern matches everything).
                    let empty = Json::obj();
                    let pattern = frame_prop.and_then(first_of).unwrap_or(&empty);
                    if value_match(pattern, o) {
                        let copied = Out::copy(&mut st.budget, o)?;
                        output.append(&mut st.budget, Some(prop), copied)?;
                    }
                }
            }
        }

        // @default fill: frame properties absent from the output get their @default
        // value (or @null), wrapped under @preserve so compaction keeps them.
        let Json::Obj(frame_members) = &*frame_obj else {
            unreachable!("validate_frame returns an object")
        };
        st.emit(8 * frame_members.len())?;
        let mut fprops: Vec<&(String, Json)> = frame_members.iter().collect();
        fprops.sort_by(|a, b| a.0.cmp(&b.0));
        for (prop, pvalue) in fprops {
            let empty = Json::obj();
            let next = first_of(pvalue).unwrap_or(&empty);
            if prop == "@type" {
                // Only the `@type: {"@default": …}` form participates in default fill.
                if next.get("@default").is_none() {
                    continue;
                }
            } else if prop.starts_with('@') {
                continue;
            }
            let omit_default = frame_flag_bool(next, "@omitDefault", st.options.omit_default);
            if !omit_default && !output.has(prop) {
                let fill = Out::default_fill(&mut st.budget, next.get("@default"), prop == "@type")?;
                output.set(&mut st.budget, prop, fill)?;
            }
        }

        // @reverse framing: find nodes of the active graph referencing this subject
        // under the reverse property and frame them under output's @reverse map.
        if let Some(Json::Obj(rev)) = frame_obj.get("@reverse") {
            st.emit(8 * rev.len())?;
            let mut rprops: Vec<&(String, Json)> = rev.iter().collect();
            rprops.sort_by(|a, b| a.0.cmp(&b.0));
            for (reverse_prop, subframe) in rprops {
                let referrers = st.referrers(maps, reverse_prop, &id)?;
                for sid in referrers {
                    let mut tmp = Out::array(&mut st.budget)?;
                    match_frame(
                        maps,
                        st,
                        &[sid],
                        subframe,
                        &mut tmp,
                        Some(reverse_prop),
                        embedded,
                    )?;
                    for item in tmp.into_items() {
                        output.append_reverse(&mut st.budget, reverse_prop, item)?;
                    }
                }
            }
        }

        parent.append(&mut st.budget, property, output)?;
        st.subject_stack.pop();
    }
    Ok(())
}

/// The implicit sub-frame for properties absent from the frame: an empty pattern
/// carrying the current flags forward (§4.2 step 3.6.2).
fn implicit_frame(flags: &Flags) -> Json {
    let embed = match flags.embed {
        Embed::Always => "@always",
        Embed::Once => "@once",
        Embed::Never => "@never",
        Embed::Last => "@last",
    };
    Json::Arr(vec![Json::Obj(vec![
        (
            "@embed".to_string(),
            Json::Arr(vec![Json::Str(embed.to_string())]),
        ),
        (
            "@explicit".to_string(),
            Json::Arr(vec![Json::Raw(flags.explicit.to_string())]),
        ),
        (
            "@requireAll".to_string(),
            Json::Arr(vec![Json::Raw(flags.require_all.to_string())]),
        ),
    ])])
}

// ---------------------------------------------------------------------------
// Frame validation + flags
// ---------------------------------------------------------------------------

/// Validate a frame (§4.2 step 1): it must be (an array holding) one map, and any
/// `@id` / `@type` pattern must hold wildcards, `@default` maps, or absolute IRIs —
/// a blank-node identifier raises `invalid frame`. Returns the frame object.
fn validate_frame(frame: &Json) -> Result<Cow<'_, Json>, JsonLdError> {
    let obj = match frame {
        Json::Obj(_) => Cow::Borrowed(frame),
        Json::Arr(items) if items.len() == 1 && items[0].is_obj() => Cow::Borrowed(&items[0]),
        Json::Arr(items) if items.is_empty() => Cow::Owned(Json::obj()),
        _ => {
            return Err(JsonLdError::with_detail(
                E::InvalidFrame,
                "a frame must be a single map",
            ))
        }
    };
    if let Some(ids) = obj.get("@id") {
        for v in as_slice(ids) {
            match v {
                Json::Obj(m) if m.is_empty() => {}
                Json::Str(s) if !s.starts_with("_:") && s.contains(':') => {}
                _ => {
                    return Err(JsonLdError::with_detail(
                        E::InvalidFrame,
                        "frame @id must be a wildcard or an absolute IRI",
                    ))
                }
            }
        }
    }
    if let Some(types) = obj.get("@type") {
        for v in as_slice(types) {
            match v {
                // Wildcard {} and the default-object form {"@default": …}.
                Json::Obj(m) if m.is_empty() || m.iter().any(|(k, _)| k == "@default") => {}
                Json::Str(s) if s == "@json" => {}
                Json::Str(s) if !s.starts_with("_:") && s.contains(':') => {}
                _ => {
                    return Err(JsonLdError::with_detail(
                        E::InvalidFrame,
                        "frame @type must be a wildcard, a default map, or an absolute IRI",
                    ))
                }
            }
        }
    }
    Ok(obj)
}

/// Read a boolean framing flag (`@explicit` / `@requireAll` / `@omitDefault`) from the
/// frame object, falling back to `default` (the option value). Tolerates the expanded
/// shapes (`[true]`, `[{"@value": true}]`).
fn frame_flag_bool(frame_obj: &Json, key: &str, default: bool) -> bool {
    let Some(v) = frame_obj.get(key) else {
        return default;
    };
    match flag_scalar(v) {
        Some(Json::Raw(r)) if r == "true" => true,
        Some(Json::Raw(r)) if r == "false" => false,
        // The suite also spells flags as strings ("@omitDefault": "true").
        Some(Json::Str(s)) if s == "true" => true,
        Some(Json::Str(s)) if s == "false" => false,
        _ => default,
    }
}

/// Read and validate the `@embed` flag (§4.1): `@always` / `@once` / `@never`, the
/// booleans, plus the documented fallbacks `@link` / `@last` → `@once`. Any other value
/// raises `invalid @embed value`.
fn frame_flag_embed(frame_obj: &Json, options: &JsonLdOptions) -> Result<Embed, JsonLdError> {
    use crate::options::EmbedFlag;
    let Some(v) = frame_obj.get("@embed") else {
        return Ok(match options.embed {
            EmbedFlag::Always => Embed::Always,
            EmbedFlag::Never => Embed::Never,
            // @link is documented as an @once fallback (module doc, design record §11).
            EmbedFlag::Once | EmbedFlag::Link => Embed::Once,
        });
    };
    match flag_scalar(v) {
        Some(Json::Str(s)) => match s.as_str() {
            "@always" => Ok(Embed::Always),
            "@never" => Ok(Embed::Never),
            // @link falls back to @once (module doc, design record §11).
            "@once" | "@link" => Ok(Embed::Once),
            // The JSON-LD 1.0 flag: the last occurrence keeps the embed.
            "@last" => Ok(Embed::Last),
            other => Err(JsonLdError::with_detail(
                E::InvalidEmbedValue,
                format!("out-of-range @embed value {other:?}"),
            )),
        },
        Some(Json::Raw(r)) if r == "true" => Ok(Embed::Once),
        Some(Json::Raw(r)) if r == "false" => Ok(Embed::Never),
        _ => Err(JsonLdError::with_detail(
            E::InvalidEmbedValue,
            "out-of-range @embed value",
        )),
    }
}

/// Unwrap a framing-flag value to its scalar: strip one array layer and one
/// `{"@value": …}` layer (the shapes frame expansion produces), returning the scalar.
fn flag_scalar(v: &Json) -> Option<&Json> {
    let first = match v {
        Json::Arr(items) => items.first()?,
        other => other,
    };
    match first {
        Json::Obj(_) => first.get("@value"),
        scalar => Some(scalar),
    }
}

// ---------------------------------------------------------------------------
// Subject filtering (§4.3 Frame Matching)
// ---------------------------------------------------------------------------

/// Filter `subjects` (already sorted) to those matching `frame_obj` (§4.3).
fn filter_subjects(
    maps: &GraphMaps,
    st: &FState<'_>,
    subjects: &[String],
    frame_obj: &Json,
    flags: &Flags,
) -> Result<Vec<String>, JsonLdError> {
    let mut sorted: Vec<&String> = subjects.iter().collect();
    sorted.sort();
    let Some(graph_subjects) = maps.graphs.get(&st.graph) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let memo = Memo::default();
    for id in sorted {
        if let Some(node) = graph_subjects.get(id) {
            if filter_subject(graph_subjects, node, frame_obj, flags, &memo)? {
                out.push(id.clone());
            }
        }
    }
    Ok(out)
}

/// Results of matching a subject against a nested node pattern, keyed by the subject's
/// and the pattern's addresses (both borrowed for the whole match) and `@requireAll`.
/// Each pair is evaluated once, so nested patterns over shared subjects cannot make
/// matching exponential.
type Memo = RefCell<BTreeMap<(usize, usize, bool), bool>>;

/// **Frame Matching** for one node (§4.3): duck-type the node against the frame's
/// `@id` / `@type` / property patterns. With `@requireAll`, every frame entry must
/// match; otherwise any match (or a wildcard frame) suffices.
fn filter_subject(
    graph_subjects: &Subjects,
    node: &Json,
    frame_obj: &Json,
    flags: &Flags,
    memo: &Memo,
) -> Result<bool, JsonLdError> {
    let Json::Obj(frame_members) = frame_obj else {
        return Ok(true);
    };
    let mut wildcard = true;
    let mut matches_some = false;
    for (key, value) in frame_members {
        let node_values = node.get(key).map(as_slice).unwrap_or_default();
        let frame_values = as_slice(value);
        let is_empty = frame_values.is_empty();
        let match_this;

        // @id and @type are mandatory constraints (json-ld11-framing, Frame Matching): a node that
        // fails either does not match, whatever @requireAll says; one that passes counts
        // as matching that property.
        if key == "@id" {
            // A wildcard `{}` matches every node, `[]` none, and a list of IRIs the
            // listed nodes.
            let nid = node.get("@id").and_then(Json::as_str);
            if !frame_values.iter().any(|v| is_empty_obj(v) || v.as_str() == nid) {
                return Ok(false);
            }
            match_this = true;
        } else if key == "@type" {
            wildcard = false;
            match_this = if is_empty {
                // Match-none: the node must have no @type.
                node_values.is_empty()
            } else if frame_values.len() == 1 && is_empty_obj(&frame_values[0]) {
                // Wildcard: the node must have some @type.
                !node_values.is_empty()
            } else if frame_values
                .first()
                .map(|f| f.get("@default").is_some())
                .unwrap_or(false)
            {
                // A default-map @type matches any node.
                true
            } else {
                frame_values
                    .iter()
                    .any(|t| node_values.iter().any(|nv| nv == t))
            };
            if !match_this {
                return Ok(false);
            }
        } else if key.starts_with('@') {
            // Other keywords do not participate in matching.
            continue;
        } else {
            let this_frame = frame_values.first();
            let mut has_default = false;
            if let Some(tf) = this_frame {
                validate_frame(tf)?;
                has_default = tf.get("@default").is_some();
            }
            wildcard = false;

            // A frame property with a @default matches regardless of node values.
            if node_values.is_empty() && has_default {
                continue;
            }
            // Match-none: the node must have no value for this property.
            if !node_values.is_empty() && is_empty {
                return Ok(false);
            }
            match this_frame {
                None => {
                    if !node_values.is_empty() {
                        return Ok(false);
                    }
                    match_this = true;
                }
                Some(tf) if is_list_object(tf) => {
                    let list_pattern = tf.get("@list").map(as_slice).unwrap_or_default();
                    let node_list = node_values
                        .first()
                        .filter(|nv| is_list_object(nv))
                        .and_then(|nv| nv.get("@list"))
                        .map(as_slice)
                        .unwrap_or_default();
                    match_this = match list_pattern.first() {
                        Some(lp) if is_value_object(lp) => {
                            node_list.iter().any(|lv| value_match(lp, lv))
                        }
                        Some(lp) => any_node_match(graph_subjects, lp, node_list, flags, memo)?,
                        None => false,
                    };
                }
                Some(tf) if is_value_object(tf) => {
                    match_this = node_values
                        .iter()
                        .any(|nv| is_value_object(nv) && value_match(tf, nv));
                }
                Some(tf) if tf.is_obj() => {
                    // A node pattern (subject / subject reference / wildcard object):
                    // match when some node value resolves to a matching subject —
                    // except the bare wildcard `{}` / flags-only pattern, which
                    // matches any present value.
                    if is_wildcard_node_pattern(tf) {
                        match_this = !node_values.is_empty();
                    } else {
                        match_this = any_node_match(graph_subjects, tf, node_values, flags, memo)?;
                    }
                }
                Some(_) => {
                    match_this = false;
                }
            }
        }

        if !match_this && flags.require_all {
            return Ok(false);
        }
        matches_some = matches_some || match_this;
    }
    Ok(wildcard || matches_some)
}

/// True iff `tf` is a wildcard node pattern: an object with no matching constraints
/// (only framing keywords / nothing) — it matches any present value.
fn is_wildcard_node_pattern(tf: &Json) -> bool {
    match tf {
        Json::Obj(members) => members.iter().all(|(k, _)| {
            matches!(
                k.as_str(),
                "@embed" | "@explicit" | "@requireAll" | "@omitDefault" | "@default"
            )
        }),
        _ => false,
    }
}

/// Whether any of `values` matches the node `pattern` ([`node_match`]).
fn any_node_match(
    graph_subjects: &Subjects,
    pattern: &Json,
    values: &[Json],
    flags: &Flags,
    memo: &Memo,
) -> Result<bool, JsonLdError> {
    for value in values {
        if node_match(graph_subjects, pattern, value, flags, memo)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// **Node Match**: a node-object value matches a node pattern iff it references a
/// subject of the active graph that itself matches the pattern (§4.3 step 2.5).
///
/// This is the one recursive entry of frame matching: it is memoised per subject and
/// pattern, and each evaluation is charged to the call's work budget and nests one
/// level deeper.
fn node_match(
    graph_subjects: &Subjects,
    pattern: &Json,
    value: &Json,
    flags: &Flags,
    memo: &Memo,
) -> Result<bool, JsonLdError> {
    let Some(id) = value.get("@id").and_then(Json::as_str) else {
        return Ok(false);
    };
    let Some(node) = graph_subjects.get(id) else {
        return Ok(false);
    };
    let key = (node as *const Json as usize, pattern as *const Json as usize, flags.require_all);
    if let Some(&known) = memo.borrow().get(&key) {
        return Ok(known);
    }
    let _nested = crate::context::budget::nest()?;
    crate::context::budget::charge(1)?;
    let matched = filter_subject(graph_subjects, node, pattern, flags, memo)?;
    memo.borrow_mut().insert(key, matched);
    Ok(matched)
}

/// **Value Pattern Matching** (§2.2 / §4.3): a value object matches a value pattern iff
/// each of the pattern's `@value` / `@type` / `@language` alternative arrays admits the
/// value's member (a wildcard `{}` admits any present member; an empty pattern admits
/// everything).
fn value_match(pattern: &Json, value: &Json) -> bool {
    let v1 = value.get("@value");
    let t1 = value.get("@type").and_then(Json::as_str);
    let l1 = value.get("@language").and_then(Json::as_str);

    let v2 = pattern.get("@value").map(as_slice).unwrap_or_default();
    let t2 = pattern.get("@type").map(as_slice).unwrap_or_default();
    let l2 = pattern.get("@language").map(as_slice).unwrap_or_default();

    if v2.is_empty() && t2.is_empty() && l2.is_empty() {
        return true;
    }
    // @value: in the alternatives, or the alternatives are the wildcard.
    let v_ok = v2.first().map(is_empty_obj).unwrap_or(false)
        || v1.map(|v| v2.contains(v)).unwrap_or(false);
    if !v_ok {
        return false;
    }
    // @type: absent-and-unconstrained, in the alternatives, or wildcard (with a type).
    let t_ok = (t2.is_empty() && t1.is_none())
        || t2.iter().any(|p| p.as_str().is_some() && p.as_str() == t1)
        || (t1.is_some() && t2.first().map(is_empty_obj).unwrap_or(false));
    if !t_ok {
        return false;
    }
    // @language: same shape, compared case-insensitively (BCP 47 tags).
    let l_ok = (l2.is_empty() && l1.is_none())
        || l2.iter().any(|p| {
            match (p.as_str(), l1) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                // A null alternative admits a language-less value.
                _ => matches!(p, Json::Raw(r) if r == "null") && l1.is_none(),
            }
        })
        || (l1.is_some() && l2.first().map(is_empty_obj).unwrap_or(false));
    l_ok
}

// ---------------------------------------------------------------------------
// Output assembly + post-processing
// ---------------------------------------------------------------------------

/// Mutable access to an object member (companion to [`Json::get`]).
fn output_member_mut<'a>(obj: &'a mut Json, key: &str) -> Option<&'a mut Json> {
    match obj {
        Json::Obj(members) => members
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v),
        _ => None,
    }
}

/// `@embed: @last` post-pass: demote every embed of `id` except the LAST (in the
/// depth-first document order framing emitted them) to a bare node reference. This is
/// behaviourally the reference processors' remove-embed (which mutates the earlier
/// parent in place), expressed as a walk over the finished per-top-level-match tree.
fn demote_all_but_last_embed(element: &mut Json, id: &str) {
    let total = count_embeds(element, id);
    if total > 1 {
        let mut seen = 0usize;
        demote_embeds(element, id, total - 1, &mut seen);
    }
}

/// Count the full embeds of each of `ids` in `j` (node objects with that `@id` and
/// other members).
fn count_all_embeds(j: &Json, ids: &BTreeSet<String>, counts: &mut BTreeMap<String, usize>) {
    match j {
        Json::Arr(items) => items.iter().for_each(|i| count_all_embeds(i, ids, counts)),
        Json::Obj(members) => {
            if members.len() > 1 {
                if let Some(id) = members.iter().find(|(k, _)| k == "@id").and_then(|(_, v)| v.as_str()) {
                    if ids.contains(id) {
                        *counts.entry(id.to_string()).or_insert(0) += 1;
                    }
                }
            }
            members.iter().for_each(|(_, v)| count_all_embeds(v, ids, counts));
        }
        _ => {}
    }
}

/// Count full embeds of `id` (node objects with that `@id` and other members).
fn count_embeds(j: &Json, id: &str) -> usize {
    match j {
        Json::Arr(items) => items.iter().map(|i| count_embeds(i, id)).sum(),
        Json::Obj(members) => {
            let own = usize::from(
                members.len() > 1
                    && members
                        .iter()
                        .any(|(k, v)| k == "@id" && v.as_str() == Some(id)),
            );
            own + members
                .iter()
                .map(|(_, v)| count_embeds(v, id))
                .sum::<usize>()
        }
        _ => 0,
    }
}

/// Demote the first `demote` embeds of `id` to `{"@id": id}` (depth-first order).
fn demote_embeds(j: &mut Json, id: &str, demote: usize, seen: &mut usize) {
    if *seen >= demote {
        return;
    }
    match j {
        Json::Arr(items) => {
            for item in items {
                demote_embeds(item, id, demote, seen);
            }
        }
        Json::Obj(members) => {
            let is_embed = members.len() > 1
                && members
                    .iter()
                    .any(|(k, v)| k == "@id" && v.as_str() == Some(id));
            if is_embed {
                *seen += 1;
                *j = Json::Obj(vec![("@id".to_string(), Json::Str(id.to_string()))]);
                return;
            }
            for (_, v) in members {
                demote_embeds(v, id, demote, seen);
            }
        }
        _ => {}
    }
}

/// Remove `@id` members whose value is a blank-node label in `to_clear`
/// (`pruneBlankNodeIdentifiers`, §4.1 step 7), recursively.
fn prune_bnode_ids(j: &mut Json, to_clear: &BTreeSet<String>) {
    match j {
        Json::Arr(items) => {
            for item in items {
                prune_bnode_ids(item, to_clear);
            }
        }
        Json::Obj(members) => {
            members.retain(|(k, v)| {
                !(k == "@id" && matches!(v, Json::Str(s) if s.starts_with("_:") && to_clear.contains(s)))
            });
            for (_, v) in members {
                prune_bnode_ids(v, to_clear);
            }
        }
        _ => {}
    }
}

/// Post-compaction cleanup (§4.1 step 8.2): unwrap `{"@preserve": …}` wrappers (a
/// preserved `"@null"` becomes JSON `null`), collapsing a single-element unwrapped
/// array when `compact_arrays`.
fn cleanup_preserve(j: Json, compact_arrays: bool) -> Json {
    match j {
        Json::Arr(items) => Json::Arr(
            items
                .into_iter()
                .map(|i| cleanup_preserve(i, compact_arrays))
                .filter(|i| !matches!(i, Json::Raw(r) if r == "null"))
                .collect(),
        ),
        Json::Obj(members) => {
            // An object carrying @preserve is replaced by its preserved value.
            if let Some(pos) = members.iter().position(|(k, _)| k == "@preserve") {
                let preserved = members[pos].1.clone();
                let unwrapped = match preserved {
                    Json::Str(s) if s == "@null" => Json::Raw("null".to_string()),
                    Json::Arr(items) if items.len() == 1 && compact_arrays => {
                        items.into_iter().next().unwrap()
                    }
                    other => other,
                };
                return match unwrapped {
                    Json::Str(s) if s == "@null" => Json::Raw("null".to_string()),
                    other => cleanup_preserve(other, compact_arrays),
                };
            }
            Json::Obj(
                members
                    .into_iter()
                    .map(|(k, v)| {
                        // A context is data, not framed output — keep it verbatim (it
                        // may legitimately contain `null` entries).
                        if k == "@context" {
                            return (k, v);
                        }
                        let was_preserve_obj =
                            matches!(&v, Json::Obj(m) if m.iter().any(|(mk, _)| mk == "@preserve"));
                        let prior_len = match &v {
                            Json::Arr(items) => Some(items.len()),
                            _ => None,
                        };
                        let mut cleaned = cleanup_preserve(v, compact_arrays);
                        if compact_arrays {
                            if let Json::Arr(items) = &cleaned {
                                // Collapse a singleton minted by a @preserve unwrap, or
                                // an array that SHRANK to one element because a
                                // preserved `@null` was dropped (the reference
                                // processors' remove-preserve collapse).
                                let shrank =
                                    prior_len.map(|n| items.len() < n).unwrap_or(false);
                                if items.len() == 1 && (was_preserve_obj || shrank) {
                                    cleaned = items[0].clone();
                                }
                            }
                        }
                        (k, cleaned)
                    })
                    .collect(),
            )
        }
        other => other,
    }
}

/// `omitGraph: false` shaping: ensure the compacted result carries a top-level `@graph`
/// array (§4.1 step 8.1). `framed_len` is the framed node count — when it is 1 the
/// compacted result is a bare node object that must be wrapped (a multi-node or empty
/// result is already `@graph`-shaped by compaction).
fn ensure_graph_envelope(compacted: Json, ctx_value: &Json, framed_len: usize) -> Json {
    let graph_key = graph_alias(ctx_value);
    let Json::Obj(members) = compacted else {
        return compacted;
    };
    let has_context = members.iter().any(|(k, _)| k == "@context");
    let inner: Vec<(String, Json)> = members
        .iter()
        .filter(|(k, _)| k != "@context")
        .cloned()
        .collect();
    // Already @graph-shaped (the multi-node compaction wrap) — nothing to do.
    if framed_len != 1 && inner.len() == 1 && inner[0].0 == graph_key {
        return Json::Obj(members);
    }
    let graph_nodes = if inner.is_empty() {
        Vec::new()
    } else {
        vec![Json::Obj(inner)]
    };
    let mut out = Vec::new();
    if has_context {
        out.push(("@context".to_string(), ctx_value.clone()));
    }
    out.push((graph_key, Json::Arr(graph_nodes)));
    Json::Obj(out)
}

/// The term the frame's context aliases to `@graph` (else `"@graph"`), for the
/// `omitGraph: false` envelope.
fn graph_alias(ctx_value: &Json) -> String {
    let scan = |obj: &Json| -> Option<String> {
        if let Json::Obj(members) = obj {
            for (k, v) in members {
                if v.as_str() == Some("@graph") && !k.starts_with('@') {
                    return Some(k.clone());
                }
            }
        }
        None
    };
    let found = match ctx_value {
        Json::Arr(items) => items.iter().rev().find_map(scan),
        other => scan(other),
    };
    found.unwrap_or_else(|| "@graph".to_string())
}

// ---------------------------------------------------------------------------
// Small shape helpers
// ---------------------------------------------------------------------------

/// View a value as a slice of items (an array borrows its items; a scalar/object is a
/// one-element view).
fn as_slice(v: &Json) -> &[Json] {
    match v {
        Json::Arr(items) => items,
        other => std::slice::from_ref(other),
    }
}

/// The first element of an array value (or the value itself when not an array).
fn first_of(v: &Json) -> Option<&Json> {
    match v {
        Json::Arr(items) => items.first(),
        other => Some(other),
    }
}

/// True iff `v` is `{}` (the wildcard).
fn is_empty_obj(v: &Json) -> bool {
    matches!(v, Json::Obj(m) if m.is_empty())
}

/// True iff `v` is a list object (`{"@list": …}`).
fn is_list_object(v: &Json) -> bool {
    v.is_obj() && v.get("@list").is_some()
}

/// True iff `v` is a value object (`{"@value": …}`).
fn is_value_object(v: &Json) -> bool {
    v.is_obj() && v.get("@value").is_some()
}

/// The `@id` of a node object / node reference (a non-value, non-list object with an
/// `@id`), else `None`. In an expanded document every node under a property is a
/// reference into the node map, so this is the "recurse into subject" discriminator.
fn subject_reference_id(v: &Json) -> Option<&str> {
    if is_value_object(v) || is_list_object(v) {
        return None;
    }
    v.get("@id").and_then(Json::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::NoopLoader;

    fn parse(s: &str) -> Json {
        Json::parse(s).expect("valid JSON fixture")
    }

    /// `FrameOptions::default` resolves the mode-dependent defaults per spec.
    #[test]
    fn frame_options_mode_defaults() {
        let o = FrameOptions::default();
        assert!(o.omit_graph(ProcessingMode::JsonLd11));
        assert!(!o.omit_graph(ProcessingMode::JsonLd10));
        assert!(o.prune(ProcessingMode::JsonLd11));
        assert!(!o.prune(ProcessingMode::JsonLd10));
        assert!(!o.frame_default);
        let explicit = FrameOptions {
            omit_graph: Some(false),
            prune_blank_node_identifiers: Some(false),
            ..FrameOptions::default()
        };
        assert!(!explicit.omit_graph(ProcessingMode::JsonLd11));
        assert!(!explicit.prune(ProcessingMode::JsonLd11));
    }

    /// `validate_frame` accepts wildcards/IRIs and rejects blank-node patterns with
    /// `invalid frame`.
    #[test]
    fn validate_frame_rejects_bnode_patterns() {
        assert!(validate_frame(&parse(r#"{"@id": ["http://ex/a", {}]}"#)).is_ok());
        assert!(validate_frame(&parse(r#"{"@type": ["http://ex/T"]}"#)).is_ok());
        let e = validate_frame(&parse(r#"{"@id": ["_:b0"]}"#)).unwrap_err();
        assert_eq!(e.code(), E::InvalidFrame);
        let e = validate_frame(&parse(r#"{"@type": ["_:T"]}"#)).unwrap_err();
        assert_eq!(e.code(), E::InvalidFrame);
        let e = validate_frame(&Json::Str("not a frame".to_string())).unwrap_err();
        assert_eq!(e.code(), E::InvalidFrame);
    }

    /// `frame_flag_embed` maps the spec values and raises `invalid @embed value` on an
    /// out-of-range value.
    #[test]
    fn embed_flag_validation() {
        let opts = JsonLdOptions::default();
        let ok = frame_flag_embed(&parse(r#"{"@embed": [{"@value": "@always"}]}"#), &opts);
        assert!(matches!(ok, Ok(Embed::Always)));
        // The documented fallback: @link → @once; @last keeps its own disposition.
        assert!(matches!(
            frame_flag_embed(&parse(r#"{"@embed": ["@link"]}"#), &opts),
            Ok(Embed::Once)
        ));
        assert!(matches!(
            frame_flag_embed(&parse(r#"{"@embed": ["@last"]}"#), &opts),
            Ok(Embed::Last)
        ));
        let e = frame_flag_embed(&parse(r#"{"@embed": ["@sometimes"]}"#), &opts).unwrap_err();
        assert_eq!(e.code(), E::InvalidEmbedValue);
    }

    /// `value_match`: wildcard, alternatives, and language case-insensitivity.
    #[test]
    fn value_pattern_matching() {
        let v = parse(r#"{"@value": "x", "@language": "EN"}"#);
        assert!(value_match(&parse("{}"), &v));
        assert!(value_match(
            &parse(r#"{"@value": ["x", "y"], "@language": ["en"]}"#),
            &v
        ));
        assert!(!value_match(&parse(r#"{"@value": ["z"]}"#), &v));
        // A pattern with only @type does not admit an untyped value (§2.2).
        assert!(!value_match(&parse(r#"{"@type": ["http://ex/T"]}"#), &v));
        let typed = parse(r#"{"@value": "1", "@type": "http://ex/T"}"#);
        assert!(value_match(
            &parse(r#"{"@value": [{}], "@type": [{}]}"#),
            &typed
        ));
    }

    /// `frame_expanded`: `@explicit` prunes unframed properties and `@default` fills
    /// missing ones under `@preserve`.
    #[test]
    fn frame_expanded_explicit_and_default() {
        let input = parse(
            r#"[{"@id": "http://ex/a", "http://ex/p": [{"@value": "v"}], "http://ex/q": [{"@value": "w"}]}]"#,
        );
        let frame = parse(
            r#"[{"@explicit": [true], "http://ex/p": [{}], "http://ex/missing": [{"@default": [{"@value": "d"}]}]}]"#,
        );
        let out = frame_expanded(&input, &frame, &JsonLdOptions::default(), &FrameOptions::default())
            .expect("frame ok");
        let Json::Arr(nodes) = &out else { panic!("array out") };
        assert_eq!(nodes.len(), 1);
        let node = &nodes[0];
        // @explicit keeps p, drops q.
        assert!(node.get("http://ex/p").is_some());
        assert!(node.get("http://ex/q").is_none());
        // @default fill arrives under @preserve.
        assert_eq!(
            node.get("http://ex/missing"),
            Some(&parse(r#"[{"@preserve": [{"@value": "d"}]}]"#))
        );
    }

    /// End-to-end `frame()`: type-match, embed a referenced node once, compact against
    /// the frame's context.
    #[test]
    fn frame_end_to_end_embeds_and_compacts() {
        let input = parse(
            r#"{"@context": {"ex": "http://ex/"},
                "@graph": [
                  {"@id": "ex:a", "@type": "ex:T", "ex:child": {"@id": "ex:b"}},
                  {"@id": "ex:b", "ex:name": "leaf"}
                ]}"#,
        );
        let frame_doc = parse(r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:T"}"#);
        let out = frame(
            &input,
            &frame_doc,
            &JsonLdOptions::default(),
            &FrameOptions::default(),
            &NoopLoader,
        )
        .expect("frame ok");
        // omitGraph (1.1 default): the single match is the bare top-level object.
        assert_eq!(out.get("@id").and_then(Json::as_str), Some("ex:a"));
        let child = out.get("ex:child").expect("embedded child");
        assert_eq!(child.get("ex:name"), Some(&Json::Str("leaf".to_string())));
    }

    /// End-to-end `frame()` with `omitGraph: false` wraps the result under `@graph`.
    #[test]
    fn frame_end_to_end_graph_envelope() {
        let input = parse(r#"{"@id": "http://ex/a", "http://ex/p": "v"}"#);
        let frame_doc = parse("{}");
        let fo = FrameOptions {
            omit_graph: Some(false),
            ..FrameOptions::default()
        };
        let out = frame(
            &input,
            &frame_doc,
            &JsonLdOptions::default(),
            &fo,
            &NoopLoader,
        )
        .expect("frame ok");
        let graph = out.get("@graph").expect("@graph envelope");
        assert!(matches!(graph, Json::Arr(items) if items.len() == 1));
    }

    /// `frame_expanded` prunes the `@id` of a blank node referenced only once
    /// (1.1 `pruneBlankNodeIdentifiers` default) but keeps a shared one.
    #[test]
    fn frame_expanded_prunes_single_use_bnode_ids() {
        let input = parse(
            r#"[{"@id": "http://ex/a", "http://ex/p": [{"http://ex/name": [{"@value": "n"}]}]}]"#,
        );
        // Select ONLY the parent node: the blank node then occurs once (embedded) in the
        // output and its @id is pruned. (An all-matching `{}` frame would also emit the
        // blank node top-level — two output objects — and the label would be kept.)
        let frame = parse(r#"[{"@id": ["http://ex/a"]}]"#);
        let out = frame_expanded(&input, &frame, &JsonLdOptions::default(), &FrameOptions::default())
            .expect("frame ok");
        let text = {
            let mut s = String::new();
            out.write(&mut s);
            s
        };
        assert!(!text.contains("_:b"), "single-use bnode id pruned: {text}");
        // With pruning off the label survives.
        let fo = FrameOptions {
            prune_blank_node_identifiers: Some(false),
            ..FrameOptions::default()
        };
        let kept = frame_expanded(&input, &frame, &JsonLdOptions::default(), &fo).expect("frame ok");
        let mut s = String::new();
        kept.write(&mut s);
        assert!(s.contains("_:b"), "bnode id kept without pruning: {s}");
    }

    /// Nested node patterns over subjects that reference each other are matched once per
    /// subject and pattern, so a deep frame no node satisfies fails fast instead of
    /// searching every path (2^50 here).
    #[test]
    fn nested_patterns_match_in_polynomial_time() {
        let doc = parse(
            r#"[{"@id":"http://ex/a","http://ex/p":[{"@id":"http://ex/a"},{"@id":"http://ex/b"}]},
                {"@id":"http://ex/b","http://ex/p":[{"@id":"http://ex/a"},{"@id":"http://ex/b"}]}]"#,
        );
        let mut pattern = r#"{"@type":"http://ex/Missing"}"#.to_string();
        for _ in 0..50 {
            pattern = format!(r#"{{"http://ex/p":{pattern}}}"#);
        }
        let frame_doc = parse(&pattern);
        let opts = JsonLdOptions::default();
        let started = std::time::Instant::now();
        let out = frame_match(&doc, &frame_doc, &opts, &FrameOptions::default(), &NoopLoader);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let out = out.expect("the frame matches nothing, cleanly");
        let mut text = String::new();
        out.write(&mut text);
        assert!(!text.contains("http://ex/a"), "{text}");
    }


    /// A layered graph where each node references both nodes of the next layer.
    fn layered(layers: usize) -> Json {
        let mut nodes = vec![format!(
            r#"{{"@id":"http://ex/root","http://ex/p":[{{"@id":"http://ex/n0a"}},{{"@id":"http://ex/n0b"}}]}}"#
        )];
        for l in 0..layers {
            for side in ["a", "b"] {
                let next = if l + 1 < layers {
                    format!(
                        r#","http://ex/p":[{{"@id":"http://ex/n{n}a"}},{{"@id":"http://ex/n{n}b"}}]"#,
                        n = l + 1
                    )
                } else {
                    String::new()
                };
                nodes.push(format!(r#"{{"@id":"http://ex/n{l}{side}"{next}}}"#));
            }
        }
        parse(&format!("[{}]", nodes.join(",")))
    }

    /// Frames `input` with `frame`, returning the result or the error code.
    fn frame_with(input: &Json, frame: &str) -> Result<Json, E> {
        frame_match(input, &parse(frame), &JsonLdOptions::default(), &FrameOptions::default(), &NoopLoader)
            .map_err(|e| e.code())
    }

    /// The output bound of a framing call over `input` with `frame`.
    fn bound(input: &Json, frame: &str) -> usize {
        let opts = JsonLdOptions { frame_expansion: true, ..JsonLdOptions::default() };
        let expanded = crate::expand(&parse(frame), &opts, &NoopLoader).expect("frame expands");
        output_bound(json_bytes(&crate::expand(input, &JsonLdOptions::default(), &NoopLoader).unwrap()), &expanded)
    }

    /// `@always` over shared subjects would embed 2^30 copies from 61 nodes; framing
    /// stops at the output bound with `context overflow` instead of exhausting memory.
    #[test]
    fn always_embeds_stop_at_the_output_bound() {
        let frame_doc = parse(r#"{"@id":"http://ex/root","@embed":"@always"}"#);
        let opts = JsonLdOptions::default();
        let started = std::time::Instant::now();
        let err = frame_match(&layered(30), &frame_doc, &opts, &FrameOptions::default(), &NoopLoader)
            .expect_err("exponential output is refused");
        assert_eq!(err.code(), E::ContextOverflow);
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
        // Within the bound the same frame succeeds, and its output stays under it.
        let input = layered(8);
        let out = frame_match(&input, &frame_doc, &opts, &FrameOptions::default(), &NoopLoader)
            .expect("small enough to embed");
        assert!(json_bytes(&out) <= bound(&input, r#"{"@id":"http://ex/root","@embed":"@always"}"#));
        // 18 layers emit about 2^19 nodes, past the bound for so small an input.
        assert_eq!(
            frame_with(&layered(18), r#"{"@id":"http://ex/root","@embed":"@always"}"#).unwrap_err(),
            E::ContextOverflow
        );
    }

    /// Copied strings count by their length: 2^12 embedded copies of a 64 KiB literal
    /// (256 MiB) stop at the output bound.
    #[test]
    fn copied_literals_count_by_their_size() {
        let mut input = layered(11);
        let big = "x".repeat(1 << 16);
        let Json::Arr(nodes) = &mut input else { unreachable!() };
        for node in nodes.iter_mut().filter(|n| n.get("http://ex/p").is_none()) {
            node.set("http://ex/v", Json::Str(big.clone()));
        }
        let started = std::time::Instant::now();
        assert_eq!(
            frame_with(&input, r#"{"@id":"http://ex/root","@embed":"@always"}"#).unwrap_err(),
            E::ContextOverflow
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
    }

    /// A 51-link chain framed with a 50-deep frame embeds every link once, within the bound.
    #[test]
    fn deep_chains_frame_within_the_output_bound() {
        let nodes: Vec<String> = (0..=50)
            .map(|i| format!(r#"{{"@id":"http://ex/c{i}","http://ex/p":{{"@id":"http://ex/c{}"}}}}"#, i + 1))
            .collect();
        let input = parse(&format!("[{}]", nodes.join(",")));
        let mut pattern = "{}".to_string();
        for _ in 0..50 {
            pattern = format!(r#"{{"http://ex/p":{pattern}}}"#);
        }
        let frame_doc = parse(&format!(
            r#"{{"@id":"http://ex/c0","@embed":"@always","http://ex/p":{pattern}}}"#
        ));
        let out = frame_match(&input, &frame_doc, &JsonLdOptions::default(), &FrameOptions::default(), &NoopLoader)
            .expect("frame ok");
        let mut text = String::new();
        out.write(&mut text);
        assert!(text.contains("http://ex/c51"), "{text}");
        assert!(json_bytes(&out) <= bound(&input, &format!(r#"{{"@id":"http://ex/c0","@embed":"@always","http://ex/p":{pattern}}}"#)));
    }

    /// The valid `@id` pattern forms: an IRI, a list of IRIs, the wildcard `{}` (alone or
    /// in a list) and the match-none `[]`.
    #[test]
    fn id_patterns_select_the_listed_nodes() {
        let input = parse(
            r#"[{"@id":"http://ex/a","http://ex/p":"x","http://ex/q":"y"},
                {"@id":"http://ex/b","http://ex/p":"z"}]"#,
        );
        let ids = |frame: &str| -> Vec<String> {
            let out = frame_match(&input, &parse(frame), &JsonLdOptions::default(), &FrameOptions::default(), &NoopLoader)
                .expect("frame ok");
            as_slice(&out).iter().filter_map(|n| n.get("@id").and_then(Json::as_str).map(str::to_string)).collect()
        };
        assert_eq!(ids(r#"{"@id":"http://ex/a"}"#), ["http://ex/a"]);
        assert_eq!(ids(r#"{"@id":["http://ex/b"]}"#), ["http://ex/b"]);
        assert_eq!(ids(r#"{"@id":["http://ex/a","http://ex/b"]}"#), ["http://ex/a", "http://ex/b"]);
        assert_eq!(ids(r#"{"@id":{}}"#), ["http://ex/a", "http://ex/b"]);
        assert_eq!(ids(r#"{"@id":[{}]}"#), ["http://ex/a", "http://ex/b"]);
        assert!(ids(r#"{"@id":[]}"#).is_empty());
        // @id and @type must both match, in either member order and whatever @requireAll says.
        let typed = parse(r#"[{"@id":"http://ex/a","@type":"http://ex/T"},{"@id":"http://ex/b","@type":"http://ex/T"}]"#);
        let typed_ids = |frame: &str| -> Vec<String> {
            let out = frame_with(&typed, frame).expect("frame ok");
            as_slice(&out).iter().filter_map(|n| n.get("@id").and_then(Json::as_str).map(str::to_string)).collect()
        };
        for all in ["false", "true"] {
            for frame in [
                format!(r#"{{"@id":"http://ex/a","@type":"http://ex/Missing","@requireAll":{all}}}"#),
                format!(r#"{{"@type":"http://ex/Missing","@id":"http://ex/a","@requireAll":{all}}}"#),
                format!(r#"{{"@id":"http://ex/a","@type":[],"@requireAll":{all}}}"#),
            ] {
                assert!(typed_ids(&frame).is_empty(), "{frame}");
            }
            for frame in [
                format!(r#"{{"@id":"http://ex/a","@type":"http://ex/T","@requireAll":{all}}}"#),
                format!(r#"{{"@type":"http://ex/T","@id":"http://ex/a","@requireAll":{all}}}"#),
                format!(r#"{{"@type":{{}},"@id":["http://ex/a"],"@requireAll":{all}}}"#),
            ] {
                assert_eq!(typed_ids(&frame), ["http://ex/a"], "{frame}");
            }
        }
        // A wildcard with @explicit keeps only the framed property.
        let out = frame_match(
            &input,
            &parse(r#"{"@id":[{}],"@explicit":true,"http://ex/p":{}}"#),
            &JsonLdOptions::default(),
            &FrameOptions::default(),
            &NoopLoader,
        )
        .expect("frame ok");
        let mut text = String::new();
        out.write(&mut text);
        assert!(text.contains("http://ex/p") && !text.contains("http://ex/q"), "{text}");
    }

    /// Member names count by their length: 2^9 embedded copies of a node whose predicate
    /// is a 64 KiB IRI stop at the output bound, though each copied value is tiny.
    #[test]
    fn copied_member_names_count_by_their_size() {
        let mut input = layered(9);
        let predicate = format!("http://ex/{}", "p".repeat(1 << 16));
        let Json::Arr(nodes) = &mut input else { unreachable!() };
        for node in nodes.iter_mut().filter(|n| n.get("http://ex/p").is_none()) {
            node.set(&predicate, Json::Str("v".to_string()));
        }
        let frame = r#"{"@id":"http://ex/root","@embed":"@always"}"#;
        assert!((1 << 9) * predicate.len() > bound(&input, frame));
        assert_eq!(frame_with(&input, frame).unwrap_err(), E::ContextOverflow);
        // One embed of the same node fits.
        let out = frame_with(&input, r#"{"@id":"http://ex/n8a"}"#).expect("within the bound");
        assert!(json_bytes(&out) <= bound(&input, r#"{"@id":"http://ex/n8a"}"#));
    }

    /// Regrouping fromRdf-shaped documents builds the graph maps node map generation
    /// builds, blank-node labels and their order included: generated documents with
    /// blank and IRI subjects, `@type`s, references, lists and named graphs, plus the W3C
    /// frame inputs (flattened, blank nodes renamed so their order changes) when fetched.
    #[test]
    fn from_rdf_graph_maps_match_node_map_generation() {
        fn check(doc: &Json) {
            for frame_default in [false, true] {
                let expected = build_graph_maps(doc, frame_default);
                let actual = graph_maps_from_rdf(doc.clone(), frame_default);
                let names = |m: &GraphMaps| m.graphs.keys().cloned().collect::<Vec<_>>();
                assert_eq!(names(&actual), names(&expected), "graphs of {}", text(doc));
                for (name, graph) in &expected.graphs {
                    assert_eq!(*actual.graphs[name], **graph, "graph {name} of {}", text(doc));
                }
            }
        }
        fn text(j: &Json) -> String {
            let mut s = String::new();
            j.write(&mut s);
            s
        }
        // A fromRdf-shaped node: sorted subjects, each with `@id`, then `@type` and
        // property arrays of value objects, references and lists.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let name = |i: u64| if i.is_multiple_of(3) { format!("http://ex/n{i}") } else { format!("_:z{}", 97 - i) };
        for _ in 0..300 {
            let mut graphs: Vec<(Option<String>, Vec<String>)> = vec![(None, Vec::new())];
            for _ in 0..next(3) {
                graphs.push((Some(name(next(12))), Vec::new()));
            }
            for (_, nodes) in &mut graphs {
                let mut ids: Vec<String> = (0..1 + next(5)).map(|_| name(next(12))).collect();
                ids.sort();
                ids.dedup();
                for id in ids {
                    let mut members = vec![format!(r#""@id":"{id}""#)];
                    if next(3) == 0 {
                        members.push(format!(r#""@type":["{}"]"#, name(next(12))));
                    }
                    for p in 0..next(3) {
                        let values: Vec<String> = (0..1 + next(3))
                            .map(|_| match next(4) {
                                0 => format!(r#"{{"@value":"v{}"}}"#, next(5)),
                                1 => format!(r#"{{"@list":[{{"@id":"{}"}},{{"@value":"x"}}]}}"#, name(next(12))),
                                _ => format!(r#"{{"@id":"{}"}}"#, name(next(12))),
                            })
                            .collect();
                        // fromRdf emits each value once.
                        let mut seen = BTreeSet::new();
                        let values: Vec<String> = values.into_iter().filter(|v| seen.insert(v.clone())).collect();
                        members.push(format!(r#""http://ex/p{p}":[{}]"#, values.join(",")));
                    }
                    nodes.push(format!("{{{}}}", members.join(",")));
                }
            }
            let mut top: Vec<(String, String)> = Vec::new();
            for (g, nodes) in &graphs {
                match g {
                    None => top.extend(nodes.iter().map(|n| (parse(n).get("@id").and_then(Json::as_str).unwrap().to_string(), n.clone()))),
                    Some(g) => top.push((g.clone(), format!(r#"{{"@id":"{g}","@graph":[{}]}}"#, nodes.join(",")))),
                }
            }
            top.sort();
            top.dedup_by(|a, b| a.0 == b.0);
            check(&parse(&format!("[{}]", top.into_iter().map(|(_, n)| n).collect::<Vec<_>>().join(","))));
        }
        // The W3C frame inputs, when the suite has been fetched.
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/w3c/json-ld-framing/tests/frame");
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.to_string_lossy().ends_with("-in.jsonld") {
                continue;
            }
            let Ok(input) = Json::parse(&std::fs::read_to_string(&path).unwrap()) else { continue };
            let Ok(expanded) = crate::expand(&input, &JsonLdOptions::default(), &NoopLoader) else { continue };
            let mut flat = text(&crate::flatten_expanded(&expanded));
            // Reverse the blank nodes' order, then sort subjects by their new labels.
            for i in (0..64).rev() {
                flat = flat.replace(&format!("\"_:b{i}\""), &format!("\"_:r{}\"", 99 - i));
            }
            let mut doc = parse(&flat);
            fn sort(j: &mut Json) {
                if let Json::Arr(nodes) = j {
                    nodes.sort_by(|a, b| a.get("@id").and_then(Json::as_str).cmp(&b.get("@id").and_then(Json::as_str)));
                    for n in nodes {
                        if let Json::Obj(members) = n {
                            members.iter_mut().filter(|(k, _)| k == "@graph").for_each(|(_, g)| sort(g));
                        }
                    }
                }
            }
            sort(&mut doc);
            check(&doc);
        }
    }

    /// A large numeric `@default` is charged by its length: 1,000 fills of a 64 KiB
    /// number stop at the bound, and 100 render within it.
    #[test]
    fn large_numeric_defaults_count_by_their_length() {
        let number = format!("1{}", "0".repeat(1 << 16));
        let frame = format!(r#"{{"http://ex/p":{{}},"http://ex/missing":{{"@default":{number}}}}}"#);
        let subjects = |n: usize| {
            let nodes: Vec<String> = (0..n).map(|i| format!(r#"{{"@id":"http://ex/s{i}","http://ex/p":"v"}}"#)).collect();
            parse(&format!("[{}]", nodes.join(",")))
        };
        assert_eq!(frame_with(&subjects(1000), &frame).unwrap_err(), E::ContextOverflow);
        let input = subjects(100);
        let out = frame_with(&input, &frame).expect("within the bound");
        let mut text = String::new();
        out.write(&mut text);
        assert!(text.len() > 100 << 16 && text.len() <= bound(&input, &frame), "{}", text.len());
    }

    /// Nested property frames are borrowed, not copied per level: a 55-deep frame over a
    /// 56-link chain ending in a 1 MiB default frames within the bound.
    #[test]
    fn nested_subframes_are_not_copied() {
        let nodes: Vec<String> = (0..=55)
            .map(|i| format!(r#"{{"@id":"http://ex/c{i}","http://ex/p":{{"@id":"http://ex/c{}"}}}}"#, i + 1))
            .collect();
        let input = parse(&format!("[{}]", nodes.join(",")));
        let mut pattern = format!(r#"{{"http://ex/p":{{}},"http://ex/q":{{"@default":"{}"}}}}"#, "x".repeat(1 << 20));
        for _ in 0..54 {
            pattern = format!(r#"{{"http://ex/p":{pattern}}}"#);
        }
        let frame = format!(r#"{{"@id":"http://ex/c0","http://ex/p":{pattern}}}"#);
        let out = frame_with(&input, &frame).expect("frame ok");
        let mut text = String::new();
        out.write(&mut text);
        assert!(text.contains("http://ex/c56") && text.len() <= bound(&input, &frame), "{}", &text[..text.len().min(300)]);
    }
}
