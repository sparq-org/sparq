//! The shared tokenizer: UAX #29 Unicode word segmentation + Unicode
//! lowercasing.
//!
//! Design (deliberately minimal — deterministic and language-neutral):
//!
//! - **Segmentation**: `unicode-segmentation`'s `unicode_words` (UAX #29 word
//!   boundaries, keeping only word-like segments — punctuation and whitespace
//!   never become tokens; numbers like `42` or `3.14` do).
//! - **Folding**: full Unicode lowercase (`str::to_lowercase`) — so `Fox` ==
//!   `fox`, `ΣΟΦΙΑ` == `σοφια`, `STRASSE` == `strasse`. No stemming, no
//!   stopword removal, no diacritic stripping (`café` ≠ `cafe`): exact-token
//!   semantics are predictable across languages and never throw data away.
//! - **CJK caveat (default analyzer)**: there is no dictionary-based word
//!   splitting. UAX #29 emits each Han/CJK ideograph as its OWN word segment
//!   (`東京都` → `東`,`京`,`都`), so an unspaced ideograph run becomes one token
//!   per character — a query token matches a document iff that ideograph occurs
//!   in it, and a multi-char CJK term behaves like the AND of its characters
//!   (use `phrase` for adjacency, which then requires the characters
//!   consecutive). Kana/Hangul runs that UAX #29 treats as a single word stay
//!   one token. (Empirically pinned in `tests/analyzer_edges.rs`.)
//! - **CJK n-gram analyzer (opt-in, [`Analyzer::CjkNgram`])**: for languages
//!   without inter-word spacing, per-ideograph tokens make a multi-char term the
//!   AND of its (very common) characters — low precision (`東京` matches any doc
//!   merely containing 東 *and* 京 anywhere). The opt-in bigram analyzer instead
//!   emits the OVERLAPPING character bigrams of each maximal CJK run (`東京都` →
//!   `東京`,`京都`), so a query for `東京` is the AND of *its* bigrams and only
//!   matches docs where that pair actually co-occurs. A lone CJK ideograph (a
//!   one-character run, or one isolated between non-CJK) still emits the single
//!   character as a unigram, so single-char queries keep working. Non-CJK text
//!   is segmented exactly as the default analyzer — the two analyzers agree on
//!   every non-CJK input. This is the standard CGImm/n-gram alternative to a
//!   heavyweight dictionary segmenter (e.g. lindera), with zero extra
//!   dependencies and full determinism. It is OPT-IN and the index must be
//!   queried with the same analyzer it was built with. sq-m3ln
//!
//! Query strings go through [`tokenize_query`], which additionally recognises
//! a trailing `*` on a token as a PREFIX (autocomplete) marker: `auto*`
//! matches `automatic`, `autonomous`, …. The `*` is only meaningful
//! immediately after a token; anywhere else it is segmentation punctuation
//! and disappears. (A `*` after a CJK run is segmentation punctuation under the
//! n-gram analyzer too — bigrams are never prefix tokens.)

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

/// Lowercases a segmented word, borrowing the overwhelmingly common already-
/// lowercase ASCII case. Non-ASCII input deliberately takes the existing full
/// Unicode lowercase path, even when it happens to be lowercase already, so
/// folding remains exactly equivalent to [`str::to_lowercase`].
#[inline]
fn lowercase_word(word: &str) -> Cow<'_, str> {
    if word.bytes().all(|byte| byte.is_ascii() && !byte.is_ascii_uppercase()) {
        Cow::Borrowed(word)
    } else {
        Cow::Owned(word.to_lowercase())
    }
}

/// The analyzer (tokenization strategy) a [`TextIndex`](crate::TextIndex) is
/// built and queried with. The default is language-neutral UAX #29 word
/// segmentation; [`Analyzer::CjkNgram`] is the opt-in CJK character-bigram
/// variant (see the module docs). An index records its analyzer so build and
/// query always agree. sq-m3ln
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Analyzer {
    /// UAX #29 Unicode word segmentation + Unicode lowercasing (the historical
    /// default): unspaced CJK runs split per ideograph.
    #[default]
    Unicode,
    /// Like [`Analyzer::Unicode`] for non-CJK text, but emits the overlapping
    /// character bigrams of each maximal CJK ideograph run (`東京都` →
    /// `東京`,`京都`); a one-character run stays a single-char unigram.
    /// sq-m3ln
    CjkNgram,
}

