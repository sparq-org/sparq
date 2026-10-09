//! Context processing — the active context, term definitions, and the inverse context.
//!
//! [OPUS-4.8] (sq-oy1f.24) / [SONNET-4.6] (sq-90mu3) The document-level **Context
//! Processing** layer of the W3C JSON-LD 1.1 pipeline (design record
//! `research/jsonld-1.1-design.md` §3.1). The public surface is:
//!
//! - `ActiveContext` — the processed result of folding a chain of `@context` definitions
//!   (JSON-LD 1.1 API §3.1), with its default base/vocab/language/direction and its map of
//!   `TermDefinition`s.
//! - `Direction` — a base direction (`ltr` / `rtl`).
//! - `process` — the Context Processing Algorithm (§4) and Create Term Definition (§4.2).
//! - `iri` — IRI Expansion (§5.2), plus RFC 3986 reference resolution.
//! - `inverse` — Inverse Context Creation (§4.3), IRI Compaction (§7.1), and Term
//!   Selection (§7.2) — the compaction-side companions of IRI Expansion (bead `sq-90mu3`).
//!
//! Spec: <https://www.w3.org/TR/json-ld11-api/#context-processing-algorithms>.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::json::Json;

pub(crate) mod budget;
pub mod inverse;
pub mod iri;
pub mod process;

/// A base direction (`@direction`): left-to-right or right-to-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `ltr` — left-to-right.
    Ltr,
    /// `rtl` — right-to-left.
    Rtl,
}

impl Direction {
    /// The spec string (`"ltr"` / `"rtl"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Ltr => "ltr",
            Direction::Rtl => "rtl",
        }
    }

    /// Parses a base direction string, or `None` if it is neither `"ltr"` nor `"rtl"`.
    pub fn parse(s: &str) -> Option<Direction> {
        match s {
            "ltr" => Some(Direction::Ltr),
            "rtl" => Some(Direction::Rtl),
            _ => None,
        }
    }
}

/// A tri-state term-scoped override for `@language` / `@direction`: whether the term
/// definition leaves the value **unset** (fall back to the context default), sets it
/// explicitly to **null** (no value), or binds a concrete value.
///
/// Modelled explicitly because the null-vs-unset distinction is load-bearing during
/// expansion: an unset language falls back to the active context's default language,
/// whereas an explicit null suppresses it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Override<T> {
    /// Not set by the term definition — fall back to the context default.
    #[default]
    Unset,
    /// Explicitly set to null — suppress the value.
    Null,
    /// Bound to a concrete value.
    Set(T),
}

impl<T> Override<T> {
    /// True iff this override was set by the term definition (either to null or a value).
    pub fn is_set(&self) -> bool {
        !matches!(self, Override::Unset)
    }
}

/// One **term definition** from an active context (JSON-LD 1.1 API §3.2).
///
/// A term definition is created by the Create Term Definition algorithm. A term whose
/// definition maps to *nothing* (`{"@id": null}` or a plain `null` context value) is a
/// retained definition with a **null IRI mapping** ([`TermDefinition::iri`] returns
/// `None`): it is recorded so expansion drops the term (and a later redefinition can be
/// detected) rather than treating the term as undefined.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TermDefinition {
    // The fully-expanded IRI mapping (`@id`). `None` means the definition has no IRI
    // mapping (an explicit `@id: null` on an otherwise-present definition).
    pub(crate) iri: Option<String>,
    // `@reverse` — this term is a reverse property.
    pub(crate) reverse: bool,
    // `@type` — the type mapping (`@id`, `@vocab`, `@json`, `@none`, or an IRI).
    pub(crate) type_mapping: Option<String>,
    // `@language` — tri-state (unset / null / value).
    pub(crate) language: Override<String>,
    // `@direction` — tri-state (unset / null / value).
    pub(crate) direction: Override<Direction>,
    // `@index` — an index mapping.
    pub(crate) index: Option<String>,
    // `@container` — the container mapping, as an ordered list of keywords.
    pub(crate) container: Vec<String>,
    // `@nest` — a nest mapping.
    pub(crate) nest: Option<String>,
    // `@context` — the raw scoped local context (validated but re-processed on use).
    pub(crate) context: Option<Json>,
    // The base URL in effect when this definition was created (for scoped-context resolution).
    pub(crate) base_url: Option<String>,
    // `@prefix` — this term may be used as the prefix of a compact IRI.
    pub(crate) prefix: bool,
    // `@protected` — this definition may not be redefined by a later context.
    pub(crate) protected: bool,
}

impl TermDefinition {
    /// The fully-expanded IRI mapping (`@id`), or `None` if the definition has no IRI
    /// mapping.
    pub fn iri(&self) -> Option<&str> {
        self.iri.as_deref()
    }

