//! [OPUS-4.8] (sq-ixc3.4) Full **W3C JSON-LD 1.1 Compaction** — hand-rolled, dependency-free.
//!
//! The existing JSON-LD writer in the parent module (`super`) emits the **expanded**
//! / **flattened** / *prefix-`@context`* forms (sq-ixc3.5/#900, sq-l5kr/#923). That
//! prefix-only "compacted" form abbreviates IRIs to `prefix:local` CURIEs but does
//! **not** implement the actual W3C Compaction Algorithm: it has no term definitions,
//! no `@vocab`, no datatype/language/container coercion, no `@reverse`, no
//! `@id`/`@type` keyword aliasing, and no value/node compaction against a
//! caller-supplied `@context`.
//!
//! This module adds that — the **full JSON-LD 1.1 Compaction Algorithm**
//! (<https://www.w3.org/TR/json-ld11-api/#compaction-algorithms>) applied to an RDF
//! graph plus a caller `@context`. It stays inside the **dependency-free**
//! `serialize-rdf` feature: it pulls in **zero** new crates (it defines its own tiny
//! [`Json`] value type rather than `serde_json`, exactly like the rest of the writer).
//!
//! ## Pipeline (faithful to the spec)
//!
//! 1. **fromRdf** ([`graph_to_expanded`]) — build an *expanded* JSON-LD model (a `Vec<Json>`
//!    of node objects) from the graph's triples. This reuses the parent writer's RDF→JSON-LD
//!    mapping semantics (`@value`/`@type`/`@language`, `@list` collapse, native scalar
//!    coercion) but materialises a [`Json`] AST instead of bytes, because the Compaction
//!    Algorithm operates over the expanded *document*, not over raw triples.
//! 2. **Context processing** ([`ActiveContext::parse`]) — turn the caller `@context` JSON
//!    into an *active context*: term definitions (IRI mapping + `@type`/`@language`/
//!    `@container`/`@reverse` coercions), `@vocab`, `@base`, default `@language`, and prefix
//!    mappings. Keyword aliases (`@id`/`@type`/etc.) are recognised.
//! 3. **Compaction** ([`ActiveContext::compact`]) — the recursive Compaction Algorithm:
//!    IRI compaction against the active context (term → compact-IRI/`@vocab`-relative →
//!    full IRI), value compaction (drop `@value`/`@type`/`@language` made redundant by a
//!    term's coercion), node compaction, `@reverse`, and `@container` framing
//!    (`@set`/`@list`/`@language`/`@index`).
//! 4. **Serialize** ([`Json::write`]) — emit the compacted document as canonical JSON.
//!
//! ## Honest scope
//!
//! This targets the **serialise / fromRdf-then-compact** path — sparq emits RDF, so the
//! input is always a graph, never an arbitrary remote JSON-LD document. The full
//! *general* compaction (compacting an already-expanded document with `@reverse` already
//! present, scoped/typed contexts, `@propagate`, remote `@context` fetching, `@import`,
//! `@protected`) is **not** needed here and is deliberately out of scope: the input
//! expanded model is the one *we* produce, so its shape is known. Term definitions,
//! `@vocab`, type/language/`@container` (`@set`/`@list`/`@language`/`@index`), `@reverse`,
//! keyword aliasing, value compaction, node compaction and IRI compaction are all
//! covered. See the crate README + `skills/data-formats/SKILL.md` for the boundary.
//!
//! ## Faithful to a strict third-party processor (sq-oy1f.12/.13/.14)
//!
//! [OPUS-4.8] The emitted compacted document is differentially verified against the pyld
//! W3C reference processor, so a strict third-party consumer re-expands it to the same
//! graph. The four faithfulness fixes that close that gap:
//!
//! - **`@reverse` edges** are emitted as a *forward member of the reverse term*, not inside
//!   an `@reverse` block — emitting the block on an already-reversed edge would double-invert
//!   (see [`relocate_reverse`] / [`REVERSE_RELOC`] and `compact_node`).
//! - **Non-string values never land in a `@language` map** — a language container only
//!   accumulates language-tagged strings; other values fall through to the normal path.
//! - **Multi-value `@language` / `@index` containers accumulate** every value rather than
//!   overwriting, so no member is lost when several values share a key.
//! - **A literal under a `@type: @id` term stays a literal** — type coercion to `@id` applies
//!   only to node references, never to a value that is already a literal.

use super::{
    coerce_native, detect_lists, ListInfo, NamedGraph, RDF_LANG_STRING, RDF_TYPE, XSD,
};
use oxrdf::{NamedOrBlankNode, Term, Triple};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// [OPUS-4.8] (sq-oy1f.12) Internal sentinel member key under which [`relocate_reverse`]
/// stashes the edges it moves onto an object node. It is NOT a JSON-LD keyword (it does not
/// start with `@` so it never collides with a real keyword, and the leading marker bytes make
/// it impossible for it to be a real predicate IRI). [`ActiveContext::compact_node`] consumes
/// it and emits each stashed edge through the matching `@reverse` *term as a forward member*
/// (the spec-faithful, pyld-verified shape) rather than an `@reverse` block (which would
/// double-invert). It never appears in the emitted document.
const REVERSE_RELOC: &str = "\u{0}sparq-reverse";

// ===========================================================================
// JSON value model — the `Json` AST, now single-sourced in the `sparq-jsonld`
// crate ([OPUS-4.8] sq-oy1f.23, epic sq-oy1f). The type moved out of this module
// VERBATIM (public API preserved: re-exported here so `compact::Json` — and the
// `serialize::JsonLdValue` alias in the parent module — resolve unchanged, and the
// writer emits byte-identical output). Its `write`/`obj`/`set`/`get`/`as_str`/
// `is_obj` helpers, previously `pub(super)`, are the crate's public `Json` API now.
// The document-level JSON-LD 1.1 pipeline is built on top of it in beads
// sq-oy1f.24+; this writer keeps using it exactly as before.
// ===========================================================================

pub use sparq_jsonld::Json;

// ===========================================================================
// Active context — the processed caller `@context`.
// ===========================================================================

/// One term definition from the active context (a member of the caller `@context` whose
/// value is an IRI string or an expanded `{ "@id": …, "@type": …, … }` object).
#[derive(Clone, Debug, Default)]
struct TermDefinition {
    /// The IRI this term maps to (`@id`). For a keyword alias this is the keyword
    /// (e.g. `"@type"`); for a normal term it is an absolute IRI.
    iri: String,
    /// `@type` coercion: an absolute IRI, or the keywords `"@id"` / `"@vocab"` / `"@json"`.
    type_mapping: Option<String>,
    /// `@language` coercion (may be the empty string to *clear* a default `@language`,
    /// distinct from "no mapping").
    language: Option<String>,
    /// `@container` mapping: one of `@set` / `@list` / `@language` / `@index` (the subset
    /// this writer supports). A single container covers the cases sparq emits.
    container: Option<String>,
    /// True when the term definition is a `@reverse` property.
    reverse: bool,
}

