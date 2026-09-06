//! Expansion Algorithm + Value Expansion (JSON-LD 1.1 API §5.1, §5.3).
//!
//! [OPUS-4.8] (sq-oy1f.25) Expansion is the pipeline's central hinge: it turns a compact
//! JSON-LD document into the canonical **expanded** form every other output projects from
//! (compaction, flattening, framing, and `toRdf`). This module implements the document-level
//! **Expansion Algorithm** (§5.1.2) and **Value Expansion** (§5.3.2) over the `Json` AST,
//! consuming the Context Processing foundation (`context`) from bead `sq-oy1f.24`.
//!
//! The public entry is [`expand`]: it initialises an active context from the processing
//! options (`base`, `expandContext`), runs the recursive algorithm, and applies the API's
//! top-level post-processing (the `@graph`-only reduction, `null` → `[]`, array wrap) so the
//! result is always a JSON array of node objects (JSON-LD 1.1 API §5.1).
//!
//! Structures an RDF-first writer cannot recover — scoped (property/type) contexts, `@nest`,
//! `@index`/`@id`/`@type`/`@graph`/`@language` container maps, keyword aliases, `@reverse`,
//! `@included`, and `@json` literals — are all handled here (design record §1.1). Invalid
//! inputs raise the exact spec `JsonLdErrorCode`.
//!
//! `frameExpansion` mode is threaded through (the flag relaxes the drop/validation rules so a
//! frame document keeps its `@default`/empty patterns); the framing pipeline that consumes it
//! lands with bead `sq-oy1f.29`.
//!
//! Spec: <https://www.w3.org/TR/json-ld11-api/#expansion-algorithm>.

use crate::context::iri::{is_absolute_iri, is_blank_node};
use crate::context::{is_keyword, ActiveContext, Direction, Override};
use crate::error::{JsonLdError, JsonLdErrorCode as E};
use crate::json::Json;
use crate::loader::DocumentLoader;
use crate::options::JsonLdOptions;

/// Constant per-call environment threaded through the recursion (keeps argument counts sane).
struct Ctx<'a> {
    loader: &'a dyn DocumentLoader,
    options: &'a JsonLdOptions,
    /// The `ordered` option — sort map keys for deterministic output.
    ordered: bool,
}

/// **Expansion** (JSON-LD 1.1 API §5.1). Expands `input` (an in-memory JSON-LD document)
/// against the processing `options` and returns the expanded document — always a JSON array
/// of node objects.
///
/// The active context is initialised from `options.base`; if `options.expand_context` is set
/// it is folded in first (JSON-LD 1.1 API §5.1 step 6). Remote `@context` / `@import`
/// references are dereferenced **only** through `loader`, which is deny-by-default
/// (`NoopLoader` refuses every fetch). Returns the first spec `JsonLdError` on invalid input.
///
/// [OPUS-4.8] (sq-oy1f.25)
pub fn expand(
    input: &Json,
    options: &JsonLdOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    // §5.1: initialise the active context from options.base.
    let mut active_context = ActiveContext::new(options.base.as_deref());

    // §5.1 step 6: if expandContext is set, fold it in first. A map carrying an @context
    // entry contributes that entry's value.
    if let Some(ctx) = &options.expand_context {
        let local = ctx.get("@context").unwrap_or(ctx);
        active_context = active_context.process(local, options.base.as_deref(), loader, options)?;
    }

    let ctx = Ctx {
        loader,
        options,
        ordered: options.ordered,
    };
    let expanded = expand_element(
        &ctx,
        &active_context,
        None,
        input,
        options.base.as_deref(),
        options.frame_expansion,
        false,
    )?;

    // §5.1 top-level post-processing.
    let mut result = expanded.unwrap_or_else(|| Json::Arr(Vec::new()));
    if let Json::Obj(members) = &result {
        if members.len() == 1 && members[0].0 == "@graph" {
            result = members[0].1.clone();
        }
    }
    if !matches!(result, Json::Arr(_)) {
        result = Json::Arr(vec![result]);
    }
    Ok(result)
}

/// The recursive **Expansion Algorithm** (JSON-LD 1.1 API §5.1.2). Returns `None` for a JSON
/// `null` (a dropped value), otherwise the expanded `Json`.
#[allow(clippy::too_many_arguments)]
fn expand_element(
    ctx: &Ctx<'_>,
    active_context: &ActiveContext,
    active_property: Option<&str>,
    element: &Json,
    base_url: Option<&str>,
    frame_expansion: bool,
    from_map: bool,
) -> Result<Option<Json>, JsonLdError> {
    // step 1: null expands to null.
    if is_null(element) {
        return Ok(None);
    }
    // step 2: @default active property forces non-frame semantics for this element.
    let frame_expansion = active_property != Some("@default") && frame_expansion;
    // step 3: a property-scoped context (the local @context on active property's definition).
    let prop_scoped: Option<(Json, Option<String>)> = active_property
        .and_then(|ap| active_context.term_definition(ap))
        .and_then(|td| td.context().cloned().map(|c| (c, td.base_url.clone())));

    match element {
        // step 5: an array expands element-wise.
        Json::Arr(items) => {
            let container = container_of(active_context, active_property);
            let mut result = Vec::new();
            for item in items {
                let expanded = expand_element(
                    ctx,
                    active_context,
                    active_property,
                    item,
                    base_url,
                    frame_expansion,
                    from_map,
                )?;
                // step 5.2.2: a @list container over a nested array becomes a list object.
                let expanded = match expanded {
                    Some(Json::Arr(inner)) if container.iter().any(|c| c == "@list") => {
                        Some(Json::Obj(vec![("@list".to_string(), Json::Arr(inner))]))
                    }
                    other => other,
                };
                // step 5.2.3: flatten arrays, drop nulls.
                match expanded {
                    Some(Json::Arr(inner)) => result.extend(inner),
                    Some(v) => result.push(v),
                    None => {}
                }
            }
            Ok(Some(Json::Arr(result)))
        }
        // step 6: a map is the substantive case.
        Json::Obj(_) => expand_map(
            ctx,
            active_context,
            active_property,
            element,
            base_url,
            frame_expansion,
            from_map,
            prop_scoped,
        ),
        // step 4: a scalar.
        _ => {
            // step 4.1: a scalar with no property (or under @graph) is dropped.
            if active_property.is_none() || active_property == Some("@graph") {
                return Ok(None);
            }
            // step 4.2: apply a property-scoped context, if any.
            let processed;
            let ac = if let Some((sctx, sbase)) = &prop_scoped {
                processed = active_context.process_scoped(
                    sctx,
                    sbase.as_deref(),
                    false,
                    true,
                    ctx.loader,
                    ctx.options,
                )?;
                &processed
            } else {
                active_context
            };
            // step 4.3: value expansion.
            Ok(Some(expand_value(ac, active_property.unwrap(), element)))
        }
    }
}

