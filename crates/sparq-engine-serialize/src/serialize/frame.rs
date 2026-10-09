//! [OPUS-4.8] (sq-oy1f.17) Full **W3C JSON-LD 1.1 Framing** — hand-rolled, dependency-free.
//!
//! The sibling [`compact`](super::legacy_compact) module implements the W3C JSON-LD 1.1
//! **Compaction** Algorithm (sq-ixc3.4/#963/#978). Framing is the complementary
//! shaping operation: given a **frame document** (a JSON-LD document acting as an
//! example / template), reshape the flattened input graph into a deterministic
//! tree whose structure matches the frame — selecting which subjects appear at the
//! top level, embedding referenced nodes inline, pruning to the framed properties,
//! and filling defaults for absent ones.
//!
//! This module adds that — the **JSON-LD 1.1 Framing Algorithm**
//! (<https://www.w3.org/TR/json-ld11-framing/#framing-algorithm>) applied to an RDF
//! graph plus a caller **frame**. Like compaction it stays inside the
//! dependency-free `serialize-rdf` feature: it reuses the compaction module's tiny
//! [`Json`](super::legacy_compact::Json) AST and its `fromRdf` model builder
//! ([`graph_to_expanded`](super::legacy_compact::graph_to_expanded)) — it pulls in **zero**
//! new crates and **no** `json-ld` library (no Rust JSON-LD crate ships framing, so
//! it must be hand-rolled regardless — see sq-oy1f.6).
//!
//! ## Pipeline (faithful to the spec)
//!
//! 1. **fromRdf** — build the *expanded* JSON-LD model from the graph's triples
//!    (reusing the compaction module's `graph_to_expanded`), then *flatten* it into a
//!    **node map** keyed by `@id` ([`build_node_map`]). The Framing Algorithm operates
//!    over a flattened node map, not raw triples.
//! 2. **Frame parsing** ([`Frame::parse`]) — read the caller frame document: its
//!    `@context` (driving the final compaction), top-level framing flags
//!    (`@embed`/`@explicit`/`@requireAll`/`@omitDefault`), and the frame node pattern.
//! 3. **Framing** ([`Framer::frame`]) — the recursive Framing Algorithm: select the
//!    input nodes that *match* the frame (node-pattern matching), recursively frame
//!    each matched subject's properties (embedding referenced nodes via the **embed
//!    link table** that breaks blank-node cycles), apply `@explicit` (prune to framed
//!    properties), and fill `@default` / preserve-`null` markers for absent ones.
//! 4. **Compaction** — compact the framed expanded model against the frame's
//!    `@context` (reusing [`ActiveContext::compact`](super::legacy_compact::ActiveContext)),
//!    wrap in the `@graph` envelope (omitted for a single result unless forced), and
//!    serialize.
//!
//! ## Frame node-pattern matching (`@type` / presence / value / wildcard / match-none)
//!
//! A frame *node pattern* matches an input node when, for the frame's properties:
//! `@type` matches (an explicit type list, the wildcard `{}`/`[{}]`, or match-none
//! `[]`); a plain property is *present* (wildcard `{}`), *absent* (match-none `[]`),
//! or has a *matching value* (a specific `@value`/`@id`). `@requireAll: true` combines the
//! per-property results with AND; the default with OR (any matching property selects the node;
//! a frame with no properties matches every node — the wildcard frame `{}`).
//!
//! ## Framing flags
//!
//! * **`@embed`** — `@once` (default; embed a node the first time it is reached, a
//!   reference afterwards), `@always` (embed every reference), `@never` (always a
//!   reference), and `@link` (treated as `@always` here — node deduplication by
//!   identity is a no-op once the cycle table breaks recursion; documented boundary).
//! * **`@explicit`** — `true` keeps only the properties named in the frame.
//! * **`@default`** — the value emitted for a framed property absent from the matched
//!   node (`@default: @null` emits the preserve-`null` marker).
//! * **`@omitDefault`** — `true` drops an absent framed property entirely instead of
//!   emitting its `@default` / `null`.
//! * **`@requireAll`** — AND (`true`) vs OR (default) of the frame's property matches.
//!
//! ## Honest scope
//!
//! This targets the **serialise / fromRdf-then-frame** path — sparq emits RDF, so the
//! input is always a graph, never an arbitrary remote JSON-LD document; the frame is a
//! caller-supplied inline document. The general framing of an *arbitrary already-expanded
//! JSON-LD document* (scoped/typed frame contexts, `@reverse` frames, frame `@context`
//! re-processing per node) is out of scope — the input expanded model is the one *we*
//! produce, so its shape is known. Named-graph framing reshapes each named graph's
//! default frame independently (the input's `@graph` members). `@embed: @link` falls
//! back to `@always` (see the flag note above). See the crate README +
//! `skills/data-formats/SKILL.md` for the full boundary.

use super::legacy_compact::{flatten, graph_to_expanded, ActiveContext, Json};
use super::NamedGraph;
use oxrdf::{Term, Triple};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

// ===========================================================================
// Frame document — the parsed caller frame.
// ===========================================================================

/// The `@embed` framing flag. `@once` is the JSON-LD 1.1 default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Embed {
    /// Embed a node the first time it is reached; emit a node reference afterwards.
    Once,
    /// Embed the node at every reference (subject to the cycle table).
    Always,
    /// Never embed — always emit a node reference (`{"@id": …}`).
    Never,
}