/// The processed caller `@context`: the inverse of context-processing applied to a
/// JSON-LD `@context`, used to drive compaction. Built by [`ActiveContext::parse`].
#[derive(Clone, Debug, Default)]
pub struct ActiveContext {
    /// term string → its definition (insertion order preserved for deterministic IRI choice).
    terms: Vec<(String, TermDefinition)>,
    /// prefix → namespace IRI (a term whose definition is a bare IRI ending in a gen-delim,
    /// usable as a compact-IRI prefix). Tracked separately for the IRI-compaction fallback.
    prefixes: Vec<(String, String)>,
    /// `@vocab` IRI, against which vocab-relative terms (predicates, `@type` values) compact.
    vocab: Option<String>,
    /// `@base` IRI (document base) — reserved; sparq emits absolute `@id`s so this is unused
    /// for output but parsed for completeness.
    base: Option<String>,
    /// Default `@language` applied to plain string values lacking a per-term `@language`.
    default_language: Option<String>,
    /// The verbatim `@context` JSON to echo back into the compacted document's `@context`.
    raw_context: Json,
}

/// Recognised JSON-LD keywords (used to tell a keyword/alias from a normal term/IRI).
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "@base"
            | "@container"
            | "@context"
            | "@direction"
            | "@graph"
            | "@id"
            | "@import"
            | "@included"
            | "@index"
            | "@json"
            | "@language"
            | "@list"
            | "@nest"
            | "@none"
            | "@prefix"
            | "@propagate"
            | "@protected"
            | "@reverse"
            | "@set"
            | "@type"
            | "@value"
            | "@version"
            | "@vocab"
    )
}

/// True when `iri`'s last character is an RDF gen-delim (`:` `/` `?` `#` `[` `]` `@`) — the
/// shape of a namespace IRI that may serve as a compact-IRI prefix.
fn ends_in_gen_delim(iri: &str) -> bool {
    matches!(
        iri.chars().last(),
        Some(':' | '/' | '?' | '#' | '[' | ']' | '@')
    )
}

impl ActiveContext {
    /// Processes a caller `@context` JSON value into an active context. The input is the
    /// value of the document's `@context` member (an object); a string/array form (remote
    /// reference) is **not** resolved (sparq supplies the context inline). Unknown members
    /// are ignored. This is the subset of Context Processing the Compaction path needs.
    pub fn parse(context: &Json) -> ActiveContext {
        let mut ctx = ActiveContext {
            raw_context: context.clone(),
            ..ActiveContext::default()
        };
        let Json::Obj(members) = context else {
            return ctx;
        };
        // First pass: the context-level keywords (@vocab/@base/@language), so that term
        // definitions referencing @vocab resolve correctly.
        for (k, v) in members {
            match k.as_str() {
                "@vocab" => ctx.vocab = v.as_str().map(str::to_string),
                "@base" => ctx.base = v.as_str().map(str::to_string),
                "@language" => ctx.default_language = v.as_str().map(str::to_string),
                _ => {}
            }
        }
        // Second pass: term definitions.
        for (term, v) in members {
            if term.starts_with('@') {
                continue; // a context keyword, already handled
            }
            if let Some(def) = ctx.parse_term_definition(term, v) {
                // A bare-IRI term ending in a gen-delim doubles as a compact-IRI prefix.
                if def.type_mapping.is_none()
                    && def.language.is_none()
                    && def.container.is_none()
                    && !def.reverse
                    && ends_in_gen_delim(&def.iri)
                    && !is_keyword(&def.iri)
                {
                    ctx.prefixes.push((term.clone(), def.iri.clone()));
                }
                ctx.terms.push((term.clone(), def));
            }
        }
        ctx
    }

    /// Builds a single term definition. `value` is either an IRI string or an expanded
    /// `{ "@id"/"@reverse": …, "@type": …, "@language": …, "@container": … }` object.
    fn parse_term_definition(&self, term: &str, value: &Json) -> Option<TermDefinition> {
        let mut def = TermDefinition::default();
        match value {
            Json::Str(iri) => {
                if is_keyword(iri) {
                    // A keyword alias: a term mapping to a keyword (e.g. {"id": "@id"}).
                    def.iri = iri.clone();
                    return Some(def);
                }
                def.iri = self.expand_context_iri(iri);
            }
            Json::Obj(_) => {
                if let Some(rev) = value.get("@reverse").and_then(Json::as_str) {
                    def.iri = self.expand_context_iri(rev);
                    def.reverse = true;
                } else if let Some(id) = value.get("@id").and_then(Json::as_str) {
                    if is_keyword(id) {
                        def.iri = id.to_string();
                    } else {
                        def.iri = self.expand_context_iri(id);
                    }
                } else if let Some(vocab) = &self.vocab {
                    // No explicit @id: the term IRI is @vocab + term (vocab-relative).
                    def.iri = format!("{}{}", vocab, term);
                } else {
                    def.iri = term.to_string();
                }
                if let Some(t) = value.get("@type").and_then(Json::as_str) {
                    def.type_mapping = Some(match t {
                        "@id" | "@vocab" | "@json" | "@none" => t.to_string(),
                        other => self.expand_context_iri(other),
                    });
                }
                if let Some(l) = value.get("@language") {
                    def.language = match l {
                        Json::Str(s) => Some(s.clone()),
                        _ => Some(String::new()), // null clears the default language
                    };
                }
                if let Some(c) = value.get("@container").and_then(Json::as_str) {
                    def.container = Some(c.to_string());
                }
            }
            _ => {
                // null term definition (removes a term) — empty IRI never matches.
                def.iri = String::new();
            }
        }
        Some(def)
    }

    /// Expands an IRI *within context processing*: a compact IRI `prefix:local` against an
    /// already-defined prefix term, a `@vocab`-relative bare term, or an absolute IRI. Used
    /// only while building the context.
    fn expand_context_iri(&self, value: &str) -> String {
        if is_keyword(value) || value.starts_with("_:") {
            return value.to_string();
        }
        if let Some((prefix, suffix)) = value.split_once(':') {
            if suffix.starts_with("//") {
                return value.to_string(); // already absolute (scheme://…)
            }
            // Compact IRI against a previously-defined term whose IRI we know.
            for (t, def) in &self.terms {
                if t == prefix {
                    return format!("{}{}", def.iri, suffix);
                }
            }
            return value.to_string();
        }
        // No colon: a vocab-relative term if @vocab is set, else verbatim.
        match &self.vocab {
            Some(v) => format!("{}{}", v, value),
            None => value.to_string(),
        }
    }

    /// Expands a vocab-position IRI (a frame property key, `@type` value, or `@id` value)
    /// against this context: a defined term → its IRI, a compact IRI `prefix:local` → the
    /// prefix's namespace + local, a `@vocab`-relative bare term → `@vocab` + term, else
    /// the value verbatim. `pub(super)` so the sibling `frame` module ([OPUS-4.8] sq-oy1f.17)
    /// can expand a caller frame against its `@context` before node-pattern matching (the
    /// node map keys are absolute IRIs). Keywords and already-absolute IRIs pass through.
    pub(super) fn expand_vocab_iri(&self, value: &str) -> String {
        if is_keyword(value) {
            return value.to_string();
        }
        // A defined term maps directly to its IRI.
        if let Some(def) = self.term_def(value) {
            if !def.iri.is_empty() {
                return def.iri.clone();
            }
        }
        self.expand_context_iri(value)
    }

    /// Looks up a term definition by exact term string.
    fn term_def(&self, term: &str) -> Option<&TermDefinition> {
        self.terms.iter().find(|(t, _)| t == term).map(|(_, d)| d)
    }

    // -----------------------------------------------------------------------
    // IRI Compaction (https://www.w3.org/TR/json-ld11-api/#iri-compaction).
    // -----------------------------------------------------------------------

