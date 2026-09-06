//! Context Processing Algorithm + Create Term Definition (JSON-LD 1.1 API §4).
//!
//! [OPUS-4.8] (sq-oy1f.24) Folds a chain of local/remote `@context` definitions into an
//! [`ActiveContext`], handling keyword aliases, `@protected` term protection,
//! `@propagate`, `@import`, `@version`, and the `@vocab`/`@base`/`@language`/`@direction`
//! defaults. Remote contexts (`@context` strings and `@import`) are dereferenced **only**
//! through the [`DocumentLoader`], which is deny-by-default.
//!
//! Spec: <https://www.w3.org/TR/json-ld11-api/#context-processing-algorithm> and
//! <https://www.w3.org/TR/json-ld11-api/#create-term-definition>.

use std::collections::{BTreeMap, BTreeSet};

use super::iri::{expand_iri_in_context, is_absolute_iri, is_blank_node, resolve_iri};
use super::{has_keyword_form, is_keyword, ActiveContext, Direction, Override, TermDefinition};
use crate::error::{JsonLdError, JsonLdErrorCode as E};
use crate::json::Json;
use crate::loader::DocumentLoader;
use crate::options::{JsonLdOptions, ProcessingMode};

/// A processor-defined ceiling on the depth of remote-context dereferencing, guarding the
/// `context overflow` error condition (JSON-LD 1.1 API §4.1.2 step 5.2).
const MAX_REMOTE_CONTEXTS: usize = 100;

/// Shared, mutable processing environment threaded through Context Processing and Create
/// Term Definition — the loader, processing mode, current base URL, the remote-context
/// recursion guard, and the scoped-context validation flag. Bundled to keep argument
/// counts sane.
pub(crate) struct Env<'a> {
    pub(crate) loader: &'a dyn DocumentLoader,
    pub(crate) mode: ProcessingMode,
    pub(crate) base_url: Option<String>,
    pub(crate) remote_contexts: Vec<String>,
    pub(crate) validate_scoped: bool,
}

impl ActiveContext {
    /// The **Context Processing Algorithm** (JSON-LD 1.1 API §4.1): folds `local_context`
    /// (a context definition, IRI string, `null`, or array thereof) into a fresh active
    /// context derived from `self`.
    ///
    /// `base_url` is the base for resolving relative references in the context (e.g. remote
    /// context IRIs and `@import`). Remote references are dereferenced only through
    /// `loader`; the default [`NoopLoader`](crate::loader::NoopLoader) refuses every fetch,
    /// so a context that references a remote document fails closed with `loading remote
    /// context failed`. Returns the processed [`ActiveContext`] or the first spec error.
    pub fn process(
        &self,
        local_context: &Json,
        base_url: Option<&str>,
        loader: &dyn DocumentLoader,
        options: &JsonLdOptions,
    ) -> Result<ActiveContext, JsonLdError> {
        let mut env = Env {
            loader,
            mode: options.processing_mode,
            base_url: base_url.map(str::to_string),
            remote_contexts: Vec::new(),
            validate_scoped: true,
        };
        process_inner(self, local_context, false, true, &mut env)
    }

    /// Applies a **scoped** context (a property-scoped or type-scoped `@context`) during
    /// Expansion (JSON-LD 1.1 API §5.1.2 steps 8 and 11). Unlike [`ActiveContext::process`]
    /// this exposes the two flags Expansion needs: `override_protected` (true when a
    /// property-scoped context re-applies, so it may legally redefine a protected term) and
    /// `propagate` (false when a type-scoped context is applied, so it does not carry into
    /// child node objects). Context validation is left on (`validate_scoped: true`, the same
    /// as [`ActiveContext::process`]); the scoped context's term definitions are only
    /// pre-checked syntactically at Create Term Definition time, so applying it here still
    /// runs the full validating Context Processing pass.
    ///
    /// [OPUS-4.8] (sq-oy1f.25)
    pub(crate) fn process_scoped(
        &self,
        local_context: &Json,
        base_url: Option<&str>,
        override_protected: bool,
        propagate: bool,
        loader: &dyn DocumentLoader,
        options: &JsonLdOptions,
    ) -> Result<ActiveContext, JsonLdError> {
        let mut env = Env {
            loader,
            mode: options.processing_mode,
            base_url: base_url.map(str::to_string),
            remote_contexts: Vec::new(),
            validate_scoped: true,
        };
        process_inner(self, local_context, override_protected, propagate, &mut env)
    }
}