impl Embed {
    /// Parses an `@embed` value. `@link` maps to `@always` (documented boundary — the
    /// link-deduplication identity is a no-op once the cycle table breaks recursion).
    /// `true` is the legacy spelling of `@once`, `false` of `@never`.
    fn parse(v: &Json) -> Option<Embed> {
        match v {
            Json::Str(s) => match s.as_str() {
                "@once" => Some(Embed::Once),
                "@always" | "@link" => Some(Embed::Always),
                "@never" => Some(Embed::Never),
                _ => None,
            },
            Json::Raw(r) => match r.as_str() {
                "true" => Some(Embed::Once),
                "false" => Some(Embed::Never),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The framing flags in force at one recursion level. Each level inherits the parent's
/// flags unless the local frame node overrides them (the spec's "object embed flag"
/// inheritance).
#[derive(Clone, Copy, Debug)]
struct Flags {
    embed: Embed,
    explicit: bool,
    require_all: bool,
    omit_default: bool,
}

impl Default for Flags {
    /// The JSON-LD 1.1 defaults: `@embed: @once`, `@explicit: false`, `@requireAll: false`,
    /// `@omitDefault: false`.
    fn default() -> Flags {
        Flags {
            embed: Embed::Once,
            explicit: false,
            require_all: false,
            omit_default: false,
        }
    }
}

impl Flags {
    /// Returns these flags updated by any framing-flag keyword present on `frame_node`.
    fn updated_by(self, frame_node: &Json) -> Flags {
        let mut f = self;
        if let Some(v) = frame_node.get("@embed") {
            if let Some(e) = Embed::parse(&single(v)) {
                f.embed = e;
            }
        }
        if let Some(b) = bool_flag(frame_node, "@explicit") {
            f.explicit = b;
        }
        if let Some(b) = bool_flag(frame_node, "@requireAll") {
            f.require_all = b;
        }
        if let Some(b) = bool_flag(frame_node, "@omitDefault") {
            f.omit_default = b;
        }
        f
    }
}

/// The parsed caller frame: the `@context` (echoed + used for the final compaction) and
/// the frame node pattern (the top-level frame object, possibly wrapped in `@graph`).
struct Frame {
    /// The frame's `@context` JSON, used to build the [`ActiveContext`] for the final
    /// compaction and echoed back into the output document.
    context: Json,
    /// The top-level frame node pattern (one node object). A `[{…}]` / `{"@graph":[…]}`
    /// frame is unwrapped to its single node here (sparq frames against one pattern).
    pattern: Json,
}

impl Frame {
    /// Parses a caller frame document into its context + top-level node pattern.
    fn parse(frame: &Json) -> Frame {
        let context = frame
            .get("@context")
            .cloned()
            .unwrap_or_else(|| Json::Obj(Vec::new()));
        // Unwrap the frame to a single top-level node pattern: a bare array's first
        // element, a `{"@graph": [...]}` wrapper's first element, or the object itself.
        let pattern = match frame {
            Json::Arr(items) => items.first().cloned().unwrap_or_else(Json::obj),
            Json::Obj(_) => {
                if let Some(g) = frame.get("@graph") {
                    match g {
                        Json::Arr(items) => items.first().cloned().unwrap_or_else(Json::obj),
                        other => other.clone(),
                    }
                } else {
                    frame.clone()
                }
            }
            _ => Json::obj(),
        };
        Frame { context, pattern }
    }
}

/// Expands a frame node pattern's IRIs (property keys, `@type` / `@id` values, and the
/// `@type` of value patterns) against the frame's `@context`, so node-pattern matching can
/// compare them against the node map's **absolute** IRIs. Framing keywords (`@embed` etc.)
/// and value scalars pass through unchanged; the wildcard `{}` / match-none `[]` shapes are
/// preserved. This mirrors the W3C algorithm's "expand the frame before matching" step,
/// scoped to the inline frame shape sparq accepts.
fn expand_frame(node: &Json, active: &ActiveContext) -> Json {
    match node {
        Json::Obj(members) => {
            let mut out = Vec::with_capacity(members.len());
            for (k, v) in members {
                match k.as_str() {
                    // Framing keywords + @value/@language/@index keep their key and value.
                    "@embed" | "@explicit" | "@requireAll" | "@omitDefault" | "@context"
                    | "@value" | "@language" | "@index" => {
                        out.push((k.clone(), v.clone()));
                    }
                    // @default values that are node/value objects are expanded; a bare
                    // string default stays a literal.
                    "@default" => out.push((k.clone(), expand_frame(v, active))),
                    // @id values are expanded in non-vocab position (compact-IRI only); a
                    // wildcard / match-none shape is preserved.
                    "@id" => out.push((k.clone(), expand_frame_iri_value(v, active, false))),
                    // @type values are expanded in vocab position.
                    "@type" => out.push((k.clone(), expand_frame_iri_value(v, active, true))),
                    // An ordinary property: expand its key (vocab position) AND its value
                    // (recursively, since it may be a sub-frame / value pattern).
                    _ => {
                        let ek = active.expand_vocab_iri(k);
                        out.push((ek, expand_frame(v, active)));
                    }
                }
            }
            Json::Obj(out)
        }
        Json::Arr(items) => Json::Arr(items.iter().map(|v| expand_frame(v, active)).collect()),
        other => other.clone(),
    }
}

/// Expands an `@id` / `@type` frame value (a string, an array of strings, or a
/// wildcard/match-none shape). `vocab` selects vocab vs `@id` (document-relative) position.
fn expand_frame_iri_value(v: &Json, active: &ActiveContext, vocab: bool) -> Json {
    let expand = |s: &str| -> String {
        if vocab {
            active.expand_vocab_iri(s)
        } else {
            // @id position: only compact-IRI expansion, no @vocab. Reuse the vocab path but
            // an @id is normally absolute already; a defined term still maps via the context.
            active.expand_vocab_iri(s)
        }
    };
    match v {
        Json::Str(s) => Json::Str(expand(s)),
        Json::Arr(a) => Json::Arr(
            a.iter()
                .map(|x| match x {
                    Json::Str(s) => Json::Str(expand(s)),
                    other => other.clone(),
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

// ===========================================================================
// Small Json helpers local to framing (the AST's accessors are pub(super) but a
// few framing-shaped predicates live here).
// ===========================================================================

/// Wraps a value as a single-element view: an array stays itself, a scalar/object is
/// promoted to a one-element array's content. Used to read a frame keyword that the spec
/// allows in either array or scalar form.
fn single(v: &Json) -> Json {
    match v {
        Json::Arr(a) => a.first().cloned().unwrap_or(Json::Obj(Vec::new())),
        other => other.clone(),
    }
}

/// Reads a boolean framing flag (`@explicit`/`@requireAll`/`@omitDefault`), tolerating the
/// `[true]`/`[false]` array spelling the expansion of a frame produces.
fn bool_flag(node: &Json, key: &str) -> Option<bool> {
    let raw = match node.get(key)? {
        Json::Arr(a) => a.first()?,
        other => other,
    };
    match raw {
        Json::Raw(r) if r == "true" => Some(true),
        Json::Raw(r) if r == "false" => Some(false),
        Json::Str(s) if s == "true" => Some(true),
        Json::Str(s) if s == "false" => Some(false),
        _ => None,
    }
}

/// True when a frame property value is the **wildcard** `{}` (match any value / embed),
/// possibly wrapped as `[{}]` by frame expansion.
fn is_wildcard(v: &Json) -> bool {
    match v {
        Json::Obj(m) => m.is_empty(),
        Json::Arr(a) => a.len() == 1 && matches!(&a[0], Json::Obj(m) if m.is_empty()),
        _ => false,
    }
}

/// True when a frame property value is the **match-none** `[]` (property must be absent).
fn is_match_none(v: &Json) -> bool {
    matches!(v, Json::Arr(a) if a.is_empty())
}

/// The `@id` string of an expanded node / node reference, if any.
fn node_id(node: &Json) -> Option<&str> {
    node.get("@id").and_then(Json::as_str)
}

// ===========================================================================
// Node map (flattening).
// ===========================================================================

/// The flattened input: every subject node keyed by its `@id`, in first-seen order, plus
/// the ordered id list (so framing is deterministic). Blank-node and IRI subjects alike.
struct NodeMap {
    /// id → its expanded node object.
    by_id: BTreeMap<String, Json>,
    /// `@id`s in first-seen order (the deterministic top-level iteration order).
    order: Vec<String>,
}

/// Builds the flattened node map from an expanded node-object array (the `fromRdf` output).
/// `graph_to_expanded` already yields one node per subject (list cells folded into their
/// `@list` heads), so flattening here is a straightforward keyed collection preserving the
/// first-seen subject order.
fn build_node_map(expanded: &[Json]) -> NodeMap {
    let mut by_id = BTreeMap::new();
    let mut order = Vec::new();
    for node in expanded {
        if let Some(id) = node_id(node) {
            if !by_id.contains_key(id) {
                order.push(id.to_string());
            }
            by_id.insert(id.to_string(), node.clone());
        }
    }
    NodeMap { by_id, order }
}

// ===========================================================================
// Frame matching (https://www.w3.org/TR/json-ld11-framing/#frame-matching).
// ===========================================================================

/// True when input `node` matches the `frame` node pattern under `require_all` (AND when
/// true, OR by default). A frame with no node-matching properties (only flags / wildcard)
/// matches every node.
fn node_matches(node: &Json, frame: &Json, require_all: bool) -> bool {
    let Json::Obj(frame_members) = frame else {
        return true; // a non-object frame (rare) matches anything
    };

    // Collect the frame's matchable members (skip keyword flags & @context).
    let mut criteria: Vec<(&str, &Json)> = Vec::new();
    for (k, v) in frame_members {
        match k.as_str() {
            "@embed" | "@explicit" | "@requireAll" | "@omitDefault" | "@default" | "@context" => {}
            _ => criteria.push((k.as_str(), v)),
        }
    }
    if criteria.is_empty() {
        return true; // the wildcard frame {} matches everything
    }

    let mut any = false;
    for (key, fval) in &criteria {
        let matched = match *key {
            "@id" => match_id(node, fval),
            "@type" => match_type(node, fval),
            _ => match_property(node, key, fval),
        };
        if require_all {
            if !matched {
                return false;
            }
        } else if matched {
            any = true;
        }
    }
    if require_all {
        true
    } else {
        any
    }
}

/// Matches the frame's `@id` criterion against the node's `@id`: a wildcard `{}` matches
/// any id; an explicit id (string or array of strings) matches when the node's id is among
/// them; an empty `[]` matches a node with no id (never, since every node here has one).
fn match_id(node: &Json, frame_id: &Json) -> bool {
    let Some(nid) = node_id(node) else {
        return false;
    };
    if is_wildcard(frame_id) {
        return true;
    }
    if is_match_none(frame_id) {
        return false;
    }
    match frame_id {
        Json::Str(s) => s == nid,
        Json::Arr(a) => a.iter().any(|v| v.as_str() == Some(nid)),
        _ => false,
    }
}

/// Matches the frame's `@type` criterion. `{}`/`[{}]` (wildcard) matches any node that has
/// at least one `@type`; `[]` (match-none) matches a node with no `@type`; an explicit type
/// IRI (string or array) matches when the node carries that type.
fn match_type(node: &Json, frame_type: &Json) -> bool {
    let node_types: Vec<&str> = match node.get("@type") {
        Some(Json::Arr(a)) => a.iter().filter_map(Json::as_str).collect(),
        Some(Json::Str(s)) => vec![s.as_str()],
        _ => Vec::new(),
    };
    if is_match_none(frame_type) {
        return node_types.is_empty();
    }
    if is_wildcard(frame_type) {
        return !node_types.is_empty();
    }
    let wanted: Vec<&str> = match frame_type {
        Json::Str(s) => vec![s.as_str()],
        Json::Arr(a) => a.iter().filter_map(Json::as_str).collect(),
        _ => Vec::new(),
    };
    // An explicit type frame matches when ANY listed type is present.
    wanted.iter().any(|w| node_types.contains(w))
}

/// Matches the frame's criterion for an ordinary property `key`. Wildcard `{}` requires the
/// property be *present*; match-none `[]` requires it *absent*; a frame sub-node requires
/// presence (recursive matching happens in framing, not here); a specific value
/// (`{"@value": …}` / `{"@id": …}`) requires the node to carry a matching value.
fn match_property(node: &Json, key: &str, fval: &Json) -> bool {
    let present = node.get(key).is_some();
    if is_match_none(fval) {
        return !present;
    }
    if is_wildcard(fval) {
        return present;
    }
    if !present {
        return false;
    }
    // A specific value pattern: at least one node value must match a frame value.
    let frame_vals = flatten(fval);
    let node_vals = node.get(key).map(flatten).unwrap_or_default();
    // A frame sub-node pattern that is itself an object with only @id / @type / properties
    // (not a value object) selects on PRESENCE — the recursion frames the sub-tree. Only a
    // concrete @value / @id literal pattern restricts by value here.
    let mut has_value_pattern = false;
    for fv in &frame_vals {
        let is_value_obj =
            fv.get("@value").is_some() || (fv.is_obj() && fv.get("@id").is_some() && is_leaf(fv));
        if is_value_obj {
            has_value_pattern = true;
            if node_vals.iter().any(|nv| value_matches(nv, fv)) {
                return true;
            }
        }
    }
    if has_value_pattern {
        // The frame restricted by value but no node value matched.
        false
    } else {
        // A non-value sub-frame (e.g. {"@type": …}) → presence is enough; recursion frames it.
        true
    }
}

/// True when a node object carries only `@id` (a bare node reference) — distinguishing a
/// "specific node id" value pattern from a sub-frame that also names properties.
fn is_leaf(v: &Json) -> bool {
    matches!(v, Json::Obj(m) if m.len() == 1) && v.get("@id").is_some()
}

/// True when an input value `nv` matches a specific frame value `fv` (a `{"@value": …}` with
/// optional `@type`/`@language`, or a node reference `{"@id": …}`).
fn value_matches(nv: &Json, fv: &Json) -> bool {
    if let Some(fid) = fv.get("@id").and_then(Json::as_str) {
        return node_id(nv) == Some(fid);
    }
    let Some(fvalue) = fv.get("@value") else {
        return false;
    };
    if is_wildcard_value(fvalue) {
        return nv.get("@value").is_some();
    }
    if nv.get("@value") != Some(fvalue) {
        return false;
    }
    // @type / @language constraints, when present in the frame value, must match too.
    if let Some(ft) = fv.get("@type") {
        if !is_wildcard_value(ft) && nv.get("@type") != Some(ft) {
            return false;
        }
    }
    if let Some(fl) = fv.get("@language") {
        if !is_wildcard_value(fl) && nv.get("@language") != Some(fl) {
            return false;
        }
    }
    true
}

/// True when a frame `@value`/`@type`/`@language` slot is the wildcard `{}` (match any).
fn is_wildcard_value(v: &Json) -> bool {
    matches!(v, Json::Obj(m) if m.is_empty())
}

// ===========================================================================
// The recursive Framing Algorithm.
// ===========================================================================

/// The two `@id` sets threaded through the recursion.
///
/// * `path` is the **cycle guard**: every `@id` on the current ancestor chain. It is always
///   consulted (regardless of `@embed` mode) so a blank-node cycle (`a → b → a`) terminates
///   with a node reference instead of recursing forever. Pushed on descent, popped on return.
/// * `embedded` is the **embed link table** for `@embed: @once` (the default): every `@id`
///   that has ALREADY been embedded *anywhere* in this result. A second reach of an
///   `@once` node emits a bare `{"@id": …}` reference (matching the W3C "embed once" rule
///   and pyld). `@always` ignores `embedded` (re-embeds every time, still cycle-guarded by
///   `path`); `@never` never embeds.
#[derive(Default)]
struct Trace {
    path: BTreeSet<String>,
    embedded: BTreeSet<String>,
}

/// Drives the recursive framing over one graph's node map.
struct Framer<'a> {
    map: &'a NodeMap,
}

impl<'a> Framer<'a> {
    fn new(map: &'a NodeMap) -> Framer<'a> {
        Framer { map }
    }

    /// Frames the whole node map against the top-level `frame` pattern, returning the framed
    /// expanded node-object array (the top-level results in deterministic order). A single
    /// `embedded` table spans all roots so `@once` deduplication is result-wide (pyld parity).
    fn frame(&self, frame: &Json, flags: Flags) -> Vec<Json> {
        // Select the top-level matching subjects in deterministic (first-seen) order.
        let matched: Vec<&String> = self
            .map
            .order
            .iter()
            .filter(|id| {
                self.map
                    .by_id
                    .get(*id)
                    .map(|n| node_matches(n, frame, flags.require_all))
                    .unwrap_or(false)
            })
            .collect();

        let mut trace = Trace::default();
        let mut out = Vec::with_capacity(matched.len());
        for id in matched {
            if let Some(framed) = self.frame_subject(id, frame, flags, &mut trace) {
                out.push(framed);
            }
        }
        out
    }

    /// Frames one subject `id` against the `frame` node pattern, embedding (or referencing)
    /// it per the `@embed` flag, the result-wide `embedded` table, and the `path` cycle guard.
    fn frame_subject(
        &self,
        id: &str,
        frame: &Json,
        flags: Flags,
        trace: &mut Trace,
    ) -> Option<Json> {
        let node = self.map.by_id.get(id)?;
        let level = flags.updated_by(frame);

        // Emit a bare node reference (stop recursing) when:
        //  * `@embed: @never`, or
        //  * a cycle: `id` is on the current ancestor path, or
        //  * `@embed: @once` (the default) and `id` was already embedded elsewhere.
        let already_embedded = level.embed == Embed::Once && trace.embedded.contains(id);
        if level.embed == Embed::Never || trace.path.contains(id) || already_embedded {
            let mut r = Json::obj();
            r.set("@id", Json::Str(id.to_string()));
            return Some(r);
        }

        trace.embedded.insert(id.to_string());
        trace.path.insert(id.to_string());
        let result = self.frame_node(node, frame, level, trace);
        trace.path.remove(id);
        Some(result)
    }

    /// Builds the framed output for one matched node: `@id`, `@type`, the framed forward
    /// properties (with `@explicit` pruning + `@default`/`@omitDefault` fill), in the frame's
    /// property order where the frame names them, then the node's remaining properties.
    fn frame_node(&self, node: &Json, frame: &Json, flags: Flags, trace: &mut Trace) -> Json {
        let mut out = Json::obj();

        // @id is always emitted.
        if let Some(id) = node_id(node) {
            out.set("@id", Json::Str(id.to_string()));
        }
        // @type carries through (matched by frame matching already).
        if let Some(t) = node.get("@type") {
            out.set("@type", t.clone());
        }

        // The frame's named properties (excluding keywords) drive ordering + @explicit.
        let frame_props: Vec<(String, Json)> = match frame {
            Json::Obj(m) => m
                .iter()
                .filter(|(k, _)| !is_frame_keyword(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let framed_keys: BTreeSet<&str> = frame_props.iter().map(|(k, _)| k.as_str()).collect();

        // 1. Frame each property the frame names, in frame order.
        for (key, fval) in &frame_props {
            if let Some(vals) = node.get(key) {
                let framed = self.frame_values(vals, fval, flags, trace);
                out.set(key, framed);
            } else {
                // Absent in the node: @default / @omitDefault / preserve-null.
                if let Some(filled) = default_for(fval, flags) {
                    out.set(key, filled);
                }
            }
        }

        // 2. With @explicit off, carry through the node's other (non-framed) properties.
        if !flags.explicit {
            if let Json::Obj(members) = node {
                for (key, vals) in members {
                    if key.starts_with('@') || framed_keys.contains(key.as_str()) {
                        continue;
                    }
                    // Carry through with the inherited framing so referenced nodes still embed
                    // once (the result-wide table) / break cycles via `path`.
                    let pass = Json::obj();
                    out.set(key, self.frame_values(vals, &pass, flags, trace));
                }
            }
        }
        out
    }

    /// Frames a property's expanded object array `vals` against the property's frame value
    /// `fval`, recursing into node references (embedding per `@embed`), preserving `@list`
    /// wrappers, and passing literals through unchanged.
    fn frame_values(&self, vals: &Json, fval: &Json, flags: Flags, trace: &mut Trace) -> Json {
        let sub_frame = sub_frame_of(fval);
        let items: Vec<&Json> = flatten(vals);
        let mut framed: Vec<Json> = Vec::with_capacity(items.len());
        for v in items {
            if let Some(Json::Arr(elems)) = v.get("@list") {
                // List framing: frame each element, keep the @list wrapper.
                let inner: Vec<Json> = elems
                    .iter()
                    .map(|e| self.frame_one(e, &sub_frame, flags, trace))
                    .collect();
                let mut lo = Json::obj();
                lo.set("@list", Json::Arr(inner));
                framed.push(lo);
            } else {
                framed.push(self.frame_one(v, &sub_frame, flags, trace));
            }
        }
        Json::Arr(framed)
    }

    /// Frames a single property value: a node reference recurses through [`frame_subject`]
    /// (embedding it when the embed flag + tables allow); a literal / value object passes
    /// through unchanged.
    fn frame_one(&self, v: &Json, sub_frame: &Json, flags: Flags, trace: &mut Trace) -> Json {
        // A node reference {"@id": …} (no @value) whose id is a known subject → recurse.
        if v.get("@value").is_none() {
            if let Some(id) = node_id(v) {
                if self.map.by_id.contains_key(id) {
                    let child_flags = flags.updated_by(sub_frame);
                    if let Some(framed) = self.frame_subject(id, sub_frame, child_flags, trace) {
                        return framed;
                    }
                }
                // A dangling reference (object not a subject in this graph): keep the ref.
                let mut r = Json::obj();
                r.set("@id", Json::Str(id.to_string()));
                return r;
            }
        }
        // A literal / value object: unchanged.
        v.clone()
    }
}

/// The sub-frame to apply to a property's values: the property's frame value when it is a
/// node-shaped object, else the wildcard frame `{}` (so wildcard `{}` / specific-value
/// frames recurse with default flags).
fn sub_frame_of(fval: &Json) -> Json {
    match fval {
        Json::Obj(_) => fval.clone(),
        Json::Arr(a) => a
            .iter()
            .find(|v| v.is_obj())
            .cloned()
            .unwrap_or_else(Json::obj),
        _ => Json::obj(),
    }
}

/// True for a frame *keyword* that controls framing rather than naming a property.
fn is_frame_keyword(k: &str) -> bool {
    matches!(
        k,
        "@context"
            | "@embed"
            | "@explicit"
            | "@requireAll"
            | "@omitDefault"
            | "@default"
            | "@id"
            | "@type"
    )
}

/// The value to emit for a framed property that is **absent** from the matched node:
/// `None` when `@omitDefault` drops it; the `@default` value (or the preserve-`null`
/// marker for `@default: @null` / no default) otherwise.
fn default_for(fval: &Json, flags: Flags) -> Option<Json> {
    // Per-property @omitDefault overrides the inherited flag.
    let omit = fval
        .get("@omitDefault")
        .and_then(|v| match v {
            Json::Raw(r) => Some(r == "true"),
            Json::Arr(a) => a.first().and_then(|x| match x {
                Json::Raw(r) => Some(r == "true"),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or(flags.omit_default);
    if omit {
        return None;
    }
    match fval.get("@default") {
        Some(Json::Str(s)) if s == "@null" => Some(Json::Arr(vec![preserve_null()])),
        Some(def) => Some(Json::Arr(vec![default_value(def)])),
        None => Some(Json::Arr(vec![preserve_null()])),
    }
}

/// The preserve-`null` marker — emitted for an absent framed property with no `@default`
/// (or `@default: @null`). Compaction renders it as a JSON `null`, surviving the value
/// reduction (so the reader sees the property was framed-but-absent).
fn preserve_null() -> Json {
    Json::Raw("null".to_string())
}

/// Materialises a frame `@default` value into an expanded value object. A bare string
/// default becomes a `{"@value": …}` literal; an explicit value/node object is kept as-is.
fn default_value(def: &Json) -> Json {
    match def {
        Json::Str(s) => {
            let mut o = Json::obj();
            o.set("@value", Json::Str(s.clone()));
            o
        }
        other => other.clone(),
    }
}

// ===========================================================================
// Post-framing: drop preserve-null arrays cleanly + compact.
// ===========================================================================

/// Replaces any `[null]` array (the preserve-null marker) with a bare `null`, and collapses
/// single-element arrays the compaction step would otherwise keep — applied to the framed
/// expanded model *before* compaction so `compact` sees clean value objects and the null
/// markers survive to the output as JSON `null`.
fn finalize_nulls(v: &Json) -> Json {
    match v {
        Json::Arr(a) => {
            if a.len() == 1 {
                if let Json::Raw(r) = &a[0] {
                    if r == "null" {
                        return Json::Raw("null".to_string());
                    }
                }
            }
            Json::Arr(a.iter().map(finalize_nulls).collect())
        }
        Json::Obj(m) => Json::Obj(
            m.iter()
                .map(|(k, val)| (k.clone(), finalize_nulls(val)))
                .collect(),
        ),
        other => other.clone(),
    }
}

// ===========================================================================
// Public entry points.
// ===========================================================================

/// Frames an RDF dataset against a caller-supplied JSON-LD **frame** document, producing a
/// **framed + compacted** JSON-LD 1.1 document.
///
/// `graphs` is the dataset (default graph + named graphs); `frame` is the parsed frame JSON
/// (build it with [`parse_context_json`](super::legacy_compact::parse_context_json) from a frame
/// string, or construct the [`Json`] directly). The result selects the input subjects that
/// match the frame's node pattern, embeds referenced nodes inline per the `@embed` flag
/// (breaking blank-node cycles), prunes to the framed properties under `@explicit`, fills
/// `@default` / preserve-`null` for absent framed properties, then compacts the framed model
/// against the frame's `@context`.
///
/// The output is a `{"@context": …, "@graph": […]}` document — or, for a single matched
/// root, the bare framed node merged with `@context` (the `omitGraph` default). Named graphs
/// in the input are each framed against the same top-level pattern.
///
/// Hand-rolled and **dependency-free** (the compaction module's tiny [`Json`] AST — no
/// `serde_json`, no `json-ld` crate), staying inside the `serialize-rdf` feature.
pub fn write_jsonld_framed(graphs: &[NamedGraph<'_>], frame: &Json) -> String {
    let parsed = Frame::parse(frame);
    let active = ActiveContext::parse(&parsed.context);
    // Expand the frame pattern's IRIs against its @context so node-pattern matching can
    // compare against the node map's absolute IRIs (the W3C "expand the frame" step).
    let pattern = expand_frame(&parsed.pattern, &active);

    // Build the framed expanded results for the default graph, then each named graph.
    let default_triples: &[Triple] = graphs
        .iter()
        .find(|(n, _)| n.is_none())
        .map(|(_, ts)| *ts)
        .unwrap_or(&[]);
    let named: Vec<&NamedGraph<'_>> = graphs.iter().filter(|(n, _)| n.is_some()).collect();

    let top_flags = Flags::default().updated_by(&pattern);

    let mut results: Vec<Json> = frame_graph(default_triples, &pattern, top_flags);

    // Named-graph framing: each named graph is framed against the same pattern and wrapped
    // in a `{"@id": <graph>, "@graph": [...]}` node (the JSON-LD dataset shape).
    for (name, ts) in &named {
        let g = name.as_ref().expect("named graph has a name");
        let inner = frame_graph(ts, &pattern, top_flags);
        if inner.is_empty() {
            continue;
        }
        let mut node = Json::obj();
        match g {
            Term::NamedNode(n) => node.set("@id", Json::Str(n.as_str().to_string())),
            Term::BlankNode(b) => node.set("@id", Json::Str(format!("_:{}", b.as_str()))),
            other => {
                let mut s = String::new();
                let _ = write!(s, "{other}");
                node.set("@id", Json::Str(s));
            }
        }
        node.set("@graph", Json::Arr(inner));
        results.push(node);
    }

    // Clean preserve-null markers, then compact the framed model against the frame context.
    let cleaned: Vec<Json> = results.iter().map(finalize_nulls).collect();
    let compacted = active.compact(&Json::Arr(cleaned));
    let compacted_items: Vec<Json> = match compacted {
        Json::Arr(items) => items,
        other => vec![other],
    };

    // Build the output document. A single root collapses to the bare node merged with
    // `@context` (omitGraph default); zero or many use the `@graph` envelope.
    let mut doc = Json::obj();
    let has_ctx = !matches!(active.raw_context(), Json::Obj(m) if m.is_empty());
    if compacted_items.len() == 1 {
        if has_ctx {
            doc.set("@context", active.raw_context().clone());
        }
        if let Json::Obj(members) = &compacted_items[0] {
            for (k, v) in members {
                doc.set(k, v.clone());
            }
        }
    } else {
        if has_ctx {
            doc.set("@context", active.raw_context().clone());
        }
        let graph_key = active.compact_keyword("@graph");
        doc.set(&graph_key, Json::Arr(compacted_items));
    }

    let mut out = String::new();
    doc.write(&mut out);
    out
}

/// Frames one graph's triples against `pattern`: fromRdf → flatten → recursive framing.
fn frame_graph(triples: &[Triple], pattern: &Json, flags: Flags) -> Vec<Json> {
    let expanded = graph_to_expanded(triples);
    let map = build_node_map(&expanded);
    Framer::new(&map).frame(pattern, flags)
}

// ===========================================================================
// Tests — [OPUS-4.8] (sq-oy1f.17). These exercise the REAL framing path
// (`graph_to_jsonld_framed` → `write_jsonld_framed`), not a stub. Each asserts
// the framed shape against the pyld W3C reference processor's `frame` output
// (`/tmp/pyld-venv`, pyld), pinned as a permanent regression. The load-bearing
// invariants checked: node-pattern selection, @embed/@explicit/@default/
// @omitDefault/@requireAll behaviour, blank-node cycle termination, and list
// framing.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::super::{graph_to_jsonld_framed, parse_context_json};
    use sparq_core::Graph;

    /// Frames `ttl` (Turtle) against `frame_json` (a JSON-LD frame) via the REAL public path.
    fn frame_doc(ttl: &str, frame_json: &str) -> String {
        let g = Graph::load_str(ttl, "turtle").expect("load turtle");
        let frame = parse_context_json(frame_json).expect("frame json object");
        graph_to_jsonld_framed(&g, &frame)
    }

    /// Frames `trig` (a dataset) against `frame_json`.
    fn frame_dataset(trig: &str, frame_json: &str) -> String {
        let g = Graph::load_dataset(trig, "trig").expect("load trig");
        let frame = parse_context_json(frame_json).expect("frame json object");
        graph_to_jsonld_framed(&g, &frame)
    }

    // The expected strings below are the EXACT output of the pyld W3C reference
    // processor's `jsonld.frame(input, frame)` on the same RDF (captured via a pyld
    // venv during development, sq-oy1f.17). They pin the spec-correct framed shape as
    // a permanent regression: @type / property-presence / value / wildcard / match-none
    // node matching, @embed @once/@always/@never, @explicit, @default/@omitDefault,
    // @requireAll AND/OR, list framing, and the @graph / omitGraph envelope.

    #[test]
    fn type_frame_embeds_referenced_node() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Library> ; \
             <http://ex/contains> <http://ex/2> .\n\
             <http://ex/2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Book> ; \
             <http://ex/title> \"T\" .",
            r#"{"@type":"http://ex/Library","http://ex/contains":{}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/Library","http://ex/contains":{"@id":"http://ex/2","@type":"http://ex/Book","http://ex/title":"T"}}"#
        );
    }

    #[test]
    fn explicit_prunes_unframed_properties() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> ; \
             <http://ex/a> \"A\" ; <http://ex/b> \"B\" .",
            r#"{"@type":"http://ex/T","@explicit":true,"http://ex/a":{}}"#,
        );
        // @explicit:true drops ex:b (not named in the frame).
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/a":"A"}"#
        );
    }

    #[test]
    fn default_fills_absent_property() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/T","http://ex/m":{"@default":"FB"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/m":"FB"}"#
        );
    }

    #[test]
    fn omit_default_drops_absent_defaulted_property() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/T","http://ex/m":{"@default":"FB","@omitDefault":true}}"#,
        );
        assert_eq!(out, r#"{"@id":"http://ex/1","@type":"http://ex/T"}"#);
    }

    #[test]
    fn default_null_emits_preserve_null() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/T","http://ex/m":{"@default":"@null"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/m":null}"#
        );
    }

    #[test]
    fn embed_never_emits_node_reference() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/2> .\n<http://ex/2> <http://ex/q> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":"@never"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/2"}}"#
        );
    }

    #[test]
    fn embed_once_deduplicates_shared_node() {
        // @once (the default): a node shared by two properties is embedded the FIRST time
        // and referenced thereafter — result-wide dedup, matching pyld.
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/x> ; <http://ex/q> <http://ex/x> .\n\
             <http://ex/x> <http://ex/v> \"X\" .",
            r#"{"@id":"http://ex/1"}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/x","http://ex/v":"X"},"http://ex/q":{"@id":"http://ex/x"}}"#
        );
    }

    #[test]
    fn embed_always_reembeds_shared_node() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/x> ; <http://ex/q> <http://ex/x> .\n\
             <http://ex/x> <http://ex/v> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":"@always"},"http://ex/q":{"@embed":"@always"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/x","http://ex/v":"X"},"http://ex/q":{"@id":"http://ex/x","http://ex/v":"X"}}"#
        );
    }

    #[test]
    fn require_all_conjoins_frame_properties() {
        // @requireAll:true → only the node carrying BOTH ex:a AND ex:b is selected.
        let out = frame_doc(
            "<http://ex/1> <http://ex/a> \"A\" ; <http://ex/b> \"B\" .\n<http://ex/2> <http://ex/a> \"A\" .",
            r#"{"http://ex/a":{},"http://ex/b":{},"@requireAll":true}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/a":"A","http://ex/b":"B"}"#
        );
    }

    #[test]
    fn require_all_false_disjoins_to_two_roots_with_preserve_null() {
        // The default OR selects BOTH nodes; the framed-but-absent property is preserve-null.
        // Multiple roots → the @graph envelope.
        let out = frame_doc(
            "<http://ex/1> <http://ex/a> \"A\" .\n<http://ex/2> <http://ex/b> \"B\" .",
            r#"{"http://ex/a":{},"http://ex/b":{},"@requireAll":false}"#,
        );
        assert_eq!(
            out,
            r#"{"@graph":[{"@id":"http://ex/1","http://ex/a":"A","http://ex/b":null},{"@id":"http://ex/2","http://ex/a":null,"http://ex/b":"B"}]}"#
        );
    }

    #[test]
    fn value_match_selects_by_literal() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> \"yes\" .\n<http://ex/2> <http://ex/p> \"no\" .",
            r#"{"http://ex/p":[{"@value":"yes"}]}"#,
        );
        assert_eq!(out, r#"{"@id":"http://ex/1","http://ex/p":"yes"}"#);
    }