/// Expansion of a map element (JSON-LD 1.1 API §5.1.2 steps 6–20).
#[allow(clippy::too_many_arguments)]
fn expand_map(
    ctx: &Ctx<'_>,
    active_context: &ActiveContext,
    active_property: Option<&str>,
    element: &Json,
    base_url: Option<&str>,
    frame_expansion: bool,
    from_map: bool,
    prop_scoped: Option<(Json, Option<String>)>,
) -> Result<Option<Json>, JsonLdError> {
    let mut active_context = active_context.clone();

    // step 7: revert a non-propagated (type-scoped) context when descending into a new node
    // object that is neither a value object nor a bare node reference.
    if active_context.previous_context.is_some() && !from_map {
        let has_value = has_key_expanding_to(&active_context, element, "@value");
        let single_id = is_single_entry_expanding_to(&active_context, element, "@id");
        if !has_value && !single_id {
            active_context = (*active_context.previous_context.clone().unwrap()).clone();
        }
    }

    // step 8: apply a property-scoped context (override protected — it was already validated
    // at definition time, so it may legally redefine a protected term).
    if let Some((sctx, sbase)) = &prop_scoped {
        active_context =
            active_context.process_scoped(sctx, sbase.as_deref(), true, true, ctx.loader, ctx.options)?;
    }

    // step 9: a local @context on the element.
    if let Some(local) = element.get("@context") {
        active_context = active_context.process(local, base_url, ctx.loader, ctx.options)?;
    }

    // step 10: snapshot the active context for expanding @type values (type-scoped).
    let type_scoped_context = active_context.clone();

    // step 11: apply type-scoped contexts (propagate: false).
    for key in sorted_keys(element, true) {
        if active_context.expand_iri(&key, false, true).as_deref() != Some("@type") {
            continue;
        }
        let mut terms: Vec<String> = string_values(element.get(&key).unwrap());
        terms.sort();
        for term in terms {
            let scoped = type_scoped_context
                .term_definition(&term)
                .and_then(|td| td.context().cloned().map(|c| (c, td.base_url.clone())));
            if let Some((sctx, sbase)) = scoped {
                active_context = active_context.process_scoped(
                    &sctx,
                    sbase.as_deref(),
                    false,
                    false,
                    ctx.loader,
                    ctx.options,
                )?;
            }
        }
    }

    // step 12: input type — the @json detection for a sibling @value entry.
    let input_is_json = first_type_input(&active_context, element).as_deref() == Some("@json");

    let mut result: Vec<(String, Json)> = Vec::new();
    let mut nests: Vec<String> = Vec::new();

    // steps 13 + 14: process every entry (recursing through @nest keys).
    expand_object(
        ctx,
        &active_context,
        &type_scoped_context,
        active_property,
        element,
        base_url,
        frame_expansion,
        input_is_json,
        &mut result,
        &mut nests,
    )?;

    // steps 15–20: value/list/set/node cleanup.
    cleanup(result, active_property, frame_expansion)
}

/// Processes each entry of a map into `result` (JSON-LD 1.1 API §5.1.2 step 13), deferring
/// `@nest` keys and recursing into them (step 14).
#[allow(clippy::too_many_arguments)]
fn expand_object(
    ctx: &Ctx<'_>,
    active_context: &ActiveContext,
    type_scoped_context: &ActiveContext,
    active_property: Option<&str>,
    element: &Json,
    base_url: Option<&str>,
    frame_expansion: bool,
    input_is_json: bool,
    result: &mut Vec<(String, Json)>,
    nests: &mut Vec<String>,
) -> Result<(), JsonLdError> {
    for key in sorted_keys(element, ctx.ordered) {
        // step 13.1: @context was already consumed.
        if key == "@context" {
            continue;
        }
        let value = element.get(&key).unwrap();
        // step 13.2: expand the key.
        let expanded_property = match active_context.expand_iri(&key, false, true) {
            Some(p) => p,
            None => continue,
        };
        // step 13.3: drop a key that is neither an IRI (contains ':') nor a keyword.
        if !expanded_property.contains(':') && !is_keyword(&expanded_property) {
            continue;
        }

        if is_keyword(&expanded_property) {
            // step 13.4: @nest defers to step 14 (needs the ORIGINAL key).
            if expanded_property == "@nest" {
                if !nests.contains(&key) {
                    nests.push(key.clone());
                }
                continue;
            }
            // step 13.4.1: no keyword may appear as a reverse property map entry.
            if active_property == Some("@reverse") {
                return Err(JsonLdError::new(E::InvalidReversePropertyMap));
            }
            // step 13.4.2: colliding keywords (except the mergeable @included / @type).
            if expanded_property != "@included"
                && expanded_property != "@type"
                && obj_get(result, &expanded_property).is_some()
            {
                return Err(JsonLdError::new(E::CollidingKeywords));
            }
            expand_keyword(
                ctx,
                active_context,
                type_scoped_context,
                active_property,
                &expanded_property,
                value,
                base_url,
                frame_expansion,
                input_is_json,
                result,
            )?;
            continue;
        }

        // step 13.5+: `expanded_property` is a real property IRI.
        let container = container_of(active_context, Some(&key));
        let expanded_value = expand_property_value(
            ctx,
            active_context,
            &key,
            value,
            base_url,
            frame_expansion,
            &container,
        )?;

        // step 13.9: drop a null expansion.
        let Some(mut expanded_value) = expanded_value else {
            continue;
        };

        // step 13.10: a @list container wraps a non-list into a list object.
        if container.iter().any(|c| c == "@list") && !is_list_object(&expanded_value) {
            expanded_value = Json::Obj(vec![("@list".to_string(), array_of(expanded_value))]);
        }
        // step 13.11: a bare @graph container wraps each value into a graph object.
        if container.iter().any(|c| c == "@graph")
            && !container.iter().any(|c| c == "@id")
            && !container.iter().any(|c| c == "@index")
        {
            let wrapped: Vec<Json> = as_array(expanded_value)
                .into_iter()
                .map(|v| Json::Obj(vec![("@graph".to_string(), array_of(v))]))
                .collect();
            expanded_value = Json::Arr(wrapped);
        }

        // step 13.12/13.13: reverse property vs. forward property.
        let is_reverse = active_context
            .term_definition(&key)
            .is_some_and(|td| td.is_reverse());
        if is_reverse {
            for item in as_array(expanded_value) {
                if is_value_object(&item) || is_list_object(&item) {
                    return Err(JsonLdError::new(E::InvalidReversePropertyValue));
                }
                add_value(reverse_map(result), &expanded_property, item);
            }
        } else {
            // step 13.14 (JSON-LD 1.1 API): the forward-property add ALWAYS uses
            // `asArray = true` (the reference `_addValue(..., propertyIsArray
            // true)`), so an EMPTY expanded value still materialises the property
            // as an empty array rather than being dropped. Only a `null` (`None`)
            // expansion is dropped, and that already `continue`d above. This is
            // what retains empty `@set`/`@list`/plain-array properties normatively
            // (W3C expand/0004, 0014, 0015, 0016).
            add_value_as_array(result, &expanded_property, expanded_value);
        }
    }

    // step 14: process deferred @nest keys into the same result.
    for nest_key in nests.clone() {
        // [SONNET-4.6] sq-oy1f.45 — §5.1.2 step 14: apply any property-scoped context
        // declared on the @nest-aliased term (tc037/tc038).  The context is applied with
        // override_protected = true and propagate = true so that term definitions from the
        // nest's scoped context remain visible when descending into child node objects
        // (e.g. the `agent → @nest` alias in tc038 must be visible inside `aut`'s value).
        let nest_ctx_and_base: Option<(Json, Option<String>)> = active_context
            .term_definition(&nest_key)
            .and_then(|td| td.context().cloned().map(|c| (c, td.base_url.clone())));
        let nest_processed;
        let nest_active: &ActiveContext = if let Some((sctx, sbase)) = &nest_ctx_and_base {
            nest_processed = active_context.process_scoped(
                sctx,
                sbase.as_deref(),
                true,  // override_protected: may redefine protected terms
                true,  // propagate: keep context in child nodes (not a type-scoped ctx)
                ctx.loader,
                ctx.options,
            )?;
            &nest_processed
        } else {
            active_context
        };
        for nv in as_array(element.get(&nest_key).cloned().unwrap_or_else(json_null)) {
            if !matches!(nv, Json::Obj(_)) || has_key_expanding_to(active_context, &nv, "@value") {
                return Err(JsonLdError::new(E::InvalidNestValue));
            }
            let mut inner_nests = Vec::new();
            expand_object(
                ctx,
                nest_active,
                type_scoped_context,
                active_property,
                &nv,
                base_url,
                frame_expansion,
                input_is_json,
                result,
                &mut inner_nests,
            )?;
        }
    }
    Ok(())
}