/// The recursive core of Context Processing (§4.1.2). `override_protected` disables the
/// protected-term guard (used when validating scoped contexts); `propagate` controls
/// whether a null context resets the previous-context link.
pub(crate) fn process_inner(
    active: &ActiveContext,
    local_context: &Json,
    override_protected: bool,
    mut propagate: bool,
    env: &mut Env,
) -> Result<ActiveContext, JsonLdError> {
    // step 1: result is a copy of the input active context.
    let mut result = active.clone();

    // step 2: a top-level @propagate overrides the inherited value.
    if let Json::Obj(_) = local_context {
        if let Some(p) = local_context.get("@propagate") {
            propagate = as_bool(p).ok_or_else(|| JsonLdError::new(E::InvalidPropagateValue))?;
        }
    }

    // step 3: when not propagating, remember the input context to revert to.
    if !propagate && result.previous_context.is_none() {
        result.previous_context = Some(std::sync::Arc::new(active.clone()));
    }

    // step 4: normalise to an array of contexts.
    let contexts: Vec<Json> = match local_context {
        Json::Arr(items) => items.clone(),
        other => vec![other.clone()],
    };

    // step 5: fold each context in turn.
    for context in &contexts {
        // 5.1: a null context resets the active context.
        if is_null(context) {
            if !override_protected && result.has_protected_terms() {
                return Err(JsonLdError::new(E::InvalidContextNullification));
            }
            let mut fresh = ActiveContext::new(active.original_base_url.as_deref());
            if !propagate {
                fresh.previous_context = Some(std::sync::Arc::new(result.clone()));
            }
            result = fresh;
            continue;
        }

        // 5.2: a string context is a remote reference.
        if let Json::Str(iri) = context {
            let resolved = match &env.base_url {
                Some(b) => resolve_iri(b, iri),
                None => iri.clone(),
            };
            // 5.2.2: skip a context already loaded when not re-validating.
            if !env.validate_scoped && env.remote_contexts.contains(&resolved) {
                continue;
            }
            // 5.2.3: recursion guard.
            if env.remote_contexts.len() >= MAX_REMOTE_CONTEXTS {
                return Err(JsonLdError::new(E::ContextOverflow));
            }
            env.remote_contexts.push(resolved.clone());
            // 5.2.5: dereference through the loader (deny-by-default).
            let doc = env
                .loader
                .load_document(&resolved)
                .map_err(|e| JsonLdError::with_detail(E::LoadingRemoteContextFailed, e.to_string()))?;
            let parsed = Json::parse(&doc.document)
                .map_err(|e| JsonLdError::with_detail(E::InvalidRemoteContext, e.to_string()))?;
            let loaded_ctx = match parsed.get("@context") {
                Some(c) => c.clone(),
                None => return Err(JsonLdError::new(E::InvalidRemoteContext)),
            };
            // 5.2.7: recurse using the retrieved document URL as the base. `remote_contexts`
            // behaves as a properly-scoped stack: `resolved` is visible to the recursion
            // (so nested self-references are detected and the recursion's own `@base` is
            // suppressed), then popped so a *sibling* context in this same array is
            // processed as if freshly invoked (matching the spec's per-invocation
            // `remote contexts` copy semantics).
            let saved_base = env.base_url.take();
            let saved_validate = env.validate_scoped;
            env.base_url = Some(doc.document_url.clone());
            env.validate_scoped = false;
            let recursed = process_inner(&result, &loaded_ctx, override_protected, propagate, env);
            env.base_url = saved_base;
            env.validate_scoped = saved_validate;
            env.remote_contexts.pop();
            result = recursed?;
            continue;
        }

        // 5.3: anything else must be a context-definition map.
        if !matches!(context, Json::Obj(_)) {
            return Err(JsonLdError::new(E::InvalidLocalContext));
        }

        // 5.5: @version.
        if let Some(v) = context.get("@version") {
            if !matches!(v, Json::Raw(r) if r == "1.1") {
                return Err(JsonLdError::new(E::InvalidVersionValue));
            }
            if env.mode == ProcessingMode::JsonLd10 {
                return Err(JsonLdError::new(E::ProcessingModeConflict));
            }
        }

        // 5.6: @import merges a remote context under the local one.
        let merged;
        let context: &Json = if let Some(imp) = context.get("@import") {
            if env.mode == ProcessingMode::JsonLd10 {
                return Err(JsonLdError::new(E::InvalidContextEntry));
            }
            let Json::Str(imp_iri) = imp else {
                return Err(JsonLdError::new(E::InvalidImportValue));
            };
            let resolved = match &env.base_url {
                Some(b) => resolve_iri(b, imp_iri),
                None => imp_iri.clone(),
            };
            let doc = env
                .loader
                .load_document(&resolved)
                .map_err(|e| JsonLdError::with_detail(E::LoadingRemoteContextFailed, e.to_string()))?;
            let parsed = Json::parse(&doc.document)
                .map_err(|e| JsonLdError::with_detail(E::InvalidRemoteContext, e.to_string()))?;
            let import_members = match parsed.get("@context") {
                Some(Json::Obj(m)) => m.clone(),
                _ => return Err(JsonLdError::new(E::InvalidRemoteContext)),
            };
            if import_members.iter().any(|(k, _)| k == "@import") {
                return Err(JsonLdError::new(E::InvalidContextEntry));
            }
            merged = merge_import(&import_members, context);
            &merged
        } else {
            context
        };

        // 5.7: @base — only for a directly-embedded (non-remote) context.
        if env.remote_contexts.is_empty() {
            if let Some(b) = context.get("@base") {
                if is_null(b) {
                    result.base_iri = None;
                } else if let Json::Str(s) = b {
                    if is_absolute_iri(s) {
                        result.base_iri = Some(s.clone());
                    } else if let Some(cur) = result.base_iri.clone() {
                        result.base_iri = Some(resolve_iri(&cur, s));
                    } else {
                        return Err(JsonLdError::new(E::InvalidBaseIri));
                    }
                } else {
                    return Err(JsonLdError::new(E::InvalidBaseIri));
                }
            }
        }

        // 5.8: @vocab.
        if let Some(v) = context.get("@vocab") {
            if is_null(v) {
                result.vocabulary_mapping = None;
            } else if let Json::Str(s) = v {
                // [OPUS-5] sq-gzsky — §4.1.2 step 5.8 tests the RAW value: "if value is
                // neither an IRI nor a blank node identifier, an invalid vocab mapping
                // error". A RELATIVE reference (including the empty string) resolved
                // against `@base` / the current `@vocab` is the JSON-LD 1.1 relaxation
                // (W3C expand/0112, a 1.1 positive); in `json-ld-1.0` processing mode the
                // 1.0 rule stands and the same shapes are an error (expand/0115 `""`,
                // expand/0116 `"/relative"`).
                if env.mode == ProcessingMode::JsonLd10
                    && !(is_absolute_iri(s) || is_blank_node(s))
                {
                    return Err(JsonLdError::new(E::InvalidVocabMapping));
                }
                match result.expand_iri_readonly(s, true, true) {
                    Some(e) if e.is_empty() || is_absolute_iri(&e) || is_blank_node(&e) => {
                        result.vocabulary_mapping = Some(e);
                    }
                    _ => return Err(JsonLdError::new(E::InvalidVocabMapping)),
                }
            } else {
                return Err(JsonLdError::new(E::InvalidVocabMapping));
            }
        }

        // 5.9: @language.
        if let Some(v) = context.get("@language") {
            if is_null(v) {
                result.default_language = None;
            } else if let Json::Str(s) = v {
                result.default_language = Some(s.clone());
            } else {
                return Err(JsonLdError::new(E::InvalidDefaultLanguage));
            }
        }

        // 5.10: @direction.
        if let Some(v) = context.get("@direction") {
            if env.mode == ProcessingMode::JsonLd10 {
                return Err(JsonLdError::new(E::InvalidContextEntry));
            }
            if is_null(v) {
                result.default_base_direction = None;
            } else if let Json::Str(s) = v {
                result.default_base_direction =
                    Some(Direction::parse(s).ok_or_else(|| JsonLdError::new(E::InvalidBaseDirection))?);
            } else {
                return Err(JsonLdError::new(E::InvalidBaseDirection));
            }
        }

        // 5.11: @propagate is validated (its effect was applied in step 2).
        if let Some(v) = context.get("@propagate") {
            if env.mode == ProcessingMode::JsonLd10 {
                return Err(JsonLdError::new(E::InvalidContextEntry));
            }
            if as_bool(v).is_none() {
                return Err(JsonLdError::new(E::InvalidPropagateValue));
            }
        }

        // 5.12/5.13: the context-wide @protected flag, then a term definition per key.
        let protected = match context.get("@protected") {
            Some(v) => as_bool(v).ok_or_else(|| JsonLdError::new(E::InvalidProtectedValue))?,
            None => false,
        };
        let mut defined: BTreeMap<String, bool> = BTreeMap::new();
        if let Json::Obj(members) = context {
            for (key, _) in members {
                if is_context_keyword(key) {
                    continue;
                }
                create_term_definition(
                    &mut result,
                    context,
                    key,
                    &mut defined,
                    protected,
                    override_protected,
                    env,
                )?;
            }
        }
    }

    Ok(result)
}

