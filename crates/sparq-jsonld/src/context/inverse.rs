//! Inverse Context Creation (JSON-LD 1.1 API §4.3), IRI Compaction (§7.1),
//! and Term Selection (§7.2).
//!
//! [SONNET-4.6] (sq-90mu3) The compaction-side companions of IRI Expansion
//! (`context::iri`, bead `sq-oy1f.24`). These are pure functions consumed by the
//! document Compaction Algorithm (§8, bead `sq-oy1f.27`). Build the
//! `InverseContext` once per compaction call with
//! `ActiveContext::inverse_context`, then pass a reference into
//! `compact_iri` for each IRI that needs compacting.
//!
//! The three algorithms implemented here are:
//!
//! - **§4.3** Inverse Context Creation — maps every IRI in the context back to
//!   the best term for each (container, type/language) slot, with tie-breaking
//!   by shortest term then lexicographic order.
//! - **§7.1** IRI Compaction — keyword aliases, inverse-context term lookup,
//!   vocab-relative suffixes, compact IRIs (`prefix:suffix`), and
//!   base-relative paths.
//! - **§7.2** Term Selection — the container × preferred-value walk over the
//!   inverse context, factored out so the document compaction algorithm can
//!   call it without re-entering `compact_iri`.
//!
//! Spec: <https://www.w3.org/TR/json-ld11-api/#context-processing-algorithms>

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::{has_keyword_form, is_keyword, ActiveContext, Direction, Override, TermDefinition};
use super::iri::relativize_iri;
use crate::fx::FxMap;
use crate::json::Json;

// ---------------------------------------------------------------------------
// §4.3 Inverse Context Creation
// ---------------------------------------------------------------------------

/// Per-container type/language map — the leaf node of the inverse context
/// tree (JSON-LD 1.1 API §4.3).
///
/// `language` is keyed by language-direction tag (e.g. `"en"`, `"en_ltr"`,
/// `"@null"`, `"@none"`). `type_` is keyed by type IRI or `"@reverse"` /
/// `"@none"` / `"@any"`. `any` holds the unconstrained fallback term under
/// `"@none"`.
#[derive(Clone, Debug, Default)]
struct TypeLangMap {
    /// `@language` sub-map: language tag (and direction suffix) to term.
    language: BTreeMap<String, String>,
    /// `@type` sub-map: type IRI / `@reverse` / `@none` / `@any` to term.
    type_: BTreeMap<String, String>,
    /// `@any` sub-map: `"@none"` to an unconstrained catch-all term.
    any: BTreeMap<String, String>,
}

/// The **inverse context** (JSON-LD 1.1 API §4.3): a multi-level map from
/// IRI → container key → `TypeLangMap`.
///
/// Build one from an `ActiveContext` via `ActiveContext::inverse_context`.
/// Pass a shared reference into `compact_iri` for each IRI that must be
/// compacted during a single compaction pass.
#[derive(Clone, Debug, Default)]
pub struct InverseContext {
    /// iri → container-key → type-language map.
    inner: FxMap<String, BTreeMap<String, TypeLangMap>>,
    /// `@prefix`-flagged `(term, iri)` pairs in (length, lexicographic) term order,
    /// precomputed for §7.1 step 5 so IRI Compaction does not re-sort them per call.
    prefixes: Vec<(String, String)>,
    /// keyword → its winning alias term (shortest, then lexicographic least).
    aliases: BTreeMap<String, String>,
    /// The default-language preferred value (`"@none"` when unset), see
    /// `compute_default_language`.
    default_language: String,
}

impl InverseContext {
    /// True iff some term of the context maps to `iri`.
    pub(crate) fn has_iri(&self, iri: &str) -> bool {
        self.inner.contains_key(iri)
    }
}

impl ActiveContext {
    /// Builds the **inverse context** from `self` (JSON-LD 1.1 API §4.3).
    ///
    /// The inverse context maps every IRI in the active context back to the
    /// "best" term for each `(container, type/language)` combination. When two
    /// terms compete for the same slot the shortest term wins; ties are broken
    /// lexicographically (the spec mandates this iteration order at §4.3 step
    /// 3).
    ///
    /// [SONNET-4.6] (sq-90mu3)
    pub fn inverse_context(&self) -> InverseContext {
        // §4.3 step 1: initialise result.
        let mut result: FxMap<String, BTreeMap<String, TypeLangMap>> = FxMap::default();

        // §4.3 steps 2–3: default_language is the active context's default
        // language (lower-cased), optionally suffixed with "_" + direction.
        // Falls back to "@none" when unset.
        let default_language: String = match &self.default_language {
            None => "@none".to_string(),
            Some(lang) => {
                let lang_lc = lang.to_lowercase();
                match self.default_base_direction {
                    Some(dir) => format!("{}_{}", lang_lc, dir.as_str()),
                    None => lang_lc,
                }
            }
        };

        // §4.3 step 4: iterate term definitions ordered by shortest term
        // length first, then lexicographically least (tie-breaking).  Because
        // each slot is first-write-wins this order guarantees the spec's
        // "shortest, then lexicographic" preference without any post-pass.
        let mut terms: Vec<(&str, &TermDefinition)> = self
            .term_definitions
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        terms.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then(a.cmp(b)));