    /// Compacts an absolute `iri` against the active context. `vocab` is true when the IRI
    /// is in *vocabulary-relative* position (a predicate or a `@type` value), enabling term
    /// and `@vocab` compaction; false for `@id` values (only compact-IRI applies).
    /// `reverse` selects only `@reverse` term definitions.
    fn compact_iri(&self, iri: &str, vocab: bool, reverse: bool) -> String {
        if vocab {
            // 1. An exact term whose IRI mapping equals the IRI (respecting reverse-ness),
            //    preferring a "plain" term (no coercion) so the inverse mapping is
            //    unambiguous; otherwise any matching term. Skip prefix-shaped terms here.
            let mut plain: Option<&str> = None;
            let mut coerced: Option<&str> = None;
            for (t, def) in &self.terms {
                if def.iri == iri && def.reverse == reverse && !t.contains(':') {
                    let is_plain = def.type_mapping.is_none()
                        && def.language.is_none()
                        && def.container.is_none();
                    if is_plain && plain.is_none() {
                        plain = Some(t);
                    } else if coerced.is_none() {
                        coerced = Some(t);
                    }
                }
            }
            if let Some(t) = plain.or(coerced) {
                return t.to_string();
            }
            // 2. `@vocab`-relative: strip the @vocab namespace if no other term shadows it.
            // [OPUS-4.8] (sq-oy1f.11) The W3C IRI-Compaction algorithm (Step 2.2) requires
            // ONLY that the stripped suffix is non-empty and is not shadowed by a term
            // definition; it does NOT exclude suffixes containing '/' or '#'. A prior
            // `!rest.contains([':', '/', '#'])` guard suppressed the spec-mandated vocab-
            // relative form for predicates whose local part contains a fragment (e.g.
            // `<…/ns#value>` against `@vocab: …/` → `ns#value`), emitting the full IRI
            // instead. Re-expanding `rest` against `@vocab` (concatenation) reproduces the
            // IRI exactly for '/'/'#' suffixes, so dropping those keeps the round-trip
            // lossless (verified against pyld). We DELIBERATELY keep the ':' exclusion:
            // a vocab-relative term whose suffix contains ':' is indistinguishable from a
            // compact IRI / absolute IRI on read-back (`a:b` re-expands to `<a:b>`, not
            // `<@vocab>a:b`), so emitting it would be a LOSSY round-trip — this guard is a
            // losslessness requirement, not the spec over-restriction the bead removes.
            if !reverse {
                if let Some(v) = &self.vocab {
                    if let Some(rest) = iri.strip_prefix(v.as_str()) {
                        if !rest.is_empty() && !rest.contains(':') && self.term_def(rest).is_none()
                        {
                            return rest.to_string();
                        }
                    }
                }
            }
        }
        // 3. Compact IRI `prefix:suffix` against the longest matching prefix namespace.
        let mut best: Option<(&str, &str)> = None;
        for (prefix, ns) in &self.prefixes {
            if let Some(suffix) = iri.strip_prefix(ns.as_str()) {
                if suffix.is_empty() {
                    continue;
                }
                match best {
                    Some((_, bns)) if bns.len() >= ns.len() => {}
                    _ => best = Some((prefix, suffix)),
                }
            }
        }
        if let Some((prefix, suffix)) = best {
            return format!("{}:{}", prefix, suffix);
        }
        // 4. No compaction possible — keep the absolute IRI.
        iri.to_string()
    }

    /// [OPUS-4.8] (sq-oy1f.8) Compacts a property `iri` in vocab position while IGNORING any
    /// term that carries a `@container` mapping OR an `@id`/`@vocab` `@type` coercion — so a
    /// co-located sibling that must NOT be re-interpreted (an `@list` sibling, a non-string
    /// language-map value, or a plain literal alongside a `@type:@id` node ref — [OPUS-4.8]
    /// sq-oy1f.8/.12/.13) gets a "plain" key. It prefers a plain (container- and `@id`/`@vocab`-
    /// coercion-free) term match for the IRI, then falls through to the `@vocab`-relative /
    /// prefix / full-IRI choices. This mirrors pyld: with `@vocab` it yields the vocab-relative
    /// form, with a plain alternate term that term, otherwise the full IRI.
    fn compact_iri_no_list(&self, iri: &str, reverse: bool) -> String {
        // A term is "re-coercing" if it would change how the sibling reads back: an `@list` /
        // `@language` / `@index` etc. container, or an `@id`/`@vocab` type coercion (which
        // turns a string value into a node IRI). Such a term is skipped here.
        let recoerces = |def: &TermDefinition| -> bool {
            def.container.is_some()
                || def
                    .type_mapping
                    .as_deref()
                    .is_some_and(|t| t == "@id" || t == "@vocab")
        };
        for (t, def) in &self.terms {
            if def.iri == iri && def.reverse == reverse && !recoerces(def) && !t.contains(':') {
                return t.to_string();
            }
        }
        // No plain exact term: reuse the @vocab-relative / prefix / full-IRI fallbacks. A term
        // that re-coerces (container or @id/@vocab) could still win `compact_iri`'s step-1
        // exact-match branch, so strip the @vocab/prefix tail by hand rather than recursing.
        if !reverse {
            if let Some(v) = &self.vocab {
                if let Some(rest) = iri.strip_prefix(v.as_str()) {
                    // A @vocab-relative bare term `rest` is safe only if no term definition
                    // shadows it with a re-coercion (an un-coerced shadow re-expands the same
                    // way, so it is fine; a re-coercing shadow would change the read-back).
                    let shadow_recoerces = self.term_def(rest).is_some_and(recoerces);
                    if !rest.is_empty() && !rest.contains(':') && !shadow_recoerces {
                        return rest.to_string();
                    }
                }
            }
        }
        let mut best: Option<(&str, &str)> = None;
        for (prefix, ns) in &self.prefixes {
            if let Some(suffix) = iri.strip_prefix(ns.as_str()) {
                if suffix.is_empty() {
                    continue;
                }
                match best {
                    Some((_, bns)) if bns.len() >= ns.len() => {}
                    _ => best = Some((prefix, suffix)),
                }
            }
        }
        if let Some((prefix, suffix)) = best {
            return format!("{}:{}", prefix, suffix);
        }
        iri.to_string()
    }

    /// The compacted spelling of a keyword (e.g. `@type`), honouring a keyword *alias*
    /// term in the context (`{"type": "@type"}` → `"type"`). Falls back to the keyword.
    pub(super) fn compact_keyword(&self, keyword: &str) -> String {
        for (t, def) in &self.terms {
            if def.iri == keyword {
                return t.clone();
            }
        }
        keyword.to_string()
    }

    /// The verbatim caller `@context` JSON, echoed into a compacted / framed document's
    /// `@context` member. `pub(super)` for the sibling `frame` module ([OPUS-4.8] sq-oy1f.17),
    /// which builds the framed output document's envelope.
    pub(super) fn raw_context(&self) -> &Json {
        &self.raw_context
    }

    // -----------------------------------------------------------------------
    // Value Compaction (https://www.w3.org/TR/json-ld11-api/#value-compaction).
    // -----------------------------------------------------------------------