/// The context-level keywords handled directly by Context Processing (not term keys).
fn is_context_keyword(key: &str) -> bool {
    matches!(
        key,
        "@base" | "@direction" | "@import" | "@language" | "@propagate" | "@protected" | "@version" | "@vocab"
    )
}

/// Merges an imported context under the local one: the imported entries provide defaults,
/// which the local context's entries override (`@import` itself is dropped).
fn merge_import(import_members: &[(String, Json)], context: &Json) -> Json {
    let mut merged = Json::Obj(import_members.to_vec());
    if let Json::Obj(members) = context {
        for (k, v) in members {
            if k == "@import" {
                continue;
            }
            merged.set(k, v.clone());
        }
    }
    merged
}

/// The **Create Term Definition** algorithm (JSON-LD 1.1 API §4.2.2). Builds (or validates)
/// the definition for `term` from `local`, storing it into `active` and recording progress
/// in `defined` (guarding against cyclic references).
pub(crate) fn create_term_definition(
    active: &mut ActiveContext,
    local: &Json,
    term: &str,
    defined: &mut BTreeMap<String, bool>,
    protected: bool,
    override_protected: bool,
    env: &mut Env,
) -> Result<(), JsonLdError> {
    // step 1: already built, or a cycle.
    match defined.get(term) {
        Some(true) => return Ok(()),
        Some(false) => return Err(JsonLdError::new(E::CyclicIriMapping)),
        None => {}
    }
    // step 2: an empty term is invalid; mark this term in-progress.
    if term.is_empty() {
        return Err(JsonLdError::new(E::InvalidTermDefinition));
    }
    defined.insert(term.to_string(), false);

    // step 3: the raw value for this term (a missing key defaults to null).
    let raw = local.get(term).cloned().unwrap_or_else(json_null);

    // steps 4/5: keyword handling.
    if term == "@type" {
        if env.mode == ProcessingMode::JsonLd10 {
            return Err(JsonLdError::new(E::KeywordRedefinition));
        }
        // Redefining @type is allowed only with @container: @set and/or @protected.
        let ok = match &raw {
            Json::Obj(members) => {
                !members.is_empty()
                    && members.iter().all(|(k, v)| {
                        (k == "@container" && matches!(v, Json::Str(s) if s == "@set"))
                            || k == "@protected"
                    })
            }
            _ => false,
        };
        if !ok {
            return Err(JsonLdError::new(E::KeywordRedefinition));
        }
    } else if is_keyword(term) {
        return Err(JsonLdError::new(E::KeywordRedefinition));
    } else if has_keyword_form(term) {
        // Keyword-shaped but unrecognised: ignore (no definition created).
        defined.insert(term.to_string(), true);
        return Ok(());
    }

    // step 6: remove and remember any existing definition (for the protected check).
    let previous_definition = active.term_definitions_mut().remove(term);

    // steps 7–9: normalise the value to a map and record whether it was a simple term.
    let (value, simple_term) = match &raw {
        Json::Str(s) => (
            Json::Obj(vec![("@id".to_string(), Json::Str(s.clone()))]),
            true,
        ),
        Json::Obj(_) => (raw.clone(), false),
        j if is_null(j) => (Json::Obj(vec![("@id".to_string(), json_null())]), false),
        _ => return Err(JsonLdError::new(E::InvalidTermDefinition)),
    };

    // step 10/11: seed the definition and apply @protected.
    let mut def = TermDefinition {
        protected,
        ..Default::default()
    };
    if let Some(p) = value.get("@protected") {
        if env.mode == ProcessingMode::JsonLd10 {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        }
        def.protected = as_bool(p).ok_or_else(|| JsonLdError::new(E::InvalidProtectedValue))?;
    }

    // step 12: @type mapping.
    if let Some(t) = value.get("@type") {
        let Json::Str(ts) = t else {
            return Err(JsonLdError::new(E::InvalidTypeMapping));
        };
        let ts = ts.clone();
        let expanded = expand_iri_in_context(active, &ts, false, true, local, defined, env)?
            .ok_or_else(|| JsonLdError::new(E::InvalidTypeMapping))?;
        let valid = match expanded.as_str() {
            "@json" | "@none" => env.mode != ProcessingMode::JsonLd10,
            "@id" | "@vocab" => true,
            other => is_absolute_iri(other),
        };
        if !valid {
            return Err(JsonLdError::new(E::InvalidTypeMapping));
        }
        def.type_mapping = Some(expanded);
    }

    // step 13: @reverse — a self-contained branch that stores and returns.
    if let Some(rev) = value.get("@reverse") {
        return create_reverse_definition(active, local, term, defined, env, &value, rev.clone(), def);
    }
    def.reverse = false;

    // step 14: derive the IRI mapping.
    let id_entry = value.get("@id").cloned();
    let id_differs = match &id_entry {
        Some(Json::Str(s)) => s != term,
        Some(_) => true, // null (or another non-string) differs from the term
        None => false,
    };
    if let Some(idj) = id_entry.filter(|_| id_differs) {
        if is_null(&idj) {
            // @id: null — a retained definition with a null IRI mapping (falls through to
            // the common tail so container/index/etc. still apply, then is stored).
            def.iri = None;
        } else {
            let Json::Str(ids) = &idj else {
                return Err(JsonLdError::new(E::InvalidIriMapping));
            };
            let ids = ids.clone();
            if ids != "@none" && !is_keyword(&ids) && has_keyword_form(&ids) {
                // @id looks like a keyword but is not one: ignore this term.
                defined.insert(term.to_string(), true);
                return Ok(());
            }
            let expanded = expand_iri_in_context(active, &ids, false, true, local, defined, env)?
                .ok_or_else(|| JsonLdError::new(E::InvalidIriMapping))?;
            if expanded == "@context" {
                return Err(JsonLdError::new(E::InvalidKeywordAlias));
            }
            if !(is_keyword(&expanded) || is_absolute_iri(&expanded) || is_blank_node(&expanded)) {
                return Err(JsonLdError::new(E::InvalidIriMapping));
            }
            def.iri = Some(expanded.clone());
            // Round-trip check for compact / relative-IRI terms (JSON-LD 1.1 §4.2.2 step
            // 14.3.3).  This check is ABSENT from the JSON-LD 1.0 algorithm — it was added
            // in 1.1 to disallow inconsistent compact-IRI redefinitions.  In JSON-LD 1.0
            // mode the check is skipped (W3C expand/0026 maps rdf:type→@type and
            // expand/0071 remaps v:term→v:somethingElse — both VALID in 1.0).
            // [OPUS-5] sq-gzsky — the check is NOT skipped when the IRI mapping is a
            // keyword. That the round-trip predicate cannot hold for a keyword is exactly
            // what §4.2.2 step 14.3.3 is FOR in 1.1: an IRI-shaped term may not be
            // remapped onto a keyword. W3C expand/er43 (`rdf:type` → `@type`, 1.1) pins the
            // error; its 1.0 twin expand/0026 is a POSITIVE and stays passing through the
            // `ProcessingMode::JsonLd10` gate below, which is the 1.0/1.1 split the spec
            // actually draws (the check is absent from the JSON-LD 1.0 algorithm — it was
            // added in 1.1 to disallow inconsistent compact-IRI redefinitions; expand/0071
            // remaps `v:term`→`v:somethingElse`, also VALID in 1.0).
            // [FABLE-5] (sq-oy1f.27) The round-trip check applies to a term containing
            // a colon "anywhere but as the first or last character" (§4.2.2 step
            // 14.3.3) — a term ENDING in a colon (the `"prefix:"` declaration form,
            // W3C compact/p003-p004) is a regular term, not a compact-IRI form.
            if env.mode != ProcessingMode::JsonLd10
                && (term.find(':').is_some_and(|i| i > 0 && i < term.len() - 1)
                    || term.contains('/'))
            {
                defined.insert(term.to_string(), true);
                let rt = expand_iri_in_context(active, term, false, true, local, defined, env)?;
                if rt.as_deref() != Some(expanded.as_str()) {
                    return Err(JsonLdError::new(E::InvalidIriMapping));
                }
            }
            // Simple-term prefix flag (§4.2.2 step 23): only a SIMPLE (string-valued)
            // term whose IRI ends in a gen-delim is a prefix — deliberately in BOTH
            // processing modes, matching jsonld.js and pyld (W3C compact/p001 pins the
            // 1.0-mode half: an expanded term definition is not a prefix in 1.0 either;
            // the one suite case that presumes 1.0-era prefixing, compact/0038, is an
            // honest FAIL for REC-conformant processors — see the compact floor doc).
            if simple_term
                && term.find(':').is_none()
                && !term.contains('/')
                && (ends_with_gen_delim(&expanded) || is_blank_node(&expanded))
            {
                def.prefix = true;
            }
        }
    } else {
        derive_iri(active, local, term, defined, env, &mut def)?;
    }

    finish_definition(
        active,
        local,
        term,
        defined,
        override_protected,
        env,
        &value,
        def,
        previous_definition,
    )
}

