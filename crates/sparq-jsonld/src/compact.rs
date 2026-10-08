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
use crate::context::{budget, ActiveContext, Direction, Override};
use crate::error::{JsonLdError, JsonLdErrorCode as E};
use crate::expand::{expand, expand_value};
use crate::fx::FxMap;
use crate::json::{Json, MAX_DEPTH};
use crate::loader::DocumentLoader;
use crate::options::{JsonLdOptions, ProcessingMode};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    crate::context::budget::with_budget(|| compact_inner(input, context, options, loader))
}

fn compact_inner(
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
    crate::context::budget::with_budget(|| compact_expanded_inner(expanded, context, options, loader))
}

fn compact_expanded_inner(
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
        visits: Cell::new(json_size(expanded).saturating_mul(VISITS_PER_VALUE).saturating_add(64)),
    };

    // The Compaction Algorithm proper, with a null active property.
    let compacted = compact_element(ctx, None, expanded, Read::Value, &env);
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
        // A root context with `@propagate: false` applies to the top-level node it is
        // embedded in, but the node was compacted under the reverted context, so a lone
        // node keeps the @graph envelope that puts it one level below the context.
        Json::Obj(members) if ctx.active.previous_context.is_some() && !members.is_empty() => {
            let mut obj = Json::obj();
            obj.set(&ctx.ciri("@graph", None, true, false), Json::Arr(vec![Json::Obj(members)]));
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
/// `@import`. A context nested deeper than a parsed one can be is not checked (false).
fn self_contained(context: &Json) -> bool {
    fn local(ctx: &Json, depth: usize) -> bool {
        match ctx {
            Json::Str(_) => false,
            Json::Arr(items) => depth < MAX_DEPTH && items.iter().all(|c| local(c, depth + 1)),
            other => nested_ok(other, depth),
        }
    }
    fn nested_ok(j: &Json, depth: usize) -> bool {
        match j {
            Json::Obj(_) | Json::Arr(_) if depth >= MAX_DEPTH => false,
            Json::Obj(m) => m.iter().all(|(k, v)| match k.as_str() {
                "@import" => false,
                "@context" => local(v, depth + 1),
                _ => nested_ok(v, depth + 1),
            }),
            Json::Arr(a) => a.iter().all(|v| nested_ok(v, depth + 1)),
            _ => true,
        }
    }
    local(context, 0)
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
    /// Calls of [`compact_element`] on arrays and objects left in this compaction: a
    /// small multiple of the input's, so no re-compaction can make the work superlinear.
    visits: Cell<usize>,
}

impl Env<'_> {
    /// Charges one call of [`compact_element`].
    fn visit(&self) -> Result<(), JsonLdError> {
        let left = self.visits.get();
        if left == 0 {
            return Err(JsonLdError::with_detail(
                E::ContextOverflow,
                "compaction exceeds its work bound for this input",
            ));
        }
        self.visits.set(left - 1);
        Ok(())
    }
}

/// The number of arrays and objects in `json`.
fn json_size(json: &Json) -> usize {
    let nested = |j: &&Json| matches!(j, Json::Arr(_) | Json::Obj(_));
    let mut size = 0;
    let mut stack = vec![json];
    while let Some(j) = stack.pop() {
        size += 1;
        match j {
            Json::Arr(items) => stack.extend(items.iter().filter(nested)),
            Json::Obj(members) => stack.extend(members.iter().map(|(_, v)| v).filter(nested)),
            _ => {}
        }
    }
    size
}

/// Calls of [`compact_element`] on an array or object allowed per input array or object:
/// one for the value itself and one for the lone node reference a type map may compact
/// again.
const VISITS_PER_VALUE: usize = 2;