    /// Compacts a JSON-LD *expanded value* (`{"@value": …}` or a node reference
    /// `{"@id": …}`) under the coercion rules of `active_property`'s term definition.
    /// Returns the most compact form the term's `@type`/`@language` mapping permits (a bare
    /// scalar/string when the value's type/language is fully implied), else a reduced value
    /// object.
    fn compact_value(&self, active_property: Option<&str>, value: &Json) -> Json {
        let def = active_property.and_then(|p| self.term_def(p));

        // A *pure* node reference `{"@id": iri}` with @type:@id / @type:@vocab coercion
        // compacts to the bare (compacted) IRI string.
        if let Some(id) = value.get("@id").and_then(Json::as_str) {
            let lone_id = matches!(value, Json::Obj(m) if m.len() == 1);
            if lone_id {
                if let Some(tm) = def.and_then(|d| d.type_mapping.as_deref()) {
                    if tm == "@id" {
                        return Json::Str(self.compact_iri(id, false, false));
                    }
                    if tm == "@vocab" {
                        return Json::Str(self.compact_iri(id, true, false));
                    }
                }
            }
            // Otherwise leave the node object for node compaction to reduce.
            return value.clone();
        }

        let Some(val) = value.get("@value") else {
            return value.clone();
        };
        let v_type = value.get("@type").and_then(Json::as_str);
        let v_lang = value.get("@language").and_then(Json::as_str);

        let type_mapping = def.and_then(|d| d.type_mapping.as_deref());
        let lang_mapping = def.and_then(|d| d.language.as_deref());
        let default_lang = self.default_language.as_deref();

        // @type:@json — keep the value verbatim (sparq does not emit @json, but be safe).
        if type_mapping == Some("@json") {
            let mut o = Json::obj();
            o.set(&self.compact_keyword("@value"), val.clone());
            o.set(&self.compact_keyword("@type"), Json::Str("@json".to_string()));
            return o;
        }

        // The value's datatype matches the term's @type coercion → the @type is redundant.
        if let (Some(vt), Some(tm)) = (v_type, type_mapping) {
            if vt == tm {
                return val.clone();
            }
        }
        // A language-tagged value whose language matches the term's @language (or the
        // default @language when the term has no override) → the @language is redundant.
        if let Some(vl) = v_lang {
            let effective = lang_mapping.or(default_lang);
            if effective == Some(vl) {
                return val.clone();
            }
        }
        // A plain string (no @type, no @language): bare iff no default @language is in
        // force, or the term explicitly clears it (@language: null / "").
        if v_type.is_none() && v_lang.is_none() {
            let term_clears_lang = lang_mapping == Some("");
            if default_lang.is_none() || term_clears_lang {
                return val.clone();
            }
        }

        // Otherwise emit a reduced value object with compacted keyword keys + a compacted
        // @type IRI (if the @type is not coerced away).
        let mut o = Json::obj();
        o.set(&self.compact_keyword("@value"), val.clone());
        if let Some(vt) = v_type {
            if Some(vt) != type_mapping {
                o.set(
                    &self.compact_keyword("@type"),
                    Json::Str(self.compact_iri(vt, true, false)),
                );
            }
        }
        if let Some(vl) = v_lang {
            let effective = lang_mapping.or(default_lang);
            if effective != Some(vl) {
                o.set(&self.compact_keyword("@language"), Json::Str(vl.to_string()));
            }
        }
        o
    }

    // -----------------------------------------------------------------------
    // Node / document Compaction.
    // -----------------------------------------------------------------------