/// Derives the IRI mapping for a term with no distinct `@id` (§4.2.2 steps 14.3–14.6):
/// a compact IRI (`prefix:suffix`), a relative-IRI term (contains `/`), the `@type`
/// keyword, or a term expanded against `@vocab`.
fn derive_iri(
    active: &mut ActiveContext,
    local: &Json,
    term: &str,
    defined: &mut BTreeMap<String, bool>,
    env: &mut Env,
    def: &mut TermDefinition,
) -> Result<(), JsonLdError> {
    if term.find(':').is_some_and(|i| i > 0) {
        let colon = term.find(':').unwrap();
        let (prefix, suffix) = (&term[..colon], &term[colon + 1..]);
        if local.get(prefix).is_some() {
            create_term_definition(active, local, prefix, defined, false, false, env)?;
        }
        if let Some(pdef) = active.term_definitions.get(prefix) {
            if let Some(piri) = &pdef.iri {
                def.iri = Some(format!("{}{}", piri, suffix));
                return Ok(());
            }
        }
        // The term is already an absolute IRI.
        def.iri = Some(term.to_string());
        Ok(())
    } else if term.contains('/') {
        match active.expand_iri_readonly(term, false, true) {
            Some(e) if is_absolute_iri(&e) => {
                def.iri = Some(e);
                Ok(())
            }
            _ => Err(JsonLdError::new(E::InvalidIriMapping)),
        }
    } else if term == "@type" {
        def.iri = Some("@type".to_string());
        Ok(())
    } else if let Some(vocab) = &active.vocabulary_mapping {
        def.iri = Some(format!("{}{}", vocab, term));
        Ok(())
    } else {
        Err(JsonLdError::new(E::InvalidIriMapping))
    }
}