        for &(term, term_def) in &terms {
            // §4.3 step 4.1: skip terms with a null IRI mapping.
            let iri = match &term_def.iri {
                Some(i) => i.clone(),
                None => continue,
            };

            // §4.3 step 4.2: compute the container key — sort the container
            // keywords and join them; use "@none" for an empty container.
            let container: String = if term_def.container.is_empty() {
                "@none".to_string()
            } else {
                let mut sorted = term_def.container.clone();
                sorted.sort();
                sorted.join("")
            };

            // §4.3 steps 4.3–4.4: ensure the result entries exist.
            let container_map = result.entry(iri.clone()).or_default();
            let tl_map = container_map
                .entry(container)
                .or_default();

            // §4.3 step 4.5 ("@any"): record this term as an unconstrained
            // fallback — first-write-wins (shortest/lexicographic guarantee
            // comes from the sort above).
            tl_map.any.entry("@none".to_string()).or_insert_with(|| term.to_string());

            if term_def.reverse {
                // §4.3 step 4.6: reverse property — store in type map under
                // "@reverse".
                tl_map
                    .type_
                    .entry("@reverse".to_string())
                    .or_insert_with(|| term.to_string());
            } else if term_def.type_mapping.as_deref() == Some("@none") {
                // §4.3 step 4.7: type mapping "@none" — this term accepts any
                // type or language; mark it with "@any" in both maps.
                tl_map
                    .language
                    .entry("@any".to_string())
                    .or_insert_with(|| term.to_string());
                tl_map
                    .type_
                    .entry("@any".to_string())
                    .or_insert_with(|| term.to_string());
            } else if let Some(type_mapping) = &term_def.type_mapping {
                // §4.3 step 4.8: concrete type mapping — register under that
                // type IRI.
                tl_map
                    .type_
                    .entry(type_mapping.clone())
                    .or_insert_with(|| term.to_string());
            } else {
                // §4.3 step 4.9: no type mapping — use language/direction.
                let lang_key = language_dir_key(&term_def.language, &term_def.direction);
                match lang_key {
                    Some(key) => {
                        // An explicit language and/or direction was given.
                        tl_map
                            .language
                            .entry(key)
                            .or_insert_with(|| term.to_string());
                    }
                    None => {
                        // §4.3 step 4.9.x: no explicit language or direction —
                        // fall back to the context default(s), then "@none".
                        if self.default_language.is_some() || self.default_base_direction.is_some() {
                            tl_map
                                .language
                                .entry(default_language.clone())
                                .or_insert_with(|| term.to_string());
                        }
                        tl_map
                            .language
                            .entry("@none".to_string())
                            .or_insert_with(|| term.to_string());
                        tl_map
                            .type_
                            .entry("@none".to_string())
                            .or_insert_with(|| term.to_string());
                    }
                }
            }
        }

        let mut prefixes = Vec::new();
        let mut aliases: BTreeMap<String, String> = BTreeMap::new();
        for (term, def) in &terms {
            if let Some(iri) = def.iri.as_deref() {
                if def.prefix {
                    prefixes.push((term.to_string(), iri.to_string()));
                }
                if is_keyword(iri) {
                    // `terms` is in winning order, so the first alias seen wins.
                    aliases.entry(iri.to_string()).or_insert_with(|| term.to_string());
                }
            }
        }

        InverseContext { inner: result, prefixes, aliases, default_language }
    }
}

/// Computes the language-direction key for the inverse context `@language`
/// sub-map from a term definition's language and direction overrides.
///
/// Returns `None` when both overrides are `Unset` (the "no explicit
/// language/direction" case, handled separately).
///
/// Key format (JSON-LD 1.1 API "Inverse Context Creation", the language-map
/// entry rules — [FABLE-5] sq-oy1f.27: re-keyed to the REC's exact formats so
/// the keys line up with the preferred values IRI Compaction derives from
/// document values, including the `"_<dir>"` underscore-suffix rule):
/// - `Set(lang)` + `Set(dir)` → `"<lang-lc>_<dir>"`
/// - `Set(lang)` + `Null`/`Unset` → `"<lang-lc>"`
/// - `Null`      + `Set(dir)` → `"_<dir>"`
/// - `Null`      + `Null`/`Unset` → `"@null"`
/// - `Unset`     + `Set(dir)` → `"_<dir>"` (the direction-only rule)
/// - `Unset`     + `Null`     → `"@none"` (the direction-only rule, null)
/// - `Unset`     + `Unset`    → `None` (caller handles defaults)
fn language_dir_key(
    language: &Override<String>,
    direction: &Override<Direction>,
) -> Option<String> {
    match (language, direction) {
        (Override::Set(lang), Override::Set(dir)) => {
            Some(format!("{}_{}", lang.to_lowercase(), dir.as_str()))
        }
        (Override::Set(lang), _) => Some(lang.to_lowercase()),
        (Override::Null, Override::Set(dir)) => Some(format!("_{}", dir.as_str())),
        (Override::Null, _) => Some("@null".to_string()),
        (Override::Unset, Override::Set(dir)) => Some(format!("_{}", dir.as_str())),
        (Override::Unset, Override::Null) => Some("@none".to_string()),
        (Override::Unset, Override::Unset) => None,
    }
}

// ---------------------------------------------------------------------------
// §7.2 Term Selection
// ---------------------------------------------------------------------------

/// **Term Selection** (JSON-LD 1.1 API §7.2).
///
/// Walks `inverse` for `iri`, trying each `container` × `preferred_value`
/// pair in order (containers are tried outermost). Returns the first matching
/// term, or `None` if nothing matched.
///
/// `type_language` is one of `"@type"`, `"@language"`, `"@any"`, or
/// `"@reverse"` (the last two are aliases over the respective sub-maps).
///
/// [SONNET-4.6] (sq-90mu3)
pub(crate) fn select_term(
    inverse: &InverseContext,
    iri: &str,
    containers: &[&str],
    type_language: &str,
    preferred_values: &[&str],
) -> Option<String> {
    let container_map = inverse.inner.get(iri)?;
    // §7.2 step 3: iterate containers, then preferred values within each.
    for container in containers {
        let tl_map = match container_map.get(*container) {
            Some(m) => m,
            None => continue,
        };
        // §7.2 step 3.5: choose the sub-map by type_language.
        let value_map: &BTreeMap<String, String> = match type_language {
            "@type" | "@reverse" => &tl_map.type_,
            "@language" => &tl_map.language,
            _ => &tl_map.any, // "@any" and anything else falls through to any
        };
        // §7.2 step 3.6: check each preferred value in order.
        for pv in preferred_values {
            if let Some(term) = value_map.get(*pv) {
                return Some(term.clone());
            }
        }
    }
    // §7.2 step 4: no match.
    None
}

