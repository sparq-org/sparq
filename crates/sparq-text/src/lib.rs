// [OPUS-4.8] sq-jxl0: single-source the crate overview from README.md so crates.io
// (package.readme) and the docs.rs front page render identical content. The README's
// `## Library use` fence is a compiled doctest; the `query_text` fence is `engine`-gated.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

pub mod complete;
#[cfg(feature = "fuzzy")]
pub mod fuzzy;
pub mod index;
#[cfg(feature = "engine")]
pub mod rewrite;
#[cfg(feature = "highlight")]
pub mod snippet;
pub mod tokenize;

pub use complete::{Candidate, CandidateKind, CompletionIndex};
#[cfg(feature = "fuzzy")]
pub use fuzzy::{FuzzyError, FuzzyHit, DEFAULT_MAX_DISTANCE, MAX_DISTANCE};
pub use index::{Hit, TermStats, TextIndex};
#[cfg(feature = "engine")]
pub use rewrite::{prepare_text, query_text, query_text_with_budget, rewrite_query};
#[cfg(feature = "highlight")]
pub use snippet::{snippet, Snippet};
pub use tokenize::Analyzer;

/// The `text:` vocabulary — magic predicates recognised by `rewrite`
/// (`http://sparq.dev/text#`, the sparq extension namespace).
pub mod vocab {
    /// `text:` — the sparq full-text-search namespace.
    pub const TEXT_NS: &str = "http://sparq.dev/text#";
    /// `?lit text:matches "query"` — `?lit` ranges over indexed literals
    /// containing EVERY query token (tokens ending in `*` match as prefixes).
    pub const MATCHES: &str = "http://sparq.dev/text#matches";
    /// `?lit text:matchesAny "query"` — literals containing AT LEAST ONE token.
    pub const MATCHES_ANY: &str = "http://sparq.dev/text#matchesAny";
    /// `?lit text:fuzzy "term"` — typo-tolerant single-token lookup, available
    /// only with the `fuzzy` feature. The edit-distance budget defaults to one
    /// and is set by a `?lit text:maxDistance N` companion (hard-capped at two).
    /// Results are ranked by distance, then BM25, then matched token. [GPT-5.6]
    #[cfg(feature = "fuzzy")]
    pub const FUZZY: &str = "http://sparq.dev/text#fuzzy";
    /// `?lit text:maxDistance N` — sets the bounded Levenshtein distance for a
    /// `text:fuzzy` request on the same variable (`0..=2`). [GPT-5.6]
    #[cfg(feature = "fuzzy")]
    pub const MAX_DISTANCE: &str = "http://sparq.dev/text#maxDistance";
    /// `?lit text:phrase "foo bar"` — literals where the query's tokens appear
    /// ADJACENT and IN ORDER (a positional phrase match, not a BM25 ranking).
    /// Requires a positions-enabled index (`TextIndex::build_with_positions`);
    /// the cheap default index stores no positions and is rejected at rewrite
    /// time. No `text:score` companion (a phrase match is boolean adjacency).
    /// [OPUS-4.8]
    pub const PHRASE: &str = "http://sparq.dev/text#phrase";
    /// `?lit text:near "foo bar"` — the proximity/slop generalisation of
    /// `text:phrase`: literals where the tokens occur IN ORDER within a bounded
    /// total gap, RELEVANCE-RANKED (tighter clustering scores higher). The gap
    /// budget defaults to `rewrite::DEFAULT_SLOP`; set it
    /// with a `?lit text:slop N` companion (a non-negative integer literal) on
    /// the same subject variable in the same basic graph pattern. Unlike
    /// `text:phrase` (boolean adjacency) it IS scored, so it also takes an
    /// optional `text:score ?s` companion. Requires a positions-enabled index
    /// (`TextIndex::build_with_positions`); `text:near "foo bar"` at slop 0 is
    /// exactly `text:phrase "foo bar"`. [OPUS-4.8]
    pub const NEAR: &str = "http://sparq.dev/text#near";
    /// `?lit text:slop N` — sets the proximity gap budget for the `text:near`
    /// on the same subject variable in the same basic graph pattern (a
    /// non-negative `xsd:integer`). Only meaningful alongside `text:near`.
    /// [OPUS-4.8]
    pub const SLOP: &str = "http://sparq.dev/text#slop";
    /// `?lit text:score ?s` — binds the relevance score of `?lit`'s match: the
    /// BM25 score for a `text:matches`/`text:matchesAny`, or the proximity score
    /// for a `text:near` (`1/(1+gap)`). Must accompany exactly one such scored
    /// match on the same subject variable in the same basic graph pattern. NOT
    /// valid for `text:phrase` (boolean adjacency, unscored). [OPUS-4.8]
    pub const SCORE: &str = "http://sparq.dev/text#score";
}