/// Builds and stores a reverse-property definition (§4.2.2 step 13).
#[allow(clippy::too_many_arguments)]
fn create_reverse_definition(
    active: &mut ActiveContext,
    local: &Json,
    term: &str,
    defined: &mut BTreeMap<String, bool>,
    env: &mut Env,
    value: &Json,
    rev: Json,
    mut def: TermDefinition,
) -> Result<(), JsonLdError> {
    if value.get("@id").is_some() || value.get("@nest").is_some() {
        return Err(JsonLdError::new(E::InvalidReverseProperty));
    }
    let Json::Str(rs) = &rev else {
        return Err(JsonLdError::new(E::InvalidIriMapping));
    };
    if has_keyword_form(rs) && !is_keyword(rs) {
        defined.insert(term.to_string(), true);
        return Ok(());
    }
    let rs = rs.clone();
    match expand_iri_in_context(active, &rs, false, true, local, defined, env)? {
        Some(e) if is_absolute_iri(&e) || is_blank_node(&e) => def.iri = Some(e),
        _ => return Err(JsonLdError::new(E::InvalidIriMapping)),
    }
    if let Some(c) = value.get("@container") {
        def.container = match c {
            Json::Str(s) if s == "@set" || s == "@index" => vec![s.clone()],
            j if is_null(j) => vec![],
            _ => return Err(JsonLdError::new(E::InvalidReverseProperty)),
        };
    }
    // [SONNET-4.6] sq-oy1f.45 — §4.2.2 step 18: `@index` applies to `@container @index`
    // terms including reverse properties (W3C expand/0131: a `@reverse` + `@container @index`
    // + `@index "predicate"` combination).  The non-reverse path processes `@index` inside
    // `finish_definition`, but `create_reverse_definition` returns early before reaching
    // `finish_definition`.  Apply it here so the property-valued-index key is stored.
    if let Some(idx) = value.get("@index") {
        if env.mode == ProcessingMode::JsonLd10 || !def.container.iter().any(|m| m == "@index") {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        }
        let Json::Str(is) = idx else {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        };
        let is = is.clone();
        match expand_iri_in_context(active, &is, false, true, local, defined, env)? {
            Some(e) if is_absolute_iri(&e) => {}
            _ => return Err(JsonLdError::new(E::InvalidTermDefinition)),
        }
        def.index = Some(is);
    }
    def.reverse = true;
    active.term_definitions_mut().insert(term.to_string(), def);
    defined.insert(term.to_string(), true);
    Ok(())
}