    /// Compacts an *expanded element* (array of node objects / a single node object / a
    /// value object) into its compacted JSON-LD form. This is the recursive Compaction
    /// Algorithm specialised to the document shapes [`graph_to_expanded`] produces.
    pub(super) fn compact(&self, element: &Json) -> Json {
        match element {
            Json::Arr(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(self.compact(it));
                }
                Json::Arr(out)
            }
            Json::Obj(_) => self.compact_node(element),
            other => other.clone(),
        }
    }

    /// Compacts one node object (or value object / `@list`) and its members.
    fn compact_node(&self, node: &Json) -> Json {
        // A top-level value object (rare for a node array) is handled by compact_value.
        if node.get("@value").is_some() {
            return self.compact_value(None, node);
        }
        let mut result = Json::obj();

        // @id keyword.
        if let Some(id) = node.get("@id").and_then(Json::as_str) {
            result.set(
                &self.compact_keyword("@id"),
                Json::Str(self.compact_iri(id, false, false)),
            );
        }
        // @type keyword (array of IRIs → compacted, vocab-relative). A single type compacts
        // to a scalar; multiple to an array (the spec's "compact arrays" rule).
        if let Some(Json::Arr(types)) = node.get("@type") {
            let compacted: Vec<Json> = types
                .iter()
                .filter_map(Json::as_str)
                .map(|t| Json::Str(self.compact_iri(t, true, false)))
                .collect();
            let value = if compacted.len() == 1 {
                compacted.into_iter().next().expect("len 1")
            } else {
                Json::Arr(compacted)
            };
            result.set(&self.compact_keyword("@type"), value);
        }
        // @graph member (named-graph sub-object or the top-level dataset graph).
        if let Some(graph) = node.get("@graph") {
            result.set(&self.compact_keyword("@graph"), self.compact(graph));
        }

        // Ordinary forward properties.
        if let Json::Obj(members) = node {
            for (key, vals) in members {
                if key == REVERSE_RELOC {
                    continue; // the relocated-reverse sentinel — emitted via reverse terms below
                }
                if key.starts_with('@') {
                    continue; // keywords handled above
                }
                self.compact_property(key, vals, false, &mut result);
            }
        }
        // [OPUS-4.8] (sq-oy1f.12) Relocated reverse edges (placed by `relocate_reverse` on the
        // object node under the `REVERSE_RELOC` sentinel): each member key is a forward-
        // predicate IRI, and the matching `@reverse` term is emitted as a *forward member* of
        // this node — NOT inside an `@reverse` block. A `@reverse` BLOCK whose key is itself a
        // `@reverse` term double-inverts (a strict third-party processor like pyld reads the
        // edge backwards: it applies the block's inversion AND the term's inversion). Emitting
        // the reverse term as a forward member (`{"children": <subject>}`) applies the
        // inversion exactly once, so pyld reconstructs the original edge direction.
        if let Some(Json::Obj(rev_members)) = node.get(REVERSE_RELOC) {
            for (iri, vals) in rev_members {
                self.compact_property(iri, vals, true, &mut result);
            }
        }
        result
    }

    /// Relocates forward edges that a `@reverse` term covers onto their object node, so node
    /// compaction can express each through the reverse term. For every node `S` with a
    /// property `P` (where `P` is the IRI of a `@reverse` term) whose objects are node
    /// references, the edge is *moved* onto each object node `O` under the internal
    /// [`REVERSE_RELOC`] sentinel as `{ P: [{"@id": S}] }`, and removed from `S`.
    ///
    /// [OPUS-4.8] (sq-oy1f.12) `compact_node` then emits each relocated edge as a **forward
    /// member keyed by the reverse term** (e.g. `{"children": <S>}` on `O`) — NOT inside an
    /// `@reverse` block. A reverse-term key *inside* an `@reverse` block double-inverts: a
    /// strict third-party processor (pyld) applies the block's inversion AND the term's
    /// inversion, reading the edge backwards. The forward-member shape inverts exactly once,
    /// so the round-trip is faithful. (The sentinel — not `@reverse` — is used precisely so a
    /// downstream `@reverse` block is never emitted.)
    ///
    /// Only applied within a single graph scope (the caller passes one graph's node array).
    fn relocate_reverse(&self, nodes: &mut [Json]) {
        // The set of forward-predicate IRIs that a reverse term covers.
        let reverse_iris: Vec<String> = self
            .terms
            .iter()
            .filter(|(_, d)| d.reverse)
            .map(|(_, d)| d.iri.clone())
            .collect();
        if reverse_iris.is_empty() {
            return;
        }
        // Index node position by @id so we can attach to the object node.
        let id_of = |n: &Json| n.get("@id").and_then(Json::as_str).map(str::to_string);
        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        for (i, n) in nodes.iter().enumerate() {
            if let Some(id) = id_of(n) {
                index.entry(id).or_insert(i);
            }
        }
        // Collect (object_pos, predicate_iri, subject_id) edges to relocate, plus the exact
        // set of relocated (subject_id, predicate_iri, object_id) edges so we strip ONLY
        // those — never an edge that stays a forward property.
        let mut moves: Vec<(usize, String, String)> = Vec::new();
        // [OPUS-4.8] (sq-oy1f.10) Track the precise edges relocated, keyed by
        // (subject, predicate, object). A forward edge whose object is NOT itself a subject
        // in this graph (no `index` entry) is never relocated, so it must survive as a
        // forward property. The prior code bulk-`retain`-stripped EVERY reverse-IRI property
        // from EVERY subject once any edge moved, dropping the un-relocated edge entirely —
        // silent data loss that violated losslessness.
        let mut relocated: std::collections::BTreeSet<(String, String, String)> =
            std::collections::BTreeSet::new();
        for n in nodes.iter() {
            let Some(subj) = id_of(n) else { continue };
            let Json::Obj(members) = n else { continue };
            for (key, vals) in members {
                if !reverse_iris.iter().any(|r| r == key) {
                    continue;
                }
                for o in flatten(vals) {
                    if let Some(oid) = o.get("@id").and_then(Json::as_str) {
                        if let Some(&pos) = index.get(oid) {
                            moves.push((pos, key.clone(), subj.clone()));
                            relocated.insert((subj.clone(), key.clone(), oid.to_string()));
                        }
                    }
                }
            }
        }
        if moves.is_empty() {
            return;
        }
        // Remove ONLY the relocated edges from each subject node: for a reverse-IRI property,
        // keep object-values whose (subject, predicate, object) edge was NOT relocated, and
        // drop the property entirely only when every one of its values moved. Non-node-ref
        // values (no `@id`) are never relocated, so they are always kept.
        for n in nodes.iter_mut() {
            let Some(subj) = id_of(n) else { continue };
            let Json::Obj(members) = n else { continue };
            members.retain_mut(|(k, vals)| {
                if !reverse_iris.iter().any(|r| r == k) {
                    return true;
                }
                let mut kept: Vec<Json> = flatten(vals)
                    .into_iter()
                    .filter(|o| match o.get("@id").and_then(Json::as_str) {
                        Some(oid) => {
                            !relocated.contains(&(subj.clone(), k.clone(), oid.to_string()))
                        }
                        None => true,
                    })
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    return false; // every edge relocated — drop the forward property
                }
                *vals = if kept.len() == 1 {
                    kept.pop().expect("len 1")
                } else {
                    Json::Arr(kept)
                };
                true
            });
        }
        // Attach each edge onto the object node's `REVERSE_RELOC` sentinel member (consumed by
        // `compact_node` and emitted via the reverse term as a forward member — never as an
        // `@reverse` block, which would double-invert).
        for (pos, pred, subj) in moves {
            let node = &mut nodes[pos];
            let mut subj_ref = Json::obj();
            subj_ref.set("@id", Json::Str(subj));
            let rev = match node.get(REVERSE_RELOC).cloned() {
                Some(Json::Obj(mut m)) => {
                    if let Some(slot) = m.iter_mut().find(|(k, _)| *k == pred) {
                        if let Json::Arr(a) = &mut slot.1 {
                            a.push(subj_ref);
                        }
                    } else {
                        m.push((pred.clone(), Json::Arr(vec![subj_ref])));
                    }
                    Json::Obj(m)
                }
                _ => {
                    let mut o = Json::obj();
                    o.set(&pred, Json::Arr(vec![subj_ref]));
                    o
                }
            };
            node.set(REVERSE_RELOC, rev);
        }
    }

    /// Compacts one property `iri` with its expanded object array `vals` into `result`,
    /// applying the term's `@container` framing (`@set`/`@list`/`@language`/`@index`) and
    /// "compact arrays" (a single-element array collapses to a scalar unless `@container`
    /// forces an array). When `reverse_only` is true, only a reverse term is emitted (the
    /// relocated-reverse edges from [`ActiveContext::compact_node`], emitted as forward
    /// members keyed by the reverse term); when false, reverse terms are skipped.
    fn compact_property(&self, iri: &str, vals: &Json, reverse_only: bool, result: &mut Json) {
        // Choose the active term for this IRI in vocab position.
        let term = self.compact_iri(iri, true, reverse_only);
        let def = self.term_def(&term).cloned();

        if !reverse_only && def.as_ref().is_some_and(|d| d.reverse) {
            return; // a reverse term — handled via the @reverse block
        }
        if reverse_only && !def.as_ref().is_some_and(|d| d.reverse) {
            return;
        }

        let container = def.as_ref().and_then(|d| d.container.clone());
        let items: Vec<Json> = match vals {
            Json::Arr(a) => a.clone(),
            single => vec![single.clone()],
        };

        // @list container: the term implies the values are an ordered list — strip the
        // {"@list": …} wrapper and emit a bare array.
        //
        // [OPUS-4.8] (sq-oy1f.8) Unwrap ONLY the pure `@list` value(s) (a `{"@list": …}`
        // object with no co-located keys). The W3C Compaction Algorithm scopes the
        // `@list`-container framing to the list value itself; a property carrying BOTH a
        // list and sibling non-list values (e.g. `ex:s ex:prop ( "a" "b" )` AND
        // `ex:s ex:prop "c"`) must NOT drop the siblings. A prior
        // `items.iter().find_map(|v| v.get("@list"))` returned the first `@list` found in
        // ANY item and silently discarded every other value — silent data loss that
        // violated losslessness.
        //
        // pyld (the W3C reference) keeps the list under the container term as a bare array
        // and emits the siblings under the property IRI compacted WITHOUT the
        // `@list`-container term (here: the full / `@vocab`-relative / prefix form, via
        // `compact_iri_no_list`). When there is exactly one pure-list value and no sibling
        // we take the bare-array fast path; otherwise we split and emit both homes so the
        // round-trip stays lossless.
        if container.as_deref() == Some("@list") {
            let is_pure_list = |v: &Json| -> bool {
                matches!(v, Json::Obj(m) if m.len() == 1) && v.get("@list").is_some()
            };
            let lists: Vec<&Json> = items.iter().filter(|v| is_pure_list(v)).collect();
            let siblings: Vec<Json> = items.iter().filter(|v| !is_pure_list(v)).cloned().collect();
            // Emit the list value(s) under the container term as a bare ordered array. A
            // single list collapses to its bare array; multiple lists (rare on the fromRdf
            // path) stay as an array of explicit `{"@list": …}` objects so each survives.
            if !lists.is_empty() {
                if lists.len() == 1 {
                    if let Some(Json::Arr(elems)) = lists[0].get("@list") {
                        let compacted: Vec<Json> = elems
                            .iter()
                            .map(|e| self.compact_value(Some(&term), e))
                            .collect();
                        result.set(&term, Json::Arr(compacted));
                    }
                } else {
                    let arr: Vec<Json> = lists
                        .iter()
                        .filter_map(|v| match v.get("@list") {
                            Some(Json::Arr(elems)) => {
                                let inner: Vec<Json> = elems
                                    .iter()
                                    .map(|e| self.compact_value(Some(&term), e))
                                    .collect();
                                let mut lo = Json::obj();
                                lo.set(&self.compact_keyword("@list"), Json::Arr(inner));
                                Some(lo)
                            }
                            _ => None,
                        })
                        .collect();
                    result.set(&term, Json::Arr(arr));
                }
            }
            // Emit any co-located non-list values under a NON-list key for this IRI, so they
            // are never dropped (matches pyld). The key is the IRI compacted while ignoring
            // any `@list`-container term (here the full / `@vocab`-relative / prefix form);
            // the siblings are then value/node-compacted with the default "compact arrays"
            // framing under that key.
            if !siblings.is_empty() {
                let sib_key = self.compact_iri_no_list(iri, reverse_only);
                let value = self.compact_values_default(&sib_key, &siblings, false);
                result.set(&sib_key, value);
            }
            return;
        }

        // @language container: { "<lang>": "<value(s)>", … } grouping language-tagged strings.
        //
        // [OPUS-4.8] (sq-oy1f.9) A value that lacks a usable `@language` (a plain string with
        // no tag, or a typed/numeric literal) must NOT be dropped.
        //
        // [OPUS-4.8] (sq-oy1f.12) But a language map's VALUES must be STRINGS per the W3C
        // JSON-LD 1.1 spec (and a strict third-party processor like pyld REJECTS the whole
        // document — `language map values must be strings` — if a non-string lands under
        // `@none`). The prior code put a native scalar (`42`) under `@none`, producing a
        // document pyld throws on. So:
        //   * a language-TAGGED string → its language-map slot;
        //   * a PLAIN string (string `@value`, no tag) → the `@none` member (valid, faithful);
        //   * a NON-string value (number/bool/typed literal) → a SEPARATE non-language key
        //     (the IRI compacted ignoring the language term), which preserves the datatype on
        //     read-back (pyld reads `42`^^xsd:integer, not a `"42"` string).
        //
        // [OPUS-4.8] (sq-oy1f.14) Several values may share one language (or `@none`); each
        // language slot accumulates into an ARRAY of strings so none is overwritten/lost (the
        // prior `map.set(lang, …)` silently clobbered an earlier same-language value).
        if container.as_deref() == Some("@language") {
            let mut map = Json::obj();
            let mut nonstring: Vec<Json> = Vec::new();
            // Accumulate string values per key (language tag or `@none`); a key with one
            // string stays a scalar, a key with several becomes an array (spec + pyld).
            let push_str = |map: &mut Json, key: &str, s: &str| match map.get(key).cloned() {
                Some(Json::Arr(mut a)) => {
                    a.push(Json::Str(s.to_string()));
                    map.set(key, Json::Arr(a));
                }
                Some(existing) => map.set(key, Json::Arr(vec![existing, Json::Str(s.to_string())])),
                None => map.set(key, Json::Str(s.to_string())),
            };
            for v in &items {
                match (
                    v.get("@value").and_then(Json::as_str),
                    v.get("@language").and_then(Json::as_str),
                ) {
                    // A language-tagged string → its language slot.
                    (Some(val), Some(lang)) => push_str(&mut map, lang, val),
                    // A plain (untyped) string with no language → the `@none` slot. The
                    // value is a JSON string AND the expanded model carries no `@type`
                    // (untyped strings are emitted as a bare `@value` string by fromRdf).
                    (Some(val), None) if v.get("@type").is_none() => {
                        push_str(&mut map, "@none", val)
                    }
                    // Anything else (a native scalar `Json::Raw`, or a typed value object) is
                    // NOT a valid language-map value — route it to a separate non-language key
                    // so its datatype survives. `compact_value(None, …)` keeps the value
                    // object as-is (no language-term coercion).
                    _ => nonstring.push(self.compact_value(None, v)),
                }
            }
            if let Json::Obj(m) = &map {
                if !m.is_empty() {
                    result.set(&term, map);
                }
            }
            if !nonstring.is_empty() {
                // The IRI compacted WITHOUT the language-container term (the full /
                // `@vocab`-relative / prefix form), so the reader does not re-read these
                // through the language map. Reuse `compact_iri_no_list` — it already skips any
                // container-bearing term and yields exactly that fallback spelling.
                let key = self.compact_iri_no_list(iri, reverse_only);
                let value = self.compact_values_default(&key, &nonstring, false);
                result.set(&key, value);
            }
            return;
        }

        // @index container: { "<index>": <value>, … }. fromRdf does not emit a per-value
        // `@index`, so every value falls under the reserved `@none` member. [OPUS-4.8]
        // (sq-oy1f.14) Several `@none` values accumulate into an array (the index-map value
        // form pyld round-trips), never overwriting one another.
        if container.as_deref() == Some("@index") {
            let mut map = Json::obj();
            for v in &items {
                let idx = v.get("@index").and_then(Json::as_str).unwrap_or("@none");
                let compacted = self.compact_value(Some(&term), v);
                match map.get(idx).cloned() {
                    Some(Json::Arr(mut existing)) => {
                        existing.push(compacted);
                        map.set(idx, Json::Arr(existing));
                    }
                    Some(other) => map.set(idx, Json::Arr(vec![other, compacted])),
                    None => map.set(idx, compacted),
                }
            }
            result.set(&term, map);
            return;
        }

        // [OPUS-4.8] (sq-oy1f.14) `@id` / `@graph` containers: the fromRdf model has no
        // per-value `@index`/`@id` map key and does not nest a named-graph under a property,
        // so sparq cannot LOSSLESSLY populate these container maps. Emitting a node reference
        // under an `@id`/`@graph` container produced a `{"@id": …}` map *value* that a strict
        // processor (pyld) rejects (`illegal key … @id` on a value object) — an invalid
        // document. Falling back to the DEFAULT (no-container) framing emits a plain node
        // reference / value that round-trips faithfully through pyld. (`@id`/`@graph`
        // container framing for an already-indexed input is out of scope — see the module
        // "Honest scope" note; sparq's input is always a graph it produced.)
        if matches!(container.as_deref(), Some("@id") | Some("@graph")) {
            // Emit under a key that does NOT carry the `@id`/`@graph` container, so the reader
            // (and pyld) does not try to read the value as a container map. `compact_iri_no_list`
            // skips container-bearing terms, yielding the `@vocab`-relative / prefix / full-IRI
            // spelling — exactly the plain key we need.
            let key = self.compact_iri_no_list(iri, reverse_only);
            let value = self.compact_values_default(&key, &items, false);
            result.set(&key, value);
            return;
        }

        // [OPUS-4.8] (sq-oy1f.13) `@type:@id` / `@type:@vocab` coercion vs a literal value.
        // A term coerced to `@id`/`@vocab` makes a *bare string* value read back as a node
        // IRI, not a literal. If the chosen term carries that coercion but this IRI also has a
        // plain literal object (a `{"@value": …}` with no `@id`), emitting that literal under
        // the coerced term CORRUPTS it on read-back (a strict processor like pyld reads
        // `"http://ex/x"` as the IRI `<http://ex/x>` — a string literal silently becomes a
        // node). Split: node references stay under the coerced term (their `{"@id"}` collapses
        // to a bare IRI, the point of the coercion); literal values move to a NON-coerced key
        // (the IRI compacted ignoring the coerced term), where they re-expand as literals.
        let coerces_id = def
            .as_ref()
            .and_then(|d| d.type_mapping.as_deref())
            .is_some_and(|t| t == "@id" || t == "@vocab");
        if coerces_id {
            let is_literal = |v: &Json| v.get("@value").is_some();
            let (lits, refs): (Vec<Json>, Vec<Json>) =
                items.into_iter().partition(|v| is_literal(v));
            if !lits.is_empty() {
                // Literals under a non-coerced key (full / @vocab-relative / prefix IRI).
                let lit_key = self.compact_iri_no_list(iri, reverse_only);
                // Guard: if the only available spelling for the IRI IS the coerced term
                // (no alternate), `compact_iri_no_list` still returns the @vocab-relative /
                // full IRI, which is distinct from the term — so the literal key never
                // collides with the coerced term key.
                let lit_val = self.compact_values_default(&lit_key, &lits, false);
                result.set(&lit_key, lit_val);
            }
            if !refs.is_empty() {
                let force_array = matches!(container.as_deref(), Some("@set"));
                let ref_val = self.compact_values_default(&term, &refs, force_array);
                result.set(&term, ref_val);
            }
            return;
        }

        // Default: compact each value, then apply "compact arrays".
        let force_array = matches!(container.as_deref(), Some("@set"));
        let value = self.compact_values_default(&term, &items, force_array);
        result.set(&term, value);
    }

    /// Compacts a list of expanded values under `term` with the default ("no special
    /// container") framing — value/node compaction per item, an explicit `{"@list": …}`
    /// wrapper for any list value, then "compact arrays" (a single value collapses to a
    /// scalar unless `force_array`). Shared by the default property path and by the
    /// sibling path of the `@list`-container split ([OPUS-4.8] sq-oy1f.8) so co-located
    /// non-list values are emitted, never dropped.
    fn compact_values_default(&self, term: &str, items: &[Json], force_array: bool) -> Json {
        let mut compacted: Vec<Json> = Vec::with_capacity(items.len());
        for v in items {
            if let Some(Json::Arr(elems)) = v.get("@list") {
                // A list value with no @list-container term keeps an explicit {"@list": …}.
                let inner: Vec<Json> = elems
                    .iter()
                    .map(|e| self.compact_value(Some(term), e))
                    .collect();
                let mut lo = Json::obj();
                lo.set(&self.compact_keyword("@list"), Json::Arr(inner));
                compacted.push(lo);
                continue;
            }
            if v.is_obj() && v.get("@id").is_some() && v.get("@value").is_none() {
                // A node reference: compact_value handles @id/@vocab coercion; otherwise it
                // is reduced to a node object.
                let cv = self.compact_value(Some(term), v);
                if cv.is_obj() {
                    compacted.push(self.compact_node(&cv));
                } else {
                    compacted.push(cv);
                }
            } else {
                compacted.push(self.compact_value(Some(term), v));
            }
        }

        if compacted.len() == 1 && !force_array {
            compacted.into_iter().next().expect("len 1")
        } else {
            Json::Arr(compacted)
        }
    }
}