/// Whether `c` is a CJK ideograph the n-gram analyzer bigrams. Covers the Han
/// (Unified + Extensions + Compatibility) blocks; kana and Hangul are
/// intentionally EXCLUDED — UAX #29 already groups their runs into words, so the
/// per-ideograph problem (and thus the n-gram fix) is specific to Han.
fn is_cjk_ideograph(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF      // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0x20000..=0x2A6DF  // Extension B
        | 0x2A700..=0x2EBEF  // Extensions C–F
        | 0x2F800..=0x2FA1F  // Compatibility Ideographs Supplement
        | 0x30000..=0x3134F  // Extension G
    )
}

/// Tokenizes document text (a literal's lexical form) with the default
/// [`Analyzer::Unicode`]: UAX #29 words, lowercased, in order (duplicates
/// preserved — term frequency matters).
pub fn tokenize(text: &str) -> Vec<String> {
    tokenize_with(text, Analyzer::Unicode)
}

/// Tokenizes document text with `analyzer`. [`Analyzer::Unicode`] is exactly
/// [`tokenize`]; [`Analyzer::CjkNgram`] additionally rewrites each maximal run
/// of CJK ideograph tokens into its overlapping lowercase character bigrams
/// (see the module docs). sq-m3ln
pub fn tokenize_with(text: &str, analyzer: Analyzer) -> Vec<String> {
    let words = text.unicode_words().map(lowercase_word).map(Cow::into_owned);
    match analyzer {
        Analyzer::Unicode => words.collect(),
        Analyzer::CjkNgram => cjk_ngram_stream(words),
    }
}

/// Whether a UAX #29 word is a single CJK ideograph — the unit the n-gram
/// analyzer accumulates into runs. UAX #29 already isolates each Han ideograph
/// as its own word (the very behaviour this analyzer compensates for), so a
/// run of ideographs arrives as a sequence of single-char words; we stitch
/// CONSECUTIVE such words back together and bigram them. sq-m3ln
fn is_cjk_word(word: &str) -> bool {
    let mut chars = word.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if is_cjk_ideograph(c))
}

