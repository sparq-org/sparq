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
use crate::node_map::generate_node_map;
use crate::options::{JsonLdOptions, ProcessingMode};
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
    // §4.1 steps 2–3: expand the input (ordinary mode) and the frame (frameExpansion).
    let mut in_opts = options.clone();
    in_opts.frame_expansion = false;
    let expanded_input = expand(input, &in_opts, loader)?;
    let mut fr_opts = options.clone();
    fr_opts.frame_expansion = true;
    let expanded_frame = expand(frame_doc, &fr_opts, loader)?;

    // §4.1 step 4: a top-level `@graph` entry in the frame DOCUMENT selects the
    // default graph instead of the merged graph (the suite's t0047 posture — the
    // reference processors read the unexpanded frame for this).
    let mut fopts = frame_options.clone();
    if frame_doc.get("@graph").is_some() {
        fopts.frame_default = true;
    }

    // §4.1 steps 4–7: frame the expanded input (returns the pruned expanded output).
    let framed = frame_expanded(&expanded_input, &expanded_frame, options, &fopts)?;
    let framed_len = match &framed {
        Json::Arr(items) => items.len(),
        _ => 1,
    };

    // §4.1 step 8: compact against the frame's @context (the caller-facing envelope).
    let ctx_value = frame_doc.get("@context").cloned().unwrap_or_default();
    let compacted = compact_expanded(&framed, &ctx_value, options, loader)?;

    // Framing post-processing: unwrap @preserve (with @null → null), then apply the
    // omitGraph shaping.
    let mut result = cleanup_preserve(compacted, options.compact_arrays);
    if !frame_options.omit_graph(options.processing_mode) {
        result = ensure_graph_envelope(result, &ctx_value, framed_len);
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
    // Normalise the expanded frame to a one-object array (an empty frame `{}` expands to
    // `[]`; re-materialise the match-everything object).
    let frame_arr = match expanded_frame {
        Json::Arr(items) if items.is_empty() => Json::Arr(vec![Json::obj()]),
        Json::Arr(_) => expanded_frame.clone(),
        other => Json::Arr(vec![other.clone()]),
    };

    // §4.1 step 4: the graph map (+ @merged unless frameDefault).
    let maps = build_graph_maps(expanded_input, frame_options.frame_default);
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
    };

    // §4.1 steps 5–6: match the frame over every subject of the active graph.
    let subjects: Vec<String> = maps
        .graphs
        .get(graph)
        .map(|g| g.keys().cloned().collect())
        .unwrap_or_default();
    let mut framed = Json::Arr(Vec::new());
    match_frame(&maps, &mut st, &subjects, &frame_arr, &mut framed, None, false)?;

    // `@embed: @last`: every occurrence embedded above; keep only the LAST embed of
    // each such id per top-level match (earlier ones demote to node references) —
    // the reference processors' remove-embed, without shared-pointer mutation.
    if !st.last_ids.is_empty() {
        if let Json::Arr(top) = &mut framed {
            for element in top {
                for id in &st.last_ids {
                    demote_all_but_last_embed(element, id);
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
    graphs: BTreeMap<String, Subjects>,
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
    let nm = generate_node_map(expanded_input);
    let mut graphs: BTreeMap<String, Subjects> = BTreeMap::new();
    for name in nm.graph_names() {
        let Some(g) = nm.graph(name) else { continue };
        let mut subjects = Subjects::new();
        for s in g.subjects() {
            if let Some(node) = g.get(s) {
                subjects.insert(s.to_string(), node.clone());
            }
        }
        graphs.insert(name.to_string(), subjects);
    }
    if !frame_default {
        let merged = merge_graphs(&graphs);
        graphs.insert("@merged".to_string(), merged);
    }
    GraphMaps { graphs }
}

/// **Merge Node Maps** (JSON-LD 1.1 API §7.3): fold every graph's node objects into one
/// `@merged` subject map — non-`@type` keywords copied, `@type` and ordinary property
/// values merged without duplicates.
fn merge_graphs(graphs: &BTreeMap<String, Subjects>) -> Subjects {
    let mut merged = Subjects::new();
    for subjects in graphs.values() {
        for (id, node) in subjects {
            let entry = merged
                .entry(id.clone())
                .or_insert_with(|| Json::Obj(vec![("@id".to_string(), Json::Str(id.clone()))]));
            let Json::Obj(members) = node else { continue };
            for (prop, value) in members {
                if prop.starts_with('@') && prop != "@type" {
                    if prop != "@id" {
                        entry.set(prop, value.clone());
                    }
                } else {
                    for v in as_slice(value) {
                        add_unique(entry, prop, v.clone());
                    }
                }
            }
        }
    }
    merged
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
    parent: &mut Json,
    property: Option<&str>,
    embedded: bool,
) -> Result<(), JsonLdError> {
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

        let Some(subject) = maps.graphs.get(&st.graph).and_then(|g| g.get(&id)).cloned() else {
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

        let mut output = Json::Obj(vec![("@id".to_string(), Json::Str(id.clone()))]);
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
                add_frame_output(parent, property, output);
                continue;
            }
            if flags.embed == Embed::Once
                && st
                    .unique_embeds
                    .get(&st.graph)
                    .map(|e| e.contains(&id))
                    .unwrap_or(false)
            {
                add_frame_output(parent, property, output);
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
            let (recurse, subframe) = match frame_obj.get("@graph") {
                None => (st.graph != "@merged", Json::obj()),
                Some(gf) => {
                    let sf = match first_of(gf) {
                        Some(f) if f.is_obj() => f.clone(),
                        _ => Json::obj(),
                    };
                    (id != "@merged" && id != "@default", sf)
                }
            };
            if recurse {
                let prev = std::mem::replace(&mut st.graph, id.clone());
                let graph_subjects: Vec<String> = maps
                    .graphs
                    .get(&id)
                    .map(|g| g.keys().cloned().collect())
                    .unwrap_or_default();
                match_frame(
                    maps,
                    st,
                    &graph_subjects,
                    &Json::Arr(vec![subframe]),
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
        let Json::Obj(subject_members) = &subject else {
            st.subject_stack.pop();
            continue;
        };
        let mut props: Vec<&(String, Json)> = subject_members.iter().collect();
        props.sort_by(|a, b| a.0.cmp(&b.0));
        for (prop, objects) in props {
            if prop == "@id" {
                continue;
            }
            if prop.starts_with('@') {
                // Keywords copy verbatim; blank-node @type values count toward pruning.
                output.set(prop, objects.clone());
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
                    let subframe = frame_prop
                        .and_then(first_of)
                        .and_then(|f0| f0.get("@list"))
                        .cloned()
                        .unwrap_or_else(|| implicit_frame(&flags));
                    let mut list = Json::Obj(vec![("@list".to_string(), Json::Arr(vec![]))]);
                    for oo in o.get("@list").map(as_slice).unwrap_or_default() {
                        if let Some(oid) = subject_reference_id(oo) {
                            match_frame(
                                maps,
                                st,
                                &[oid.to_string()],
                                &subframe,
                                &mut list,
                                Some("@list"),
                                true,
                            )?;
                        } else {
                            add_frame_output(&mut list, Some("@list"), oo.clone());
                        }
                    }
                    add_frame_output(&mut output, Some(prop), list);
                } else if let Some(oid) = subject_reference_id(o) {
                    // Node reference: recurse with the property sub-frame (or the
                    // implicit frame inheriting the current flags).
                    let subframe = frame_prop
                        .cloned()
                        .unwrap_or_else(|| implicit_frame(&flags));
                    match_frame(
                        maps,
                        st,
                        &[oid.to_string()],
                        &subframe,
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
                        add_frame_output(&mut output, Some(prop), o.clone());
                    }
                }
            }
        }

        // @default fill: frame properties absent from the output get their @default
        // value (or @null), wrapped under @preserve so compaction keeps them.
        let Json::Obj(frame_members) = &frame_obj else {
            unreachable!("validate_frame returns an object")
        };
        let mut fprops: Vec<&(String, Json)> = frame_members.iter().collect();
        fprops.sort_by(|a, b| a.0.cmp(&b.0));
        for (prop, pvalue) in fprops {
            let next = first_of(pvalue).cloned().unwrap_or_default();
            if prop == "@type" {
                // Only the `@type: {"@default": …}` form participates in default fill.
                if next.get("@default").is_none() {
                    continue;
                }
            } else if prop.starts_with('@') {
                continue;
            }
            let omit_default = frame_flag_bool(&next, "@omitDefault", st.options.omit_default);
            if !omit_default && output.get(prop).is_none() {
                let preserve = match next.get("@default") {
                    Some(d) => Json::Arr(as_slice(d).into_iter().cloned().collect()),
                    None => Json::Arr(vec![Json::Str("@null".to_string())]),
                };
                if prop == "@type" {
                    // A `@type` default fills DIRECTLY (its values are IRIs that must
                    // go through IRI compaction; `@preserve` would shield them).
                    output.set(prop, preserve);
                } else {
                    output.set(
                        prop,
                        Json::Arr(vec![Json::Obj(vec![("@preserve".to_string(), preserve)])]),
                    );
                }
            }
        }

        // @reverse framing: find nodes of the active graph referencing this subject
        // under the reverse property and frame them under output's @reverse map.
        if let Some(Json::Obj(rev)) = frame_obj.get("@reverse") {
            let mut rprops: Vec<&(String, Json)> = rev.iter().collect();
            rprops.sort_by(|a, b| a.0.cmp(&b.0));
            for (reverse_prop, subframe) in rprops {
                let referrers: Vec<String> = maps
                    .graphs
                    .get(&st.graph)
                    .map(|g| {
                        g.iter()
                            .filter(|(_, node)| {
                                node.get(reverse_prop).map(as_slice).unwrap_or_default().iter().any(
                                    |v| v.get("@id").and_then(Json::as_str) == Some(id.as_str()),
                                )
                            })
                            .map(|(sid, _)| sid.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for sid in referrers {
                    let mut tmp = Json::Arr(Vec::new());
                    match_frame(
                        maps,
                        st,
                        &[sid],
                        subframe,
                        &mut tmp,
                        Some(reverse_prop),
                        embedded,
                    )?;
                    if let Json::Arr(items) = tmp {
                        for item in items {
                            add_reverse_output(&mut output, reverse_prop, item);
                        }
                    }
                }
            }
        }

        add_frame_output(parent, property, output);
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
fn validate_frame(frame: &Json) -> Result<Json, JsonLdError> {
    let obj = match frame {
        Json::Obj(_) => frame.clone(),
        Json::Arr(items) if items.len() == 1 && items[0].is_obj() => items[0].clone(),
        Json::Arr(items) if items.is_empty() => Json::obj(),
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
    for id in sorted {
        if let Some(node) = graph_subjects.get(id) {
            if filter_subject(graph_subjects, node, frame_obj, flags)? {
                out.push(id.clone());
            }
        }
    }
    Ok(out)
}

/// **Frame Matching** for one node (§4.3): duck-type the node against the frame's
/// `@id` / `@type` / property patterns. With `@requireAll`, every frame entry must
/// match; otherwise any match (or a wildcard frame) suffices.
fn filter_subject(
    graph_subjects: &Subjects,
    node: &Json,
    frame_obj: &Json,
    flags: &Flags,
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

        if key == "@id" {
            if is_empty || frame_values.first().map(is_empty_obj).unwrap_or(false) {
                match_this = true;
            } else {
                let nid = node.get("@id").and_then(Json::as_str);
                match_this = frame_values.iter().any(|v| v.as_str() == nid);
            }
            if !flags.require_all {
                return Ok(match_this);
            }
        } else if key == "@type" {
            wildcard = false;
            if is_empty {
                // Match-none: the node must have no @type.
                if !node_values.is_empty() {
                    return Ok(false);
                }
                match_this = true;
            } else if frame_values.len() == 1 && is_empty_obj(&frame_values[0]) {
                // Wildcard: the node must have some @type.
                match_this = !node_values.is_empty();
            } else if frame_values
                .first()
                .map(|f| f.get("@default").is_some())
                .unwrap_or(false)
            {
                // A default-map @type matches any node.
                match_this = true;
            } else {
                match_this = frame_values
                    .iter()
                    .any(|t| node_values.iter().any(|nv| nv == t));
            }
            if !flags.require_all {
                return Ok(match_this);
            }
        } else if key.starts_with('@') {
            // Other keywords do not participate in matching.
            continue;
        } else {
            let this_frame = frame_values.first();
            let mut has_default = false;
            if let Some(tf) = this_frame {
                validate_frame(&Json::Arr(vec![(*tf).clone()]))?;
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
                        Some(lp) => node_list
                            .iter()
                            .any(|lv| node_match(graph_subjects, lp, lv, flags).unwrap_or(false)),
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
                        match_this = node_values
                            .iter()
                            .any(|nv| node_match(graph_subjects, tf, nv, flags).unwrap_or(false));
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

/// **Node Match**: a node-object value matches a node pattern iff it references a
/// subject of the active graph that itself matches the pattern (§4.3 step 2.5).
fn node_match(
    graph_subjects: &Subjects,
    pattern: &Json,
    value: &Json,
    flags: &Flags,
) -> Result<bool, JsonLdError> {
    let Some(id) = value.get("@id").and_then(Json::as_str) else {
        return Ok(false);
    };
    let Some(node) = graph_subjects.get(id) else {
        return Ok(false);
    };
    filter_subject(graph_subjects, node, pattern, flags)
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
        || v1.map(|v| v2.contains(&v)).unwrap_or(false);
    if !v_ok {
        return false;
    }
    // @type: absent-and-unconstrained, in the alternatives, or wildcard (with a type).
    let t_ok = (t2.is_empty() && t1.is_none())
        || t2.iter().any(|p| p.as_str() == t1)
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

/// Add a framed `output` to `parent`: appended to an array parent, or merged into the
/// `property` array of an object parent (§4.2 "add output to parent").
fn add_frame_output(parent: &mut Json, property: Option<&str>, output: Json) {
    match parent {
        Json::Arr(items) => items.push(output),
        Json::Obj(_) => {
            if let Some(prop) = property {
                // Duplicates are retained (the reference processors add with
                // allowDuplicate — a @list legitimately repeats values).
                append_value(parent, prop, output);
            }
        }
        _ => {}
    }
}

/// Merge one framed referrer into `output`'s `@reverse` map under `reverse_prop`.
fn add_reverse_output(output: &mut Json, reverse_prop: &str, item: Json) {
    if output.get("@reverse").is_none() {
        output.set("@reverse", Json::obj());
    }
    if let Some(rev) = output_member_mut(output, "@reverse") {
        append_value(rev, reverse_prop, item);
    }
}

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

/// Append `value` to the array member `prop` of `obj` (creating it), skipping an exact
/// duplicate of an existing entry (the Merge Node Maps "without duplicates" rule).
fn add_unique(obj: &mut Json, prop: &str, value: Json) {
    if obj.get(prop).is_none() {
        obj.set(prop, Json::Arr(Vec::new()));
    }
    if let Some(Json::Arr(items)) = output_member_mut(obj, prop) {
        if !items.contains(&value) {
            items.push(value);
        }
    }
}

/// Append `value` to the array member `prop` of `obj` (creating it), retaining
/// duplicates (framing's add-output rule).
fn append_value(obj: &mut Json, prop: &str, value: Json) {
    if obj.get(prop).is_none() {
        obj.set(prop, Json::Arr(Vec::new()));
    }
    if let Some(Json::Arr(items)) = output_member_mut(obj, prop) {
        items.push(value);
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
                !(k == "@id" && matches!(v, Json::Str(s) if to_clear.contains(s)))
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
fn as_slice(v: &Json) -> Vec<&Json> {
    match v {
        Json::Arr(items) => items.iter().collect(),
        other => vec![other],
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
fn is_empty_obj(v: &&Json) -> bool {
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
}