    /// True iff this is a reverse property (`@reverse`).
    pub fn is_reverse(&self) -> bool {
        self.reverse
    }

    /// The type mapping (`@type`), if any.
    pub fn type_mapping(&self) -> Option<&str> {
        self.type_mapping.as_deref()
    }

    /// The term-scoped language override (`@language`).
    pub fn language(&self) -> &Override<String> {
        &self.language
    }

    /// The term-scoped direction override (`@direction`).
    pub fn direction(&self) -> &Override<Direction> {
        &self.direction
    }

    /// The index mapping (`@index`), if any.
    pub fn index(&self) -> Option<&str> {
        self.index.as_deref()
    }

    /// The container mapping (`@container`) as an ordered slice of keywords.
    pub fn container(&self) -> &[String] {
        &self.container
    }

    /// The nest mapping (`@nest`), if any.
    pub fn nest(&self) -> Option<&str> {
        self.nest.as_deref()
    }

    /// The raw scoped local context (`@context`), if any.
    pub fn context(&self) -> Option<&Json> {
        self.context.as_ref()
    }

    /// True iff this term may be used as the prefix of a compact IRI (`@prefix`).
    pub fn is_prefix(&self) -> bool {
        self.prefix
    }

    /// True iff this definition is protected (`@protected`) against redefinition.
    pub fn is_protected(&self) -> bool {
        self.protected
    }

    /// True iff two definitions are equal ignoring only the `@protected` flag — the
    /// comparison the spec uses to decide whether a redefinition of a protected term is a
    /// no-op (allowed) or a `protected term redefinition` error.
    pub(crate) fn same_except_protected(&self, other: &TermDefinition) -> bool {
        let a = TermDefinition {
            protected: false,
            ..self.clone()
        };
        let b = TermDefinition {
            protected: false,
            ..other.clone()
        };
        a == b
    }
}

/// The **active context**: the processed result of applying a chain of `@context`
/// definitions (JSON-LD 1.1 API §3.1). Construct one with [`ActiveContext::new`] and fold
/// contexts into it with [`ActiveContext::process`].
#[derive(Clone, Debug, Default)]
pub struct ActiveContext {
    // Term → definition. A present key is a defined term (its definition may carry a null
    // IRI mapping, i.e. the term expands to null / is dropped); an absent key means the
    // term is undefined.
    //
    // [FABLE-5] (sq-hmd7l.42) Behind an `Arc` so `ActiveContext::clone` is O(1): the
    // Expansion Algorithm clones the active context per node object (§5.1.2 steps 7–11)
    // and Context Processing clones it per fold (§4.1.2 step 1), which on a context-heavy
    // document (~100 term definitions) made the deep `BTreeMap` clone >50% of expand time
    // (measured with `perf`, work box, NON-canonical). Mutation goes through
    // [`ActiveContext::term_definitions_mut`] (copy-on-write via `Arc::make_mut`), so a
    // shared map is deep-copied at most once per actual context-processing operation —
    // the same amortisation jsonld.js gets from its resolved-context cache.
    pub(crate) term_definitions: Arc<BTreeMap<String, TermDefinition>>,
    // `@base` — the current base IRI for resolving relative references.
    pub(crate) base_iri: Option<String>,
    // The document's original base URL, restored when a context is nullified.
    pub(crate) original_base_url: Option<String>,
    // `@vocab` — the vocabulary mapping.
    pub(crate) vocabulary_mapping: Option<String>,
    // `@language` — the default language.
    pub(crate) default_language: Option<String>,
    // `@direction` — the default base direction.
    pub(crate) default_base_direction: Option<Direction>,
    // The previous context, retained for `@propagate: false` reversion during expansion.
    // `Arc` (not `Box`) so cloning an active context never deep-copies the chain
    // ([FABLE-5] sq-hmd7l.42); the previous context is only ever replaced, never mutated
    // in place.
    pub(crate) previous_context: Option<Arc<ActiveContext>>,
}

impl ActiveContext {
    /// A newly-initialized active context whose base IRI and original base URL are `base`
    /// (JSON-LD 1.1 API §4.1.2 step 1) and which has no term definitions.
    pub fn new(base: Option<&str>) -> ActiveContext {
        ActiveContext {
            term_definitions: Arc::new(BTreeMap::new()),
            base_iri: base.map(str::to_string),
            original_base_url: base.map(str::to_string),
            vocabulary_mapping: None,
            default_language: None,
            default_base_direction: None,
            previous_context: None,
        }
    }

