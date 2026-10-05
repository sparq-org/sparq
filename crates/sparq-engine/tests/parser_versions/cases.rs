//! [OPUS-5.5] Shared VERSION-prologue corpus for the parser contract tests.
//!
//! Included by `parser_versions.rs` (stable upstream parser API; also compiled
//! by the registry-only consumer gate) and by `parser_versions_fork.rs` (the
//! vendored-fork differential). Both targets use every item here.

use spargebra::SparqlParser;

/// Outcome of one versioned parse in one Unicode escaping mode.
#[derive(Clone, Copy, Debug)]
pub enum Expect {
    /// Accepted, with exactly these labels in source order.
    Labels(&'static [&'static str]),
    /// Rejected by the parser's own syntax check.
    Syntax,
}

/// One request with its outcome in each upstream Unicode escaping mode.
pub struct Case {
    pub name: &'static str,
    pub text: &'static str,
    /// Outcome when the linked parser reads the source verbatim.
    verbatim: Expect,
    /// Outcome under `spargebra`'s `standard-unicode-escaping` preprocessing.
    preprocessed: Expect,
}

impl Case {
    /// The outcome for the linked parser's actual escaping mode.
    pub fn expected(&self, preprocessing: bool) -> Expect {
        if preprocessing {
            self.preprocessed
        } else {
            self.verbatim
        }
    }
}

const fn same(name: &'static str, text: &'static str, expect: Expect) -> Case {
    Case { name, text, verbatim: expect, preprocessed: expect }
}

const fn split(name: &'static str, text: &'static str, verbatim: Expect, preprocessed: Expect) -> Case {
    Case { name, text, verbatim, preprocessed }
}

/// Whether the linked parser preprocesses Unicode escapes, asked by behavior.
///
/// Deliberately a different probe from the engine's: an escaped line feed ends
/// the comment early, exposing a stray `}` only in the preprocessing mode.
pub fn linked_parser_preprocesses() -> bool {
    SparqlParser::new().parse_query("ASK {} #\\u000A}").is_err()
}