// ===========================================================================
// fromRdf — build an expanded JSON-LD model (a Vec<Json> of node objects).
// ===========================================================================

/// Builds the *expanded* JSON-LD value for one object [`Term`], honouring collapsed
/// `@list` heads (`lists`). Mirrors the parent writer's `write_jsonld_object` semantics but
/// yields a [`Json`] AST.
fn term_to_json(term: &Term, lists: &ListInfo) -> Json {
    match term {
        Term::NamedNode(n) => {
            let mut o = Json::obj();
            o.set("@id", Json::Str(n.as_str().to_string()));
            o
        }
        Term::BlankNode(b) => {
            if let Some(elems) = lists.heads.get(b) {
                let mut o = Json::obj();
                let arr: Vec<Json> = elems.iter().map(|e| term_to_json(e, lists)).collect();
                o.set("@list", Json::Arr(arr));
                return o;
            }
            let mut o = Json::obj();
            o.set("@id", Json::Str(format!("_:{}", b.as_str())));
            o
        }
        Term::Literal(l) => literal_to_json(l),
        Term::Triple(t) => {
            // RDF 1.2 triple term — no standard JSON-LD encoding; preserve the canonical
            // N-Triples spelling as an opaque @id (same choice as the parent writer).
            let mut o = Json::obj();
            let mut nt = String::new();
            let _ = write!(nt, "{}", Term::Triple(t.clone()));
            o.set("@id", Json::Str(nt));
            o
        }
    }
}

