//! Compaction Algorithm (JSON-LD 1.1 API §7) — document-level, over expanded input.
//!
//! [FABLE-5] (sq-oy1f.27) The W3C **Compaction Algorithm** and **Value Compaction**
//! (spec "Compaction Algorithms", <https://www.w3.org/TR/json-ld11-api/#compaction-algorithms>),
//! running over the *expanded document* with a real
//! [`ActiveContext`] — rather than as a bespoke inverse of
//! the RDF writer — which is what structurally removes the self-reparse-invisible
//! data-loss class of bug (design record §3.2). The compaction-side context machinery it
//! composes landed earlier: Inverse Context Creation, IRI Compaction, and Term Selection
//! live in `context::inverse` (bead `sq-90mu3`); this module adds the document walk:
//!
//! - **array + singleton collapse** honouring `compactArrays` and the `@list`/`@set`
//!   container mappings;
//! - **previous-context reversion** for non-propagating (type-scoped) contexts, and
//!   **property-scoped / type-scoped context application** during the walk;
//! - **Value Compaction** — `@id`/`@vocab` type coercions, matching `@type`, language +
//!   direction matching (case-insensitive), `@json` literals, `@index` pass-through;
//! - **keyword aliasing** of `@id`/`@type`/`@reverse`/`@value`/`@language`/`@index`/
//!   `@direction`/`@graph`/`@list`/`@none` via IRI Compaction;
//! - **container reshaping**: `@list`, `@language`/`@index`/`@id`/`@type` maps
//!   (including property-valued `@index` maps), and the `@graph` container forms
//!   (`@graph`, `@graph`+`@id`, `@graph`+`@index`, `@included` wrapping);
//! - **`@nest` grouping** (with the `invalid @nest value` error), `@reverse`
//!   redistribution onto reverse terms, and `@preserve` pass-through for the framing
//!   pipeline.
//!
//! ## Options honoured
//!
//! `compactArrays`, `ordered`, `processingMode`, and `compactToRelative` from
//! [`JsonLdOptions`]. One modelling note: this crate has no remote-document layer yet, so
//! [`JsonLdOptions::base`] stands in for *both* the API's `base` override *and* the
//! document URL. `compactToRelative: false` therefore disables base-relative IRI
//! compaction entirely (the spec's letter would still relativise against an explicit
//! `base` option); a context's own `@base` continues to apply. This matches how the W3C
//! harness drives the flag (the `compactToRelative` suite cases set no `base` option).
//!
//! Remote `@context` / `@import` references (in the caller context or reachable during
//! the initial expansion) are dereferenced only through the [`DocumentLoader`]
//! (deny-by-default via [`NoopLoader`](crate::loader::NoopLoader)).