// ---------------------------------------------------------------------------
// §7.1 IRI Compaction
// ---------------------------------------------------------------------------

/// True iff `j` has the **graph object** shape: a map with `@graph` whose other
/// entries are at most `@id`, `@index`, and `@context`. Used by the container-
/// preference derivation (a graph object prefers `@graph*` containers, and its
/// `@index` names the graph rather than an index entry).
///
/// [FABLE-5] (sq-oy1f.27)
fn is_graph_object_shape(j: &Json) -> bool {
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

/// **IRI Compaction** (JSON-LD 1.1 API §7.1).
///
/// Compacts `iri` (or a keyword) to the shortest available representation in
/// the active context, consulting the pre-built `inverse` context:
///
/// 1. Keyword alias lookup (step 2).
/// 2. Inverse-context term selection — preferred container × type/language
///    lookup, with containers and preferred values derived from `value`
///    (step 3).
/// 3. Vocab-relative suffix — strip the `@vocab` prefix when the suffix is
///    unambiguous (step 4).
/// 4. Compact IRI — `prefix:suffix` via `@prefix`-flagged terms (step 5).
/// 5. Base-relative path when `vocab` is false (step 6).
/// 6. Return `iri` unchanged (step 7).
///
/// `value` is the JSON value being compacted for (a node-object or value
/// object); it governs which containers and type/language keys are preferred.
/// Pass `None` when compacting a bare IRI (e.g. a `@type` value).
/// `vocab` indicates vocab-relative compaction; `reverse` marks a
/// `@reverse` property.
///
/// [SONNET-4.6] (sq-90mu3)
pub fn compact_iri(
    ctx: &ActiveContext,
    inverse: &InverseContext,
    iri: &str,
    value: Option<&Json>,
    vocab: bool,
    reverse: bool,
) -> String {
    // §7.1 step 1: empty / null — return as-is.
    if iri.is_empty() {
        return iri.to_string();
    }

    // §7.1 step 2: keyword or keyword alias (shortest, then lexicographic least).
    if is_keyword(iri) {
        return inverse.aliases.get(iri).cloned().unwrap_or_else(|| iri.to_string());
    }

    // §7.1 step 3 (spec "IRI Compaction" step 2): vocab=true AND iri appears in the
    // inverse context — derive the container preferences and the type/language lookup
    // from the SHAPE of `value`, then run Term Selection.
    //
    // [FABLE-5] (sq-oy1f.27) Rewritten spec-faithful for the document-level Compaction
    // Algorithm: the prior version pre-dated the document walk and diverged on the
    // load-bearing shapes the W3C compact suite exercises — @index containers were not
    // considered for reverse properties (the spec appends them BEFORE the reverse
    // branch), reverse selected the wrong sub-map (@type/@reverse, not a "@reverse"
    // sub-map), list objects ignored the common-type/common-language derivation across
    // their items, graph objects had no @graph* container preferences, and the 1.1
    // trailing @index/@language container fallbacks plus the @vocab-vs-@id preferred-
    // value ordering and the "_<direction>" preferred-value suffix rule were missing.
    if vocab && inverse.inner.contains_key(iri) {
        let default_language = inverse.default_language.as_str();

        // step 2.1: containers, tried in order by Term Selection.
        let mut containers: Vec<&str> = Vec::with_capacity(16);
        // steps 2.2-2.3: the sub-map selector and its preferred value.
        let mut type_language: &str = "@language";
        let mut type_language_value: Option<Cow<'_, str>> = None; // null → "@null" below

        // Unwrap @preserve to its first element (framing input).
        let value = value.and_then(|v| {
            if let Some(pres) = v.get("@preserve") {
                match pres {
                    Json::Arr(arr) => arr.first(),
                    other => Some(other),
                }
            } else {
                Some(v)
            }
        });

        let is_map = matches!(value, Some(Json::Obj(_)));
        let has_index = is_map && value.and_then(|v| v.get("@index")).is_some();
        let graph_object = value.map(is_graph_object_shape).unwrap_or(false);
        let list_object = is_map && value.and_then(|v| v.get("@list")).is_some();

        // step 2.4: an indexed (non-graph) value prefers @index containers — for
        // FORWARD and REVERSE properties alike (this runs before the reverse branch).
        if has_index && !graph_object {
            containers.push("@index");
            containers.push("@index@set");
        }

        if reverse {
            // step 2.5: reverse property — the @type sub-map under "@reverse".
            type_language = "@type";
            type_language_value = Some(Cow::Borrowed("@reverse"));
            containers.push("@set");
        } else if list_object {
            // step 2.6: list object — derive the most specific common type or
            // language across the list items.
            let v = value.expect("list_object implies value");
            if v.get("@index").is_none() {
                containers.push("@list");
            }
            let list: &[Json] = match v.get("@list") {
                Some(Json::Arr(a)) => a.as_slice(),
                _ => &[],
            };
            let mut common_type: Option<String> = None;
            let mut common_language: Option<String> = if list.is_empty() {
                Some(default_language.to_string())
            } else {
                None
            };
            for item in list {
                let mut item_language = "@none".to_string();
                let mut item_type = "@none".to_string();
                if item.get("@value").is_some() {
                    if let Some(Json::Str(dir)) = item.get("@direction") {
                        // Language+direction key, matching the inverse-context key
                        // convention (language_dir_key): "<lang>_<dir>" or "_<dir>".
                        item_language = match item.get("@language") {
                            Some(Json::Str(lang)) => {
                                format!("{}_{}", lang.to_lowercase(), dir.to_lowercase())
                            }
                            _ => format!("_{}", dir.to_lowercase()),
                        };
                    } else if let Some(Json::Str(lang)) = item.get("@language") {
                        item_language = lang.to_lowercase();
                    } else if let Some(Json::Str(t)) = item.get("@type") {
                        item_type = t.clone();
                    } else {
                        item_language = "@null".to_string();
                    }
                } else {
                    item_type = "@id".to_string();
                }
                match &common_language {
                    None => common_language = Some(item_language.clone()),
                    Some(cl) if *cl != item_language && item.get("@value").is_some() => {
                        common_language = Some("@none".to_string());
                    }
                    _ => {}
                }
                match &common_type {
                    None => common_type = Some(item_type.clone()),
                    Some(ct) if *ct != item_type => {
                        common_type = Some("@none".to_string());
                    }
                    _ => {}
                }
                if common_language.as_deref() == Some("@none")
                    && common_type.as_deref() == Some("@none")
                {
                    break; // no common language or type amongst the items
                }
            }
            let common_language = common_language.unwrap_or_else(|| "@none".to_string());
            let common_type = common_type.unwrap_or_else(|| "@none".to_string());
            if common_type != "@none" {
                type_language = "@type";
                type_language_value = Some(Cow::Owned(common_type));
            } else {
                type_language_value = Some(Cow::Owned(common_language));
            }
        } else if graph_object {
            // step 2.7: graph object — prefer the matching @graph* containers.
            let v = value.expect("graph_object implies value");
            if v.get("@index").is_some() {
                containers.push("@graph@index");
                containers.push("@graph@index@set");
            }
            if v.get("@id").is_some() {
                containers.push("@graph@id");
                containers.push("@graph@id@set");
            }
            containers.push("@graph");
            containers.push("@graph@set");
            containers.push("@set");
            if v.get("@index").is_none() {
                containers.push("@graph@index");
                containers.push("@graph@index@set");
            }
            if v.get("@id").is_none() {
                containers.push("@graph@id");
                containers.push("@graph@id@set");
            }
            containers.push("@index");
            containers.push("@index@set");
            type_language = "@type";
            type_language_value = Some(Cow::Borrowed("@id"));
        } else {
            // step 2.8: value objects match on language/direction/type; node
            // objects prefer @id/@type containers.
            let is_value_obj = is_map && value.and_then(|v| v.get("@value")).is_some();
            if is_value_obj {
                let v = value.expect("value object");
                if let Some(Json::Str(dir)) = v.get("@direction") {
                    if v.get("@index").is_none() {
                        type_language_value = Some(Cow::Owned(match v.get("@language") {
                            Some(Json::Str(lang)) => {
                                format!("{}_{}", lang.to_lowercase(), dir.to_lowercase())
                            }
                            _ => format!("_{}", dir.to_lowercase()),
                        }));
                        containers.push("@language");
                        containers.push("@language@set");
                    }
                } else if let Some(Json::Str(lang)) = v.get("@language") {
                    if v.get("@index").is_none() {
                        type_language_value = Some(Cow::Owned(lang.to_lowercase()));
                        containers.push("@language");
                        containers.push("@language@set");
                    }
                } else if let Some(Json::Str(t)) = v.get("@type") {
                    type_language = "@type";
                    type_language_value = Some(Cow::Borrowed(t.as_str()));
                }
            } else {
                type_language = "@type";
                type_language_value = Some(Cow::Borrowed("@id"));
                containers.push("@id");
                containers.push("@id@set");
                containers.push("@type");
                containers.push("@set@type");
            }
            containers.push("@set");
        }

        // step 2.9: the no-container fallback is always tried last.
        containers.push("@none");
        // steps 2.10-2.11 (JSON-LD 1.1): an un-indexed value may still live in an
        // @index container; a lone-@value map may still live in a @language container.
        if !is_map || !has_index {
            containers.push("@index");
            containers.push("@index@set");
        }
        // Only a STRING @value may live in a language map: expansion rejects any other
        // language-map value ("invalid language map value"), so a lone native number or
        // boolean must not be offered the @language containers.
        let lone_value_map = matches!(value, Some(Json::Obj(m)) if m.len() == 1)
            && matches!(value.and_then(|v| v.get("@value")), Some(Json::Str(_)));
        if lone_value_map {
            containers.push("@language");
            containers.push("@language@set");
        }

        // step 2.12: null values are stored under "@null" in the inverse context.
        let type_language_value = type_language_value.unwrap_or(Cow::Borrowed("@null"));

        // steps 2.13-2.16: preferred values, most specific first.
        let mut preferred_values: Vec<Cow<'_, str>> = Vec::with_capacity(6);
        if type_language_value == "@reverse" {
            preferred_values.push(Cow::Borrowed("@reverse"));
        }
        let id_entry = value.and_then(|v| v.get("@id")).and_then(Json::as_str);
        if let ("@id" | "@reverse", Some(id_iri)) = (type_language_value.as_ref(), id_entry) {
            // Prefer @vocab-coercing terms when the nested @id round-trips through a
            // term; otherwise prefer @id-coercing terms.
            // Only a term mapping to `id_iri` can round-trip, so skip the nested
            // compaction when there is none.
            let round_trips = inverse.has_iri(id_iri) && {
                let compacted_id = compact_iri(ctx, inverse, id_iri, None, true, false);
                ctx.term_definitions
                    .get(&compacted_id)
                    .map(|d| d.iri.as_deref() == Some(id_iri))
                    .unwrap_or(false)
            };
            if round_trips {
                preferred_values.push(Cow::Borrowed("@vocab"));
                preferred_values.push(Cow::Borrowed("@id"));
            } else {
                preferred_values.push(Cow::Borrowed("@id"));
                preferred_values.push(Cow::Borrowed("@vocab"));
            }
            preferred_values.push(Cow::Borrowed("@none"));
        } else {
            preferred_values.push(type_language_value.clone());
            preferred_values.push(Cow::Borrowed("@none"));
            // An empty list matches any term (its items constrain nothing).
            let empty_list = matches!(
                value.and_then(|v| v.get("@list")),
                Some(Json::Arr(a)) if a.is_empty()
            );
            if empty_list {
                type_language = "@any";
            }
        }
        preferred_values.push(Cow::Borrowed("@any"));
        // step 2.16: a "<lang>_<dir>" preferred value also tries its bare
        // "_<dir>" suffix (any-language, fixed-direction terms).
        let suffixes: Vec<Cow<'_, str>> = preferred_values
            .iter()
            .filter_map(|pv| pv.find('_').map(|i| Cow::Owned(pv[i..].to_string())))
            .collect();
        preferred_values.extend(suffixes);

        // step 2.17: Term Selection.
        let pvs: Vec<&str> = preferred_values.iter().map(|pv| pv.as_ref()).collect();
        if let Some(term) = select_term(inverse, iri, &containers, type_language, &pvs) {
            return term;
        }
    }

    // §7.1 step 4: vocab-relative suffix — if @vocab is a prefix of iri and
    // the suffix is either undefined or unambiguously maps back to this iri.
    if vocab {
        if let Some(vocab_iri) = ctx.vocabulary_mapping.as_deref() {
            if iri.starts_with(vocab_iri) && iri.len() > vocab_iri.len() {
                let suffix = &iri[vocab_iri.len()..];
                // Only a suffix with NO term definition (spec step 4.1). A defined term
                // reaching here was rejected by Term Selection for this value (e.g. a
                // `@type: @id` term offered a plain literal), so using it would change
                // the value's meaning on re-expansion.
                if !ctx.term_definitions.contains_key(suffix) {
                    return suffix.to_string();
                }
            }
        }
    }

    // §7.1 step 5: compact IRI via @prefix-flagged terms.
    // Try every prefix term, build "prefix:suffix", pick the shortest (then
    // lexicographic least) candidate that does not conflict with an existing
    // term definition.
    let mut best_compact: Option<String> = None;

    for (term, prefix_iri) in &inverse.prefixes {
        // Skip a prefix that IS the target (no suffix) or does not start it; the
        // suffix must not start with "//" (it would read as a URL authority).
        let Some(suffix) = iri.strip_prefix(prefix_iri.as_str()) else { continue };
        if suffix.is_empty() || suffix.starts_with("//") {
            continue;
        }
        // Keep the shortest candidate, ties broken lexicographically; skip the
        // allocation for one that is already longer than the best.
        let len = term.len() + 1 + suffix.len();
        if best_compact.as_ref().is_some_and(|b| len > b.len()) {
            continue;
        }
        let candidate = format!("{}:{}", term, suffix);
        if best_compact.as_ref().is_some_and(|b| len == b.len() && candidate >= *b) {
            continue;
        }

        // Guard: if this compact IRI already denotes a term, only use it when that
        // term maps to the same IRI AND there is no value (a value might imply
        // type/container constraints that break the match).
        let ok = match ctx.term_definitions.get(&candidate) {
            Some(d) => d.iri.as_deref() == Some(iri) && value.is_none(),
            None => true,
        };
        if ok {
            best_compact = Some(candidate);
        }
    }

    if let Some(compact) = best_compact {
        // §7.1 step 5.x: use the compact IRI if vocab is false, or if it
        // does not itself appear in the inverse context (which would mean it
        // was already resolved to a term above).
        if !vocab || !inverse.inner.contains_key(&compact) {
            return compact;
        }
    }

    // §7.1 step 6: base-relative IRI (vocab=false path). RFC 3986 §5.3-style
    // relative-reference generation via `relativize_iri` (the inverse of
    // `context::iri::resolve_iri`). This supersedes the prior
    // `iri.strip_prefix(base)` which only handled the literal-prefix case
    // and missed same-directory ("http://ex/a/b" + "http://ex/a/c" → "c"),
    // parent-traversal ("../c"), and query/fragment preservation.
    // [SONNET-4.6] sq-90mu3 defect fix.
    if !vocab {
        if let Some(base) = ctx.base_iri.as_deref() {
            if let Some(relative) = relativize_iri(base, iri) {
                // [FABLE-5] (sq-oy1f.27) A relative reference with keyword form
                // ("@" + ALPHA) would be misread as a keyword where an IRI is
                // expected — disambiguate with a "./" prefix (W3C compact/0111).
                if has_keyword_form(&relative) {
                    return format!("./{}", relative);
                }
                return relative;
            }
        }
    }

    // §7.1 step 7: nothing worked — return the IRI unchanged.
    iri.to_string()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::Json;
    use crate::loader::NoopLoader;
    use crate::options::JsonLdOptions;

    const BASE: &str = "http://example.org/";

    fn json(s: &str) -> Json {
        Json::parse(s).expect("valid JSON")
    }

    fn ctx_of(src: &str) -> ActiveContext {
        ActiveContext::new(Some(BASE))
            .process(&json(src), Some(BASE), &NoopLoader, &JsonLdOptions::default())
            .expect("context should process")
    }

    // -----------------------------------------------------------------------
    // §4.3 Inverse Context Creation — shape + tie-break tests
    // -----------------------------------------------------------------------

    /// Two terms mapping to the same IRI: the shorter one must win every slot.
    /// Derived from the §4.3 spec invariant: "ordered by shortest and then
    /// lexicographically least".
    #[test]
    fn inverse_ctx_shorter_term_wins_over_longer() {
        // "z" (1 char) beats "aa" (2 chars) for the same IRI.
        let ac = ctx_of(
            r#"{"z": "http://ex/foo", "aa": "http://ex/foo"}"#,
        );
        let inv = ac.inverse_context();
        // The "@any"/"@none" slot must be "z", not "aa".
        let iri_map = inv.inner.get("http://ex/foo").expect("iri present");
        let tl = iri_map.get("@none").expect("@none container present");
        assert_eq!(
            tl.any.get("@none").map(String::as_str),
            Some("z"),
            "shorter term 'z' must beat 'aa'"
        );
        assert_eq!(
            tl.language.get("@none").map(String::as_str),
            Some("z")
        );
        assert_eq!(
            tl.type_.get("@none").map(String::as_str),
            Some("z")
        );
    }

    /// Equal-length terms: lexicographically least must win.
    #[test]
    fn inverse_ctx_lexicographic_tie_break() {
        // "aa" < "ab" lexicographically.
        let ac = ctx_of(
            r#"{"ab": "http://ex/bar", "aa": "http://ex/bar"}"#,
        );
        let inv = ac.inverse_context();
        let tl = inv
            .inner
            .get("http://ex/bar")
            .unwrap()
            .get("@none")
            .unwrap();
        assert_eq!(
            tl.any.get("@none").map(String::as_str),
            Some("aa"),
            "lexicographic least 'aa' must beat 'ab'"
        );
    }

    /// Type-mapped term lands in the `@type` sub-map under its type IRI.
    #[test]
    fn inverse_ctx_type_mapped_term_in_type_submap() {
        let ac = ctx_of(
            r#"{"typed": {"@id": "http://ex/p", "@type": "http://ex/T"}}"#,
        );
        let inv = ac.inverse_context();
        let tl = inv
            .inner
            .get("http://ex/p")
            .unwrap()
            .get("@none")
            .unwrap();
        assert_eq!(
            tl.type_.get("http://ex/T").map(String::as_str),
            Some("typed")
        );
    }

    /// A `@language`-typed term lands in the `@language` sub-map under its
    /// lower-cased language tag.
    #[test]
    fn inverse_ctx_language_mapped_term_in_language_submap() {
        let ac = ctx_of(
            r#"{"label": {"@id": "http://ex/label", "@language": "en"}}"#,
        );
        let inv = ac.inverse_context();
        let tl = inv
            .inner
            .get("http://ex/label")
            .unwrap()
            .get("@none")
            .unwrap();
        assert_eq!(
            tl.language.get("en").map(String::as_str),
            Some("label")
        );
    }

    /// Container mapping produces the sorted-join key.
    /// Term with `@container: ["@language"]` → container key `"@language"`.
    #[test]
    fn inverse_ctx_container_key_sorted_join() {
        let ac = ctx_of(
            r#"{"labels": {"@id": "http://ex/labels", "@container": "@language"}}"#,
        );
        let inv = ac.inverse_context();
        // The inverse context should have a "@language" entry, not "@none".
        assert!(
            inv.inner
                .get("http://ex/labels")
                .unwrap()
                .contains_key("@language"),
            "term with @container @language must appear under the '@language' key"
        );
    }

    /// `@prefix`-flagged term with null IRI is skipped.
    #[test]
    fn inverse_ctx_null_iri_term_is_skipped() {
        let ac = ctx_of(r#"{"dropped": null}"#);
        let inv = ac.inverse_context();
        // A null-mapped term has no IRI: nothing should appear in the inverse.
        assert!(inv.inner.is_empty() || !inv.inner.values().any(|cm| {
            cm.values().any(|tl| tl.any.values().any(|t| t == "dropped"))
        }));
    }

    // -----------------------------------------------------------------------
    // §7.1 IRI Compaction — keyword alias, term lookup, compact IRI, vocab
    // -----------------------------------------------------------------------

    /// Keyword alias: a term aliasing `@type` compacts back to the alias.
    ///
    /// Fixture derivation: compact/0008 has `"uri": "@id"`, which means
    /// compact_iri("@id", vocab=true) → "uri".  The same shape holds for
    /// `@type`.
    #[test]
    fn compact_iri_keyword_alias() {
        // W3C compact/0008: {"uri": "@id"} — here we test the same shape
        // for "@type" to keep the fixture id range anchored.
        let ac = ctx_of(r#"{"rdftype": "@type"}"#);
        let inv = ac.inverse_context();
        let result = compact_iri(&ac, &inv, "@type", None, true, false);
        assert_eq!(result, "rdftype", "keyword alias should compact to the alias term");
    }

    /// Basic term lookup via inverse context.
    ///
    /// Oracle: W3C compact/0002 — context {"t1": "http://example.com/t1"}.
    /// Compacting `http://example.com/t1` with no value should yield `"t1"`.
    ///
    /// Fixture: tests/w3c/json-ld-api/tests/compact/0002-context.jsonld,
    ///          tests/w3c/json-ld-api/tests/compact/0002-out.jsonld
    ///          (`"@type": "t1"` in the compacted output).
    #[test]
    fn compact_iri_basic_term_lookup_w3c_0002() {
        let ac = ctx_of(
            r#"{"t1": "http://example.com/t1",
               "t2": "http://example.com/t2",
               "term1": "http://example.com/term1",
               "term2": "http://example.com/term2",
               "term3": "http://example.com/term3",
               "term4": "http://example.com/term4",
               "term5": "http://example.com/term5"}"#,
        );
        let inv = ac.inverse_context();

        // "t1" is the term for http://example.com/t1
        assert_eq!(
            compact_iri(&ac, &inv, "http://example.com/t1", None, true, false),
            "t1"
        );
        // "term1" is the term for http://example.com/term1
        assert_eq!(
            compact_iri(&ac, &inv, "http://example.com/term1", None, true, false),
            "term1"
        );
        // A @value literal with @language: "en" — term3 has no language
        // constraint, so it still matches via the "@none" fallback.
        let value = json(r#"{"@value": "v3", "@language": "en"}"#);
        assert_eq!(
            compact_iri(
                &ac,
                &inv,
                "http://example.com/term3",
                Some(&value),
                true,
                false
            ),
            "term3",
            "term with no language mapping should match via @none fallback"
        );
    }

    /// Compact IRI via `@prefix`-flagged term.
    ///
    /// Oracle: W3C compact/0005 — context {"ex": "http://example.org/"}.
    /// Compacting `http://example.org/id1` with vocab=false (no vocab match)
    /// should yield `"ex:id1"`.
    ///
    /// Fixture: tests/w3c/json-ld-api/tests/compact/0005-context.jsonld,
    ///          tests/w3c/json-ld-api/tests/compact/0005-out.jsonld
    ///          (`"@id": "ex:id1"` in the compacted output).
    #[test]
    fn compact_iri_prefix_compact_iri_w3c_0005() {
        let ac = ctx_of(
            r#"{"ex": "http://example.org/",
               "term1": {"@id": "ex:term1", "@type": "ex:datatype"},
               "term2": {"@id": "ex:term2", "@type": "@id"}}"#,
        );
        let inv = ac.inverse_context();

        // @id position: vocab=false, base-relative or compact-IRI.
        // "ex" is a prefix term (prefix=true when used as a prefix in term1/2).
        // compact_iri with vocab=false should produce "ex:id1".
        let result = compact_iri(&ac, &inv, "http://example.org/id1", None, false, false);
        assert_eq!(result, "ex:id1", "prefix:suffix compact IRI");

        // Type position: vocab=true — "ex:Type1" is the compact IRI.
        let result2 = compact_iri(
            &ac,
            &inv,
            "http://example.org/Type1",
            None,
            true,
            false,
        );
        assert_eq!(result2, "ex:Type1");
    }

    /// Compact IRI via a prefix with multiple candidates.
    ///
    /// Oracle: W3C compact/0007 — context {"foaf": "http://xmlns.com/foaf/0.1/",
    /// "dc11": "http://purl.org/dc/elements/1.1/"}.
    /// Compacting `http://xmlns.com/foaf/0.1/name` → `"foaf:name"`.
    ///
    /// Fixture: tests/w3c/json-ld-api/tests/compact/0007-context.jsonld,
    ///          tests/w3c/json-ld-api/tests/compact/0007-out.jsonld
    ///          (`"foaf:name"` appears in the compacted output).
    #[test]
    fn compact_iri_foaf_prefix_w3c_0007() {
        let ac = ctx_of(
            r#"{"dc11": "http://purl.org/dc/elements/1.1/",
               "ex": "http://example.org/vocab#",
               "foaf": "http://xmlns.com/foaf/0.1/"}"#,
        );
        let inv = ac.inverse_context();
        let result = compact_iri(
            &ac,
            &inv,
            "http://xmlns.com/foaf/0.1/name",
            None,
            true,
            false,
        );
        assert_eq!(result, "foaf:name");

        let dc_result = compact_iri(
            &ac,
            &inv,
            "http://purl.org/dc/elements/1.1/title",
            None,
            true,
            false,
        );
        assert_eq!(dc_result, "dc11:title");
    }

    /// Vocab-relative suffix: when `@vocab` is a prefix of the IRI and the
    /// suffix doesn't conflict with a defined term, return the suffix alone.
    #[test]
    fn compact_iri_vocab_relative_suffix() {
        let ac = ctx_of(
            r#"{"@vocab": "http://example.org/vocab/"}"#,
        );
        let inv = ac.inverse_context();
        let result = compact_iri(
            &ac,
            &inv,
            "http://example.org/vocab/Thing",
            None,
            true,
            false,
        );
        assert_eq!(result, "Thing", "vocab-relative suffix compaction");
    }

    /// Base-relative IRI: with vocab=false and no prefix match, a suffix
    /// relative to @base is returned.
    #[test]
    fn compact_iri_base_relative() {
        // BASE = "http://example.org/", set on the active context.
        let ac = ActiveContext::new(Some(BASE));
        let inv = ac.inverse_context();
        let result = compact_iri(&ac, &inv, "http://example.org/doc", None, false, false);
        assert_eq!(result, "doc", "base-relative IRI suffix");
    }

    /// Unresolvable IRI — returned unchanged.
    #[test]
    fn compact_iri_unknown_iri_returned_unchanged() {
        let ac = ctx_of(r#"{"ex": "http://example.org/"}"#);
        let inv = ac.inverse_context();
        let unrelated = "https://other.example/foo";
        let result = compact_iri(&ac, &inv, unrelated, None, true, false);
        assert_eq!(result, unrelated);
    }

    // -----------------------------------------------------------------------
    // §7.2 Term Selection — preference order
    // -----------------------------------------------------------------------

    /// select_term prefers the first matching preferred_value.
    #[test]
    fn select_term_preference_order() {
        let ac = ctx_of(
            r#"{"en_label": {"@id": "http://ex/label", "@language": "en"},
               "any_label": {"@id": "http://ex/label"}}"#,
        );
        let inv = ac.inverse_context();

        // With preferred_values = ["en", "@none"], "en" should match
        // "en_label" before "@none" matches "any_label".
        let result = select_term(
            &inv,
            "http://ex/label",
            &["@none"],
            "@language",
            &["en", "@none"],
        );
        assert_eq!(result.as_deref(), Some("en_label"), "specific language term preferred");

        // With preferred_values = ["fr", "@none"], "fr" has no match, so
        // "@none" fallback picks "any_label".
        let result2 = select_term(
            &inv,
            "http://ex/label",
            &["@none"],
            "@language",
            &["fr", "@none"],
        );
        assert_eq!(result2.as_deref(), Some("any_label"), "@none fallback");
    }

    /// select_term returns None when nothing matches.
    #[test]
    fn select_term_no_match_returns_none() {
        let inv = InverseContext::default();
        assert!(select_term(&inv, "http://ex/unknown", &["@none"], "@language", &["@none"]).is_none());
    }

    /// Reverse property: type_language "@reverse" looks in the type_ sub-map
    /// for the "@reverse" key.
    #[test]
    fn select_term_reverse_property() {
        let ac = ctx_of(
            r#"{"parentOf": {"@reverse": "http://ex/childOf"}}"#,
        );
        let inv = ac.inverse_context();

        let result = select_term(
            &inv,
            "http://ex/childOf",
            &["@set", "@none"],
            "@reverse",
            &["@reverse", "@none"],
        );
        assert_eq!(
            result.as_deref(),
            Some("parentOf"),
            "reverse term should be found under '@reverse' key"
        );
    }

    // -----------------------------------------------------------------------
    // Fix 1 regression: §7.1 step 3.3 wrong-key lookup
    // [SONNET-4.6] sq-90mu3 — old code: ctx.term_definitions.get(iri) where
    // iri is the expanded IRI; term_definitions keyed by term NAME, so the
    // lookup returned None when iri ≠ any term name, making the guard vacuous
    // (always extended containers).  The old code is ALSO broken in the
    // opposite direction when a term happens to be NAMED with the IRI being
    // compacted: it returns the wrong term definition (one that maps to a
    // DIFFERENT IRI) and incorrectly suppresses the @index search, causing
    // compact_iri to miss the correct @index-container term.
    //
    // Verify by reverting the fix: the old code returns "http://ex/p"
    // (absolute IRI, step 7 fallback) instead of "idx_term".
    // -----------------------------------------------------------------------

    /// §7.1 step 3.3 — wrong-key lookup regression.
    ///
    /// The old code called `ctx.term_definitions.get(iri)` using the
    /// EXPANDED IRI as a key.  `term_definitions` is keyed by TERM NAME, so
    /// the lookup only succeeds when a term happens to be NAMED with the same
    /// string as the expanded IRI (an "IRI-named term").  When that term also
    /// has `@index` in its container, the old code incorrectly suppressed the
    /// `@index` container search — causing `select_term` to miss a
    /// shorter-named term that also maps to the same IRI via `@index`, and
    /// falling back to returning the full IRI at §7.1 step 7.
    ///
    /// The fixed code (unconditionally add `@index`/`@index@set` when value
    /// has `@index`) finds the shorter-named term and returns it.
    #[test]
    fn compact_iri_index_guard_iri_keyed_term_regression() {
        // Context:
        //   "http://ex/p"  — IRI-named term; maps to itself with @index
        //                    container (no @id needed: the term name is the IRI).
        //   "idx_term"     — shorter normal term; also maps to "http://ex/p"
        //                    with @index container.
        //
        // compact_iri("http://ex/p", value_with_@index, vocab=true):
        //
        //   Old code: ctx.term_definitions.get("http://ex/p") returns the
        //   IRI-named term definition (iri="http://ex/p", container=["@index"]).
        //   Its container has @index → !ctx_containers.contains("@index") = false
        //   → DOES NOT push @index to containers → select_term searches
        //   "@set"/"@none" but both terms are stored under "@index" in the
        //   inverse context → nothing found → falls through to §7.1 step 7
        //   → returns "http://ex/p" (full IRI). WRONG.
        //
        //   New code: unconditionally pushes @index → select_term finds the
        //   SHORTER term "idx_term" (8 chars < 12 chars) in the "@index" slot
        //   → returns "idx_term". CORRECT.
        let ac = ctx_of(
            r#"{
                "http://ex/p": {"@container": "@index"},
                "idx_term":    {"@id": "http://ex/p", "@container": "@index"}
            }"#,
        );
        let inv = ac.inverse_context();
        let value = json(r#"{"@index": "key", "@value": "hello"}"#);
        let result = compact_iri(&ac, &inv, "http://ex/p", Some(&value), true, false);
        assert_eq!(
            result, "idx_term",
            "compact_iri must find the shorter @index-container term via the @index \
             slot; the old wrong-key lookup suppressed that search and returned the \
             full IRI"
        );
    }

    // -----------------------------------------------------------------------
    // Fix 2: §7.1 step 6 base-relative IRI via relativize_iri
    // [SONNET-4.6] sq-90mu3 — old code: iri.strip_prefix(base) only worked
    // for literal-prefix cases.  The new code uses the full RFC 3986
    // relativization that handles same-directory, parent-traversal,
    // query/fragment preservation.
    // -----------------------------------------------------------------------

    /// §7.1 step 6 — same-directory base-relative compaction.
    ///
    /// base = "http://example.org/" (= BASE const)
    /// compact_iri("http://example.org/a/c", vocab=false) should yield a
    /// relative ref.  With strip_prefix the literal-prefix case works only
    /// when the base itself is a literal prefix (which it is here: "a/c").
    /// The relativize_iri path also covers same-directory siblings.
    #[test]
    fn compact_iri_base_relative_same_directory() {
        // BASE = "http://example.org/" — parent of "a/b" and "a/c".
        // A fresh context with just a base IRI set.
        let ac = ActiveContext::new(Some("http://example.org/a/b"));
        let inv = ac.inverse_context();
        let result = compact_iri(&ac, &inv, "http://example.org/a/c", None, false, false);
        // With RFC 3986 relativization: "http://example.org/a/b" + "c" → "http://example.org/a/c"
        assert_eq!(result, "c", "same-directory relative compaction");
    }

    /// §7.1 step 6 — parent-traversal relative compaction.
    ///
    /// old `strip_prefix("http://example.org/a/b/c")` cannot produce "../d";
    /// `relativize_iri` correctly generates "../../d".
    #[test]
    fn compact_iri_base_relative_parent_traversal() {
        let ac = ActiveContext::new(Some("http://example.org/a/b/c"));
        let inv = ac.inverse_context();
        let result = compact_iri(&ac, &inv, "http://example.org/a/d", None, false, false);
        assert_eq!(result, "../d", "parent-traversal relative compaction");
    }

    /// §7.1 step 6 — query and fragment are preserved in the relative ref.
    #[test]
    fn compact_iri_base_relative_query_fragment() {
        let ac = ActiveContext::new(Some("http://example.org/a/b"));
        let inv = ac.inverse_context();
        let result = compact_iri(
            &ac,
            &inv,
            "http://example.org/a/c?q=1#sec",
            None,
            false,
            false,
        );
        assert_eq!(result, "c?q=1#sec", "query and fragment preserved in relative ref");
    }

    /// §7.1 step 6 — cross-scheme IRI is returned unchanged (relativize returns None).
    #[test]
    fn compact_iri_cross_scheme_returned_unchanged() {
        let ac = ActiveContext::new(Some("http://example.org/a/b"));
        let inv = ac.inverse_context();
        let result = compact_iri(&ac, &inv, "https://example.org/a/c", None, false, false);
        assert_eq!(
            result, "https://example.org/a/c",
            "cross-scheme IRI must be returned unchanged"
        );
    }
}
