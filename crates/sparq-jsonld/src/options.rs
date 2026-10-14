//! [`JsonLdOptions`] — the JSON-LD 1.1 processing options, modelled once and honoured
//! everywhere in the pipeline (design record §2 goal 5, §3.1 `options.rs`).
//!
//! (sq-oy1f.23) Introduced as the option **shape** and its spec defaults; each
//! field's doc names the spec option it mirrors. (`sq-ghdfw`) The algorithms that
//! read these fields have since landed (`sq-oy1f.24`+ — context processing, expansion,
//! flattening, compaction, framing, from-RDF).

use crate::json::Json;

/// The JSON-LD processing mode (`@version`). Defaults to [`ProcessingMode::JsonLd11`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessingMode {
    /// `json-ld-1.0` — the legacy 1.0 processing mode.
    JsonLd10,
    /// `json-ld-1.1` — the default.
    #[default]
    JsonLd11,
}

impl ProcessingMode {
    /// The spec string (`"json-ld-1.0"` / `"json-ld-1.1"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessingMode::JsonLd10 => "json-ld-1.0",
            ProcessingMode::JsonLd11 => "json-ld-1.1",
        }
    }
}

/// How base-direction and language are serialised to RDF (`rdfDirection`). Defaults to
/// [`RdfDirection::None`] — no special direction handling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RdfDirection {
    /// No direction serialisation (the default).
    #[default]
    None,
    /// `i18n-datatype` — encode language+direction in an `https://www.w3.org/ns/i18n#`
    /// datatype IRI.
    I18nDatatype,
    /// `compound-literal` — encode direction via a compound-literal blank node.
    CompoundLiteral,
}

impl RdfDirection {
    /// The spec string, or `None` for [`RdfDirection::None`].
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            RdfDirection::None => Option::None,
            RdfDirection::I18nDatatype => Some("i18n-datatype"),
            RdfDirection::CompoundLiteral => Some("compound-literal"),
        }
    }
}

/// The framing `@embed` flag — how node objects are embedded in the framed output.
/// Defaults to [`EmbedFlag::Once`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmbedFlag {
    /// `@always` — always embed the node object.
    Always,
    /// `@once` — embed a node object the first time it is referenced (the default).
    #[default]
    Once,
    /// `@never` — never embed; emit a node reference (`{ "@id": … }`).
    Never,
    /// `@link` — identity-preserving embedding (documented as an `@once` fallback until a
    /// consumer needs real object identity — design record §11).
    Link,
}

impl EmbedFlag {
    /// The spec string (`"@always"` / `"@once"` / `"@never"` / `"@link"`).
    pub fn as_str(self) -> &'static str {
        match self {
            EmbedFlag::Always => "@always",
            EmbedFlag::Once => "@once",
            EmbedFlag::Never => "@never",
            EmbedFlag::Link => "@link",
        }
    }
}

/// The JSON-LD 1.1 processing options. Construct with [`JsonLdOptions::default`] and adjust
/// the fields you need; the defaults are the specification's defaults.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct JsonLdOptions {
    /// `base` — the base IRI against which relative IRIs are resolved.
    pub base: Option<String>,
    /// `processingMode` — `json-ld-1.1` by default.
    pub processing_mode: ProcessingMode,
    /// `expandContext` — a context used to initialise the active context. Per the JSON-LD
    /// API this may be an IRI string (carried as a [`Json::Str`]), an inline context object
    /// (a [`Json::Obj`]), or an array of any of these (a [`Json::Arr`]), matching the
    /// context forms accepted by the Context Processing algorithm.
    pub expand_context: Option<Json>,
    /// `rdfDirection` — how base direction is serialised to RDF.
    pub rdf_direction: RdfDirection,
    /// `compactArrays` — collapse single-element arrays during compaction (default `true`).
    pub compact_arrays: bool,
    /// `compactToRelative` — compact IRIs to relative form against `base` (default `true`).
    pub compact_to_relative: bool,
    /// `ordered` — sort map keys/array entries for deterministic output (default `false`).
    pub ordered: bool,
    /// `produceGeneralizedRdf` — allow blank-node predicates in `toRdf` (default `false`).
    pub produce_generalized_rdf: bool,
    /// `frameExpansion` — expand in framing mode (default `false`).
    pub frame_expansion: bool,
    /// `extractAllScripts` — extract every `application/ld+json` script from an HTML input
    /// (default `false`).
    pub extract_all_scripts: bool,
    /// Framing `@embed` flag (default [`EmbedFlag::Once`]).
    pub embed: EmbedFlag,
    /// Framing `@explicit` flag (default `false`).
    pub explicit: bool,
    /// Framing `@omitDefault` flag (default `false`).
    pub omit_default: bool,
    /// Framing `@requireAll` flag (default `false`).
    pub require_all: bool,
}

impl Default for JsonLdOptions {
    fn default() -> Self {
        JsonLdOptions {
            base: None,
            processing_mode: ProcessingMode::JsonLd11,
            expand_context: None,
            rdf_direction: RdfDirection::None,
            compact_arrays: true,
            compact_to_relative: true,
            ordered: false,
            produce_generalized_rdf: false,
            frame_expansion: false,
            extract_all_scripts: false,
            embed: EmbedFlag::Once,
            explicit: false,
            omit_default: false,
            require_all: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let o = JsonLdOptions::default();
        assert_eq!(o.base, None);
        assert_eq!(o.processing_mode, ProcessingMode::JsonLd11);
        assert_eq!(o.expand_context, None);
        assert_eq!(o.rdf_direction, RdfDirection::None);
        assert!(o.compact_arrays);
        assert!(o.compact_to_relative);
        assert!(!o.ordered);
        assert!(!o.produce_generalized_rdf);
        assert!(!o.frame_expansion);
        assert!(!o.extract_all_scripts);
        assert_eq!(o.embed, EmbedFlag::Once);
        assert!(!o.explicit);
        assert!(!o.omit_default);
        assert!(!o.require_all);
    }

    #[test]
    fn enum_defaults() {
        assert_eq!(ProcessingMode::default(), ProcessingMode::JsonLd11);
        assert_eq!(RdfDirection::default(), RdfDirection::None);
        assert_eq!(EmbedFlag::default(), EmbedFlag::Once);
    }

    #[test]
    fn processing_mode_as_str() {
        assert_eq!(ProcessingMode::JsonLd10.as_str(), "json-ld-1.0");
        assert_eq!(ProcessingMode::JsonLd11.as_str(), "json-ld-1.1");
    }

    #[test]
    fn rdf_direction_as_str() {
        assert_eq!(RdfDirection::None.as_str(), None);
        assert_eq!(RdfDirection::I18nDatatype.as_str(), Some("i18n-datatype"));
        assert_eq!(RdfDirection::CompoundLiteral.as_str(), Some("compound-literal"));
    }

    #[test]
    fn embed_flag_as_str() {
        assert_eq!(EmbedFlag::Always.as_str(), "@always");
        assert_eq!(EmbedFlag::Once.as_str(), "@once");
        assert_eq!(EmbedFlag::Never.as_str(), "@never");
        assert_eq!(EmbedFlag::Link.as_str(), "@link");
    }
}