/// Handles a single keyword entry (JSON-LD 1.1 API §5.1.2 step 13.4).
#[allow(clippy::too_many_arguments)]
fn expand_keyword(
    ctx: &Ctx<'_>,
    active_context: &ActiveContext,
    type_scoped_context: &ActiveContext,
    active_property: Option<&str>,
    expanded_property: &str,
    value: &Json,
    base_url: Option<&str>,
    frame_expansion: bool,
    input_is_json: bool,
    result: &mut Vec<(String, Json)>,
) -> Result<(), JsonLdError> {
    match expanded_property {
        "@id" => match value {
            Json::Str(s) => {
                // [SONNET-4.6] sq-oy1f.45 — §5.1.2 step 13.4.1: "set result[@id] to the
                // result of IRI expanding value" — the spec always stores the result,
                // including null (a keyword-form string that is not a recognised keyword
                // expands to null per §5.2 step 2).  W3C expand/0122 expects
                // `{"@id": null}` for `@ignoreMe`, not an empty node object `{}`.
                match active_context.expand_iri(s, true, false) {
                    Some(iri) => set_obj(result, "@id", Json::Str(iri)),
                    None => set_obj(result, "@id", json_null()),
                }
            }
            Json::Arr(items) if frame_expansion => {
                let mut ids = Vec::new();
                for it in items {
                    let Json::Str(s) = it else {
                        return Err(JsonLdError::new(E::InvalidIdValue));
                    };
                    if let Some(iri) = active_context.expand_iri(s, true, false) {
                        ids.push(Json::Str(iri));
                    }
                }
                set_obj(result, "@id", Json::Arr(ids));
            }
            Json::Obj(m) if frame_expansion && m.is_empty() => {
                set_obj(result, "@id", Json::Arr(vec![Json::obj()]));
            }
            _ => return Err(JsonLdError::new(E::InvalidIdValue)),
        },
        "@type" => {
            // [FABLE-5] sq-oy1f.29 — frameExpansion keeps the framing @type PATTERN
            // shapes: the wildcard `{}`, the match-none `[]`, and the default map
            // `{"@default": …}` (json-ld11-framing §2.1/§2.3). Dropping them turned a
            // constrained frame into the match-everything frame.
            match value {
                Json::Obj(m) if frame_expansion && m.is_empty() => {
                    set_obj(result, "@type", Json::Arr(vec![Json::obj()]));
                }
                Json::Obj(m)
                    if frame_expansion && m.len() == 1 && m[0].0 == "@default" =>
                {
                    let defaults: Vec<Json> = as_array(m[0].1.clone())
                        .iter()
                        .filter_map(|t| {
                            t.as_str()
                                .and_then(|s| type_scoped_context.expand_iri(s, true, true))
                                .map(Json::Str)
                        })
                        .collect();
                    set_obj(
                        result,
                        "@type",
                        Json::Arr(vec![Json::Obj(vec![(
                            "@default".to_string(),
                            Json::Arr(defaults),
                        )])]),
                    );
                }
                Json::Arr(items) if frame_expansion && items.is_empty() => {
                    set_obj(result, "@type", Json::Arr(Vec::new()));
                }
                Json::Arr(items)
                    if frame_expansion
                        && items.len() == 1
                        && matches!(&items[0], Json::Obj(m) if m.is_empty()) =>
                {
                    set_obj(result, "@type", Json::Arr(vec![Json::obj()]));
                }
                _ => {
                    // step 13.4.4: @type expands against the type-scoped context
                    // (vocab + relative).
                    let mut expanded_types = Vec::new();
                    for t in type_values(value, frame_expansion)? {
                        if let Some(iri) = type_scoped_context.expand_iri(&t, true, true) {
                            expanded_types.push(Json::Str(iri));
                        }
                    }
                    for t in expanded_types {
                        add_value(result, "@type", t);
                    }
                }
            }
        }
        "@graph" => {
            let expanded = expand_element(
                ctx,
                active_context,
                Some("@graph"),
                value,
                base_url,
                frame_expansion,
                false,
            )?;
            let arr = array_of(expanded.unwrap_or_else(|| Json::Arr(Vec::new())));
            set_obj(result, "@graph", arr);
        }
        "@included" => {
            let expanded = expand_element(
                ctx,
                active_context,
                None,
                value,
                base_url,
                frame_expansion,
                false,
            )?;
            // §5.1.2 step 13.4.13: "ensuring that the result is an array", then EVERY
            // element must be a node object. A DROPPED expansion (`None` — a free-floating
            // scalar, value object, or list object under the null active property) is not
            // an empty array: arrayifying null yields a one-element array whose element is
            // not a node object, so it is an `invalid @included value`. Collapsing it to
            // `[]` instead would vacuously accept `@included: "string"` / `{"@value": …}` /
            // `{"@list": […]}` (W3C expand/in07, in08, in09), which the suite requires to
            // raise. Matches jsonld.js (`_asArray(null)` → `[null]`, `_isSubject(null)`
            // false) and pyld.
            let items = match expanded {
                Some(j) => as_array(j),
                None => vec![json_null()],
            };
            for item in &items {
                if !is_node_object(item) {
                    return Err(JsonLdError::new(E::InvalidIncludedValue));
                }
            }
            for item in items {
                add_value(result, "@included", item);
            }
        }
        "@value" => {
            // step 13.4.7: a @json-typed node keeps its value verbatim.
            if input_is_json {
                set_obj(result, "@value", value.clone());
            } else if is_null(value) {
                set_obj(result, "@value", json_null());
            } else if is_scalar(value)
                || (frame_expansion && matches!(value, Json::Arr(_) | Json::Obj(_)))
            {
                set_obj(result, "@value", value.clone());
            } else {
                return Err(JsonLdError::new(E::InvalidValueObjectValue));
            }
        }
        "@language" => match value {
            // [FABLE-5] sq-oy1f.29 — language tags are normalised to lower case
            // (§5.1.2 step 13.4.6 "Processors MAY normalize language tags to lower
            // case"; the reference processors and the framing suite do).
            Json::Str(s) => set_obj(result, "@language", Json::Str(s.to_lowercase())),
            Json::Arr(items) if frame_expansion => {
                let lowered: Vec<Json> = items
                    .iter()
                    .map(|i| match i {
                        Json::Str(s) => Json::Str(s.to_lowercase()),
                        other => other.clone(),
                    })
                    .collect();
                set_obj(result, "@language", Json::Arr(lowered));
            }
            // [FABLE-5] sq-oy1f.29 — frameExpansion also admits the wildcard `{}`
            // language pattern (framing value patterns, json-ld11-framing §2.2).
            Json::Obj(m) if frame_expansion && m.is_empty() => {
                set_obj(result, "@language", value.clone());
            }
            _ => return Err(JsonLdError::new(E::InvalidLanguageTaggedString)),
        },
        "@direction" => match value {
            Json::Str(s) if Direction::parse(s).is_some() => {
                set_obj(result, "@direction", value.clone());
            }
            Json::Arr(_) if frame_expansion => set_obj(result, "@direction", value.clone()),
            _ => return Err(JsonLdError::new(E::InvalidBaseDirection)),
        },
        "@index" => match value {
            Json::Str(_) => set_obj(result, "@index", value.clone()),
            _ => return Err(JsonLdError::new(E::InvalidIndexValue)),
        },
        "@list" => {
            // step 13.4.11: a list with no property (or under @graph) is dropped.
            if active_property.is_none() || active_property == Some("@graph") {
                return Ok(());
            }
            let expanded = expand_element(
                ctx,
                active_context,
                active_property,
                value,
                base_url,
                frame_expansion,
                false,
            )?;
            let arr = array_of(expanded.unwrap_or_else(|| Json::Arr(Vec::new())));
            set_obj(result, "@list", arr);
        }
        "@set" => {
            let expanded = expand_element(
                ctx,
                active_context,
                active_property,
                value,
                base_url,
                frame_expansion,
                false,
            )?;
            if let Some(v) = expanded {
                set_obj(result, "@set", v);
            }
        }
        "@reverse" => {
            let Json::Obj(_) = value else {
                return Err(JsonLdError::new(E::InvalidReverseValue));
            };
            let expanded = expand_element(
                ctx,
                active_context,
                Some("@reverse"),
                value,
                base_url,
                frame_expansion,
                false,
            )?;
            if let Some(Json::Obj(members)) = expanded {
                for (prop, items) in members {
                    if prop == "@reverse" {
                        // A double reverse folds back into forward properties.
                        if let Json::Obj(rev) = items {
                            for (p, its) in rev {
                                for it in as_array(its) {
                                    add_value(result, &p, it);
                                }
                            }
                        }
                    } else {
                        for it in as_array(items) {
                            if is_value_object(&it) || is_list_object(&it) {
                                return Err(JsonLdError::new(E::InvalidReversePropertyValue));
                            }
                            add_value(reverse_map(result), &prop, it);
                        }
                    }
                }
            }
        }
        // Framing keywords (frameExpansion mode); the framing pipeline (sq-oy1f.29) consumes
        // these. Outside frameExpansion they are dropped.
        "@default" | "@embed" | "@explicit" | "@omitDefault" | "@requireAll" if frame_expansion => {
            let expanded = expand_element(
                ctx,
                active_context,
                Some(expanded_property),
                value,
                base_url,
                frame_expansion,
                false,
            )?;
            if let Some(v) = expanded {
                set_obj(result, expanded_property, array_of(v));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Expands the value of a real property key (JSON-LD 1.1 API §5.1.2 steps 13.6–13.8):
/// container maps (`@language` / `@index` / `@id` / `@type` / `@graph`), the `@json` type
/// mapping, or the general recursion.
#[allow(clippy::too_many_arguments)]
fn expand_property_value(
    ctx: &Ctx<'_>,
    active_context: &ActiveContext,
    key: &str,
    value: &Json,
    base_url: Option<&str>,
    frame_expansion: bool,
    container: &[String],
) -> Result<Option<Json>, JsonLdError> {
    let has = |c: &str| container.iter().any(|m| m == c);
    let td = active_context.term_definition(key);

    // step 13.6: @language container (language map).
    if has("@language") && matches!(value, Json::Obj(_)) {
        let direction = match td.map(|t| t.direction()) {
            Some(Override::Set(d)) => Some(*d),
            Some(Override::Null) => None,
            _ => active_context.default_base_direction(),
        };
        let mut arr = Vec::new();
        for (lang, lang_value) in sorted_obj(value, ctx.ordered) {
            for item in as_array(lang_value) {
                if is_null(&item) {
                    continue;
                }
                let Json::Str(s) = &item else {
                    return Err(JsonLdError::new(E::InvalidLanguageMapValue));
                };
                let mut v = vec![("@value".to_string(), Json::Str(s.clone()))];
                let drop_lang =
                    lang == "@none" || active_context.expand_iri(&lang, false, true).as_deref() == Some("@none");
                if !drop_lang {
                    // [FABLE-5] sq-oy1f.29 — language tags normalise to lower case.
                    v.push(("@language".to_string(), Json::Str(lang.to_lowercase())));
                }
                if let Some(d) = direction {
                    v.push(("@direction".to_string(), Json::Str(d.as_str().to_string())));
                }
                arr.push(Json::Obj(v));
            }
        }
        return Ok(Some(Json::Arr(arr)));
    }

    // step 13.7: @index / @id / @type container maps.
    if (has("@index") || has("@id") || has("@type")) && matches!(value, Json::Obj(_)) {
        let index_key = td.and_then(|t| t.index()).unwrap_or("@index").to_string();
        let mut arr = Vec::new();
        for (index, index_value) in sorted_obj(value, ctx.ordered) {
            // step 13.8.3: "Initialize expanded index to the result of IRI expanding index"
            // (vocab) — every @none guard below compares against THIS expanded form, so a
            // term aliased to `@none` (`"none": "@none"`, W3C expand/m012) is recognised.
            let expanded_index = active_context.expand_iri(&index, false, true);
            let index_is_none = expanded_index.as_deref() == Some("@none");
            // Map context selection for @id / @type maps.
            let mut map_context = if has("@id") || has("@type") {
                active_context
                    .previous_context
                    .as_deref()
                    .cloned()
                    .unwrap_or_else(|| active_context.clone())
            } else {
                active_context.clone()
            };
            if has("@type") {
                let scoped = map_context
                    .term_definition(&index)
                    .and_then(|itd| itd.context().cloned().map(|c| (c, itd.base_url.clone())));
                if let Some((sctx, sbase)) = scoped {
                    map_context = map_context.process_scoped(
                        &sctx,
                        sbase.as_deref(),
                        false,
                        true,
                        ctx.loader,
                        ctx.options,
                    )?;
                }
            }

            let expanded = expand_element(
                ctx,
                &map_context,
                Some(key),
                &array_of(index_value),
                base_url,
                frame_expansion,
                true,
            )?;
            for mut item in as_array(expanded.unwrap_or_else(|| Json::Arr(Vec::new()))) {
                // @graph container: wrap a non-graph value.
                if has("@graph") && !is_graph_object(&item) {
                    item = Json::Obj(vec![("@graph".to_string(), array_of(item))]);
                }
                if has("@index") {
                    if index_key != "@index" {
                        // step 13.8.3.7: a property-valued index — only when the expanded
                        // index is not @none (W3C expand/pi10: an @none key adds no property).
                        if !index_is_none {
                            // 13.8.3.7.1: re-expand the index via Value Expansion against the
                            // ACTIVE context, with the index key as active property.
                            let reexpanded =
                                expand_value(active_context, &index_key, &Json::Str(index.clone()));
                            // 13.8.3.7.2: IRI-expand the index key.
                            let prop = active_context
                                .expand_iri(&index_key, false, true)
                                .unwrap_or_else(|| index_key.clone());
                            if let Json::Obj(m) = &mut item {
                                // 13.8.3.7.3–4: "an array consisting of re-expanded index
                                // FOLLOWED BY the existing values" (W3C expand/pi07, pi09).
                                prepend_value(m, &prop, reexpanded);
                                // 13.8.3.7.5: a value object MUST NOT gain an extra property
                                // (W3C expand/pi05).
                                if obj_get(m, "@value").is_some() {
                                    return Err(JsonLdError::new(E::InvalidValueObject));
                                }
                            }
                        }
                    } else if !index_is_none {
                        // step 13.8.3.8: a plain @index map entry.
                        if let Json::Obj(m) = &mut item {
                            if obj_get(m, "@index").is_none() {
                                set_obj(m, "@index", Json::Str(index.clone()));
                            }
                        }
                    }
                } else if has("@id") && !index_is_none {
                    // step 13.8.3.9: @id map — the guard is on the vocab-EXPANDED index; the
                    // @id value itself expands document-relatively.
                    if let Json::Obj(m) = &mut item {
                        if obj_get(m, "@id").is_none() {
                            let id = active_context
                                .expand_iri(&index, true, false)
                                .unwrap_or_else(|| index.clone());
                            set_obj(m, "@id", Json::Str(id));
                        }
                    }
                } else if has("@type") && !index_is_none {
                    // step 13.8.3.10: @type map — "types … consisting of expanded index
                    // FOLLOWED BY any existing values of @type" (W3C expand/m004), guarded
                    // against an @none alias (W3C expand/m012).
                    let ti = expanded_index.clone().unwrap_or_else(|| index.clone());
                    if let Json::Obj(m) = &mut item {
                        prepend_value(m, "@type", Json::Str(ti));
                    }
                }
                arr.push(item);
            }
        }
        return Ok(Some(Json::Arr(arr)));
    }

    // step 13.8: a @json type mapping keeps the value verbatim as a JSON literal.
    if td.and_then(|t| t.type_mapping()) == Some("@json") {
        return Ok(Some(Json::Obj(vec![
            ("@value".to_string(), value.clone()),
            ("@type".to_string(), Json::Str("@json".to_string())),
        ])));
    }

    // step 13.8 (else): the general recursion.
    expand_element(ctx, active_context, Some(key), value, base_url, frame_expansion, false)
}

/// **Value Expansion** (JSON-LD 1.1 API §5.3.2): expands a scalar `value` for `active_property`
/// into a value object (or a node reference under an `@id` / `@vocab` type coercion).
fn expand_value(active_context: &ActiveContext, active_property: &str, value: &Json) -> Json {
    let td = active_context.term_definition(active_property);
    let type_mapping = td.and_then(|t| t.type_mapping());

    // @id / @vocab coercion of a string value → a node reference (§5.3.2 steps 1–2): "return
    // a new map containing a single entry where the key is @id and the value is the result of
    // IRI expanding value". When IRI Expansion returns `None` (a keyword-shaped token, or a
    // `@vocab` term bound to null; see `context::iri`) the spec-literal result is therefore
    // `{"@id": null}`. The entry is KEPT with a JSON null, never emitted as an empty-string
    // `@id` or an empty `{}`. This matches the suite's `@id`-keyword precedent
    // (W3C expand/0122-out retains `"@id": null`).
    if let Json::Str(s) = value {
        if type_mapping == Some("@id") {
            let id = active_context.expand_iri(s, true, false).map_or_else(json_null, Json::Str);
            return Json::Obj(vec![("@id".to_string(), id)]);
        }
        if type_mapping == Some("@vocab") {
            let id = active_context.expand_iri(s, true, true).map_or_else(json_null, Json::Str);
            return Json::Obj(vec![("@id".to_string(), id)]);
        }
    }

    let mut result = vec![("@value".to_string(), value.clone())];
    match type_mapping {
        Some(tm) if tm != "@id" && tm != "@vocab" && tm != "@none" => {
            result.push(("@type".to_string(), Json::Str(tm.to_string())));
        }
        _ => {
            if matches!(value, Json::Str(_)) {
                let language = match td.map(|t| t.language()) {
                    Some(Override::Set(l)) => Some(l.clone()),
                    Some(Override::Null) => None,
                    _ => active_context.default_language().map(str::to_string),
                };
                let direction = match td.map(|t| t.direction()) {
                    Some(Override::Set(d)) => Some(*d),
                    Some(Override::Null) => None,
                    _ => active_context.default_base_direction(),
                };
                if let Some(l) = language {
                    // [FABLE-5] sq-oy1f.29 — language tags normalise to lower case.
                    result.push(("@language".to_string(), Json::Str(l.to_lowercase())));
                }
                if let Some(d) = direction {
                    result.push(("@direction".to_string(), Json::Str(d.as_str().to_string())));
                }
            }
        }
    }
    Json::Obj(result)
}

/// Value/list/set/node cleanup (JSON-LD 1.1 API §5.1.2 steps 15–20). Returns `None` when the
/// expanded node is dropped (a free-floating value, a null value object, etc.).
fn cleanup(
    mut members: Vec<(String, Json)>,
    active_property: Option<&str>,
    frame_expansion: bool,
) -> Result<Option<Json>, JsonLdError> {
    let has = |m: &[(String, Json)], k: &str| m.iter().any(|(kk, _)| kk == k);

    // step 19 (W3C JSON-LD 1.1 API §5.1.2): under a null or `@graph` active
    // property, drop free-floating values — a value object (ANY map with
    // `@value`, e.g. also carrying `@language`/`@type`/`@index`), a list object
    // (`@list`), an empty map, or an `@id`-only node object. Checked BEFORE the
    // value/list branches return so the drop reaches those shapes (W3C
    // expand/0045, 0046). Reference: jsonld.js drops when the result contains
    // `@value` or `@list`, not merely when those are the SOLE key.
    let free_floating =
        !frame_expansion && (active_property.is_none() || active_property == Some("@graph"));
    if free_floating {
        let only_id = members.len() == 1 && members[0].0 == "@id";
        if members.is_empty() || has(&members, "@value") || has(&members, "@list") || only_id {
            return Ok(None);
        }
    }

    // step 15: a value object.
    if has(&members, "@value") {
        const ALLOWED: &[&str] = &["@value", "@type", "@language", "@index", "@direction"];
        if members.iter().any(|(k, _)| !ALLOWED.contains(&k.as_str())) {
            return Err(JsonLdError::new(E::InvalidValueObject));
        }
        // §5.1.2 step 15.1: the value object "must not contain an `@type` entry if it
        // contains either `@language` or `@direction` entries" — a typed literal cannot
        // also carry a language tag OR a base direction (W3C expand/di09 pins the
        // `@direction` half).
        if has(&members, "@type") && (has(&members, "@language") || has(&members, "@direction")) {
            return Err(JsonLdError::new(E::InvalidValueObject));
        }
        // A value object's `@type` is a SINGLE value in the JSON-LD data model
        // (unlike a node object's `@type`, which is an array). The general
        // keyword-entry path (step 13.4.4) arrayifies every `@type`; collapse a
        // single-element array back to a scalar here so the expanded value
        // object matches the normative shape (W3C expand/0002, 0013, tjs15/16).
        // Frame-expansion keeps arrays (a frame may match multiple types).
        if !frame_expansion {
            if let Some(slot) = members.iter_mut().find(|(k, _)| k == "@type") {
                if let Json::Arr(a) = &slot.1 {
                    if a.len() == 1 {
                        slot.1 = a[0].clone();
                    }
                }
            }
        }
        // [FABLE-5] sq-oy1f.29 — `@type: "@json"` may arrive as a kept single-element
        // array under frameExpansion; recognise both shapes.
        let is_json = match obj_get(&members, "@type") {
            Some(Json::Str(s)) => s == "@json",
            Some(Json::Arr(a)) if frame_expansion => {
                matches!(a.as_slice(), [Json::Str(s)] if s == "@json")
            }
            _ => false,
        };
        let v = obj_get(&members, "@value").unwrap();
        if !is_json {
            // An empty-array `@value` is a match-none value PATTERN under
            // frameExpansion (kept); outside framing the value object is dropped.
            if is_null(v) || (!frame_expansion && matches!(v, Json::Arr(a) if a.is_empty())) {
                return Ok(None);
            }
            if has(&members, "@language")
                && !matches!(v, Json::Str(_))
                // [FABLE-5] sq-oy1f.29 — frameExpansion also admits the wildcard `{}`
                // and alternative-array `@value` patterns alongside `@language`.
                && !(frame_expansion && matches!(v, Json::Arr(_) | Json::Obj(_)))
            {
                return Err(JsonLdError::new(E::InvalidLanguageTaggedValue));
            }
            if let Some(t) = obj_get(&members, "@type") {
                // [FABLE-5] sq-oy1f.29 — a frame value pattern's @type alternative may
                // also be the wildcard `{}` or `@json`.
                // §5.1.2 step 15.4: the value of `@type` must be an IRI — a blank node
                // identifier is NOT one, so a `_:`-datatyped literal is an
                // `invalid typed value` (W3C expand/er40). Blank-node datatypes are only
                // meaningful under the `GeneralizedRdf` optional feature, which sparq does
                // not opt into (those suite cases are the `requires` skip bucket).
                // Frame expansion keeps admitting a blank node so a frame can pattern-match
                // one (json-ld11-framing §2.2 value patterns).
                let type_ok = |e: &Json| match e {
                    Json::Str(s) => {
                        is_absolute_iri(s) || (frame_expansion && (is_blank_node(s) || s == "@json"))
                    }
                    Json::Obj(m) => frame_expansion && m.is_empty(),
                    _ => false,
                };
                let ok = match t {
                    // Frame-expansion permits an array of types on a value object.
                    Json::Arr(a) if frame_expansion => a.iter().all(type_ok),
                    other => type_ok(other),
                };
                if !ok {
                    return Err(JsonLdError::new(E::InvalidTypedValue));
                }
            }
        }
        return Ok(Some(Json::Obj(members)));
    }

    // step 17: a list or set object.
    if has(&members, "@list") || has(&members, "@set") {
        const ALLOWED: &[&str] = &["@list", "@set", "@index"];
        if members.iter().any(|(k, _)| !ALLOWED.contains(&k.as_str())) {
            return Err(JsonLdError::new(E::InvalidSetOrListObject));
        }
        if has(&members, "@set") {
            // Unwrap @set to its value.
            let set_val = members.into_iter().find(|(k, _)| k == "@set").map(|(_, v)| v).unwrap();
            return Ok(Some(set_val));
        }
        return Ok(Some(Json::Obj(members)));
    }

    // step 16: a node object's @type is always an array.
    if let Some(slot) = members.iter_mut().find(|(k, _)| k == "@type") {
        if !matches!(slot.1, Json::Arr(_)) {
            slot.1 = Json::Arr(vec![slot.1.clone()]);
        }
    }

    // step 18: a map containing only @language is dropped.
    if !frame_expansion && members.len() == 1 && members[0].0 == "@language" {
        return Ok(None);
    }

    // step 19 (node object): the empty / `@id`-only free-floating drop already
    // ran up-front (before the value/list branches) so it reaches every shape.
    Ok(Some(Json::Obj(members)))
}

// ---------------------------------------------------------------------------------------
// Small helpers over the `Json` AST.
// ---------------------------------------------------------------------------------------

/// True iff `j` is a JSON `null`.
fn is_null(j: &Json) -> bool {
    matches!(j, Json::Raw(r) if r == "null")
}

/// True iff `j` is a JSON scalar (string, number, or boolean) — i.e. not null, array, or map.
fn is_scalar(j: &Json) -> bool {
    match j {
        Json::Str(_) => true,
        Json::Raw(r) => r != "null",
        _ => false,
    }
}

/// The `Json` representation of a JSON `null`.
fn json_null() -> Json {
    Json::Raw("null".to_string())
}

/// Consumes `j` into a vector of items: the elements of an array, or `[j]` for a single value.
fn as_array(j: Json) -> Vec<Json> {
    match j {
        Json::Arr(v) => v,
        other => vec![other],
    }
}

/// Wraps `j` into an array unless it already is one.
fn array_of(j: Json) -> Json {
    match j {
        Json::Arr(_) => j,
        other => Json::Arr(vec![other]),
    }
}

/// The keys of a map, in insertion order, or lexicographically sorted when `ordered`.
fn sorted_keys(element: &Json, ordered: bool) -> Vec<String> {
    let mut keys: Vec<String> = match element {
        Json::Obj(m) => m.iter().map(|(k, _)| k.clone()).collect(),
        _ => Vec::new(),
    };
    if ordered {
        keys.sort();
    }
    keys
}

/// The (key, value) members of a map, in insertion order, or sorted by key when `ordered`.
fn sorted_obj(element: &Json, ordered: bool) -> Vec<(String, Json)> {
    let mut members: Vec<(String, Json)> = match element {
        Json::Obj(m) => m.clone(),
        _ => Vec::new(),
    };
    if ordered {
        members.sort_by(|a, b| a.0.cmp(&b.0));
    }
    members
}

/// The string values of a JSON value (a lone string, or the string items of an array).
fn string_values(j: &Json) -> Vec<String> {
    match j {
        Json::Str(s) => vec![s.clone()],
        Json::Arr(items) => items.iter().filter_map(Json::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Validates and extracts a `@type` value (a string or array of strings; frame mode is
/// lenient), raising `invalid type value` on a malformed value.
fn type_values(value: &Json, frame_expansion: bool) -> Result<Vec<String>, JsonLdError> {
    match value {
        Json::Str(s) => Ok(vec![s.clone()]),
        Json::Arr(items) => {
            let mut out = Vec::new();
            for it in items {
                match it {
                    Json::Str(s) => out.push(s.clone()),
                    _ if frame_expansion => {}
                    _ => return Err(JsonLdError::new(E::InvalidTypeValue)),
                }
            }
            Ok(out)
        }
        Json::Obj(_) if frame_expansion => Ok(Vec::new()),
        _ => Err(JsonLdError::new(E::InvalidTypeValue)),
    }
}

/// The IRI expansion of the last value of the first `@type`-expanding entry (JSON-LD 1.1 API
/// §5.1.2 step 12) — used only to detect a `@json`-typed sibling `@value`.
fn first_type_input(active_context: &ActiveContext, element: &Json) -> Option<String> {
    for key in sorted_keys(element, true) {
        if active_context.expand_iri(&key, false, true).as_deref() == Some("@type") {
            let v = element.get(&key).unwrap();
            let last = match v {
                Json::Arr(a) => a.last().and_then(Json::as_str),
                other => other.as_str(),
            };
            return last.and_then(|t| active_context.expand_iri(t, false, true));
        }
    }
    None
}

/// True iff any key of `element` IRI-expands (vocab) to `keyword`.
fn has_key_expanding_to(active_context: &ActiveContext, element: &Json, keyword: &str) -> bool {
    match element {
        Json::Obj(m) => m
            .iter()
            .any(|(k, _)| active_context.expand_iri(k, false, true).as_deref() == Some(keyword)),
        _ => false,
    }
}

/// True iff `element` has exactly one entry whose key IRI-expands (vocab) to `keyword`.
fn is_single_entry_expanding_to(active_context: &ActiveContext, element: &Json, keyword: &str) -> bool {
    match element {
        Json::Obj(m) => {
            m.len() == 1 && active_context.expand_iri(&m[0].0, false, true).as_deref() == Some(keyword)
        }
        _ => false,
    }
}

/// The container mapping of `active_property` in `active_context`, or an empty slice.
fn container_of(active_context: &ActiveContext, active_property: Option<&str>) -> Vec<String> {
    active_property
        .and_then(|ap| active_context.term_definition(ap))
        .map(|td| td.container().to_vec())
        .unwrap_or_default()
}

/// Borrows a member of an ordered map by key.
fn obj_get<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Sets (overwriting) a member of an ordered map, preserving first-insertion position.
fn set_obj(members: &mut Vec<(String, Json)>, key: &str, value: Json) {
    if let Some(slot) = members.iter_mut().find(|(k, _)| k == key) {
        slot.1 = value;
    } else {
        members.push((key.to_string(), value));
    }
}

/// Appends `value` to the array stored at `key` (JSON-LD `addValue` with `propertyIsArray`):
/// expansion always accumulates property values into an array. An array `value` contributes
/// each of its items.
fn add_value(members: &mut Vec<(String, Json)>, key: &str, value: Json) {
    if let Json::Arr(items) = value {
        for it in items {
            add_value(members, key, it);
        }
        return;
    }
    match members.iter_mut().find(|(k, _)| k == key) {
        Some((_, Json::Arr(a))) => a.push(value),
        Some((_, other)) => {
            let old = std::mem::replace(other, Json::Arr(Vec::new()));
            if let Json::Arr(a) = other {
                a.push(old);
                a.push(value);
            }
        }
        None => members.push((key.to_string(), Json::Arr(vec![value]))),
    }
}

/// `addValue` with `asArray = true` (JSON-LD 1.1 API, Add Value algorithm step 1):
/// ENSURE `key` maps to an array first, THEN add `value`. Unlike [`add_value`],
/// this RETAINS the property when `value` is an empty array — the load-bearing
/// difference for a term whose value expands to `[]` (a `@set`/`@list` container,
/// or a plain empty-array property value), whose empty-array expansion is a
/// normative, retained property (W3C expand/0004, 0015, 0016). Used on EVERY
/// forward-property add (the reference `_addValue(..., propertyIsArray true)` is
/// unconditional); a `null` (`None`) expansion is dropped by the caller before
/// this is reached.
fn add_value_as_array(members: &mut Vec<(String, Json)>, key: &str, value: Json) {
    // step 1: seed an empty array at key if absent / non-array (folding any
    // pre-existing scalar into it) — this is what makes an empty add retain.
    match members.iter_mut().find(|(k, _)| k == key) {
        Some((_, Json::Arr(_))) => {}
        Some((_, other)) => {
            let old = std::mem::replace(other, Json::Arr(vec![]));
            if let Json::Arr(a) = other {
                a.push(old);
            }
        }
        None => members.push((key.to_string(), Json::Arr(vec![]))),
    }
    // step 2/3: add value (an array contributes each element; scalar appends).
    add_value(members, key, value);
}

/// Prepends `value` to the array stored at `key`: the index-map steps place the (re-)expanded
/// index BEFORE any existing values ("an array consisting of re-expanded index followed by the
/// existing values", §5.1.2 step 13.8.3.7.3; likewise the @type-map step 13.8.3.10).
fn prepend_value(members: &mut Vec<(String, Json)>, key: &str, value: Json) {
    match members.iter_mut().find(|(k, _)| k == key) {
        Some((_, Json::Arr(a))) => a.insert(0, value),
        Some((_, other)) => {
            let old = std::mem::replace(other, Json::Arr(Vec::new()));
            if let Json::Arr(a) = other {
                a.push(value);
                a.push(old);
            }
        }
        None => members.push((key.to_string(), Json::Arr(vec![value]))),
    }
}

/// Gets or inserts the `@reverse` sub-map of an ordered map and returns its members.
fn reverse_map(members: &mut Vec<(String, Json)>) -> &mut Vec<(String, Json)> {
    if !members.iter().any(|(k, _)| k == "@reverse") {
        members.push(("@reverse".to_string(), Json::Obj(Vec::new())));
    }
    let slot = members.iter_mut().find(|(k, _)| k == "@reverse").unwrap();
    match &mut slot.1 {
        Json::Obj(m) => m,
        _ => unreachable!("@reverse is always a map"),
    }
}

/// True iff `j` is a value object (a map with an `@value` entry).
fn is_value_object(j: &Json) -> bool {
    matches!(j, Json::Obj(_)) && j.get("@value").is_some()
}

/// True iff `j` is a list object (a map with an `@list` entry).
fn is_list_object(j: &Json) -> bool {
    matches!(j, Json::Obj(_)) && j.get("@list").is_some()
}

/// True iff `j` is a graph object (a map with an `@graph` entry).
fn is_graph_object(j: &Json) -> bool {
    matches!(j, Json::Obj(_)) && j.get("@graph").is_some()
}

/// True iff `j` is a node object (a map that is not a value / list / set object).
fn is_node_object(j: &Json) -> bool {
    matches!(j, Json::Obj(_))
        && j.get("@value").is_none()
        && j.get("@list").is_none()
        && j.get("@set").is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(s: &str) -> Json {
        Json::Raw(s.to_string())
    }

    #[test]
    fn null_and_scalar_predicates() {
        assert!(is_null(&raw("null")));
        assert!(!is_null(&raw("42")));
        assert!(!is_null(&Json::Str("x".into())));
        assert!(is_scalar(&Json::Str("x".into())));
        assert!(is_scalar(&raw("42")));
        assert!(is_scalar(&raw("true")));
        assert!(!is_scalar(&raw("null")));
        assert!(!is_scalar(&Json::Arr(vec![])));
        assert!(!is_scalar(&Json::obj()));
    }

    #[test]
    fn as_array_and_array_of_round_trip() {
        assert_eq!(as_array(Json::Arr(vec![raw("1")])), vec![raw("1")]);
        assert_eq!(as_array(Json::Str("x".into())), vec![Json::Str("x".into())]);
        assert_eq!(array_of(Json::Str("x".into())), Json::Arr(vec![Json::Str("x".into())]));
        // Already an array: array_of is the identity.
        assert_eq!(array_of(Json::Arr(vec![raw("1")])), Json::Arr(vec![raw("1")]));
    }

    #[test]
    fn prepend_value_inserts_before_existing_values() {
        let mut m = Vec::new();
        // Absent key → a fresh single-item array.
        prepend_value(&mut m, "p", Json::Str("b".into()));
        assert_eq!(obj_get(&m, "p"), Some(&Json::Arr(vec![Json::Str("b".into())])));
        // Present key → the new value goes FIRST (§5.1.2 step 13.8.3.7.3 ordering).
        prepend_value(&mut m, "p", Json::Str("a".into()));
        assert_eq!(
            obj_get(&m, "p"),
            Some(&Json::Arr(vec![Json::Str("a".into()), Json::Str("b".into())]))
        );
        // A non-array existing value is array-wrapped with the new value first.
        let mut m2 = vec![("q".to_string(), Json::Str("old".into()))];
        prepend_value(&mut m2, "q", Json::Str("new".into()));
        assert_eq!(
            obj_get(&m2, "q"),
            Some(&Json::Arr(vec![Json::Str("new".into()), Json::Str("old".into())]))
        );
    }

    #[test]
    fn add_value_accumulates_into_arrays() {
        let mut m = Vec::new();
        add_value(&mut m, "p", Json::Str("a".into()));
        add_value(&mut m, "p", Json::Str("b".into()));
        // An array value contributes each of its items.
        add_value(&mut m, "p", Json::Arr(vec![Json::Str("c".into())]));
        assert_eq!(
            obj_get(&m, "p"),
            Some(&Json::Arr(vec![
                Json::Str("a".into()),
                Json::Str("b".into()),
                Json::Str("c".into()),
            ]))
        );
    }

    #[test]
    fn set_obj_overwrites_in_place() {
        let mut m = Vec::new();
        set_obj(&mut m, "k", raw("1"));
        set_obj(&mut m, "k", raw("2"));
        assert_eq!(m.len(), 1);
        assert_eq!(obj_get(&m, "k"), Some(&raw("2")));
    }

    #[test]
    fn reverse_map_is_created_once() {
        let mut m = Vec::new();
        add_value(reverse_map(&mut m), "p", Json::Str("a".into()));
        add_value(reverse_map(&mut m), "q", Json::Str("b".into()));
        // Exactly one @reverse entry, holding both properties.
        assert_eq!(m.iter().filter(|(k, _)| k == "@reverse").count(), 1);
    }

    #[test]
    fn object_kind_predicates() {
        let value = Json::Obj(vec![("@value".into(), raw("1"))]);
        let list = Json::Obj(vec![("@list".into(), Json::Arr(vec![]))]);
        let graph = Json::Obj(vec![("@graph".into(), Json::Arr(vec![]))]);
        let node = Json::Obj(vec![("@id".into(), Json::Str("http://x".into()))]);
        assert!(is_value_object(&value) && !is_node_object(&value));
        assert!(is_list_object(&list) && !is_node_object(&list));
        assert!(is_graph_object(&graph) && is_node_object(&graph));
        assert!(is_node_object(&node) && !is_value_object(&node));
    }

    #[test]
    fn string_values_extracts_only_strings() {
        let arr = Json::Arr(vec![Json::Str("a".into()), raw("1"), Json::Str("b".into())]);
        assert_eq!(string_values(&arr), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(string_values(&Json::Str("solo".into())), vec!["solo".to_string()]);
        assert!(string_values(&raw("1")).is_empty());
    }

    #[test]
    fn type_values_validates() {
        assert_eq!(type_values(&Json::Str("T".into()), false).unwrap(), vec!["T".to_string()]);
        assert!(type_values(&raw("42"), false).is_err());
        // Frame expansion tolerates a map / non-string members.
        assert!(type_values(&Json::obj(), true).unwrap().is_empty());
    }

    /// Builds an active context that coerces term `id_term` with `@type: @id` and `vocab_term`
    /// with `@type: @vocab`, over a `@vocab` base so a plain vocab reference resolves.
    fn coercion_context() -> ActiveContext {
        let ctx = Json::parse(
            r#"{"@vocab":"http://ex/v#","id_term":{"@type":"@id"},"vocab_term":{"@type":"@vocab"}}"#,
        )
        .unwrap();
        ActiveContext::new(None)
            .process(&ctx, None, &crate::loader::NoopLoader, &JsonLdOptions::default())
            .expect("context processes")
    }

    #[test]
    fn expand_value_id_coercion_null_expansion_yields_null_id() {
        let ac = coercion_context();
        // A resolvable absolute IRI under `@id` coercion → a `{"@id": <iri>}` node reference.
        assert_eq!(
            expand_value(&ac, "id_term", &Json::Str("http://ex/target".into())),
            Json::Obj(vec![(
                "@id".to_string(),
                Json::Str("http://ex/target".into()),
            )]),
        );
        assert_eq!(
            expand_value(&ac, "id_term", &Json::Str("@notakeyword".into())),
            Json::Obj(vec![("@id".to_string(), json_null())]),
            "a null IRI expansion must remain an explicit null @id",
        );
    }

    #[test]
    fn expand_value_vocab_coercion_null_expansion_yields_null_id() {
        let ac = coercion_context();
        // A plain term under `@vocab` coercion resolves against `@vocab`.
        assert_eq!(
            expand_value(&ac, "vocab_term", &Json::Str("thing".into())),
            Json::Obj(vec![(
                "@id".to_string(),
                Json::Str("http://ex/v#thing".into()),
            )]),
        );
        assert_eq!(
            expand_value(&ac, "vocab_term", &Json::Str("@notakeyword".into())),
            Json::Obj(vec![("@id".to_string(), json_null())]),
        );
    }
}