    /// The current base IRI (`@base`), if any.
    pub fn base_iri(&self) -> Option<&str> {
        self.base_iri.as_deref()
    }

    /// The vocabulary mapping (`@vocab`), if any.
    pub fn vocabulary_mapping(&self) -> Option<&str> {
        self.vocabulary_mapping.as_deref()
    }

    /// The default language (`@language`), if any.
    pub fn default_language(&self) -> Option<&str> {
        self.default_language.as_deref()
    }

    /// The default base direction (`@direction`), if any.
    pub fn default_base_direction(&self) -> Option<Direction> {
        self.default_base_direction
    }

    /// The term definition for `term`, or `None` if `term` is undefined. A defined term
    /// with a null IRI mapping still returns `Some` (its [`TermDefinition::iri`] is `None`).
    pub fn term_definition(&self, term: &str) -> Option<&TermDefinition> {
        self.term_definitions.get(term)
    }

    /// True iff `term` is defined in this context (even with a null IRI mapping).
    pub fn has_term(&self, term: &str) -> bool {
        self.term_definitions.contains_key(term)
    }

    /// The number of defined terms.
    pub fn term_count(&self) -> usize {
        self.term_definitions.len()
    }

    /// True iff the context contains any protected term definition (used by the null-context
    /// nullification guard, §4.1.2 step 5.1.1).
    pub(crate) fn has_protected_terms(&self) -> bool {
        self.term_definitions.values().any(TermDefinition::is_protected)
    }

    /// Mutable access to the term-definition map, copy-on-write ([FABLE-5] sq-hmd7l.42).
    ///
    /// The map sits behind an `Arc` so [`Clone`] is O(1); this deep-copies the map iff it
    /// is currently shared with another active context (`Arc::make_mut`), so a burst of
    /// Create Term Definition inserts into one context pays at most one copy.
    pub(crate) fn term_definitions_mut(&mut self) -> &mut BTreeMap<String, TermDefinition> {
        Arc::make_mut(&mut self.term_definitions)
    }
}

/// The set of JSON-LD 1.1 keywords recognised by the processing algorithms.
///
/// [FABLE-5] sq-oy1f.29 — the Framing keywords (`@default`, `@embed`, `@explicit`,
/// `@omitDefault`, `@preserve`, `@requireAll`) ARE keywords of the grammar
/// (json-ld11-framing "Syntax Tokens and Keywords"). Without them here, Expansion
/// step 13.3 dropped them as free terms before the `frameExpansion` keyword arm
/// could retain them, so expanded frames lost their flags.
pub(crate) const KEYWORDS: &[&str] = &[
    "@base",
    "@container",
    "@context",
    "@default",
    "@direction",
    "@embed",
    "@explicit",
    "@graph",
    "@id",
    "@import",
    "@included",
    "@index",
    "@json",
    "@language",
    "@list",
    "@nest",
    "@none",
    "@omitDefault",
    "@prefix",
    "@preserve",
    "@propagate",
    "@protected",
    "@requireAll",
    "@reverse",
    "@set",
    "@type",
    "@value",
    "@version",
    "@vocab",
];

/// True iff `s` is one of the recognised JSON-LD 1.1 [`KEYWORDS`].
pub(crate) fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