/// Yields the elements of a `Json::Arr`, or the single value itself otherwise. A small
/// helper for iterating an expanded object array (which is always an array in our model,
/// but stays total).
pub(super) fn flatten(v: &Json) -> Vec<&Json> {
    match v {
        Json::Arr(a) => a.iter().collect(),
        other => vec![other],
    }
}

/// Builds the *expanded* value object for a literal: `{"@value": …}` with `@language`
/// (language-tagged) or `@type` (any non-string datatype). Mirrors the parent writer's
/// `write_jsonld_literal`, including the lossless native-scalar coercion.
fn literal_to_json(lit: &oxrdf::Literal) -> Json {
    let dt = lit.datatype().as_str();
    let mut o = Json::obj();
    if lit.language().is_none() {
        if let Some(native) = coerce_native(lit.value(), dt) {
            o.set("@value", Json::Raw(native));
            return o;
        }
    }
    o.set("@value", Json::Str(lit.value().to_string()));
    if let Some(lang) = lit.language() {
        o.set("@language", Json::Str(lang.to_string()));
    } else if dt != format!("{}string", XSD) && dt != RDF_LANG_STRING {
        o.set("@type", Json::Str(dt.to_string()));
    }
    o
}

/// Builds the expanded node-object array for one graph's triples (the `fromRdf` output for
/// that graph), reusing the parent writer's list detection + first-seen ordering.
pub(super) fn graph_to_expanded(triples: &[Triple]) -> Vec<Json> {
    let lists = detect_lists(triples);
    // Per-node: @type IRIs + predicate→objects (first-seen predicate order).
    struct Node {
        subject: NamedOrBlankNode,
        types: Vec<String>,
        pred_order: Vec<String>,
        preds: BTreeMap<String, Vec<Term>>,
    }
    let mut order_idx: BTreeMap<String, usize> = BTreeMap::new();
    let mut nodes: Vec<Node> = Vec::new();
    let key = |s: &NamedOrBlankNode| -> String {
        match s {
            NamedOrBlankNode::NamedNode(n) => format!("I{}", n.as_str()),
            NamedOrBlankNode::BlankNode(b) => format!("B{}", b.as_str()),
        }
    };
    for t in triples {
        if let NamedOrBlankNode::BlankNode(b) = &t.subject {
            if lists.cells.contains(b) {
                continue; // list cell — content lives inside the @list head
            }
        }
        let k = key(&t.subject);
        let i = *order_idx.entry(k).or_insert_with(|| {
            nodes.push(Node {
                subject: t.subject.clone(),
                types: Vec::new(),
                pred_order: Vec::new(),
                preds: BTreeMap::new(),
            });
            nodes.len() - 1
        });
        let node = &mut nodes[i];
        if t.predicate.as_str() == RDF_TYPE {
            if let Term::NamedNode(n) = &t.object {
                node.types.push(n.as_str().to_string());
                continue;
            }
        }
        let p = t.predicate.as_str().to_string();
        if !node.preds.contains_key(&p) {
            node.pred_order.push(p.clone());
        }
        node.preds.entry(p).or_default().push(t.object.clone());
    }
    // `nodes` is already in first-seen subject order (we push on first sight); `order_idx`
    // only deduplicates. No re-sort.

    let mut result = Vec::with_capacity(nodes.len());
    for node in &nodes {
        let mut obj = Json::obj();
        match &node.subject {
            NamedOrBlankNode::NamedNode(n) => obj.set("@id", Json::Str(n.as_str().to_string())),
            NamedOrBlankNode::BlankNode(b) => {
                obj.set("@id", Json::Str(format!("_:{}", b.as_str())))
            }
        }
        if !node.types.is_empty() {
            let arr: Vec<Json> = node.types.iter().map(|t| Json::Str(t.clone())).collect();
            obj.set("@type", Json::Arr(arr));
        }
        for p in &node.pred_order {
            let arr: Vec<Json> = node.preds[p].iter().map(|o| term_to_json(o, &lists)).collect();
            obj.set(p, Json::Arr(arr));
        }
        result.push(obj);
    }
    result
}