use crate::context::inverse::{compact_iri, InverseContext};
use crate::context::{ActiveContext, Direction, Override};
use crate::error::{JsonLdError, JsonLdErrorCode as E};
use crate::expand::expand;
use crate::fx::FxMap;
use crate::json::Json;
use crate::loader::DocumentLoader;
use crate::options::{JsonLdOptions, ProcessingMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// **Compaction** (the `compact()` API operation). Expands `input` against `options`
/// (via [`expand`]), then compacts the expanded document against `context`, returning
/// the compacted document with `context` embedded under `@context` (unless the context
/// is empty).
///
/// `context` may be a context definition, an IRI string, an array of these, or a map
/// carrying the context under an `@context` entry (one layer is unwrapped, matching the
/// API). Returns the first spec [`JsonLdError`] raised by expansion, context processing,
/// or compaction.
///
/// [FABLE-5] (sq-oy1f.27)
pub fn compact(
    input: &Json,
    context: &Json,
    options: &JsonLdOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    let expanded = expand(input, options, loader)?;
    compact_expanded(&expanded, context, options, loader)
}

/// **Compaction** over an already-expanded document. Splits out so callers that already
/// hold the expanded form (the conformance lane, the flatten-then-compact composition)
/// skip re-expansion. Applies the API's post-processing: an empty-array output becomes
/// `{}`, a multi-node array is wrapped under (a possibly aliased) `@graph`, and the
/// caller `context` is embedded under `@context` unless it is empty (`null`, `{}`, or
/// `[]`).
///
/// [FABLE-5] (sq-oy1f.27)
pub fn compact_expanded(
    expanded: &Json,
    context: &Json,
    options: &JsonLdOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    // API step: a map with an @context entry contributes that entry's value.
    let ctx_value = context
        .get("@context")
        .cloned()
        .unwrap_or_else(|| context.clone());

    // API step: the active context's base IRI. `options.base` stands in for the document
    // URL (no remote-document layer), so `compactToRelative: false` clears the initial
    // base (see the module doc); a context `@base` may still set one below.
    let base = if options.compact_to_relative {
        options.base.as_deref()
    } else {
        None
    };
    let ctx = root_ctx(&ctx_value, base, options, loader)?;
    let ctx: &Ctx = &ctx;
    let env = Env {
        loader,
        options,
        root: ctx,
    };

    // The Compaction Algorithm proper, with a null active property.
    let compacted = compact_element(ctx, None, expanded, &env);
    ctx.trim_memos();
    let compacted = compacted?;

    // API post-processing: [] → {}; a remaining array is wrapped under aliased @graph.
    let mut result = match compacted {
        Json::Arr(items) if items.is_empty() => Json::obj(),
        Json::Arr(items) => {
            let mut obj = Json::obj();
            obj.set(&ctx.ciri("@graph", None, true, false), Json::Arr(items));
            obj
        }
        other => other,
    };

    // API post-processing: embed the (unwrapped) caller context unless it is empty.
    if !context_is_empty(&ctx_value) {
        if let Json::Obj(members) = &mut result {
            members.insert(0, ("@context".to_string(), ctx_value));
        }
    }
    Ok(result)
}

thread_local! {
    /// The last self-contained root context processed on this thread, keyed by the
    /// context value and options. Callers serialising many documents against one
    /// context (the engine writer, the playground) skip re-processing it and rebuilding
    /// its inverse context on every call.
    static LAST_ROOT: RefCell<Option<(Json, JsonLdOptions, Rc<Ctx>)>> = const { RefCell::new(None) };
}

/// The processed root context for `ctx_value`, reusing [`LAST_ROOT`] when it matches.
/// Only contexts that cannot reach the loader (no remote references, no `@import`) are
/// cached, since a loader may resolve the same IRI differently between calls.
fn root_ctx(
    ctx_value: &Json,
    base: Option<&str>,
    options: &JsonLdOptions,
    loader: &dyn DocumentLoader,
) -> Result<Rc<Ctx>, JsonLdError> {
    let cacheable = self_contained(ctx_value);
    if cacheable {
        let hit = LAST_ROOT.with(|last| {
            last.borrow()
                .as_ref()
                .filter(|(v, o, _)| v == ctx_value && o == options)
                .map(|(_, _, c)| Rc::clone(c))
        });
        if let Some(ctx) = hit {
            return Ok(ctx);
        }
    }
    let active =
        ActiveContext::new(base).process(ctx_value, options.base.as_deref(), loader, options)?;
    let ctx = Rc::new(Ctx::new(active));
    if cacheable {
        LAST_ROOT.with(|last| {
            *last.borrow_mut() = Some((ctx_value.clone(), options.clone(), Rc::clone(&ctx)));
        });
    }
    Ok(ctx)
}

/// True iff processing `context` never consults the document loader: no remote context
/// IRI (a string context, at the top level or under any nested `@context`) and no
/// `@import`.
fn self_contained(context: &Json) -> bool {
    fn local(ctx: &Json) -> bool {
        match ctx {
            Json::Str(_) => false,
            Json::Arr(items) => items.iter().all(local),
            other => nested_ok(other),
        }
    }
    fn nested_ok(j: &Json) -> bool {
        match j {
            Json::Obj(m) => m.iter().all(|(k, v)| match k.as_str() {
                "@import" => false,
                "@context" => local(v),
                _ => nested_ok(v),
            }),
            Json::Arr(a) => a.iter().all(nested_ok),
            _ => true,
        }
    }
    local(context)
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

/// Per-call environment (the loader and options), threaded through the walk.
struct Env<'a> {
    loader: &'a dyn DocumentLoader,
    options: &'a JsonLdOptions,
    /// The root context, which owns the memo of derived contexts.
    root: &'a Ctx,
}

/// Identity of a derived context: the context reverted to, or the scoped context of
/// `term` (looked up in `lookup`) applied onto `base`.
enum DerivedKey {
    Revert(Arc<ActiveContext>),
    Scoped {
        lookup: *const Ctx,
        base: *const Ctx,
        term: String,
        type_scoped: bool,
    },
}

impl DerivedKey {
    fn same(&self, other: &DerivedKey) -> bool {
        match (self, other) {
            (DerivedKey::Revert(a), DerivedKey::Revert(b)) => Arc::ptr_eq(a, b),
            (
                DerivedKey::Scoped { lookup, base, term, type_scoped },
                DerivedKey::Scoped { lookup: l2, base: b2, term: t2, type_scoped: s2 },
            ) => lookup == l2 && base == b2 && term == t2 && type_scoped == s2,
            _ => false,
        }
    }
}

impl Env<'_> {
    /// The memoised context for `key`, building it with `build` on first use.
    fn derived(
        &self,
        key: DerivedKey,
        build: impl FnOnce() -> Result<ActiveContext, JsonLdError>,
    ) -> Result<Rc<Ctx>, JsonLdError> {
        let derived = &self.root.derived;
        if let Some((_, c)) = derived.borrow().iter().find(|(k, _)| k.same(&key)) {
            return Ok(Rc::clone(c));
        }
        let ctx = Rc::new(Ctx::new(build()?));
        derived.borrow_mut().push((key, Rc::clone(&ctx)));
        Ok(ctx)
    }
}

/// An [`ActiveContext`] paired with its (eagerly built) inverse context. The inverse is
/// rebuilt whenever a scoped context changes the active context mid-walk — context
/// switches are rare relative to nodes, so the eager rebuild keeps the common path free
/// of repeated inverse construction.
struct Ctx {
    active: ActiveContext,
    inverse: InverseContext,
    /// Memoised IRI Compaction results: iri → (shape, vocab, reverse, result).
    memo: RefCell<FxMap<String, Vec<MemoEntry>>>,
    /// On the root context only: contexts derived mid-walk (step 4 reversion, step 5
    /// property-scoped and step 9 type-scoped contexts), memoised so each distinct switch
    /// builds its active and inverse context once rather than once per node (and, with
    /// the root cached in [`LAST_ROOT`], once across calls). The entries own every
    /// derived [`Ctx`] (and each reverted-to `Arc`) for the root's lifetime, so the
    /// address keys can never be reused by another context.
    derived: RefCell<Vec<(DerivedKey, Rc<Ctx>)>>,
}

impl Ctx {
    fn new(active: ActiveContext) -> Ctx {
        let inverse = active.inverse_context();
        Ctx { active, inverse, memo: RefCell::default(), derived: RefCell::default() }
    }

    /// Clears this root's IRI memos (its own and its derived contexts') once they hold
    /// more than [`MEMO_CAP`] IRIs. The root outlives the call in [`LAST_ROOT`] and node
    /// ids differ per document, so without this the memos would keep every IRI ever
    /// compacted on the thread; within one call they stay unbounded, which keeps large
    /// documents fast. Each IRI holds at most [`MEMO_SHAPES`] value shapes.
    fn trim_memos(&self) {
        let derived = self.derived.borrow();
        for memo in std::iter::once(&self.memo).chain(derived.iter().map(|(_, c)| &c.memo)) {
            if memo.borrow().len() > MEMO_CAP {
                memo.borrow_mut().clear();
            }
        }
    }