/// True iff `s` has the *form* of a keyword — `@` followed by one or more ASCII letters
/// and nothing else — regardless of whether it is a recognised keyword. A token with this
/// form that is **not** a recognised keyword is silently ignored (not treated as an IRI).
pub(crate) fn has_keyword_form(s: &str) -> bool {
    match s.strip_prefix('@') {
        Some(rest) => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphabetic()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_roundtrips() {
        assert_eq!(Direction::Ltr.as_str(), "ltr");
        assert_eq!(Direction::Rtl.as_str(), "rtl");
        assert_eq!(Direction::parse("ltr"), Some(Direction::Ltr));
        assert_eq!(Direction::parse("rtl"), Some(Direction::Rtl));
        assert_eq!(Direction::parse("upside-down"), None);
    }

    #[test]
    fn override_is_set_discriminates() {
        assert!(!Override::<String>::Unset.is_set());
        assert!(Override::<String>::Null.is_set());
        assert!(Override::Set("en".to_string()).is_set());
        assert_eq!(Override::<String>::default(), Override::Unset);
    }

    #[test]
    fn new_active_context_sets_base_and_is_empty() {
        let ac = ActiveContext::new(Some("http://example.org/"));
        assert_eq!(ac.base_iri(), Some("http://example.org/"));
        assert_eq!(ac.vocabulary_mapping(), None);
        assert_eq!(ac.default_language(), None);
        assert_eq!(ac.default_base_direction(), None);
        assert_eq!(ac.term_count(), 0);
        assert!(!ac.has_term("x"));
        assert_eq!(ac.term_definition("x"), None);

        let none = ActiveContext::new(None);
        assert_eq!(none.base_iri(), None);
    }

    #[test]
    fn term_definition_accessors_expose_the_fields() {
        let def = TermDefinition {
            iri: Some("http://example.org/name".to_string()),
            reverse: true,
            type_mapping: Some("@id".to_string()),
            language: Override::Set("en".to_string()),
            direction: Override::Null,
            index: Some("http://example.org/idx".to_string()),
            container: vec!["@set".to_string()],
            nest: Some("@nest".to_string()),
            context: Some(Json::obj()),
            base_url: Some("http://example.org/".to_string()),
            prefix: true,
            protected: true,
        };
        assert_eq!(def.iri(), Some("http://example.org/name"));
        assert!(def.is_reverse());
        assert_eq!(def.type_mapping(), Some("@id"));
        assert_eq!(def.language(), &Override::Set("en".to_string()));
        assert_eq!(def.direction(), &Override::Null);
        assert_eq!(def.index(), Some("http://example.org/idx"));
        assert_eq!(def.container(), &["@set".to_string()]);
        assert_eq!(def.nest(), Some("@nest"));
        assert_eq!(def.context(), Some(&Json::obj()));
        assert!(def.is_prefix());
        assert!(def.is_protected());
    }

    #[test]
    fn same_except_protected_ignores_only_the_protected_flag() {
        let base = TermDefinition {
            iri: Some("http://example.org/a".to_string()),
            ..Default::default()
        };
        let protected = TermDefinition {
            protected: true,
            ..base.clone()
        };
        assert!(base.same_except_protected(&protected));
        let different = TermDefinition {
            iri: Some("http://example.org/b".to_string()),
            ..base.clone()
        };
        assert!(!base.same_except_protected(&different));
    }

    #[test]
    fn keyword_predicates() {
        assert!(is_keyword("@id"));
        assert!(is_keyword("@vocab"));
        assert!(!is_keyword("@id2"));
        assert!(!is_keyword("id"));

        assert!(has_keyword_form("@id"));
        assert!(has_keyword_form("@notakeyword"));
        assert!(!has_keyword_form("@id2")); // digit breaks the ALPHA-only form
        assert!(!has_keyword_form("@"));
        assert!(!has_keyword_form("foo"));
    }

    /// [FABLE-5] (sq-hmd7l.42) The COW contract behind the O(1) clone: a clone SHARES the
    /// term-definition map (no deep copy — this is the perf fix), and a subsequent
    /// mutation of either side copies-on-write instead of aliasing through to the other.
    #[test]
    fn clone_shares_term_definitions_and_mutation_does_not_alias() {
        let mut original = ActiveContext::new(None);
        original.term_definitions_mut().insert(
            "a".to_string(),
            TermDefinition {
                iri: Some("http://example.org/a".to_string()),
                ..Default::default()
            },
        );

        let mut cloned = original.clone();
        // The clone shares the map (O(1) clone), it does not deep-copy it.
        assert!(Arc::ptr_eq(&original.term_definitions, &cloned.term_definitions));

        // Mutating the clone copies-on-write: the original keeps its view.
        cloned.term_definitions_mut().insert(
            "b".to_string(),
            TermDefinition {
                iri: Some("http://example.org/b".to_string()),
                ..Default::default()
            },
        );
        assert!(!Arc::ptr_eq(&original.term_definitions, &cloned.term_definitions));
        assert!(cloned.has_term("b"));
        assert!(!original.has_term("b"), "mutating a clone must not alias into the original");
        assert!(original.has_term("a") && cloned.has_term("a"));

        // Removal through the mutable accessor is COW-isolated too.
        cloned.term_definitions_mut().remove("a");
        assert!(!cloned.has_term("a"));
        assert!(original.has_term("a"));
    }

    #[test]
    fn has_protected_terms_detects_a_protected_definition() {
        let mut ac = ActiveContext::new(None);
        ac.term_definitions_mut().insert(
            "a".to_string(),
            TermDefinition {
                protected: true,
                ..Default::default()
            },
        );
        assert!(ac.has_protected_terms());
        let mut plain = ActiveContext::new(None);
        plain
            .term_definitions_mut()
            .insert("b".to_string(), TermDefinition::default());
        // A defined term with a null IRI mapping is retained but is not protected.
        plain.term_definitions_mut().insert(
            "c".to_string(),
            TermDefinition {
                iri: None,
                ..Default::default()
            },
        );
        assert!(!plain.has_protected_terms());
    }
}