// ===========================================================================
// Public entry points.
// ===========================================================================

/// Serialises an RDF dataset (default + named graphs) as a **compacted** JSON-LD 1.1
/// document against the caller-supplied `context`, applying the full W3C Compaction
/// Algorithm (term definitions, `@vocab`, type/language/`@container` coercion, `@reverse`,
/// `@id`/`@type` keyword aliasing, value + node + IRI compaction).
///
/// The output is a `{"@context": …, "@graph": […]}` document. Round-tripping it through a
/// JSON-LD-to-RDF processor reconstructs the same triples (the compaction is lossless —
/// every coercion it applies is invertible against the same `@context`).
pub fn write_jsonld_compact(graphs: &[NamedGraph<'_>], context: &Json) -> String {
    let active = ActiveContext::parse(context);

    // Build the expanded fromRdf model: default-graph node objects, plus a node object per
    // named graph carrying its own `@graph` array (the JSON-LD dataset shape).
    let default_triples: &[Triple] = graphs
        .iter()
        .find(|(n, _)| n.is_none())
        .map(|(_, ts)| *ts)
        .unwrap_or(&[]);
    let named: Vec<&NamedGraph<'_>> = graphs.iter().filter(|(n, _)| n.is_some()).collect();

    let mut expanded: Vec<Json> = graph_to_expanded(default_triples);
    // Relocate forward edges covered by `@reverse` terms onto their object nodes (per graph
    // scope), so node compaction can express them through the reverse term.
    active.relocate_reverse(&mut expanded);
    for (name, ts) in &named {
        let g = name.as_ref().expect("named graph has a name");
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
        let mut inner = graph_to_expanded(ts);
        active.relocate_reverse(&mut inner);
        node.set("@graph", Json::Arr(inner));
        expanded.push(node);
    }

    // Compact the expanded array under the active context.
    let compacted = active.compact(&Json::Arr(expanded));

    // Build the output document: {"@context": <raw context>, "@graph": [...]}. We keep the
    // `@graph` envelope (rather than collapsing a single node) so the `@context` always has
    // a home and named graphs round-trip.
    let mut doc = Json::obj();
    if !matches!(&active.raw_context, Json::Obj(m) if m.is_empty()) {
        doc.set("@context", active.raw_context.clone());
    }
    let graph_key = active.compact_keyword("@graph");
    match compacted {
        Json::Arr(items) => doc.set(&graph_key, Json::Arr(items)),
        other => doc.set(&graph_key, Json::Arr(vec![other])),
    }

    let mut out = String::new();
    doc.write(&mut out);
    out
}

/// Parses a caller `@context` from its JSON text into the [`Json`] model used by
/// [`write_jsonld_compact`]. A convenience for callers that hold the context as a string
/// (a CLI flag, an HTTP request profile). Returns `None` if the text is not a JSON object.
/// Dependency-free — a tiny recursive-descent JSON parser, no serde_json.
pub fn parse_context_json(text: &str) -> Option<Json> {
    let mut p = JsonParser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return None;
    }
    matches!(v, Json::Obj(_)).then_some(v)
}

/// A minimal recursive-descent JSON parser (objects/arrays/strings/numbers/true/false/null)
/// used only to read a caller `@context` string. Numbers/bools/null become [`Json::Raw`]
/// (we only ever read string-valued context members, but the parser stays total).
struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl JsonParser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Option<Json> {
        self.skip_ws();
        match self.bytes.get(self.pos)? {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => self.parse_string().map(Json::Str),
            b't' => self.parse_lit("true").then(|| Json::Raw("true".into())),
            b'f' => self.parse_lit("false").then(|| Json::Raw("false".into())),
            b'n' => self.parse_lit("null").then(|| Json::Raw("null".into())),
            _ => self.parse_number(),
        }
    }

    fn parse_lit(&mut self, lit: &str) -> bool {
        if self.bytes[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn parse_number(&mut self) -> Option<Json> {
        let start = self.pos;
        while self.pos < self.bytes.len()
            && matches!(
                self.bytes[self.pos],
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'
            )
        {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()
            .map(|s| Json::Raw(s.to_string()))
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return None;
        }
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            self.pos += 1;
            match c {
                b'"' => return Some(s),
                b'\\' => {
                    let e = *self.bytes.get(self.pos)?;
                    self.pos += 1;
                    match e {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'b' => s.push('\u{08}'),
                        b'f' => s.push('\u{0C}'),
                        b'u' => {
                            let hex = self.bytes.get(self.pos..self.pos + 4)?;
                            self.pos += 4;
                            let cp = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            s.push(char::from_u32(cp)?);
                        }
                        _ => return None,
                    }
                }
                _ => {
                    // Re-decode the UTF-8 byte sequence starting at the previous position.
                    let start = self.pos - 1;
                    let mut end = self.pos;
                    while end < self.bytes.len() && (self.bytes[end] & 0xC0) == 0x80 {
                        end += 1;
                    }
                    s.push_str(std::str::from_utf8(&self.bytes[start..end]).ok()?);
                    self.pos = end;
                }
            }
        }
        None
    }

    fn parse_array(&mut self) -> Option<Json> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Some(Json::Arr(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.bytes.get(self.pos)? {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    return Some(Json::Arr(items));
                }
                _ => return None,
            }
        }
    }

    fn parse_object(&mut self) -> Option<Json> {
        self.pos += 1; // '{'
        let mut members = Vec::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Some(Json::Obj(members));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return None;
            }
            self.pos += 1;
            let val = self.parse_value()?;
            members.push((key, val));
            self.skip_ws();
            match self.bytes.get(self.pos)? {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    return Some(Json::Obj(members));
                }
                _ => return None,
            }
        }
    }
}