/// Applies the CJK-bigram rewrite to a stream of already-lowercased UAX #29
/// words: a maximal run of CONSECUTIVE single-ideograph words (`東`,`京`,`都`)
/// becomes its overlapping bigrams (`東京`,`京都`); a one-character run stays a
/// single unigram; every non-CJK word passes through unchanged. Non-CJK input
/// therefore yields exactly the default analyzer's tokens. sq-m3ln
fn cjk_ngram_stream(words: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    // Buffer the current maximal run of single-ideograph words.
    let mut run: Vec<char> = Vec::new();
    let flush = |run: &mut Vec<char>, out: &mut Vec<String>| {
        match run.len() {
            0 => {}
            // A lone ideograph stays a single-char unigram (single-char queries
            // keep working).
            1 => out.push(run[0].to_string()),
            // ≥2 ideographs: overlapping character bigrams.
            _ => out.extend(run.windows(2).map(|pair| pair.iter().collect::<String>())),
        }
        run.clear();
    };
    for w in words {
        if is_cjk_word(&w) {
            run.push(w.chars().next().expect("is_cjk_word => exactly one char"));
        } else {
            flush(&mut run, &mut out);
            out.push(w);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// One parsed query token: the lowercased token text and whether it is a
/// prefix (`*`-suffixed) token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryToken {
    pub token: String,
    pub prefix: bool,
}

/// Tokenizes a query string with the default [`Analyzer::Unicode`]:
/// [`tokenize`]'s pipeline, plus a token immediately followed by `*` becomes a
/// prefix token.
pub fn tokenize_query(query: &str) -> Vec<QueryToken> {
    tokenize_query_with(query, Analyzer::Unicode)
}

/// Tokenizes a query string with `analyzer`. For [`Analyzer::Unicode`] this is
/// exactly [`tokenize_query`]. For [`Analyzer::CjkNgram`], a non-CJK word
/// behaves identically (including the trailing-`*` prefix marker), while a run
/// of CJK ideographs is expanded into its character bigrams — each a plain
/// (non-prefix) token, so an index built with the n-gram analyzer is searched on
/// the same bigram vocabulary it stored. A `*` immediately after a CJK run is
/// segmentation punctuation (bigrams are never prefix tokens). sq-m3ln
pub fn tokenize_query_with(query: &str, analyzer: Analyzer) -> Vec<QueryToken> {
    if analyzer == Analyzer::Unicode {
        return query
            .unicode_word_indices()
            .map(|(start, word)| QueryToken {
                token: lowercase_word(word).into_owned(),
                prefix: query[start + word.len()..].starts_with('*'),
            })
            .collect();
    }
    // CjkNgram: stitch consecutive single-ideograph words into runs and bigram
    // them (plain tokens); a non-CJK word keeps its trailing-`*` prefix marker.
    let mut out: Vec<QueryToken> = Vec::new();
    let mut run: Vec<char> = Vec::new();
    let flush = |run: &mut Vec<char>, out: &mut Vec<QueryToken>| {
        match run.len() {
            0 => {}
            1 => out.push(QueryToken { token: run[0].to_string(), prefix: false }),
            _ => out.extend(run.windows(2).map(|pair| QueryToken {
                token: pair.iter().collect(),
                prefix: false,
            })),
        }
        run.clear();
    };
    for (start, word) in query.unicode_word_indices() {
        let lower = lowercase_word(word);
        if is_cjk_word(&lower) {
            run.push(lower.chars().next().expect("is_cjk_word => one char"));
        } else {
            flush(&mut run, &mut out);
            out.push(QueryToken {
                token: lower.into_owned(),
                prefix: query[start + word.len()..].starts_with('*'),
            });
        }
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(text: &str) -> Vec<String> {
        tokenize(text)
    }

    #[test]
    fn tokenize_ascii_fastpath_matches_unicode_fold() {
        // Differentially pin result-equivalence across the borrowed fast path,
        // ASCII uppercase, and several Unicode lowercase expansions/scripts.
        let corpus = [
            "lowercase ascii identifiers uri42",
            "Mixed UPPER CamelCase",
            "Straße ΣΟΦΊΑ МОСКВА İSTANBUL",
            "café 東京 한국어",
        ];
        for text in corpus {
            let fast: Vec<String> = text
                .unicode_words()
                .map(lowercase_word)
                .map(Cow::into_owned)
                .collect();
            let reference: Vec<String> = text.unicode_words().map(str::to_lowercase).collect();
            assert_eq!(fast, reference, "folding differs for {text:?}");
        }

        for word in "lowercase ascii identifiers uri42".unicode_words() {
            assert!(matches!(lowercase_word(word), Cow::Borrowed(value) if value == word));
        }
        assert!(matches!(lowercase_word("UPPER"), Cow::Owned(_)));
        assert!(matches!(lowercase_word("café"), Cow::Owned(_)));
    }

    #[test]
    fn words_and_punctuation() {
        assert_eq!(toks("The quick, brown fox!"), ["the", "quick", "brown", "fox"]);
        assert_eq!(toks("state-of-the-art"), ["state", "of", "the", "art"]);
        // Apostrophes stay inside a UAX #29 word.
        assert_eq!(toks("can't stop"), ["can't", "stop"]);
        assert_eq!(toks(""), Vec::<String>::new());
        assert_eq!(toks("  \t\n .,;!? "), Vec::<String>::new());
    }

    #[test]
    fn numbers_are_tokens() {
        assert_eq!(toks("RFC 3986 v2.1"), ["rfc", "3986", "v2.1"]);
    }

    #[test]
    fn unicode_case_folding() {
        assert_eq!(toks("ΣΟΦΊΑ Sophia"), ["σοφία", "sophia"]);
        assert_eq!(toks("МОСКВА"), ["москва"]);
        // ß has no uppercase round-trip issue on the lowercase-only fold.
        assert_eq!(toks("Straße STRASSE"), ["straße", "strasse"]);
    }

    #[test]
    fn no_diacritic_folding() {
        // Documented: café and cafe are DIFFERENT tokens.
        assert_eq!(toks("Café"), ["café"]);
        assert_ne!(toks("café"), toks("cafe"));
    }

    #[test]
    fn duplicates_preserved_in_order() {
        assert_eq!(toks("fox fox FOX"), ["fox", "fox", "fox"]);
    }

    #[test]
    fn query_prefix_markers() {
        let q = tokenize_query("Auto* complete");
        assert_eq!(
            q,
            [
                QueryToken { token: "auto".into(), prefix: true },
                QueryToken { token: "complete".into(), prefix: false },
            ]
        );
        // `*` not adjacent to a token end is plain punctuation.
        assert_eq!(
            tokenize_query("* fox"),
            [QueryToken { token: "fox".into(), prefix: false }]
        );
        // A mid-word `*` splits the word; only the left half is a prefix token.
        assert_eq!(
            tokenize_query("auto*complete"),
            [
                QueryToken { token: "auto".into(), prefix: true },
                QueryToken { token: "complete".into(), prefix: false },
            ]
        );
    }

    // ---- CJK n-gram analyzer (sq-m3ln) ----------------------------

    #[test]
    fn ngram_agrees_with_default_on_non_cjk() {
        // The whole point of the opt-in: non-CJK text tokenizes identically, so
        // an n-gram index over a Latin/Cyrillic/Greek corpus is byte-for-byte
        // the default one.
        for s in ["The quick, brown fox!", "state-of-the-art", "RFC 3986 v2.1", "ΣΟΦΊΑ", "café", ""] {
            assert_eq!(
                tokenize_with(s, Analyzer::CjkNgram),
                tokenize_with(s, Analyzer::Unicode),
                "n-gram analyzer must match default on non-CJK input {s:?}"
            );
        }
    }

    #[test]
    fn ngram_emits_overlapping_bigrams_for_han_runs() {
        // 東京都 -> overlapping bigrams 東京, 京都 (NOT 東,京,都).
        assert_eq!(tokenize_with("東京都", Analyzer::CjkNgram), ["東京", "京都"]);
        // A two-char run is a single bigram.
        assert_eq!(tokenize_with("東京", Analyzer::CjkNgram), ["東京"]);
        // A lone ideograph (one-char run) stays a single unigram, so single-char
        // queries keep working.
        assert_eq!(tokenize_with("京", Analyzer::CjkNgram), ["京"]);
    }

    #[test]
    fn ngram_runs_are_bounded_by_non_cjk() {
        // Mixed CJK + Latin: the Latin run is one token (as the default), and the
        // ideograph run on each side bigrams independently (no bigram straddles
        // the Latin word).
        assert_eq!(
            tokenize_with("東京Tokyo大阪", Analyzer::CjkNgram),
            ["東京", "tokyo", "大阪"]
        );
        // UAX #29 separates the digit run; the surrounding ideographs each form
        // their own single-char run here (lone ideograph = single-char unigram).
        assert_eq!(tokenize_with("第3章", Analyzer::CjkNgram), ["第", "3", "章"]);
        // Two ideographs split by a digit: each side is its own run.
        assert_eq!(tokenize_with("北42京", Analyzer::CjkNgram), ["北", "42", "京"]);
    }

    #[test]
    fn ngram_query_bigrams_are_never_prefix() {
        // A CJK query expands to plain (non-prefix) bigram tokens.
        assert_eq!(
            tokenize_query_with("東京都", Analyzer::CjkNgram),
            [
                QueryToken { token: "東京".into(), prefix: false },
                QueryToken { token: "京都".into(), prefix: false },
            ]
        );
        // A trailing `*` after a CJK run is segmentation punctuation, NOT a
        // prefix marker (you cannot prefix-match a bigram).
        assert_eq!(
            tokenize_query_with("東京*", Analyzer::CjkNgram),
            [QueryToken { token: "東京".into(), prefix: false }]
        );
        // A non-CJK word keeps the trailing-`*` prefix marker under the n-gram
        // analyzer.
        assert_eq!(
            tokenize_query_with("auto*", Analyzer::CjkNgram),
            [QueryToken { token: "auto".into(), prefix: true }]
        );
    }

    #[test]
    fn ngram_kana_hangul_left_to_uax29() {
        // Kana/Hangul are NOT bigrammed (UAX #29 already groups their runs into a
        // word). They tokenize identically under both analyzers.
        for s in ["ひらがな", "한국어", "カタカナ"] {
            assert_eq!(
                tokenize_with(s, Analyzer::CjkNgram),
                tokenize_with(s, Analyzer::Unicode),
                "kana/hangul {s:?} must not be bigrammed"
            );
        }
    }
}