/// Applies the remaining term-definition entries (@container, @index, @context, @language,
/// @direction, @nest, @prefix), validates the key set, enforces the protected-redefinition
/// rule, and stores the definition (§4.2.2 steps 17–29).
#[allow(clippy::too_many_arguments)]
fn finish_definition(
    active: &mut ActiveContext,
    local: &Json,
    term: &str,
    defined: &mut BTreeMap<String, bool>,
    override_protected: bool,
    env: &mut Env,
    value: &Json,
    mut def: TermDefinition,
    previous_definition: Option<TermDefinition>,
) -> Result<(), JsonLdError> {
    // @container.
    if let Some(c) = value.get("@container") {
        let members = normalize_container(c).ok_or_else(|| JsonLdError::new(E::InvalidContainerMapping))?;
        if !valid_container(&members, env.mode, matches!(c, Json::Str(_))) {
            return Err(JsonLdError::new(E::InvalidContainerMapping));
        }
        def.container = members;
        if def.container.iter().any(|m| m == "@type") {
            match def.type_mapping.as_deref() {
                None => def.type_mapping = Some("@id".to_string()),
                Some("@id") | Some("@vocab") => {}
                Some(_) => return Err(JsonLdError::new(E::InvalidTypeMapping)),
            }
        }
    }

    // @index.
    if let Some(idx) = value.get("@index") {
        if env.mode == ProcessingMode::JsonLd10 || !def.container.iter().any(|m| m == "@index") {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        }
        let Json::Str(is) = idx else {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        };
        let is = is.clone();
        match expand_iri_in_context(active, &is, false, true, local, defined, env)? {
            Some(e) if is_absolute_iri(&e) => {}
            _ => return Err(JsonLdError::new(E::InvalidTermDefinition)),
        }
        def.index = Some(is);
    }

    // @context (a scoped context) — validated now, re-processed on use.
    if let Some(ctx) = value.get("@context") {
        if env.mode == ProcessingMode::JsonLd10 {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        }
        let saved_validate = env.validate_scoped;
        env.validate_scoped = false;
        let validated = process_inner(active, ctx, true, true, env);
        env.validate_scoped = saved_validate;
        validated.map_err(|_| JsonLdError::new(E::InvalidScopedContext))?;
        def.context = Some(ctx.clone());
        def.base_url = env.base_url.clone();
    }

    // @language / @direction only apply when there is no type mapping.
    let has_type = value.get("@type").is_some();
    if let Some(l) = value.get("@language") {
        if !has_type {
            def.language = if is_null(l) {
                Override::Null
            } else if let Json::Str(s) = l {
                Override::Set(s.clone())
            } else {
                return Err(JsonLdError::new(E::InvalidLanguageMapping));
            };
        }
    }
    if let Some(d) = value.get("@direction") {
        if !has_type {
            def.direction = if is_null(d) {
                Override::Null
            } else if let Json::Str(s) = d {
                Override::Set(Direction::parse(s).ok_or_else(|| JsonLdError::new(E::InvalidBaseDirection))?)
            } else {
                return Err(JsonLdError::new(E::InvalidBaseDirection));
            };
        }
    }

    // @nest.
    if let Some(n) = value.get("@nest") {
        if env.mode == ProcessingMode::JsonLd10 {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        }
        let Json::Str(ns) = n else {
            return Err(JsonLdError::new(E::InvalidNestValue));
        };
        // [OPUS-4.8] (sq-oy1f.24) Per JSON-LD 1.1 Create Term Definition, an `@nest` value
        // must not be a keyword *or have keyword form* other than `@nest` itself — a token
        // like `@bogus` (keyword-form but unrecognised) is still an invalid @nest value.
        if ns != "@nest" && has_keyword_form(ns) {
            return Err(JsonLdError::new(E::InvalidNestValue));
        }
        def.nest = Some(ns.clone());
    }

    // @prefix.
    if let Some(p) = value.get("@prefix") {
        if env.mode == ProcessingMode::JsonLd10 || term.contains(':') || term.contains('/') {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        }
        def.prefix = as_bool(p).ok_or_else(|| JsonLdError::new(E::InvalidPrefixValue))?;
        if def.prefix && def.iri.as_deref().is_some_and(is_keyword) {
            return Err(JsonLdError::new(E::InvalidTermDefinition));
        }
    }

    // step 27: reject unknown term-definition keys.
    const ALLOWED: &[&str] = &[
        "@id",
        "@reverse",
        "@container",
        "@context",
        "@direction",
        "@index",
        "@language",
        "@nest",
        "@prefix",
        "@protected",
        "@type",
    ];
    if let Json::Obj(members) = value {
        for (k, _) in members {
            if !ALLOWED.contains(&k.as_str()) {
                return Err(JsonLdError::new(E::InvalidTermDefinition));
            }
        }
    }

    // step 28: protected-term redefinition guard.
    if !override_protected {
        if let Some(prev) = &previous_definition {
            if prev.is_protected() {
                if !def.same_except_protected(prev) {
                    return Err(JsonLdError::new(E::ProtectedTermRedefinition));
                }
                // Identical up to @protected: keep the protected definition.
                def = prev.clone();
            }
        }
    }

    active.term_definitions_mut().insert(term.to_string(), def);
    defined.insert(term.to_string(), true);
    Ok(())
}