    /// IRI Compaction against this context (see `context::inverse`'s `compact_iri`),
    /// memoised for the value shapes in [`Shape`].
    fn ciri(&self, iri: &str, value: Option<&Json>, vocab: bool, reverse: bool) -> String {
        let Some(shape) = Shape::of(value, &self.inverse) else {
            return compact_iri(&self.active, &self.inverse, iri, value, vocab, reverse);
        };
        let hit = |e: &&MemoEntry| e.1 == vocab && e.2 == reverse && e.0.is(&shape);
        if let Some(entries) = self.memo.borrow().get(iri) {
            if let Some(e) = entries.iter().find(hit) {
                debug_assert_eq!(
                    e.3,
                    compact_iri(&self.active, &self.inverse, iri, value, vocab, reverse),
                    "memoised IRI compaction diverged for {iri}"
                );
                return e.3.clone();
            }
        }
        let result = compact_iri(&self.active, &self.inverse, iri, value, vocab, reverse);
        let mut memo = self.memo.borrow_mut();
        let entries = match memo.get_mut(iri) {
            Some(entries) => entries,
            None => memo.entry(iri.to_string()).or_default(),
        };
        // Shapes vary with datatypes and language tags, so they are capped per IRI too.
        if entries.len() < MEMO_SHAPES {
            entries.push((shape.to_owned(), vocab, reverse, result.clone()));
        }
        result
    }
}

/// Most distinct IRIs a [`Ctx`] keeps memoised between calls (see [`Ctx::trim_memos`]).
const MEMO_CAP: usize = 4096;

/// Most value shapes memoised per IRI.
const MEMO_SHAPES: usize = 8;

/// One memoised IRI Compaction: (value shape, vocab, reverse, result).
type MemoEntry = (OwnedShape, bool, bool, String);

/// Everything IRI Compaction reads from a `value`, for the shapes it is cheap to
/// summarise exactly: no value, a plain value object (`@value` plus optional `@type` /
/// `@language`), or a node object without `@index` / `@list` / `@graph` / `@preserve`
/// whose `@id` (if any) names no term (so it cannot round-trip through one).
#[derive(PartialEq)]
enum Shape<'a> {
    Bare,
    Value { ty: Option<&'a str>, lang: Option<&'a str>, lone_str: bool },
    Node { has_id: bool },
}

#[derive(PartialEq)]
enum OwnedShape {
    Bare,
    Value { ty: Option<String>, lang: Option<String>, lone_str: bool },
    Node { has_id: bool },
}