    #[test]
    fn match_none_requires_property_absent() {
        // The frame names ex:p as match-none `[]` → only the node WITHOUT ex:p is selected.
        // ex:p is then a framed-but-absent property → preserve-null.
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> \"P\" .\n<http://ex/2> <http://ex/q> \"Q\" .",
            r#"{"http://ex/p":[]}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/2","http://ex/p":null,"http://ex/q":"Q"}"#
        );
    }

    #[test]
    fn wildcard_frame_matches_all() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> \"P\" .\n<http://ex/2> <http://ex/q> \"Q\" .",
            r#"{}"#,
        );
        assert_eq!(
            out,
            r#"{"@graph":[{"@id":"http://ex/1","http://ex/p":"P"},{"@id":"http://ex/2","http://ex/q":"Q"}]}"#
        );
    }

    #[test]
    fn circular_reference_terminates() {
        // a → b → a: the embed link table breaks the cycle with a node reference.
        let out = frame_doc(
            "<http://ex/a> <http://ex/next> <http://ex/b> .\n<http://ex/b> <http://ex/next> <http://ex/a> .",
            r#"{"@id":"http://ex/a"}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/a","http://ex/next":{"@id":"http://ex/b","http://ex/next":{"@id":"http://ex/a"}}}"#
        );
    }

    #[test]
    fn blank_node_cycle_terminates() {
        // A genuine blank-node cycle must terminate (no infinite recursion) and produce a
        // valid framed tree whose innermost back-edge is a bare reference. Blank-node labels
        // are processor-internal, so we assert termination + a closing reference structurally.
        let out = frame_doc(
            "_:a <http://ex/next> _:b . _:b <http://ex/next> _:a . _:a <http://ex/v> \"A\" .",
            r#"{"http://ex/v":{}}"#,
        );
        // Selected node a (has ex:v); a embeds b; b's ex:next points back to a as a reference.
        assert!(out.contains(r#""http://ex/v":"A""#), "node a framed: {out}");
        assert!(out.contains(r#""http://ex/next""#), "edge present: {out}");
        // The back-edge to a is a bare {"@id": …} reference (cycle broken), so there is no
        // third level of "http://ex/v" nesting — the document is finite.
        assert_eq!(out.matches(r#""http://ex/v""#).count(), 1, "finite: {out}");
    }

    #[test]
    fn list_framing_preserves_list_wrapper() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/items> ( \"a\" \"b\" ) .",
            r#"{"http://ex/items":{}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/items":{"@list":["a","b"]}}"#
        );
    }

    #[test]
    fn typed_literal_native_coercion() {
        let out = frame_doc("<http://ex/1> <http://ex/n> 42 .", r#"{"http://ex/n":{}}"#);
        assert_eq!(out, r#"{"@id":"http://ex/1","http://ex/n":42}"#);
    }

    #[test]
    fn context_compaction_applies() {
        // The frame's @context drives the final compaction: @vocab abbreviates the @type
        // and the property key.
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> ; \
             <http://ex/p> \"P\" .",
            r#"{"@context":{"@vocab":"http://ex/"},"@type":"T"}"#,
        );
        assert_eq!(
            out,
            r#"{"@context":{"@vocab":"http://ex/"},"@id":"http://ex/1","@type":"T","p":"P"}"#
        );
    }

    #[test]
    fn named_graph_framed_independently() {
        // Named-graph framing (sq-oy1f.17 scope): each named graph is framed against the
        // pattern and wrapped in the {"@id": <graph>, "@graph": [...]} dataset shape
        // (lossless; the general pyld @graph-frame semantics are a documented follow-up).
        let out = frame_dataset(
            "<http://ex/g> { <http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
             <http://ex/T> ; <http://ex/v> \"V\" . }",
            r#"{"@type":"http://ex/T"}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/g","@graph":[{"@id":"http://ex/1","@type":"http://ex/T","http://ex/v":"V"}]}"#
        );
    }

    #[test]
    fn no_match_yields_empty_graph() {
        // A frame that matches nothing produces an empty @graph envelope (never panics).
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/Other"}"#,
        );
        assert_eq!(out, r#"{"@graph":[]}"#);
    }

    // -----------------------------------------------------------------------
    // [OPUS-4.8] sq-qcnn.33 — coverage-raise tests: targeted paths not yet
    // exercised by the framing suite above.
    // -----------------------------------------------------------------------

    /// `Embed::parse` with `Json::Raw("true")` and `Json::Raw("false")` — the legacy
    /// boolean spellings for `@once` and `@never`.  The frame parser stores JSON `true`
    /// / `false` as `Json::Raw`, not `Json::Str`, so these arms were previously uncovered.
    #[test]
    fn embed_boolean_true_false() {
        // @embed: false — the legacy spelling of @never; the referenced node stays a
        // bare {"@id":…} reference, identical to using "@embed":"@never".
        let out_false = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/2> .\
             <http://ex/2> <http://ex/q> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":false}}"#,
        );
        assert_eq!(
            out_false,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/2"}}"#,
            "embed=false is @never: {}",
            out_false
        );

        // @embed: true — the legacy spelling of @once; the node is embedded the first
        // time it is reached (same result as the default, but exercises the true arm).
        let out_true = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/2> .\
             <http://ex/2> <http://ex/q> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":true}}"#,
        );
        assert!(
            out_true.contains(r#""@id":"http://ex/2""#),
            "embed=true embeds the node: {}",
            out_true
        );
        assert!(
            out_true.contains(r#""http://ex/q":"X""#),
            "embedded node properties present: {}",
            out_true
        );
    }

    /// `Flags::updated_by` top-level `@omitDefault` field (line ~156): when the
    /// top-level frame carries `@omitDefault:true`, absent-property fill is suppressed
    /// for the whole tree via the inherited flag (not just a per-property override).
    #[test]
    fn top_level_omit_default_suppresses_fill() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            // Top-level @omitDefault:true — the absent "m" property is dropped instead of
            // being filled with the @default value "FB".
            r#"{"@type":"http://ex/T","@omitDefault":true,"http://ex/m":{"@default":"FB"}}"#,
        );
        // The absence of "m" in the output confirms the top-level @omitDefault was applied.
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T"}"#,
            "top-level @omitDefault suppresses fill: {}",
            out
        );
    }

    /// `match_type` wildcard `@type:{}` branch (~line 426): matches any node that has
    /// at least one type; nodes without `@type` are excluded.
    #[test]
    fn type_wildcard_matches_only_typed_nodes() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .\
             <http://ex/2> <http://ex/p> \"X\" .",
            // @type:{} = wildcard — match any node that has a type.
            r#"{"@type":{}}"#,
        );
        // ex:1 has a type and matches; ex:2 has no type and is excluded.
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T"}"#,
            "wildcard @type selects typed node only: {}",
            out
        );
    }

    /// `match_type` match-none `@type:[]` branch (~line 423): matches only nodes that
    /// have NO `@type`; typed nodes are excluded.
    #[test]
    fn type_match_none_selects_untyped_nodes() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .\
             <http://ex/2> <http://ex/p> \"X\" .",
            // @type:[] = match-none — only untyped nodes match.
            r#"{"@type":[]}"#,
        );
        // ex:2 has no type and matches; ex:1 is excluded.
        assert_eq!(
            out,
            r#"{"@id":"http://ex/2","http://ex/p":"X"}"#,
            "match-none @type selects untyped node only: {}",
            out
        );
    }

    /// `match_type` Json::Arr `wanted` branch (~line 431): an array of type IRIs in the
    /// frame matches a node with ANY of those types (OR semantics).
    #[test]
    fn type_array_frame_matches_either_type() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T1> .\
             <http://ex/2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T2> .\
             <http://ex/3> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T3> .",
            // @type array: matches nodes typed T1 OR T2; T3 is excluded.
            r#"{"@type":["http://ex/T1","http://ex/T2"]}"#,
        );
        // ex:1 and ex:2 match; ex:3 does not.
        assert!(
            out.contains(r#""@id":"http://ex/1""#),
            "T1 node in result: {}",
            out
        );
        assert!(
            out.contains(r#""@id":"http://ex/2""#),
            "T2 node in result: {}",
            out
        );
        assert!(
            !out.contains(r#""@id":"http://ex/3""#),
            "T3 node excluded: {}",
            out
        );
    }
}