/// Normalises an `@container` value (a string or array of strings) to a list of keywords,
/// or `None` if it is not a string / string-array.
fn normalize_container(c: &Json) -> Option<Vec<String>> {
    match c {
        Json::Str(s) => Some(vec![s.clone()]),
        Json::Arr(items) => {
            let mut v = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    Json::Str(s) => v.push(s.clone()),
                    _ => return None,
                }
            }
            Some(v)
        }
        _ => None,
    }
}

/// Validates an `@container` mapping against the allowed keyword combinations (§4.2.2
/// container-mapping rules), honouring the JSON-LD 1.0 restriction to a single
/// `@list`/`@set`/`@index`.
///
/// `raw_is_string` reports whether the `@container` entry was written as a bare string
/// rather than an array. [OPUS-5] sq-gzsky — §4.2.2 step 21.2 rejects a container value
/// that "is @graph, @id, or @type, **or is otherwise not a string**" in `json-ld-1.0`
/// processing mode, so `"@container": ["@set"]` is an `invalid container mapping` in 1.0
/// even though the single member `@set` is itself 1.0-legal (W3C expand/es01,
/// compact/ep12). Normalising to a member list alone loses that distinction.
fn valid_container(members: &[String], mode: ProcessingMode, raw_is_string: bool) -> bool {
    let set: BTreeSet<&str> = members.iter().map(String::as_str).collect();
    if set.len() != members.len() {
        return false; // duplicates
    }
    if mode == ProcessingMode::JsonLd10 {
        return raw_is_string
            && members.len() == 1
            && matches!(members[0].as_str(), "@list" | "@set" | "@index");
    }
    if members.len() == 1 {
        return matches!(
            members[0].as_str(),
            "@graph" | "@id" | "@index" | "@language" | "@list" | "@set" | "@type"
        );
    }
    if set.contains("@list") {
        return false; // @list never combines
    }
    if set.contains("@graph") {
        let extra: BTreeSet<&str> = set.iter().filter(|m| **m != "@graph").copied().collect();
        if !extra.iter().all(|m| matches!(*m, "@id" | "@index" | "@set")) {
            return false;
        }
        return !(extra.contains("@id") && extra.contains("@index"));
    }
    if set.contains("@set") {
        let extra: BTreeSet<&str> = set.iter().filter(|m| **m != "@set").copied().collect();
        return extra.len() == 1
            && extra
                .iter()
                .all(|m| matches!(*m, "@index" | "@id" | "@type" | "@language"));
    }
    false
}

/// True iff `s` ends with an RFC 3986 gen-delim character (`:` `/` `?` `#` `[` `]` `@`).
fn ends_with_gen_delim(s: &str) -> bool {
    s.ends_with([':', '/', '?', '#', '[', ']', '@'])
}

/// The `Json` representation of a JSON `null`.
fn json_null() -> Json {
    Json::Raw("null".to_string())
}

/// True iff `j` is a JSON `null`.
fn is_null(j: &Json) -> bool {
    matches!(j, Json::Raw(r) if r == "null")
}

/// Interprets a JSON boolean scalar, or `None` for any non-boolean value.
fn as_bool(j: &Json) -> Option<bool> {
    match j {
        Json::Raw(r) if r == "true" => Some(true),
        Json::Raw(r) if r == "false" => Some(false),
        _ => None,
    }
}