impl<'a> Shape<'a> {
    fn of(value: Option<&'a Json>, inverse: &InverseContext) -> Option<Shape<'a>> {
        let Some(value) = value else {
            return Some(Shape::Bare);
        };
        let Json::Obj(members) = value else {
            return None;
        };
        if let Some(v) = value.get("@value") {
            let (mut ty, mut lang) = (None, None);
            for (k, m) in members {
                match (k.as_str(), m) {
                    ("@value", _) => {}
                    ("@type", Json::Str(t)) => ty = Some(t.as_str()),
                    ("@language", Json::Str(l)) => lang = Some(l.as_str()),
                    _ => return None,
                }
            }
            let lone_str = members.len() == 1 && matches!(v, Json::Str(_));
            return Some(Shape::Value { ty, lang, lone_str });
        }
        let mut has_id = false;
        for (k, m) in members {
            match k.as_str() {
                "@index" | "@list" | "@graph" | "@preserve" | "@direction" | "@language" => {
                    return None
                }
                "@id" => match m {
                    Json::Str(id) if !inverse.has_iri(id) => has_id = true,
                    _ => return None,
                },
                _ => {}
            }
        }
        Some(Shape::Node { has_id })
    }

    fn to_owned(&self) -> OwnedShape {
        match *self {
            Shape::Bare => OwnedShape::Bare,
            Shape::Value { ty, lang, lone_str } => OwnedShape::Value {
                ty: ty.map(str::to_string),
                lang: lang.map(str::to_string),
                lone_str,
            },
            Shape::Node { has_id } => OwnedShape::Node { has_id },
        }
    }
}

impl OwnedShape {
    fn is(&self, shape: &Shape<'_>) -> bool {
        match (self, shape) {
            (OwnedShape::Bare, Shape::Bare) => true,
            (
                OwnedShape::Value { ty, lang, lone_str },
                Shape::Value { ty: t2, lang: l2, lone_str: s2 },
            ) => ty.as_deref() == *t2 && lang.as_deref() == *l2 && lone_str == s2,
            (OwnedShape::Node { has_id }, Shape::Node { has_id: h2 }) => has_id == h2,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// The Compaction Algorithm
// ---------------------------------------------------------------------------

/// The recursive core of the Compaction Algorithm. `active_property` is the *compacted*
/// term (or keyword) whose value `element` is; `None` at the document root.
fn compact_element(
    ctx: &Ctx,
    active_property: Option<&str>,
    element: &Json,
    env: &Env,
) -> Result<Json, JsonLdError> {
    // step 1: retain the incoming context — values may be relevant to a previous
    // type-scoped context (used for @type compaction + type-scoped term lookups below).
    let type_scoped = ctx;

    // step 2: scalars (and null) are already in compact form.
    if is_scalar(element) || is_null(element) {
        return Ok(element.clone());
    }

    // step 3: arrays — compact each item (dropping nulls), then collapse a singleton
    // unless disallowed by compactArrays / @graph / @set / a @list-@set container.
    if let Json::Arr(items) = element {
        let mut result: Vec<Json> = Vec::new();
        for item in items {
            let compacted = compact_element(ctx, active_property, item, env)?;
            if !is_null(&compacted) {
                result.push(compacted);
            }
        }
        let container = term_container(&ctx.active, active_property);
        let keep_array = result.len() != 1
            || !env.options.compact_arrays
            || matches!(active_property, Some("@graph") | Some("@set"))
            || container.iter().any(|c| c == "@list" || c == "@set");
        return Ok(if keep_array {
            Json::Arr(result)
        } else {
            result.into_iter().next().expect("exactly one element")
        });
    }

    let Json::Obj(members) = element else {
        // Str / Raw were handled by the scalar step.
        return Ok(element.clone());
    };

    // steps 4-5: context adjustments. `owned` carries a replacement context when the
    // previous-context reversion or a property-scoped context applies.
    let mut owned: Option<Rc<Ctx>> = None;

    // step 4: non-propagated (type-scoped) contexts do not apply when processing a new
    // node object — revert to the previous context unless element is a value object or
    // a lone node reference.
    if let Some(prev) = &ctx.active.previous_context {
        let single_id = members.len() == 1 && members[0].0 == "@id";
        if element.get("@value").is_none() && !single_id {
            owned = Some(env.derived(DerivedKey::Revert(Arc::clone(prev)), || {
                Ok((**prev).clone())
            })?);
        }
    }

    // step 5: apply the active property's property-scoped context, if any. The term
    // LOOKUP runs against the INCOMING (pre-reversion) context — a term defined by the
    // parent's type-scoped context still carries its property-scoped context into the
    // child node (W3C compact/c013) — while the application folds onto the (possibly
    // reverted) context from step 4, mirroring the reference implementations.
    if let Some(ap) = active_property {
        if let Some(def) = ctx.active.term_definition(ap) {
            if let Some(local) = def.context() {
                let base: &Ctx = owned.as_deref().unwrap_or(ctx);
                let key = DerivedKey::Scoped {
                    lookup: ctx,
                    base,
                    term: ap.to_string(),
                    type_scoped: false,
                };
                owned = Some(env.derived(key, || {
                    base.active.process_scoped(
                        local,
                        def.base_url.as_deref(),
                        true, // override protected
                        true, // propagate
                        env.loader,
                        env.options,
                    )
                })?);
            }
        }
    }
    let cur: &Ctx = owned.as_deref().unwrap_or(ctx);

    // step 6: value objects / node references — Value Compaction. Return the result when
    // it is a scalar, or unconditionally for a @json-typed term (its payload is raw JSON).
    if element.get("@value").is_some() || element.get("@id").is_some() {
        if let Some(v) = value_compact(cur, active_property, element) {
            let json_mapped = active_property
                .and_then(|p| cur.active.term_definition(p))
                .and_then(|d| d.type_mapping())
                == Some("@json");
            if is_scalar(&v) || json_mapped {
                return Ok(v);
            }
        }
    }

    // step 7: a list object under a @list-container term compacts to its bare items.
    if is_list_object(element)
        && term_container(&cur.active, active_property)
            .iter()
            .any(|c| c == "@list")
    {
        let list = element.get("@list").expect("list object");
        return compact_element(cur, active_property, list, env);
    }

    // step 8: reverse-property scope.
    let inside_reverse = active_property == Some("@reverse");

    // step 9: apply any type-scoped contexts declared on the node's (compacted) types,
    // in lexicographic order of the compacted forms, with propagate false. Lookups run
    // against the retained type-scoped context (step 1).
    let mut owned_t: Option<Rc<Ctx>> = None;
    if let Some(types) = element.get("@type") {
        let mut compacted_types: Vec<String> = type_strings(types)
            .into_iter()
            .map(|t| type_scoped.ciri(t, None, true, false))
            .collect();
        compacted_types.sort();
        for term in &compacted_types {
            if let Some(def) = type_scoped.active.term_definition(term) {
                if let Some(local) = def.context() {
                    let base: &Ctx = owned_t.as_deref().unwrap_or(cur);
                    let key = DerivedKey::Scoped {
                        lookup: type_scoped,
                        base,
                        term: term.clone(),
                        type_scoped: true,
                    };
                    owned_t = Some(env.derived(key, || {
                        base.active.process_scoped(
                            local,
                            def.base_url.as_deref(),
                            false, // override protected
                            false, // propagate
                            env.loader,
                            env.options,
                        )
                    })?);
                }
            }
        }
    }
    let cur: &Ctx = owned_t.as_deref().unwrap_or(cur);

    // steps 10-12: build the compacted node.
    let mut result = Json::obj();

    let mut entries: Vec<(&str, &Json)> = members.iter().map(|(k, v)| (k.as_str(), v)).collect();
    if env.options.ordered {
        entries.sort_by(|a, b| a.0.cmp(b.0));
    }

    for (key, expanded_value) in entries {
        match key {
            // step 12.1: @id — compact the IRI (document-relative) under the alias.
            "@id" => {
                let compacted = match expanded_value {
                    Json::Str(s) => Json::Str(cur.ciri(s, None, false, false)),
                    // Frame-expanded documents may carry @id arrays; compact each
                    // (the framing bead consumes this — plain expansion always yields
                    // a single string).
                    Json::Arr(ids) => Json::Arr(
                        ids.iter()
                            .map(|id| match id {
                                Json::Str(s) => Json::Str(cur.ciri(s, None, false, false)),
                                other => other.clone(),
                            })
                            .collect(),
                    ),
                    other => other.clone(),
                };
                let alias = cur.ciri("@id", None, true, false);
                result.set(&alias, compacted);
                continue;
            }
            // step 12.2: @type — compact each type against the TYPE-SCOPED context;
            // array-ness follows the alias's @set container (1.1) or compactArrays.
            "@type" => {
                let compacted = match expanded_value {
                    Json::Str(s) => Json::Str(type_scoped.ciri(s, None, true, false)),
                    Json::Arr(ts) => Json::Arr(
                        ts.iter()
                            .map(|t| match t {
                                Json::Str(s) => Json::Str(type_scoped.ciri(s, None, true, false)),
                                other => other.clone(),
                            })
                            .collect(),
                    ),
                    other => other.clone(),
                };
                let alias = cur.ciri("@type", None, true, false);
                let as_array = (env.options.processing_mode == ProcessingMode::JsonLd11
                    && term_container(&cur.active, Some(&alias))
                        .iter()
                        .any(|c| c == "@set"))
                    || !env.options.compact_arrays;
                add_value(&mut result, &alias, compacted, as_array);
                continue;
            }
            // step 12.3: @reverse — compact recursively, then redistribute entries whose
            // term is a reverse property onto the node itself.
            "@reverse" => {
                let compacted = compact_element(cur, Some("@reverse"), expanded_value, env)?;
                if let Json::Obj(rev_members) = compacted {
                    let mut remaining: Vec<(String, Json)> = Vec::new();
                    for (prop, val) in rev_members {
                        let is_rev = cur
                            .active
                            .term_definition(&prop)
                            .map(|d| d.is_reverse())
                            .unwrap_or(false);
                        if is_rev {
                            let as_array = term_container(&cur.active, Some(&prop))
                                .iter()
                                .any(|c| c == "@set")
                                || !env.options.compact_arrays;
                            add_value(&mut result, &prop, val, as_array);
                        } else {
                            remaining.push((prop, val));
                        }
                    }
                    if !remaining.is_empty() {
                        let alias = cur.ciri("@reverse", None, true, false);
                        result.set(&alias, Json::Obj(remaining));
                    }
                }
                continue;
            }
            // step 12.4: @preserve (framing) — compact the payload, keep the keyword.
            "@preserve" => {
                let compacted = compact_element(cur, active_property, expanded_value, env)?;
                if !matches!(expanded_value, Json::Arr(a) if a.is_empty()) {
                    result.set("@preserve", compacted);
                }
                continue;
            }
            // step 12.5: an @index re-expressed by the active property's @index
            // container is dropped.
            "@index"
                if term_container(&cur.active, active_property)
                    .iter()
                    .any(|c| c == "@index") =>
            {
                continue;
            }
            // step 12.6: @direction / @index / @language / @value pass through verbatim
            // under their aliases.
            "@direction" | "@index" | "@language" | "@value" => {
                let alias = cur.ciri(key, None, true, false);
                result.set(&alias, expanded_value.clone());
                continue;
            }
            _ => {}
        }

        // step 12.7: an empty-array value survives as an empty array under its term.
        if matches!(expanded_value, Json::Arr(a) if a.is_empty()) {
            let iap = cur.ciri(key, Some(expanded_value), true, inside_reverse);
            let nest = nest_target(&mut result, cur, &iap)?;
            add_value(nest, &iap, Json::Arr(Vec::new()), true);
            continue;
        }

        // step 12.8: per-item compaction. Expanded values are arrays; tolerate a bare
        // value defensively.
        let items: &[Json] = match expanded_value {
            Json::Arr(a) => a.as_slice(),
            other => std::slice::from_ref(other),
        };
        for item in items {
            // 12.8.1: the item's own term selection (container/type/language aware).
            let iap = cur.ciri(key, Some(item), true, inside_reverse);
            let container = term_container(&cur.active, Some(&iap));
            // 12.8.4: array-ness for this term.
            let as_array = container.iter().any(|c| c == "@set")
                || iap == "@graph"
                || iap == "@list"
                || !env.options.compact_arrays;

            let item_is_list = is_list_object(item);
            let item_is_graph = is_graph_object(item);

            // 12.8.5: recurse — a list/graph object contributes its @list/@graph value.
            let inner: &Json = if item_is_list {
                item.get("@list").expect("list object")
            } else if item_is_graph {
                item.get("@graph").expect("graph object")
            } else {
                item
            };
            let mut compacted_item = compact_element(cur, Some(&iap), inner, env)?;

            // 12.8.6: list objects.
            if item_is_list {
                if !matches!(compacted_item, Json::Arr(_)) {
                    compacted_item = Json::Arr(vec![compacted_item]);
                }
                if container.iter().any(|c| c == "@list") {
                    // A @list-container term holds exactly one list — set directly.
                    let nest = nest_target(&mut result, cur, &iap)?;
                    nest.set(&iap, compacted_item);
                    continue;
                }
                // Re-wrap as a list object under the @list alias (+ verbatim @index).
                let mut wrapper = Json::obj();
                wrapper.set(&cur.ciri("@list", None, true, false), compacted_item);
                if let Some(idx) = item.get("@index") {
                    wrapper.set(&cur.ciri("@index", None, true, false), idx.clone());
                }
                let nest = nest_target(&mut result, cur, &iap)?;
                add_value(nest, &iap, wrapper, as_array);
                continue;
            }

            // 12.8.7: graph objects — the four @graph container forms.
            if item_is_graph {
                compact_graph_item(&mut result, cur, &iap, container, item, compacted_item, as_array)?;
                continue;
            }

            // 12.8.9: @language / @index / @id / @type container maps (without @graph).
            let map_kind = ["@language", "@index", "@id", "@type"]
                .into_iter()
                .find(|k| container.iter().any(|c| c == k));
            if let Some(kind) = map_kind {
                if !container.iter().any(|c| c == "@graph") {
                    add_to_container_map(
                        &mut result,
                        cur,
                        &iap,
                        kind,
                        item,
                        compacted_item,
                        as_array,
                        env,
                    )?;
                    continue;
                }
            }

            // 12.8.10: plain term entry.
            let nest = nest_target(&mut result, cur, &iap)?;
            add_value(nest, &iap, compacted_item, as_array);
        }
    }

    Ok(result)
}

/// Step 12.8.7 — place one compacted **graph object** according to the `@graph`
/// container forms of its term, adding into `result` (through the `@nest` target).
fn compact_graph_item(
    result: &mut Json,
    cur: &Ctx,
    iap: &str,
    container: &[String],
    item: &Json,
    mut compacted_item: Json,
    as_array: bool,
) -> Result<(), JsonLdError> {
    let has_graph = container.iter().any(|c| c == "@graph");
    let has_id = container.iter().any(|c| c == "@id");
    let has_index = container.iter().any(|c| c == "@index");
    let simple = is_simple_graph(item);

    if has_graph && has_id {
        // 12.8.7.1: an @graph+@id map keyed by the (document-relative) graph name.
        let map_key = match item.get("@id").and_then(Json::as_str) {
            Some(id) => cur.ciri(id, None, false, false),
            None => cur.ciri("@none", None, true, false),
        };
        let nest = nest_target(result, cur, iap)?;
        let map_obj = get_or_create_map(nest, iap);
        add_value(map_obj, &map_key, compacted_item, as_array);
        return Ok(());
    }
    if has_graph && has_index && simple {
        // 12.8.7.2: an @graph+@index map keyed by the graph's @index; an absent
        // @index files under @none, IRI-COMPACTED (12.8.7.2.2 "IRI compacting that
        // value") so a context alias for @none is honoured — same as 12.8.7.1 and
        // 12.8.9.9 above/below.
        let map_key = item
            .get("@index")
            .and_then(Json::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| cur.ciri("@none", None, true, false));
        let nest = nest_target(result, cur, iap)?;
        let map_obj = get_or_create_map(nest, iap);
        add_value(map_obj, &map_key, compacted_item, as_array);
        return Ok(());
    }
    if has_graph && simple {
        // 12.8.7.3: a bare @graph container; several nodes need an @included wrapper so
        // they are not read back as distinct named graphs.
        if matches!(&compacted_item, Json::Arr(a) if a.len() > 1) {
            let mut wrapper = Json::obj();
            wrapper.set(&cur.ciri("@included", None, true, false), compacted_item);
            compacted_item = wrapper;
        }
        let nest = nest_target(result, cur, iap)?;
        add_value(nest, iap, compacted_item, as_array);
        return Ok(());
    }

    // 12.8.7.4: no matching @graph container — re-wrap as an explicit graph object.
    let mut wrapper = Json::obj();
    wrapper.set(&cur.ciri("@graph", None, true, false), compacted_item);
    if let Some(id) = item.get("@id").and_then(Json::as_str) {
        wrapper.set(
            &cur.ciri("@id", None, true, false),
            Json::Str(cur.ciri(id, None, false, false)),
        );
    }
    if let Some(idx) = item.get("@index") {
        wrapper.set(&cur.ciri("@index", None, true, false), idx.clone());
    }
    let nest = nest_target(result, cur, iap)?;
    add_value(nest, iap, wrapper, as_array);
    Ok(())
}

/// Step 12.8.9 — add one compacted item into a `@language` / `@index` / `@id` / `@type`
/// container map under its map key.
#[allow(clippy::too_many_arguments)]
fn add_to_container_map(
    result: &mut Json,
    cur: &Ctx,
    iap: &str,
    kind: &str,
    item: &Json,
    mut compacted_item: Json,
    as_array: bool,
    env: &Env,
) -> Result<(), JsonLdError> {
    // 12.8.9.2: the container key (the alias of the container keyword).
    let mut container_key = cur.ciri(kind, None, true, false);
    // 12.8.9.3: a property-valued index uses the term's index mapping instead of @index.
    let index_key = cur
        .active
        .term_definition(iap)
        .and_then(|d| d.index())
        .unwrap_or("@index")
        .to_string();
    let mut map_key: Option<String> = None;

    if kind == "@language" && item.get("@value").is_some() {
        // 12.8.9.4: language maps hold the bare @value; the key is the item's @language.
        compacted_item = item.get("@value").expect("guarded").clone();
        map_key = item
            .get("@language")
            .and_then(Json::as_str)
            .map(str::to_string);
    } else if kind == "@index" && index_key == "@index" {
        // 12.8.9.5: plain index maps key on the item's @index.
        map_key = item
            .get("@index")
            .and_then(Json::as_str)
            .map(str::to_string);
    } else if kind == "@index" {
        // 12.8.9.6: property-valued index maps — the key is the first value of the
        // (compacted) index property; remaining values stay on the property.
        container_key = cur.ciri(&index_key, None, true, false);
        if let Some(taken) = take_entry(&mut compacted_item, &container_key) {
            let mut vals = match taken {
                Json::Arr(a) => a,
                other => vec![other],
            };
            if !vals.is_empty() {
                let first = vals.remove(0);
                map_key = first.as_str().map(str::to_string);
                for v in vals {
                    add_value(&mut compacted_item, &container_key, v, false);
                }
                // A non-string first value cannot key a map — keep it on the property.
                if map_key.is_none() {
                    add_value(&mut compacted_item, &container_key, first, false);
                }
            }
        }
    } else if kind == "@id" {
        // 12.8.9.7: id maps key on the item's @id, which the map entry no longer repeats.
        // The spec takes the entry named by this context's @id alias, but the item was
        // compacted under the term's scoped context, which may alias @id differently and
        // reuse this alias for data; so the item is compacted again without its @id.
        if let Some(Json::Str(id)) = item.get("@id") {
            map_key = Some(cur.ciri(id, None, false, false));
            let mut rest = item.clone();
            take_entry(&mut rest, "@id");
            compacted_item = compact_element(cur, Some(iap), &rest, env)?;
        }
    } else if kind == "@type" {
        // 12.8.9.8: type maps key on the first compacted type; remaining types stay.
        if let Some(taken) = take_entry(&mut compacted_item, &container_key) {
            let mut vals = match taken {
                Json::Arr(a) => a,
                other => vec![other],
            };
            if !vals.is_empty() {
                let first = vals.remove(0);
                map_key = first.as_str().map(str::to_string);
                for v in vals {
                    add_value(&mut compacted_item, &container_key, v, false);
                }
                if map_key.is_none() {
                    add_value(&mut compacted_item, &container_key, first, false);
                }
            }
        }
        // 12.8.9.8.4: a leftover lone node reference re-compacts (it may collapse to a
        // string under an @id/@vocab-typed term).
        let lone_id = match &compacted_item {
            Json::Obj(m) if m.len() == 1 => {
                cur.active.expand_iri(&m[0].0, false, true).as_deref() == Some("@id")
            }
            _ => false,
        };
        if lone_id {
            let mut single = Json::obj();
            single.set(
                "@id",
                item.get("@id")
                    .cloned()
                    .unwrap_or(Json::Raw("null".to_string())),
            );
            compacted_item = compact_element(cur, Some(iap), &single, env)?;
        }
    }

    // 12.8.9.9: an absent key files under (a possibly aliased) @none.
    let map_key = map_key.unwrap_or_else(|| cur.ciri("@none", None, true, false));
    let nest = nest_target(result, cur, iap)?;
    let map_obj = get_or_create_map(nest, iap);
    add_value(map_obj, &map_key, compacted_item, as_array);
    Ok(())
}

// ---------------------------------------------------------------------------
// Value Compaction
// ---------------------------------------------------------------------------

/// **Value Compaction**. Returns `Some(compacted)` when the value object / node
/// reference compacts to a bare value under `active_property`'s term definition
/// (type-mapping match, `@id`/`@vocab` coercion, language + direction matching,
/// non-string literal, or a `@json` payload), or `None` when compaction is disabled and
/// the caller must fall through to the general (map-shaped) path.
fn value_compact(cur: &Ctx, active_property: Option<&str>, value: &Json) -> Option<Json> {
    let def = active_property.and_then(|p| cur.active.term_definition(p));
    let container: &[String] = def.map(|d| d.container()).unwrap_or(&[]);
    let type_mapping = def.and_then(|d| d.type_mapping());

    // steps 4-5: the effective language / direction for the property (term overrides
    // fall back to the context defaults; an explicit null suppresses).
    let language: Option<String> = match def.map(|d| d.language()) {
        Some(Override::Set(l)) => Some(l.clone()),
        Some(Override::Null) => None,
        _ => cur.active.default_language().map(str::to_string),
    };
    let direction: Option<Direction> = match def.map(|d| d.direction()) {
        Some(Override::Set(d)) => Some(*d),
        Some(Override::Null) => None,
        _ => cur.active.default_base_direction(),
    };

    // The @index pass-through condition shared by steps 9-10.
    let index_ok = value.get("@index").is_none() || container.iter().any(|c| c == "@index");

    let keys: Vec<&str> = match value {
        Json::Obj(m) => m.iter().map(|(k, _)| k.as_str()).collect(),
        _ => return None,
    };
    let tval = value.get("@type").and_then(Json::as_str);

    // step 6: a node reference (@id plus at most @index) under an @id/@vocab-typed term
    // compacts to the compacted IRI.
    if value.get("@id").is_some() && keys.iter().all(|k| matches!(*k, "@id" | "@index")) {
        if let Some(id) = value.get("@id").and_then(Json::as_str) {
            match type_mapping {
                Some("@id") => return Some(Json::Str(cur.ciri(id, None, false, false))),
                Some("@vocab") => return Some(Json::Str(cur.ciri(id, None, true, false))),
                _ => {}
            }
        }
        return None;
    }
    // step 7: a matching @type drops to the bare @value (this is also the @json path)
    // — GUARDED by `index_ok`: the REC's literal text has no @index condition here,
    // but dropping to a bare @value while the object carries an @index that no
    // @index container re-expresses would silently LOSE the @index (the
    // self-reparse-invisible data-loss class this module exists to prevent).
    // jsonld.js guards identically (`preserveIndex`); with the guard the value
    // falls through to the general map path, which keeps @type + @index verbatim.
    if tval.is_some() && tval == type_mapping && index_ok {
        return value.get("@value").cloned();
    }
    // step 8: compaction disabled — @none type mapping, or a non-matching @type.
    if type_mapping == Some("@none") || (tval.is_some() && tval != type_mapping) {
        return None;
    }
    // step 9: non-string literals compact whenever the @index (if any) is re-expressed
    // by an @index container.
    if let Some(v) = value.get("@value") {
        if !matches!(v, Json::Str(_)) {
            return if index_ok { Some(v.clone()) } else { None };
        }
        // step 10: string literals compact when language AND direction match the
        // property's effective mappings (case-insensitively; absence matches null).
        let vlang = value.get("@language").and_then(Json::as_str);
        let vdir = value.get("@direction").and_then(Json::as_str);
        let lang_matches = match (&language, vlang) {
            (Some(l), Some(vl)) => l.eq_ignore_ascii_case(vl),
            (None, None) => true,
            _ => false,
        };
        let dir_matches = match (direction, vdir) {
            (Some(d), Some(vd)) => d.as_str().eq_ignore_ascii_case(vd),
            (None, None) => true,
            _ => false,
        };
        if lang_matches && dir_matches && index_ok {
            return Some(v.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The spec's **add value** helper: adds `value` to `obj[key]`, promoting to (or
/// creating) an array per `as_array`, and flattening array values element-wise.
fn add_value(obj: &mut Json, key: &str, value: Json, as_array: bool) {
    if as_array {
        match obj.get(key) {
            None => obj.set(key, Json::Arr(Vec::new())),
            Some(Json::Arr(_)) => {}
            Some(existing) => {
                let e = existing.clone();
                obj.set(key, Json::Arr(vec![e]));
            }
        }
    }
    if let Json::Arr(items) = value {
        for v in items {
            add_value(obj, key, v, false);
        }
        return;
    }
    match obj.get(key) {
        None => obj.set(key, value),
        Some(Json::Arr(_)) => {
            if let Some(Json::Arr(items)) = obj_get_mut(obj, key) {
                items.push(value);
            }
        }
        Some(existing) => {
            let e = existing.clone();
            obj.set(key, Json::Arr(vec![e, value]));
        }
    }
}

/// Resolves the `@nest` target for a term: `result` itself, or the (created-on-demand)
/// nest map named by the term's nest mapping. Raises `invalid @nest value` when the nest
/// term neither is `@nest` nor expands to it.
fn nest_target<'a>(
    result: &'a mut Json,
    cur: &Ctx,
    iap: &str,
) -> Result<&'a mut Json, JsonLdError> {
    let nest_term = match cur.active.term_definition(iap).and_then(|d| d.nest()) {
        Some(n) => n.to_string(),
        None => return Ok(result),
    };
    if nest_term != "@nest"
        && cur.active.expand_iri(&nest_term, false, true).as_deref() != Some("@nest")
    {
        return Err(JsonLdError::with_detail(
            E::InvalidNestValue,
            format!("nest term {} does not expand to @nest", nest_term),
        ));
    }
    if result.get(&nest_term).is_none() {
        result.set(&nest_term, Json::obj());
    }
    Ok(obj_get_mut(result, &nest_term).expect("nest entry just ensured"))
}

/// Mutable member lookup on a JSON object (companion of [`Json::get`]).
fn obj_get_mut<'a>(obj: &'a mut Json, key: &str) -> Option<&'a mut Json> {
    match obj {
        Json::Obj(members) => members.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

/// Removes and returns the `key` member of a JSON object, if present.
fn take_entry(obj: &mut Json, key: &str) -> Option<Json> {
    match obj {
        Json::Obj(members) => {
            let idx = members.iter().position(|(k, _)| k == key)?;
            Some(members.remove(idx).1)
        }
        _ => None,
    }
}

/// The `key` map entry of `parent`, created as an empty map when absent.
fn get_or_create_map<'a>(parent: &'a mut Json, key: &str) -> &'a mut Json {
    if parent.get(key).is_none() {
        parent.set(key, Json::obj());
    }
    obj_get_mut(parent, key).expect("entry just ensured")
}

/// The container mapping of `term` in `active`, or an empty slice.
fn term_container<'a>(active: &'a ActiveContext, term: Option<&str>) -> &'a [String] {
    term.and_then(|t| active.term_definition(t))
        .map(|d| d.container())
        .unwrap_or(&[])
}

/// The string members of a `@type` value (a string or an array of strings).
fn type_strings(j: &Json) -> Vec<&str> {
    match j {
        Json::Str(s) => vec![s.as_str()],
        Json::Arr(a) => a.iter().filter_map(Json::as_str).collect(),
        _ => Vec::new(),
    }
}

/// True iff `j` is a JSON `null`.
fn is_null(j: &Json) -> bool {
    matches!(j, Json::Raw(r) if r == "null")
}

/// True iff `j` is a scalar (string, number, or boolean — not `null`).
fn is_scalar(j: &Json) -> bool {
    match j {
        Json::Str(_) => true,
        Json::Raw(r) => r != "null",
        _ => false,
    }
}

/// True iff `j` is a list object (a map with an `@list` entry).
fn is_list_object(j: &Json) -> bool {
    j.is_obj() && j.get("@list").is_some()
}

/// True iff `j` is a graph object: a map with `@graph` whose other entries are at most
/// `@id`, `@index`, and `@context`.
fn is_graph_object(j: &Json) -> bool {
    match j {
        Json::Obj(members) => {
            j.get("@graph").is_some()
                && members
                    .iter()
                    .all(|(k, _)| matches!(k.as_str(), "@graph" | "@id" | "@index" | "@context"))
        }
        _ => false,
    }
}

/// True iff `j` is a **simple** graph object (a graph object without `@id`).
fn is_simple_graph(j: &Json) -> bool {
    is_graph_object(j) && j.get("@id").is_none()
}

/// True iff a caller context value is empty (`null`, `{}`, or `[]`) — an empty context
/// is not embedded in the compacted output.
fn context_is_empty(ctx: &Json) -> bool {
    match ctx {
        Json::Raw(r) => r == "null",
        Json::Obj(m) => m.is_empty(),
        Json::Arr(a) => a.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopLoader;

    // The cached root context outlives each call, so its IRI memo must stay bounded
    // however many distinct node ids pass through it.
    #[test]
    fn cached_root_memo_is_bounded() {
        let ctx = Json::parse(r#"{"@vocab":"http://ex/"}"#).unwrap();
        let opts = JsonLdOptions::default();
        for batch in 0..3 {
            let nodes: Vec<String> = (0..MEMO_CAP)
                .map(|i| format!(r#"{{"@id":"http://ex/n{batch}-{i}","http://ex/p":[{{"@value":"v"}}]}}"#))
                .collect();
            let doc = Json::parse(&format!("[{}]", nodes.join(","))).unwrap();
            compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        }
        let len = LAST_ROOT.with(|last| last.borrow().as_ref().map(|(_, _, c)| c.memo.borrow().len()));
        assert!(len.is_some_and(|n| n <= MEMO_CAP), "memo size {len:?}");
    }

    // Language tags and datatypes make value shapes unbounded under one IRI, so the
    // shapes memoised per IRI are capped too.
    #[test]
    fn memo_shapes_per_iri_are_bounded() {
        let ctx = Json::parse(r#"{"@vocab":"http://ex/","@language":"en"}"#).unwrap();
        let opts = JsonLdOptions::default();
        for i in 0..4 * MEMO_SHAPES {
            let doc = Json::parse(&format!(
                r#"[{{"@id":"http://ex/s","http://ex/p":[{{"@value":"v","@language":"en-x-{i:08}"}}]}}]"#
            ))
            .unwrap();
            compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        }
        let shapes = LAST_ROOT.with(|last| {
            let last = last.borrow();
            let (_, _, c) = last.as_ref().unwrap();
            let memo = c.memo.borrow();
            memo.get("http://ex/p").map(Vec::len)
        });
        assert!(shapes.is_some_and(|n| n <= MEMO_SHAPES), "shapes {shapes:?}");
    }
}