/// Identity of a derived context: the context reverted to, or the scoped context of
/// `term` (looked up in `lookup`) applied onto `base`, both named by [`Ctx::id`].
enum DerivedKey {
    Revert(Arc<ActiveContext>),
    Scoped {
        lookup: u64,
        base: u64,
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
    ///
    /// Building runs context processing, which charges the call's work budget (see
    /// `context::budget`); building the inverse context is charged here too. A built
    /// context is cached while the cache holds fewer than [`DERIVED_CAP`] contexts and
    /// [`RETAIN_BUDGET`] definitions (counting those its reversion target keeps alive);
    /// from the first one that does not fit until the call ends, contexts are used and
    /// dropped, as without the cache.
    fn derived(
        &self,
        key: DerivedKey,
        build: impl FnOnce() -> Result<ActiveContext, JsonLdError>,
    ) -> Result<Rc<Ctx>, JsonLdError> {
        let root = self.root;
        if let Some((_, c)) = root.derived.borrow().iter().find(|(k, _)| k.same(&key)) {
            return Ok(Rc::clone(c));
        }
        let active = build()?;
        budget::charge(active.term_count() + 1)?;
        let mut held = active.term_count() + 1;
        let mut prev = active.previous_context.as_deref();
        while let Some(p) = prev {
            held += p.term_count() + 1;
            prev = p.previous_context.as_deref();
        }
        let retained = root.retained.get() + held;
        let ctx = Rc::new(Ctx::with_budget(active, Rc::clone(&root.memo_used)));
        // Once a context has not fitted, the cache takes no more until the next call.
        let fits = !root.overflowed.get()
            && root.derived.borrow().len() < DERIVED_CAP
            && retained <= RETAIN_BUDGET;
        if fits {
            root.retained.set(retained);
            root.derived.borrow_mut().push((key, Rc::clone(&ctx)));
        } else {
            root.overflowed.set(true);
        }
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
    /// the root cached in [`LAST_ROOT`], once across calls), within the limits in
    /// [`Env::derived`]. Keys name contexts by id, so a dropped context is never mistaken
    /// for a later one.
    derived: RefCell<Vec<(DerivedKey, Rc<Ctx>)>>,
    /// IRIs memoised across a root and its derived contexts, shared between them.
    memo_used: Rc<Cell<usize>>,
    /// This context's identity in [`DerivedKey`]s; never reused.
    id: u64,
    /// On the root context only: term definitions held by the cached derived contexts.
    retained: Cell<usize>,
    /// On the root context only: whether a context this call did not fit in the cache.
    overflowed: Cell<bool>,
}

impl Ctx {
    fn new(active: ActiveContext) -> Ctx {
        Ctx::with_budget(active, Rc::default())
    }

    /// A context whose memo insertions count against `memo_used`.
    fn with_budget(active: ActiveContext, memo_used: Rc<Cell<usize>>) -> Ctx {
        let inverse = active.inverse_context();
        Ctx {
            active,
            inverse,
            memo: RefCell::default(),
            derived: RefCell::default(),
            memo_used,
            id: NEXT_CTX_ID.fetch_add(1, Ordering::Relaxed),
            retained: Cell::new(0),
            overflowed: Cell::new(false),
        }
    }

    /// Clears this root's IRI memos (its own and its derived contexts') once they hold
    /// more than [`MEMO_CAP`] IRIs between them. The root outlives the call in
    /// [`LAST_ROOT`] and node ids differ per document, so without this the memos would keep
    /// every IRI ever compacted on the thread. Within one call they hold up to
    /// [`MEMO_CALL_CAP`] IRIs, which keeps large documents fast, and each IRI holds at most
    /// [`MEMO_SHAPES`] value shapes. The derived contexts are dropped here once the cache
    /// has run out of room.
    fn trim_memos(&self) {
        if self.overflowed.replace(false) || self.derived.borrow().len() >= DERIVED_CAP {
            self.derived.borrow_mut().clear();
            self.retained.set(0);
            self.memo_used.set(self.memo.borrow().len());
        }
        if self.memo_used.get() > MEMO_CAP {
            self.memo.borrow_mut().clear();
            for (_, c) in self.derived.borrow().iter() {
                c.memo.borrow_mut().clear();
            }
            self.memo_used.set(0);
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
            // A new IRI is memoised only within the budget shared with the root.
            None if self.memo_used.get() < MEMO_CALL_CAP => {
                self.memo_used.set(self.memo_used.get() + 1);
                memo.entry(iri.to_string()).or_default()
            }
            None => return result,
        };
        // Shapes vary with datatypes and language tags, so they are capped per IRI too.
        if entries.len() < MEMO_SHAPES {
            entries.push((shape.to_owned(), vocab, reverse, result.clone()));
        }
        result
    }
}

/// Most distinct IRIs a root and its derived contexts keep memoised between calls (see
/// [`Ctx::trim_memos`]).
const MEMO_CAP: usize = 4096;

/// Most distinct IRIs a root and its derived contexts memoise within one call.
const MEMO_CALL_CAP: usize = 1 << 16;

/// Most value shapes memoised per IRI.
const MEMO_SHAPES: usize = 8;

/// Most derived (scoped or reverted) contexts a root keeps.
const DERIVED_CAP: usize = 256;

/// Most term definitions the cached derived contexts hold between them.
#[cfg(not(test))]
const RETAIN_BUDGET: usize = 1 << 18;
#[cfg(test)]
const RETAIN_BUDGET: usize = 1 << 16;

/// The source of [`Ctx::id`]s.
static NEXT_CTX_ID: AtomicU64 = AtomicU64::new(0);

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
/// term (or keyword) whose value `element` is; `None` at the document root. `read` is how
/// expansion will read the result (see [`Read`]).
///
/// Every recursive descent of compaction comes back through here, so this is where the
/// walk's depth and visit bounds are charged.
fn compact_element(
    ctx: &Ctx,
    active_property: Option<&str>,
    element: &Json,
    read: Read,
    env: &Env,
) -> Result<Json, JsonLdError> {
    // step 1: retain the incoming context — values may be relevant to a previous
    // type-scoped context (used for @type compaction + type-scoped term lookups below).
    let type_scoped = ctx;

    // step 2: scalars (and null) are already in compact form.
    if is_scalar(element) || is_null(element) {
        return Ok(element.clone());
    }
    env.visit()?;

    // step 3: arrays — compact each item (dropping nulls), then collapse a singleton
    // unless disallowed by compactArrays / @graph / @set / a @list-@set container.
    if let Json::Arr(items) = element {
        let mut result: Vec<Json> = Vec::new();
        for item in items {
            // Recursion is counted per object; an array directly in an array counts too.
            let _nested = if matches!(item, Json::Arr(_)) { Some(budget::nest()?) } else { None };
            let compacted = compact_element(ctx, active_property, item, read, env)?;
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

    let owned = node_ctx(ctx, active_property, element, read, env)?;
    let cur: &Ctx = owned.as_deref().unwrap_or(ctx);

    // step 6: value objects / node references — Value Compaction. Return the result when
    // it is a scalar, or unconditionally for a @json-typed term (its payload is raw JSON).
    if element.get("@value").is_some() || element.get("@id").is_some() {
        if let Some(v) = value_compact(cur, active_property, element, read == Read::IndexKey) {
            let json_mapped = active_property
                .and_then(|p| cur.active.term_definition(p))
                .and_then(|d| d.type_mapping())
                == Some("@json");
            if is_scalar(&v) || json_mapped {
                return Ok(v);
            }
        }
    }

    // Everything below recurses into this object's members.
    let _nested = budget::nest()?;

    // step 7: a list object under a @list-container term compacts to its bare items.
    // A bare array reverts no context, so they are read (and compacted) under the
    // incoming one.
    if is_list_object(element)
        && term_container(&ctx.active, active_property)
            .iter()
            .any(|c| c == "@list")
    {
        let list = element.get("@list").expect("list object");
        return compact_element(ctx, active_property, list, read, env);
    }

    // A list object outside a @list-container term (one nested in another list) is read
    // with its items under the enclosing active property, not under @list, so they are
    // compacted that way too (a type coercion on the property applies to them).
    if is_list_object(element) {
        let list = element.get("@list").expect("list object");
        let items = match compact_element(cur, active_property, list, Read::Value, env)? {
            items @ Json::Arr(_) => items,
            other => Json::Arr(vec![other]),
        };
        let mut wrapper = Json::obj();
        wrapper.set(&cur.ciri("@list", None, true, false), items);
        if let Some(idx) = element.get("@index") {
            wrapper.set(&cur.ciri("@index", None, true, false), idx.clone());
        }
        if read == Read::Value {
            check_read(ctx, &wrapper, element)?;
        }
        return Ok(wrapper);
    }

    // step 8: reverse-property scope.
    let inside_reverse = active_property == Some("@reverse");

    // Expansion finds @type entries here, after property-scoped contexts and before
    // type-scoped ones.
    let before_types = cur;
    let owned_t = type_ctx(type_scoped, cur, element, env)?;
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
                    Json::Str(s) => Json::Str(type_term(type_scoped, before_types, s)),
                    Json::Arr(ts) => Json::Arr(
                        ts.iter()
                            .map(|t| match t {
                                Json::Str(s) => Json::Str(type_term(type_scoped, before_types, s)),
                                other => other.clone(),
                            })
                            .collect(),
                    ),
                    other => other.clone(),
                };
                // Expansion finds @type entries before it applies their type-scoped
                // contexts, so the key must read as @type both then and after; an alias
                // only one of those contexts defines would turn the types into data.
                let alias = before_types.ciri("@type", None, true, false);
                let alias = if cur.active.expand_iri(&alias, false, true).as_deref() == Some("@type") {
                    alias
                } else {
                    "@type".to_string()
                };
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
                let compacted = compact_element(cur, Some("@reverse"), expanded_value, Read::Value, env)?;
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
                let compacted = compact_element(cur, active_property, expanded_value, read, env)?;
                if !matches!(expanded_value, Json::Arr(a) if a.is_empty()) {
                    result.set("@preserve", compacted);
                }
                continue;
            }
            // step 12.5: an @index re-expressed by the active property's @index
            // container is dropped.
            "@index" if read == Read::IndexKey => {
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
            let list_container = container.iter().any(|c| c == "@list");
            // A plain @index map keys an item (not a graph's nodes) by the item's @index.
            let read = Read::of(container);
            let plain_index =
                read == Read::IndexMap && cur.active.term_definition(&iap).is_some_and(|d| d.index().is_none());
            let read = if plain_index && !item_is_graph { Read::IndexKey } else { read };

            // 12.8.5: recurse — a list/graph object contributes its @list/@graph value.
            let inner: &Json = if item_is_list {
                item.get("@list").expect("list object")
            } else if item_is_graph {
                item.get("@graph").expect("graph object")
            } else {
                item
            };

            // A @list-container term holds exactly one list. The spec overwrites an
            // earlier list here; to keep it, a further list on the same property goes
            // under the absolute IRI as a list object (an error when that spelling does
            // not expand back to it). Decided before recursing, so each item is
            // compacted once.
            let second_list = item_is_list
                && list_container
                && nest_target(&mut result, cur, &iap)?.get(&iap).is_some();
            if second_list
                && (cur.active.term_definition(key).is_some()
                    || cur.active.expand_iri(key, false, true).as_deref() != Some(key))
            {
                return Err(JsonLdError::with_detail(
                    E::InvalidSetOrListObject,
                    format!("several lists for the @list term {iap}"),
                ));
            }
            // The property the item is written under: the absolute IRI carries no scoped
            // context, which could change how the second list's values read back.
            let property: &str = if second_list { key } else { &iap };

            // An @id map entry does not repeat the item's @id, which keys the map. The spec
            // takes the entry named by this context's @id alias, but the item's own
            // (scoped) context may alias @id differently and reuse this alias for data,
            // so the item is compacted without its @id instead.
            let id_keyed = read == Read::Reverted
                && !item_is_graph
                && container.iter().any(|c| c == "@id")
                && matches!(item.get("@id"), Some(Json::Str(_)));
            let without_id;
            let inner: &Json = if id_keyed {
                let mut rest = item.clone();
                take_entry(&mut rest, "@id");
                without_id = rest;
                &without_id
            } else {
                inner
            };

            // Expansion reads an explicit list or graph object's contents under the
            // wrapper's own context (from `read`, with the property's scoped context): a
            // list's items as values of the property, a graph's nodes as values of @graph,
            // and the nodes of a bare @graph container's @included wrapper with no
            // property. A @list-container term's bare array, a @graph map's entries and a
            // bare @graph container's lone node are read as the property's values.
            let has = |k: &str| container.iter().any(|c| c == k);
            let simple = item_is_graph && is_simple_graph(item);
            // An empty graph only survives as an explicit graph object (see
            // `compact_graph_item`).
            let filled = !matches!(inner, Json::Arr(a) if a.is_empty());
            // A property-valued @graph @index map would read its key as a property, so
            // there graphs are written as explicit graph objects. (An @graph @id map and
            // a bare @graph container drop a graph's @index, as W3C compact/0088 and
            // 0079 require.)
            let graph_entry = item_is_graph
                && filled
                && has("@graph")
                && (has("@id") || (has("@index") && simple && plain_index));
            let lone_node = matches!(inner, Json::Arr(a) if a.len() == 1);
            let bare_graph = item_is_graph && filled && has("@graph") && simple && read == Read::Value;
            let wrapped = (item_is_list && (!list_container || second_list))
                || (item_is_graph && !graph_entry && !(bare_graph && lone_node));
            let wrapper_ctx = if wrapped { node_ctx(cur, Some(property), item, read, env)? } else { None };
            let wctx: &Ctx = wrapper_ctx.as_deref().unwrap_or(cur);
            let mut compacted_item = if !wrapped {
                compact_element(cur, Some(property), inner, read, env)?
            } else if item_is_list {
                compact_element(wctx, Some(property), inner, Read::Value, env)?
            } else if bare_graph {
                compact_element(wctx, None, inner, Read::Value, env)?
            } else {
                // Read as @graph's value, but written like the property's (12.8.7.4).
                match compact_element(wctx, Some("@graph"), inner, Read::Value, env)? {
                    Json::Arr(mut nodes) if nodes.len() == 1 && env.options.compact_arrays => {
                        nodes.pop().expect("one node")
                    }
                    nodes => nodes,
                }
            };

            // 12.8.6: list objects.
            if item_is_list {
                if !matches!(compacted_item, Json::Arr(_)) {
                    compacted_item = Json::Arr(vec![compacted_item]);
                }
                if list_container && !second_list {
                    nest_target(&mut result, cur, &iap)?.set(&iap, compacted_item);
                    continue;
                }
                // Re-wrap as a list object under the @list alias (+ verbatim @index),
                // spelled under the context expansion reads the wrapper with.
                let mut wrapper = Json::obj();
                wrapper.set(&wctx.ciri("@list", None, true, false), compacted_item);
                if let Some(idx) = item.get("@index") {
                    wrapper.set(&wctx.ciri("@index", None, true, false), idx.clone());
                }
                if second_list {
                    check_read(cur, &wrapper, item)?;
                    add_value(&mut result, key, wrapper, false);
                } else {
                    place_wrapper(&mut result, cur, &iap, container, item, wrapper, as_array)?;
                }
                continue;
            }

            // 12.8.7: graph objects — the four @graph container forms.
            if item_is_graph {
                compact_graph_item(&mut result, cur, wctx, &iap, container, item, compacted_item, as_array)?;
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

    if read == Read::Value {
        check_read(ctx, &result, element)?;
    }
    Ok(result)
}

/// Step 12.8.7 — place one compacted **graph object** according to the `@graph`
/// container forms of its term, adding into `result` (through the `@nest` target).
#[allow(clippy::too_many_arguments)]
fn compact_graph_item(
    result: &mut Json,
    cur: &Ctx,
    wctx: &Ctx,
    iap: &str,
    container: &[String],
    item: &Json,
    mut compacted_item: Json,
    as_array: bool,
) -> Result<(), JsonLdError> {
    // The @graph container forms read an empty graph back as no value at all.
    let filled = !matches!(item.get("@graph"), Some(Json::Arr(a)) if a.is_empty());
    let has_graph = filled && container.iter().any(|c| c == "@graph");
    let has_id = container.iter().any(|c| c == "@id");
    let has_index = container.iter().any(|c| c == "@index");
    let simple = is_simple_graph(item);

    if has_graph && has_id {
        // 12.8.7.1: an @graph+@id map keyed by the (document-relative) graph name.
        let map_key = match item.get("@id").and_then(Json::as_str) {
            Some(id) => id_key(cur, id),
            None => cur.ciri("@none", None, true, false),
        };
        let nest = nest_target(result, cur, iap)?;
        let map_obj = get_or_create_map(nest, iap);
        add_value(map_obj, &map_key, compacted_item, as_array);
        return Ok(());
    }
    let plain_index = cur.active.term_definition(iap).is_some_and(|d| d.index().is_none());
    if has_graph && has_index && simple && plain_index {
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
    // Expansion reads a wrapper's keys under `wctx`, the context it gives the
    // property's values.
    if has_graph && simple && !has_id && !has_index {
        // 12.8.7.3: a bare @graph container; several nodes need an @included wrapper so
        // they are not read back as distinct named graphs.
        if matches!(&compacted_item, Json::Arr(a) if a.len() > 1) {
            let mut wrapper = Json::obj();
            wrapper.set(&wctx.ciri("@included", None, true, false), compacted_item);
            check_read(cur, &wrapper, item)?;
            compacted_item = wrapper;
        }
        let nest = nest_target(result, cur, iap)?;
        add_value(nest, iap, compacted_item, as_array);
        return Ok(());
    }

    // A bare @graph container wraps every value in a graph object, so an empty graph,
    // which only an explicit graph object can carry, cannot go under it. (A named one
    // is written as the explicit graph object W3C compact/0080 requires, which reads
    // back as a graph holding that graph.)
    let graph_container = container.iter().any(|c| c == "@graph");
    if !filled && graph_container && !has_id && !has_index {
        return Err(JsonLdError::with_detail(
            E::InvalidSetOrListObject,
            format!("this graph cannot go under the @graph container term {iap}"),
        ));
    }
    // 12.8.7.4: no matching @graph container — re-wrap as an explicit graph object.
    let mut wrapper = Json::obj();
    wrapper.set(&wctx.ciri("@graph", None, true, false), compacted_item);
    if let Some(id) = item.get("@id").and_then(Json::as_str) {
        wrapper.set(&wctx.ciri("@id", None, true, false), Json::Str(id_key(wctx, id)));
    }
    if let Some(idx) = item.get("@index") {
        wrapper.set(&wctx.ciri("@index", None, true, false), idx.clone());
    }
    if graph_container && !has_id && (!has_index || (plain_index && filled)) {
        // A bare @graph container and a plain @graph @index map take the explicit
        // graph object directly (W3C compact/0080, 0083).
        check_read(cur, &wrapper, item)?;
        let nest = nest_target(result, cur, iap)?;
        add_value(nest, iap, wrapper, as_array);
        return Ok(());
    }
    place_wrapper(result, cur, iap, container, item, wrapper, as_array)
}

/// Adds the list or graph object `wrapper` (for `item`) under `iap`. A map container
/// would read the wrapper's own keys as map keys, so there it goes under the map's
/// `@none` key, which expansion adds nothing for.
fn place_wrapper(
    result: &mut Json,
    cur: &Ctx,
    iap: &str,
    container: &[String],
    item: &Json,
    wrapper: Json,
    as_array: bool,
) -> Result<(), JsonLdError> {
    let has = |k: &str| container.iter().any(|c| c == k);
    if has("@language") {
        return Err(JsonLdError::with_detail(
            E::InvalidLanguageMapValue,
            format!("a list or graph object cannot go in the language map {iap}"),
        ));
    }
    let nest = nest_target(result, cur, iap)?;
    if Read::of(container) == Read::Value {
        check_read(cur, &wrapper, item)?;
        add_value(nest, iap, wrapper, as_array);
    } else {
        let none = cur.ciri("@none", None, true, false);
        add_value(get_or_create_map(nest, iap), &none, wrapper, as_array);
    }
    Ok(())
}

/// The (document-relative) spelling of `id` as a map key or graph name under `cur`: its
/// compacted form when that expands back to it and does not read as an `@none` alias,
/// else the full id.
fn id_key(cur: &Ctx, id: &str) -> String {
    let key = cur.ciri(id, None, false, false);
    let round_trips = cur.active.expand_iri(&key, true, false).as_deref() == Some(id)
        && cur.active.expand_iri(&key, false, true).as_deref() != Some("@none");
    if round_trips {
        key
    } else {
        id.to_string()
    }
}

/// Whether the type-map key `key` brings the same scoped context into expansion as into
/// compaction of `item`. Expansion looks the key up in `map_ctx` (the reverted context),
/// compaction applied the item's type-scoped contexts in `node`, so both must agree.
/// Expansion also applies a key's scoped context before the property's (`iap`) and lets
/// it propagate into nested objects, unlike a node's own type-scoped contexts, and reads
/// the item's other types under it, so with one the property must carry no scoped
/// context, either before the key's context or under it (as in `typed`, the item's
/// context), and the item must hold only values and no other type.
fn type_key_scoped_alike(cur: &Ctx, map_ctx: &Ctx, node: &Ctx, typed: &Ctx, iap: &str, key: &str, item: &Json) -> bool {
    let scoped = |c: &Ctx| c.active.term_definition(key).and_then(|d| d.context()).cloned();
    let expanded = scoped(map_ctx);
    if expanded != scoped(node) {
        return false;
    }
    let lone_type = item.get("@type").is_some_and(|t| type_strings(t).len() == 1);
    expanded.is_none()
        || ([cur, typed].iter().all(|c| c.active.term_definition(iap).and_then(|d| d.context()).is_none())
            && lone_type
            && !embeds_nodes(item))
}

/// Whether a property value of the node object `node` is anything but a value object:
/// a node object or reference, or a list or graph object, whose context reverts.
fn embeds_nodes(node: &Json) -> bool {
    let not_value = |v: &Json| matches!(v, Json::Obj(_)) && v.get("@value").is_none();
    match node {
        Json::Obj(members) => members.iter().any(|(k, v)| match (k.as_str(), v) {
            ("@id" | "@type" | "@index", _) => false,
            (_, Json::Arr(items)) => items.iter().any(not_value),
            (_, v) => not_value(v),
        }),
        _ => false,
    }
}

/// How expansion reads a value of a property, which decides the context the value is
/// compacted in (see [`node_ctx`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Read {
    /// A plain property value (or one in a `@list`, `@set` or bare `@graph` container):
    /// a non-propagated context reverts unless the value reads as a value object or a
    /// lone node reference, and the property's scoped context is looked up before that.
    Value,
    /// An `@index` map entry: nothing reverts.
    IndexMap,
    /// An `@index` map entry keyed by its own `@index` (a plain `@index` map), which the
    /// entry therefore does not repeat. Read as [`Read::IndexMap`].
    IndexKey,
    /// An `@id` or `@type` map entry: the map context is always the reverted one, and the
    /// property's scoped context is looked up in it.
    Reverted,
}

impl Read {
    /// How expansion reads a value of a property with `container` (an item, or the list
    /// or graph object wrapping it): a map container makes it a map entry.
    fn of(container: &[String]) -> Read {
        let has = |k: &str| container.iter().any(|c| c == k);
        if has("@id") || has("@type") {
            Read::Reverted
        } else if has("@index") {
            Read::IndexMap
        } else {
            Read::Value
        }
    }
}

/// Checks that expansion, reading `out` (the compacted `item`) as a property value under
/// `incoming`, makes the same step-4 reversion choice compaction made from `item`.
/// Expansion decides by `out`'s keys, so an alias that reads as `@value` or `@id` only
/// in `incoming` (a type-scoped context) would make it read the object differently.
fn check_read(incoming: &Ctx, out: &Json, item: &Json) -> Result<(), JsonLdError> {
    let (Some(_), Json::Obj(members)) = (&incoming.active.previous_context, out) else {
        return Ok(());
    };
    let reads_as = |k: &str, kw: &str| k == kw || incoming.active.expand_iri(k, false, true).as_deref() == Some(kw);
    let expansion_keeps =
        members.iter().any(|(k, _)| reads_as(k, "@value")) || (members.len() == 1 && reads_as(&members[0].0, "@id"));
    let lone_ref = matches!(item, Json::Obj(m) if m.len() == 1 && m[0].0 == "@id");
    let compaction_kept = item.get("@value").is_some() || lone_ref;
    if expansion_keeps == compaction_kept {
        Ok(())
    } else {
        Err(JsonLdError::with_detail(
            E::InvalidScopedContext,
            "a type-scoped keyword alias would change how a compacted value reads back",
        ))
    }
}

/// Steps 4–5: the context a node object `element` under `active_property` is compacted
/// in, when it differs from `ctx` (previous-context reversion or a property-scoped
/// context). Every context switch of the compaction walk goes through here and through
/// [`type_ctx`]; `read` mirrors how expansion will choose the context for the value.
fn node_ctx(
    ctx: &Ctx,
    active_property: Option<&str>,
    element: &Json,
    read: Read,
    env: &Env,
) -> Result<Option<Rc<Ctx>>, JsonLdError> {
    let mut owned: Option<Rc<Ctx>> = None;

    // step 4: non-propagated (type-scoped) contexts do not apply when processing a new
    // node object — revert to the previous context unless element is a value object or
    // a lone node reference. Expansion's map entries decide this by container instead.
    if let Some(prev) = &ctx.active.previous_context {
        let revert = match read {
            Read::Value => {
                let single_id = matches!(element, Json::Obj(m) if m.len() == 1 && m[0].0 == "@id");
                element.get("@value").is_none() && !single_id
            }
            Read::IndexMap | Read::IndexKey => false,
            Read::Reverted => true,
        };
        if revert {
            owned = Some(env.derived(DerivedKey::Revert(Arc::clone(prev)), || {
                budget::charge(prev.term_count() + 1)?;
                Ok((**prev).clone())
            })?);
        }
    }

    // step 5: apply the active property's property-scoped context, if any. The term
    // LOOKUP runs against the INCOMING (pre-reversion) context — a term defined by the
    // parent's type-scoped context still carries its property-scoped context into the
    // child node (W3C compact/c013) — while the application folds onto the (possibly
    // reverted) context from step 4, mirroring the reference implementations.
    // An @id or @type map looks the property up in the reverted map context instead.
    if let Some(ap) = active_property {
        let lookup: &Ctx = match read {
            Read::Reverted => owned.as_deref().unwrap_or(ctx),
            _ => ctx,
        };
        if let Some(def) = lookup.active.term_definition(ap) {
            if let Some(local) = def.context() {
                let base: &Ctx = owned.as_deref().unwrap_or(ctx);
                let key = DerivedKey::Scoped {
                    lookup: lookup.id,
                    base: base.id,
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
    Ok(owned)
}

/// Step 12.2's spelling of the type `iri`. The spec compacts it against `type_scoped`, the
/// incoming context, but expansion reads @type values under `expansion` (the node's
/// property-scoped context), which may give that spelling another meaning; then the
/// spelling is taken from `expansion` itself, or the IRI is kept whole.
fn type_term(type_scoped: &Ctx, expansion: &Ctx, iri: &str) -> String {
    if std::ptr::eq(type_scoped, expansion) {
        return type_scoped.ciri(iri, None, true, false);
    }
    let reads_back = |t: &str| expansion.active.expand_iri(t, true, true).as_deref() == Some(iri);
    let term = type_scoped.ciri(iri, None, true, false);
    if reads_back(&term) {
        return term;
    }
    let term = expansion.ciri(iri, None, true, false);
    if reads_back(&term) {
        term
    } else {
        iri.to_string()
    }
}

/// Step 9 — the context with the type-scoped contexts of `element`'s types applied
/// onto `cur` (in lexicographic order of the compacted types, propagate false). Type
/// terms are spelled by [`type_term`] and looked up in `cur`, as expansion will read
/// them. `None` when no type carries a scoped context.
fn type_ctx(
    type_scoped: &Ctx,
    cur: &Ctx,
    element: &Json,
    env: &Env,
) -> Result<Option<Rc<Ctx>>, JsonLdError> {
    let mut owned_t: Option<Rc<Ctx>> = None;
    if let Some(types) = element.get("@type") {
        let mut compacted_types: Vec<String> = type_strings(types)
            .into_iter()
            .map(|t| type_term(type_scoped, cur, t))
            .collect();
        compacted_types.sort();
        for term in &compacted_types {
            if let Some(def) = cur.active.term_definition(term) {
                if let Some(local) = def.context() {
                    let base: &Ctx = owned_t.as_deref().unwrap_or(cur);
                    let key = DerivedKey::Scoped {
                        lookup: cur.id,
                        base: base.id,
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
    Ok(owned_t)
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
        // The item was compacted under its own (property- and type-scoped) context, which
        // may give the index name another meaning; the entry is only taken when it agrees.
        let node = node_ctx(cur, Some(iap), item, Read::IndexMap, env)?;
        let node: &Ctx = node.as_deref().unwrap_or(cur);
        let typed = type_ctx(cur, node, item, env)?;
        // When that context reads the term's index name as another property, a reader
        // could resolve the map key either way, so the value stays put under @none.
        let item_ctx = typed.as_deref().unwrap_or(node);
        let index_iri = cur.active.expand_iri(&index_key, false, true).unwrap_or_default();
        let agrees = item_ctx.active.expand_iri(&index_key, false, true).as_deref()
            == Some(index_iri.as_str());
        // The item's entries are named under the item's context, so the index property's
        // entry is the one that expands to the index IRI there; another entry carrying the
        // same name in the enclosing context is a different property.
        let reads_as_index =
            |k: &str| item_ctx.active.expand_iri(k, false, true).as_deref() == Some(index_iri.as_str());
        // A list value of the index property is one value, whose items cannot key the map.
        let holds_list = match item.get(&index_iri) {
            Some(Json::Arr(vals)) => vals.iter().any(is_list_object),
            Some(v) => is_list_object(v),
            None => false,
        };
        let container_key = match &compacted_item {
            Json::Obj(m) if agrees && !holds_list => m.iter().map(|(k, _)| k.clone()).find(|k| reads_as_index(k)),
            _ => None,
        };
        let taken = container_key.as_deref().and_then(|k| take_entry(&mut compacted_item, k));
        let container_key = container_key.unwrap_or_default();
        if let Some(taken) = taken {
            let mut vals = match taken {
                Json::Arr(a) => a,
                other => vec![other],
            };
            if !vals.is_empty() {
                let first = vals.remove(0);
                // Expansion re-reads the key by Value Expansion under this context, which
                // may differ from the item's (e.g. a type-scoped @type: @id). A key that
                // does not read back as the original value, or that reads as an @none
                // alias and would be dropped, stays on the property instead.
                let original = item.get(&index_iri).and_then(|v| match v {
                    Json::Arr(a) => a.first(),
                    other => Some(other),
                });
                map_key = first.as_str().map(str::to_string).filter(|k| {
                    cur.active.expand_iri(k, false, true).as_deref() != Some("@none")
                        && original.is_some_and(|o| {
                            let key = Json::Str(k.clone());
                            same_entries(&expand_value(&cur.active, &index_key, &key), o)
                        })
                });
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
        // 12.8.9.7: id maps key on the item's @id, which the caller compacted the item
        // without (see `id_keyed`).
        if let Some(Json::Str(id)) = item.get("@id") {
            map_key = Some(id_key(cur, id));
        }
    } else if kind == "@type" {
        // Expansion reads a type map's values under the reverted context, which the
        // caller compacted the item under (`Read::Reverted`).
        let reverted = node_ctx(cur, None, &Json::obj(), Read::Reverted, env)?;
        let map_ctx: &Ctx = reverted.as_deref().unwrap_or(cur);
        // 12.8.9.8: type maps key on the first compacted type; remaining types stay. The
        // spec names the entry by this context's @type alias, but the item was compacted
        // under the term's scoped context, which may alias @type differently and reuse
        // this alias for data, so the alias is taken from that context (including the
        // type-scoped contexts of the item's own types).
        let node = node_ctx(cur, Some(iap), item, Read::Reverted, env)?;
        let node: &Ctx = node.as_deref().unwrap_or(cur);
        let typed = type_ctx(cur, node, item, env)?;
        let item_ctx: &Ctx = typed.as_deref().unwrap_or(node);
        let container_key = item_ctx.ciri("@type", None, true, false);
        if let Some(taken) = take_entry(&mut compacted_item, &container_key) {
            let mut vals = match taken {
                Json::Arr(a) => a,
                other => vec![other],
            };
            if !vals.is_empty() {
                let first = vals.remove(0);
                // Expansion reads the key under this context, and applies a key's scoped
                // context differently (see `type_key_scoped_alike`); a key read as
                // another type, or a scoped context that would read the item
                // differently, leaves the type on the item.
                let ty = item.get("@type").and_then(|t| type_strings(t).first().copied());
                map_key = first.as_str().map(str::to_string).filter(|k| {
                    cur.active.expand_iri(k, false, true).as_deref() == ty
                        && type_key_scoped_alike(cur, map_ctx, node, item_ctx, iap, k, item)
                });
                for v in vals {
                    add_value(&mut compacted_item, &container_key, v, false);
                }
                if map_key.is_none() {
                    add_value(&mut compacted_item, &container_key, first, false);
                }
            }
        }
        // 12.8.9.8.4: a leftover lone node reference re-compacts (it may collapse to a
        // string under an @id/@vocab-typed term). Its key was written under the item's
        // context, and only an item that has an @id is a reference. A key with a scoped
        // context changes how expansion reads the property's values, so there the
        // reference stays a node object.
        let key_scoped = map_key.as_deref().is_some_and(|k| {
            map_ctx.active.term_definition(k).is_some_and(|d| d.context().is_some())
        });
        let id = item.get("@id").filter(|id| !key_scoped && matches!(id, Json::Str(_)));
        let lone_id = match &compacted_item {
            Json::Obj(m) if m.len() == 1 && id.is_some() => {
                item_ctx.active.expand_iri(&m[0].0, false, true).as_deref() == Some("@id")
            }
            _ => false,
        };
        if let (true, Some(id)) = (lone_id, id) {
            let mut single = Json::obj();
            single.set("@id", id.clone());
            compacted_item = compact_element(map_ctx, Some(iap), &single, Read::Reverted, env)?;
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
fn value_compact(cur: &Ctx, active_property: Option<&str>, value: &Json, index_keyed: bool) -> Option<Json> {
    let def = active_property.and_then(|p| cur.active.term_definition(p));
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

    // The @index pass-through condition shared by steps 6-10: an @index survives only
    // as the key of a plain @index map holding the value.
    let index_ok = value.get("@index").is_none() || index_keyed;

    let keys: Vec<&str> = match value {
        Json::Obj(m) => m.iter().map(|(k, _)| k.as_str()).collect(),
        _ => return None,
    };
    let tval = value.get("@type").and_then(Json::as_str);

    // step 6: a node reference (@id plus at most @index) under an @id/@vocab-typed term
    // compacts to the compacted IRI.
    if value.get("@id").is_some() && keys.iter().all(|k| matches!(*k, "@id" | "@index")) && index_ok {
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
    // A bare value would be read back under the property's type mapping, which here the
    // value does not carry (term selection avoids such a term, but a list nested in a
    // list keeps its property): a datatype coerces any value, @id/@vocab a string.
    let coerces = |string: bool| match type_mapping {
        Some("@id" | "@vocab") => string,
        Some(_) => true,
        None => false,
    };
    if let Some(v) = value.get("@value") {
        if !matches!(v, Json::Str(_)) {
            return if index_ok && !coerces(false) { Some(v.clone()) } else { None };
        }
        if coerces(true) {
            return None;
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
/// JSON equality ignoring the order of object members.
fn same_entries(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Obj(x), Json::Obj(y)) => {
            x.len() == y.len()
                && x.iter().all(|(k, v)| y.iter().any(|(k2, v2)| k == k2 && same_entries(v, v2)))
        }
        (Json::Arr(x), Json::Arr(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same_entries(v, w))
        }
        _ => a == b,
    }
}

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

    // Distinct scoped-context sequences across calls must not grow the cached root's
    // derived contexts without bound.
    #[test]
    fn derived_contexts_are_bounded() {
        let terms: Vec<String> = (0..2 * DERIVED_CAP)
            .map(|i| format!(r#""T{i}":{{"@id":"http://ex/T{i}","@context":{{"@language":"en"}}}}"#))
            .collect();
        let ctx = Json::parse(&format!(r#"{{"@vocab":"http://ex/",{}}}"#, terms.join(","))).unwrap();
        let opts = JsonLdOptions::default();
        for i in 0..2 * DERIVED_CAP {
            let doc = Json::parse(&format!(
                r#"[{{"@id":"http://ex/s","@type":["http://ex/T{i}"],"http://ex/p":[{{"@id":"http://ex/o","http://ex/q":[{{"@value":"v"}}]}}]}}]"#
            ))
            .unwrap();
            compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        }
        let n = LAST_ROOT.with(|last| last.borrow().as_ref().map(|(_, _, c)| c.derived.borrow().len()));
        assert!(n.is_some_and(|n| n <= DERIVED_CAP + 4), "derived {n:?}");
    }

    // A property-valued index key is re-read by expansion under the enclosing context;
    // when a type-scoped context reads the index property as @id, the extracted string
    // would come back as a literal, so the value stays on the property.
    #[test]
    fn index_key_must_expand_back_to_the_original_value() {
        let ctx = Json::parse(
            r#"{"@vocab":"http://ex/","key":"http://ex/key",
                "T":{"@id":"http://ex/T","@context":{"key":{"@id":"http://ex/key","@type":"@id"}}},
                "p":{"@id":"http://ex/p","@container":"@index","@index":"key"}}"#,
        )
        .unwrap();
        let doc = Json::parse(
            r#"[{"@id":"http://ex/a","http://ex/p":[{"@id":"http://ex/b","@type":["http://ex/T"],
                "http://ex/key":[{"@id":"http://ex/x"}]}]}]"#,
        )
        .unwrap();
        let opts = JsonLdOptions::default();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        let back = expand(&out, &opts, &NoopLoader).unwrap();
        assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
    }

    // Within one call, new IRIs stop being memoised once the shared budget is spent.
    #[test]
    fn memo_is_bounded_within_a_call() {
        let ctx = Ctx::new(ActiveContext::new(None));
        for i in 0..MEMO_CALL_CAP + 10 {
            ctx.ciri(&format!("http://ex/n{i}"), None, false, false);
        }
        assert_eq!(ctx.memo.borrow().len(), MEMO_CALL_CAP);
        assert_eq!(ctx.memo_used.get(), MEMO_CALL_CAP);
    }

    // One document with more distinct type-scoped contexts than the cache holds still
    // compacts correctly, and the cache is emptied for the next call.
    #[test]
    fn derived_contexts_are_bounded_within_a_call() {
        let n = DERIVED_CAP + 64;
        let terms: Vec<String> = (0..n)
            .map(|i| format!(r#""T{i}":{{"@id":"http://ex/T{i}","@context":{{"q{i}":"http://ex/q"}}}}"#))
            .collect();
        let ctx = Json::parse(&format!(r#"{{"@vocab":"http://ex/",{}}}"#, terms.join(","))).unwrap();
        let nodes: Vec<String> = (0..n)
            .map(|i| format!(r#"{{"@id":"http://ex/s{i}","@type":["http://ex/T{i}"],"http://ex/q":[{{"@value":"v"}}]}}"#))
            .collect();
        let doc = Json::parse(&format!("[{}]", nodes.join(","))).unwrap();
        let opts = JsonLdOptions::default();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        let back = expand(&out, &opts, &NoopLoader).unwrap();
        let sorted = |j: &Json| {
            let mut v = match j {
                Json::Arr(a) => a.clone(),
                _ => vec![],
            };
            v.sort_by_key(|x| x.get("@id").and_then(Json::as_str).map(str::to_string));
            Json::Arr(v)
        };
        assert!(same_entries(&sorted(&back), &sorted(&doc)), "round trip changed the data");
        let len = LAST_ROOT.with(|last| last.borrow().as_ref().map(|(_, _, c)| c.derived.borrow().len()));
        assert_eq!(len, Some(0));
    }

    // A type-scoped context's own @type alias would not be recognised by expansion, which
    // finds @type entries before applying type-scoped contexts.
    #[test]
    fn type_key_is_readable_before_its_scoped_context() {
        let ctx = Json::parse(
            r#"{"@vocab":"http://ex/","type":"@type",
                "T":{"@id":"http://ex/T","@context":{"t":"@type","label":"http://ex/q"}}}"#,
        )
        .unwrap();
        let doc = Json::parse(
            r#"[{"@id":"http://ex/s","@type":["http://ex/T"],"http://ex/q":[{"@value":"v"}]}]"#,
        )
        .unwrap();
        let opts = JsonLdOptions::default();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        let back = expand(&out, &opts, &NoopLoader).unwrap();
        assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
    }

    // The index property's entry is looked up under the item's own context, so a term the
    // item's scoped context gives another meaning is left alone.
    #[test]
    fn index_entry_is_named_under_the_item_context() {
        let ctx = Json::parse(
            r#"{"label":"http://ex/label",
                "p":{"@id":"http://ex/p","@container":"@index","@index":"http://ex/label",
                     "@context":{"label":"http://ex/data","key":"http://ex/label"}}}"#,
        )
        .unwrap();
        let doc = Json::parse(
            r#"[{"@id":"http://ex/a","http://ex/p":[{"@id":"http://ex/b",
                "http://ex/label":[{"@value":"K"}],"http://ex/data":[{"@value":"K"}]}]}]"#,
        )
        .unwrap();
        let opts = JsonLdOptions::default();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        let back = expand(&out, &opts, &NoopLoader).unwrap();
        assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
    }

    // Expansion's own scoped-context processing is charged to the same per-call budget.
    #[test]
    fn expansion_is_budgeted() {
        let defs: Vec<String> = (0..1024).map(|i| format!(r#""u{i}":"http://ex/u{i}""#)).collect();
        let nodes: Vec<String> = (0..budget::WORK_BUDGET / 1024 + 2)
            .map(|i| format!(r#"{{"@id":"http://ex/s{i}","p":{{"@id":"http://ex/o"}}}}"#))
            .collect();
        let doc = Json::parse(&format!(
            r#"{{"@context":{{"p":{{"@id":"http://ex/p","@context":[{{{}}},null]}}}},"@graph":[{}]}}"#,
            defs.join(","),
            nodes.join(",")
        ))
        .unwrap();
        let err = expand(&doc, &JsonLdOptions::default(), &NoopLoader).unwrap_err();
        assert_eq!(err.code(), E::ContextOverflow);
    }

    // A type-map item's leftover entry is read under the item's own context: a scoped
    // alias that reuses the outer @id alias for data is kept as data.
    #[test]
    fn type_map_leftover_is_read_under_the_item_context() {
        let ctx = Json::parse(
            r#"{"id":"@id","type":"@type","p":{"@id":"http://ex/p","@container":"@type",
                "@context":{"id":"http://ex/data","i":"@id","type":"http://ex/unused","kind":"@type"}}}"#,
        )
        .unwrap();
        let doc = Json::parse(
            r#"[{"@id":"http://ex/a","http://ex/p":[{"@type":["http://ex/T"],"http://ex/data":[{"@value":"kept"}]}]}]"#,
        )
        .unwrap();
        let opts = JsonLdOptions::default();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        let back = expand(&out, &opts, &NoopLoader).unwrap();
        assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
    }

    // Processing a context directly is budgeted too.
    #[test]
    fn direct_context_processing_is_budgeted() {
        let defs: Vec<String> = (0..1024).map(|i| format!(r#""u{i}":"http://ex/u{i}""#)).collect();
        let one = format!("{{{}}},null", defs.join(","));
        let parts = vec![one; budget::WORK_BUDGET / 1024 + 2];
        let local = Json::parse(&format!("[{}]", parts.join(","))).unwrap();
        let err = ActiveContext::new(None)
            .process(&local, None, &NoopLoader, &JsonLdOptions::default())
            .unwrap_err();
        assert_eq!(err.code(), E::ContextOverflow);
    }

    // A long chain of terms defined through one another fails cleanly instead of
    // recursing once per link until the stack runs out; a short chain still resolves.
    #[test]
    fn term_dependency_chains_are_depth_bounded() {
        let chain = |n: usize| {
            let mut defs: Vec<String> = (0..n).map(|i| format!(r#""t{i}":"t{}""#, i + 1)).collect();
            defs.push(format!(r#""t{n}":"http://ex/p""#));
            Json::parse(&format!("{{{}}}", defs.join(","))).unwrap()
        };
        let process = |local: &Json| {
            ActiveContext::new(None).process(local, None, &NoopLoader, &JsonLdOptions::default())
        };
        let short = process(&chain(100)).unwrap();
        assert_eq!(short.expand_iri("t0", false, true).as_deref(), Some("http://ex/p"));
        let err = process(&chain(4096)).unwrap_err();
        assert_eq!(err.code(), E::ContextOverflow);
    }

    // Retention counts the term table a null-reset context keeps for reversion.
    #[test]
    fn retention_counts_reversion_targets() {
        let defs: Vec<String> = (0..4096).map(|i| format!(r#""u{i}":"http://ex/u{i}""#)).collect();
        let ctx = Json::parse(&format!(
            r#"{{"@vocab":"http://ex/","B":{{"@id":"http://ex/B","@context":[{{{}}},null]}}}}"#,
            defs.join(",")
        ))
        .unwrap();
        let doc = Json::parse(r#"[{"@id":"http://ex/s","@type":["http://ex/B"]}]"#).unwrap();
        compact_expanded(&doc, &ctx, &JsonLdOptions::default(), &NoopLoader).unwrap();
        let retained = LAST_ROOT.with(|last| last.borrow().as_ref().map(|(_, _, c)| c.retained.get()));
        assert!(retained.is_some_and(|n| n > 4096), "retained {retained:?}");
    }

    // A property-scoped context that redefines the outer @type alias hides it from
    // expansion, so the embedded node's types use @type itself.
    #[test]
    fn type_key_is_readable_under_a_property_scoped_context() {
        let ctx = Json::parse(
            r#"{"@vocab":"http://ex/","type":"@type",
                "T":{"@id":"http://ex/T","@context":{"type":"@type","label":"http://ex/q"}},
                "p":{"@id":"http://ex/p","@context":{"type":"http://ex/data","t":"@type"}}}"#,
        )
        .unwrap();
        let doc = Json::parse(
            r#"[{"@id":"http://ex/a","http://ex/p":[{"@id":"http://ex/b","@type":["http://ex/T"],
                "http://ex/q":[{"@value":"v"}]}]}]"#,
        )
        .unwrap();
        let opts = JsonLdOptions::default();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        let back = expand(&out, &opts, &NoopLoader).unwrap();
        assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
    }

    // Expansion reads an embedded node's types, and picks their scoped contexts, under
    // the property-scoped context, so compaction spells and selects them there too.
    #[test]
    fn types_are_spelled_under_the_property_scoped_context() {
        let opts = JsonLdOptions::default();
        let round_trip = |ctx: &str, doc: &str| {
            let ctx = Json::parse(ctx).unwrap();
            let doc = Json::parse(doc).unwrap();
            let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
            let back = expand(&out, &opts, &NoopLoader).unwrap();
            assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
        };
        // The scoped @vocab would read the outer spelling "T" as http://other/T.
        round_trip(
            r#"{"@vocab":"http://ex/","p":{"@id":"http://ex/p","@context":{"@vocab":"http://other/"}}}"#,
            r#"[{"@id":"http://ex/a","http://ex/p":[{"@id":"http://ex/b","@type":["http://ex/T"]}]}]"#,
        );
        // The scoped redefinition of "T" carries no context, so expansion never sees "label".
        round_trip(
            r#"{"@vocab":"http://ex/",
                "T":{"@id":"http://ex/T","@context":{"label":"http://ex/q"}},
                "p":{"@id":"http://ex/p","@context":{"T":{"@id":"http://ex/T"}}}}"#,
            r#"[{"@id":"http://ex/a","http://ex/p":[{"@id":"http://ex/b","@type":["http://ex/T"],
                "http://ex/q":[{"@value":"v"}]}]}]"#,
        );
    }

    // List and graph wrappers are read under the property's scoped context, so their
    // keyword aliases come from there, not from the enclosing context.
    #[test]
    fn wrappers_are_spelled_under_the_property_scoped_context() {
        let opts = JsonLdOptions::default();
        let round_trip = |ctx: &str, doc: &str| {
            let ctx = Json::parse(ctx).unwrap();
            let doc = Json::parse(doc).unwrap();
            let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
            let back = expand(&out, &opts, &NoopLoader).unwrap();
            assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
        };
        round_trip(
            r#"{"l":"@list","p":{"@id":"http://ex/p","@context":{"l":"http://ex/data"}}}"#,
            r#"[{"@id":"http://ex/s","http://ex/p":[{"@list":[{"@value":"a"}]}]}]"#,
        );
        round_trip(
            r#"{"g":"@graph","i":"@id",
                "p":{"@id":"http://ex/p","@context":{"g":"http://ex/data","i":"http://ex/data2"}}}"#,
            r#"[{"@id":"http://ex/s","http://ex/p":[{"@id":"http://ex/g1",
                "@graph":[{"@id":"http://ex/x","http://ex/q":[{"@value":"v"}]}]}]}]"#,
        );
    }

    // A type-map key is read under the enclosing context; one that reads as another
    // type there leaves the type on the item.
    #[test]
    fn type_map_keys_read_back_under_the_enclosing_context() {
        let ctx = Json::parse(
            r#"{"@vocab":"http://ex/","T":"http://ex/T",
                "p":{"@id":"http://ex/p","@container":"@type",
                     "@context":{"T":"http://other/T","U":"http://ex/T"}}}"#,
        )
        .unwrap();
        let doc =
            Json::parse(r#"[{"@id":"http://ex/a","http://ex/p":[{"@id":"http://ex/b","@type":["http://ex/T"]}]}]"#)
                .unwrap();
        let opts = JsonLdOptions::default();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        let back = expand(&out, &opts, &NoopLoader).unwrap();
        assert!(same_entries(&back, &doc), "round trip changed the data: {back:?}");
    }

    // Scoped contexts nested as deeply as a parsed context allows are processed (or
    // refused) without overflowing the stack.
    #[test]
    fn deeply_scoped_contexts_stay_on_the_stack() {
        let mut ctx = r#"{"q":"http://ex/q"}"#.to_string();
        let mut levels = 0;
        loop {
            let next = format!(r#"{{"t":{{"@id":"http://ex/t","@context":{ctx}}}}}"#);
            if Json::parse(&next).is_err() {
                break;
            }
            ctx = next;
            levels += 1;
        }
        assert!(levels > 100, "{levels}");
        let local = Json::parse(&ctx).unwrap();
        let _ = ActiveContext::new(None).process(&local, None, &NoopLoader, &JsonLdOptions::default());
    }

    // Recursive walks fail cleanly past their nesting bound instead of overflowing.
    #[test]
    fn deep_documents_fail_cleanly() {
        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(Json::parse(&deep(crate::json::MAX_DEPTH)).is_ok());
        assert!(Json::parse(&deep(crate::json::MAX_DEPTH + 1)).is_err());
        assert!(Json::parse(&deep(1_000_000)).is_err());
        // A value built in code is not bounded by the parser; the walks bound themselves.
        let mut doc = Json::obj();
        for _ in 0..4 * budget::MAX_NESTING {
            let mut node = Json::obj();
            node.set("http://ex/p", Json::Arr(vec![doc]));
            doc = node;
        }
        let opts = JsonLdOptions::default();
        let ctx = Json::parse(r#"{"@vocab":"http://ex/"}"#).unwrap();
        let err = expand(&doc, &opts, &NoopLoader).unwrap_err();
        assert_eq!(err.code(), E::ContextOverflow);
        let err = compact_expanded(&Json::Arr(vec![doc.clone()]), &ctx, &opts, &NoopLoader).unwrap_err();
        assert_eq!(err.code(), E::ContextOverflow);
        // Dismantle iteratively: dropping a value this deep recurses too.
        while let Json::Obj(mut m) = doc {
            doc = match m.pop() {
                Some((_, Json::Arr(mut a))) => a.pop().unwrap_or(Json::Raw("null".into())),
                _ => Json::Raw("null".into()),
            };
        }
    }

    // An @id map's entries are read under the reverted map context, which looks the
    // property's scoped context up there: a property only a type-scoped context defines
    // brings none, so its entries are compacted without it.
    #[test]
    fn id_map_entries_use_the_map_context() {
        let opts = JsonLdOptions::default();
        let ctx = Json::parse(
            r#"{"@vocab":"http://outer/","T":{"@id":"http://ex/T","@context":{
                "p":{"@id":"http://ex/p","@container":"@id","@context":{"q":"http://ex/q"}}}}}"#,
        )
        .unwrap();
        let doc = Json::parse(
            r#"[{"@id":"http://ex/a","@type":["http://ex/T"],
                "http://ex/p":[{"http://ex/q":[{"@value":"v"}],"@id":"http://ex/b"}]}]"#,
        )
        .unwrap();
        let out = compact_expanded(&doc, &ctx, &opts, &NoopLoader).unwrap();
        assert_eq!(expand(&out, &opts, &NoopLoader).unwrap(), doc, "{out:?}");
    }

    // Expansion decides whether a list object reverts a type-scoped context from its own
    // keys, so a list alias that the type-scoped context reads as @value cannot be used.
    #[test]
    fn wrapper_aliases_keep_the_reversion() {
        let opts = JsonLdOptions::default();
        let ctx = Json::parse(
            r#"{"l":"@list","p":"http://ex/p","T":{"@id":"http://ex/T","@context":{"l":"@value"}}}"#,
        )
        .unwrap();
        let doc = Json::parse(
            r#"[{"@id":"http://ex/s","@type":["http://ex/T"],"http://ex/p":[{"@list":[{"@value":"a"}]}]}]"#,
        )
        .unwrap();
        match compact_expanded(&doc, &ctx, &opts, &NoopLoader) {
            Ok(out) => assert_eq!(expand(&out, &opts, &NoopLoader).unwrap(), doc),
            Err(e) => assert_eq!(e.code(), E::InvalidScopedContext),
        }
    }

    // Definitions a scoped context discards with a later null are charged too: once the
    // cache is full, re-applying `[{...}, null]` (an empty result) exhausts the budget.
    #[test]
    fn context_resets_are_charged() {
        let defs: Vec<String> = (0..1024).map(|i| format!(r#""u{i}":"http://ex/u{i}""#)).collect();
        let mut terms: Vec<String> = (0..DERIVED_CAP)
            .map(|i| format!(r#""T{i}":{{"@id":"http://ex/T{i}","@context":{{"q":"http://ex/q"}}}}"#))
            .collect();
        terms.push(format!(r#""B":{{"@id":"http://ex/B","@context":[{{{}}},null]}}"#, defs.join(",")));
        let ctx = Json::parse(&format!(r#"{{"@vocab":"http://ex/",{}}}"#, terms.join(","))).unwrap();
        let nodes: Vec<String> = (0..DERIVED_CAP)
            .map(|i| format!("T{i}"))
            .chain(std::iter::repeat_n("B".to_string(), 2 * budget::WORK_BUDGET / 1024))
            .enumerate()
            .map(|(n, t)| format!(r#"{{"@id":"http://ex/s{n}","@type":["http://ex/{t}"]}}"#))
            .collect();
        let doc = Json::parse(&format!("[{}]", nodes.join(","))).unwrap();
        let err = compact_expanded(&doc, &ctx, &JsonLdOptions::default(), &NoopLoader).unwrap_err();
        assert_eq!(err.code(), E::ContextOverflow);
    }

    // Rebuilding derived contexts past the cache cap shares one work budget per call.
    #[test]
    fn post_cap_rebuilds_are_budgeted() {
        let plain = 64;
        let mut terms: Vec<String> = (0..plain).map(|i| format!(r#""p{i}":"http://ex/p{i}""#)).collect();
        terms.extend(
            (0..=DERIVED_CAP)
                .map(|i| format!(r#""T{i}":{{"@id":"http://ex/T{i}","@context":{{"q":"http://ex/q"}}}}"#)),
        );
        let ctx = Json::parse(&format!(r#"{{"@vocab":"http://ex/",{}}}"#, terms.join(","))).unwrap();
        // Fill the cache, then use the last type (never cached) until the budget runs out.
        let nodes: Vec<String> = (0..DERIVED_CAP)
            .chain(std::iter::repeat_n(DERIVED_CAP, 2 * budget::WORK_BUDGET / plain))
            .enumerate()
            .map(|(n, t)| format!(r#"{{"@id":"http://ex/s{n}","@type":["http://ex/T{t}"]}}"#))
            .collect();
        let doc = Json::parse(&format!("[{}]", nodes.join(","))).unwrap();
        let err = compact_expanded(&doc, &ctx, &JsonLdOptions::default(), &NoopLoader).unwrap_err();
        assert_eq!(err.code(), E::ContextOverflow);
        // The budget is per call: the next call starts afresh.
        let small = Json::parse(r#"[{"@id":"http://ex/s","@type":["http://ex/T0"]}]"#).unwrap();
        compact_expanded(&small, &ctx, &JsonLdOptions::default(), &NoopLoader).unwrap();
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