pub const QUERY_CASES: &[Case] = &[
    same("ordinary", "SELECT * WHERE { ?s ?p ?o }", Expect::Labels(&[])),
    same("single-quoted", "VERSION '1.2' ASK {}", Expect::Labels(&["1.2"])),
    same("double-quoted", "VERSION \"1.2-basic\" SELECT * {}", Expect::Labels(&["1.2-basic"])),
    same(
        "escaped-label",
        "VERSION 'a\\'b\\\"c\\\\d\\te' ASK {}",
        Expect::Labels(&["a'b\"c\\d\te"]),
    ),
    same(
        "empty-and-non-ascii-labels",
        "VERSION '' VERSION '\u{e9}\u{2014}1' ASK {}",
        Expect::Labels(&["", "\u{e9}\u{2014}1"]),
    ),
    same(
        "comments-with-fake-declarations",
        "# VERSION '1.1'\r\nVERSION '1.2' # VERSION 'x'\n\tASK {}",
        Expect::Labels(&["1.2"]),
    ),
    same(
        "token-in-iri-prefix-variable-and-literal",
        "PREFIX version: <http://ex/VERSION#> ASK { <http://ex/VERSION> version:VERSION ?VERSION FILTER(?VERSION != \"VERSION '1.1'\") }",
        Expect::Labels(&[]),
    ),
    same(
        "base-and-prefix-between-declarations",
        "BASE <http://ex/> PREFIX ex: <p#> VERSION '1.2' BASE <sub/> PREFIX : <q#> VERSION \"1.2-basic\" ASK { ex:a :b <c> }",
        Expect::Labels(&["1.2", "1.2-basic"]),
    ),
    same(
        "case-insensitive-keywords-without-delimiters",
        "version'1.1'Prefix ex:<http://ex/>vErSiOn\"1.2\"ASK{}",
        Expect::Labels(&["1.1", "1.2"]),
    ),
    same("prefix-keyword-without-delimiter", "PREFIXex:<http://ex/>VERSION'1.1'ASK{}", Expect::Labels(&["1.1"])),
    same(
        "repeated-labels",
        "VERSION '1.2' VERSION '1.2' VERSION '1.2-basic' SELECT * {}",
        Expect::Labels(&["1.2", "1.2", "1.2-basic"]),
    ),
    same(
        "unknown-and-conflicting-labels",
        "VERSION 'unknown' VERSION '1.1' VERSION '1.2' ASK {}",
        Expect::Labels(&["unknown", "1.1", "1.2"]),
    ),
    same("token-after-prologue", "SELECT (\"VERSION '1.2'\" AS ?v) {} # VERSION '1.1'", Expect::Labels(&[])),
    same("unquoted-label", "VERSION 1.2 ASK {}", Expect::Syntax),
    same("unterminated-label", "VERSION '1.2 ASK {}", Expect::Syntax),
    same("line-break-in-label", "VERSION '1.\n2' ASK {}", Expect::Syntax),
    same("long-string-label", "VERSION '''1.2''' ASK {}", Expect::Syntax),
    same("invalid-escape", "VERSION '1\\q' ASK {}", Expect::Syntax),
    same("surrogate-escape", "VERSION '\\uD800' ASK {}", Expect::Syntax),
    same("truncated-keyword", "VERSION", Expect::Syntax),
    same("prologue-only", "VERSION '1.2'", Expect::Syntax),
    same("declaration-after-query", "ASK {} VERSION '1.2'", Expect::Syntax),
    same("relative-base-without-configured-base", "BASE <rel/> VERSION '1.2' ASK {}", Expect::Syntax),
    split(
        "escaped-line-feed-ends-comment",
        "# note\\u000A VERSION '1.2'\nASK {}",
        Expect::Labels(&[]),
        Expect::Labels(&["1.2"]),
    ),
    split(
        "long-escaped-line-feed-ends-comment",
        "# note\\U0000000AVERSION \"1.1\"\nASK {}",
        Expect::Labels(&[]),
        Expect::Labels(&["1.1"]),
    ),
    same("escaped-label-character", "VERSION \"1\\u002E2\" ASK {}", Expect::Labels(&["1.2"])),
    same("long-escaped-label-character", "VERSION '\\U0001F600' ASK {}", Expect::Labels(&["\u{1F600}"])),
    same(
        "escape-only-in-body",
        "VERSION '1.2' ASK { FILTER(\"\\u0041\" = \"A\") }",
        Expect::Labels(&["1.2"]),
    ),
    split("escaped-keyword-letter", "\\u0056ERSION '1.2' ASK {}", Expect::Syntax, Expect::Labels(&["1.2"])),
    split("plus-signed-escape", "\\u+056ERSION '1.2' ASK {}", Expect::Syntax, Expect::Labels(&["1.2"])),
    split(
        "escaped-comment-marker",
        "\\u0023 VERSION '1.2'\nVERSION '1.1' ASK {}",
        Expect::Syntax,
        Expect::Labels(&["1.1"]),
    ),
    split(
        "escaped-prefix-name",
        "PREFIX \\u0065x: <http://ex/> VERSION '1.1' ASK { ex:s ex:p ex:o }",
        Expect::Syntax,
        Expect::Labels(&["1.1"]),
    ),
    split("escaped-quote-label", "VERSION \"\\u0022\" ASK {}", Expect::Labels(&["\""]), Expect::Syntax),
    split(
        "escaped-backslash-before-u",
        "VERSION '\\\\u0031.2' ASK {}",
        Expect::Labels(&["\\u0031.2"]),
        Expect::Syntax,
    ),
];

pub const UPDATE_CASES: &[Case] = &[
    same(
        "ordinary-update",
        "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }",
        Expect::Labels(&[]),
    ),
    same("empty-update", "", Expect::Labels(&[])),
    same("update-label", "VERSION '1.1' INSERT DATA {}", Expect::Labels(&["1.1"])),
    same("update-prologue-only", "VERSION '1.1' VERSION \"1.2\"", Expect::Labels(&["1.1", "1.2"])),
    same(
        "update-prefix-comment-and-literal",
        "PREFIX ex: <http://ex/> VERSION \"1.2\" # VERSION '1.1'\nINSERT DATA { ex:s ex:p \"VERSION '1.1'\" }",
        Expect::Labels(&["1.2"]),
    ),
    same("update-no-prologue-after-separator", "CLEAR ALL ; VERSION '1.2' CLEAR ALL", Expect::Syntax),
    same("update-declaration-after-operation", "INSERT DATA {} VERSION '1.1'", Expect::Syntax),
    split(
        "update-escaped-line-feed-ends-comment",
        "# \\u000AVERSION '1.2' INSERT DATA {}",
        Expect::Labels(&[]),
        Expect::Labels(&["1.2"]),
    ),
];
