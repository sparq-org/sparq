//! Cheap query canonicalization for the result-cache key
//! (research/concurrent-serving.md §6.3).
//!
//! sq-jluc — written while Fable 5 unavailable; re-review when Fable
//! returns.
//!
//! §6.3 prescribes the canonicalization ladder: "whitespace/prefix normalization +
//! variable renaming (Papailiou-style canonical labeling is the ceiling; **start
//! with cheap normalization, measure hit-rate delta**)". This module is exactly
//! the cheap rung — a single linear pass over the query text, no parse, no algebra:
//!
//! 1. **Whitespace normalization** — runs of ASCII whitespace collapse to one
//!    space; leading/trailing space is trimmed. Two queries that differ only in
//!    indentation or line breaks share a key.
//! 2. **Variable renaming (alpha-renaming)** — `?x`/`$x` variables are renamed to
//!    positional `?v0`, `?v1`, ... in first-occurrence order, so two queries that
//!    are identical up to a consistent variable renaming share a key. (SPARQL
//!    variable names are not semantically meaningful beyond binding identity, so
//!    this is sound for the *result rows* — `SELECT ?a WHERE { ?a ?b ?c }` and
//!    `SELECT ?x WHERE { ?x ?y ?z }` compute the same answer rows; the column
//!    *labels* differ, see the caveat below.)
//!
//! **Honest scope / caveat (load-bearing).** Column labels are part of a SPARQL
//! results document: `SELECT ?a ...` emits a `head.vars=["a"]`, `SELECT ?x ...`
//! emits `["x"]`. So alpha-renamed queries do **not** produce byte-identical
//! `sparql-results+json`. The cache value is therefore keyed *and stored* with the
//! caller's chosen result `format`, and alpha-renaming is only sound when the
//! cached bytes are post-processed to the requesting query's projection labels, OR
//! when renaming is disabled. To stay correct-by-construction in v1 this module
//! exposes BOTH a [`canonicalize`] (whitespace only — always label-safe) and a
//! [`canonicalize_renamed`] (whitespace + alpha-rename — higher hit-rate,
//! label-unsafe unless the consumer re-labels), and the cache defaults to the
//! **label-safe** whitespace-only form. The renaming form is opt-in per call so the
//! measured hit-rate delta (§6.3 "measure before going deeper") can be evaluated
//! without risking a wrong column label. Comments, string-literal contents, and
//! IRIs are preserved verbatim — we never reach inside a quoted literal (a `?x`
//! inside a string is data, not a variable). This is intentionally conservative: a
//! missed canonicalization only costs a cache miss (re-execution), never a wrong
//! answer.
//!
//! Prefix normalization (rewriting `PREFIX`/`@prefix` declarations to a canonical
//! order, or expanding prefixed names) is the named next rung; it needs lexical
//! awareness this cheap pass deliberately avoids, and §6.3 says measure first.

/// Canonicalizes query text for cache keying: whitespace collapse only.
///
/// **Label-safe**: the projection variable names are preserved, so the cached
/// result document's `head.vars` matches the requesting query exactly. Two queries
/// that differ only in whitespace/line-breaks produce the same string. This is the
/// cache's default canonical form.
///
/// Whitespace *inside* a quoted string literal (`"..."`, `'...'`, `"""..."""`,
/// `'''...'''`) is preserved verbatim — only whitespace in the query's structural
/// text is collapsed.
pub fn canonicalize(query: &str) -> String {
    rewrite(query, false)
}

/// Canonicalizes with whitespace collapse **and** positional variable renaming
/// (alpha-renaming): `?x`/`$x` become `?v0`, `?v1`, ... in first-occurrence order.
///
/// **Higher hit-rate, but label-unsafe** — see the module-level caveat. Use only
/// when the consumer re-labels the cached bytes to the requesting query's
/// projection, or when the result format carries no column labels. The cache only
/// applies this when explicitly configured to.
pub fn canonicalize_renamed(query: &str) -> String {
    rewrite(query, true)
}

/// One linear pass. `rename` toggles alpha-renaming. State machine over three
/// lexical contexts the cheap pass must distinguish to stay sound:
/// - normal query text (collapse whitespace; rename variables),
/// - inside a string literal (verbatim — a `?x` here is data),
/// - inside a comment (`# ... <eol>` — dropped; a `?x` in a comment must not
///   consume a rename slot or it would desync two queries that differ only in a
///   comment).
fn rewrite(query: &str, rename: bool) -> String {
    let mut out = String::with_capacity(query.len());
    let mut renamer = Renamer::default();
    let bytes = query.as_bytes();
    let mut i = 0;
    // Whether we owe a single collapsed space, emitted lazily so a run of
    // whitespace (and trailing whitespace) costs at most one space and a leading
    // run costs none.
    let mut pending_space = false;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            // String literals: copy the whole literal verbatim, quotes included.
            b'"' | b'\'' => {
                flush_space(&mut out, &mut pending_space);
                let (lit, next) = scan_string(query, i);
                out.push_str(lit);
                i = next;
            }
            // Line comment: `# ... ` to end of line. Dropped entirely (treated as
            // whitespace), so two queries differing only in a comment canonicalize
            // equal.
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                // The trailing newline (if any) is handled as whitespace next loop.
            }
            // Variable: `?name` or `$name`.
            b'?' | b'$' => {
                flush_space(&mut out, &mut pending_space);
                let (var, next) = scan_var(query, i);
                if rename {
                    out.push_str(&renamer.rename(var));
                } else {
                    out.push_str(var);
                }
                i = next;
            }
            _ if b.is_ascii_whitespace() => {
                // Collapse: remember we owe a single space if anything precedes it.
                pending_space = !out.is_empty();
                i += 1;
            }
            _ => {
                flush_space(&mut out, &mut pending_space);
                // Copy this UTF-8 scalar verbatim (handles multi-byte IRIs/literals).
                let ch_len = utf8_len(b);
                out.push_str(&query[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    out
}

/// Emits the single owed collapsed space, if any, then clears the flag.
fn flush_space(out: &mut String, pending: &mut bool) {
    if *pending {
        out.push(' ');
        *pending = false;
    }
}

/// UTF-8 sequence length from a leading byte (1..=4). Defensive: a stray
/// continuation byte is treated as length 1 so we never index past a boundary (the
/// input is `&str`, so it is valid UTF-8 — this is belt-and-braces).
fn utf8_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Scans a SPARQL string literal starting at `start` (a quote byte). Returns the
/// literal slice (quotes included) and the index just past it. Handles long quotes
/// (`"""`/`'''`) and backslash escapes; an unterminated literal scans to EOF
/// (conservative — the engine's parser will reject it, we only need a stable key).
fn scan_string(query: &str, start: usize) -> (&str, usize) {
    let bytes = query.as_bytes();
    let quote = bytes[start];
    // Long-quote form: three identical quote bytes.
    let long = start + 2 < bytes.len() && bytes[start + 1] == quote && bytes[start + 2] == quote;
    let mut i = if long { start + 3 } else { start + 1 };
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 2; // skip the escaped char
            continue;
        }
        if b == quote {
            if long {
                if i + 2 < bytes.len() && bytes[i + 1] == quote && bytes[i + 2] == quote {
                    i += 3;
                    return (&query[start..i], i);
                }
                i += 1;
            } else {
                i += 1;
                return (&query[start..i], i);
            }
        } else {
            i += 1;
        }
    }
    (&query[start..], bytes.len())
}

/// Scans a variable token starting at `start` (a `?` or `$`). Returns the token
/// (sigil + name, the name as written) and the index just past it. The SPARQL
/// `VARNAME` grammar is approximated by "name continues over alphanumerics, `_`,
/// and non-ASCII"; any byte that cannot be in a name ends the token. A bare sigil
/// with no name is left as just the sigil (a parse error the engine will catch).
fn scan_var(query: &str, start: usize) -> (&str, usize) {
    let bytes = query.as_bytes();
    let mut i = start + 1; // past the sigil
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'_' || b.is_ascii_alphanumeric() || b >= 0x80 {
            i += 1;
        } else {
            break;
        }
    }
    (&query[start..i], i)
}

/// First-occurrence positional renamer: each distinct variable token (by its
/// written name, sigil-normalized so `?x` and `$x` collapse) maps to `?v0`, `?v1`,
/// ... in the order first seen.
#[derive(Default)]
struct Renamer {
    // Small linear map: queries rarely have many distinct variables, and a Vec
    // probe beats a hash map's setup cost at this size. Keyed on the de-sigiled
    // name so `?x`/`$x` collapse.
    seen: Vec<(String, String)>,
}

impl Renamer {
    fn rename(&mut self, var: &str) -> String {
        let name = &var[1..]; // strip the sigil
        if let Some((_, canon)) = self.seen.iter().find(|(n, _)| n == name) {
            return canon.clone();
        }
        let canon = format!("?v{}", self.seen.len());
        self.seen.push((name.to_string(), canon.clone()));
        canon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // sq-bvdyd: exercise every lexical context that can affect whether
    // a second canonicalization pass changes the cache key.
    const IDEMPOTENCE_QUERIES: [&str; 4] = [
        "  SELECT   ?x\n WHERE {\t?x ?p ?o } # trailing comment\n",
        "SELECT ?x # ignored ?comment\nWHERE { ?x ?p \"literal  ?data and $data\" }",
        "ASK { $x ?p 'literal ?x and $x' ; ?q $x .\n}",
        "",
    ];

    #[test]
    fn whitespace_collapses() {
        let a = canonicalize("SELECT  ?x\n WHERE {\t?x ?y ?z }");
        let b = canonicalize("SELECT ?x WHERE { ?x ?y ?z }");
        assert_eq!(a, b);
        assert_eq!(a, "SELECT ?x WHERE { ?x ?y ?z }");
    }

    #[test]
    fn leading_trailing_trimmed() {
        assert_eq!(canonicalize("   SELECT *  "), "SELECT *");
    }

    #[test]
    fn label_safe_keeps_var_names() {
        // The default form must NOT rename — column labels are preserved.
        assert_eq!(
            canonicalize("SELECT ?a WHERE { ?a ?b ?c }"),
            "SELECT ?a WHERE { ?a ?b ?c }"
        );
    }

    #[test]
    fn renaming_alpha_equates() {
        let a = canonicalize_renamed("SELECT ?a WHERE { ?a ?b ?c }");
        let b = canonicalize_renamed("SELECT ?x WHERE { ?x ?y ?z }");
        assert_eq!(a, b);
        assert_eq!(a, "SELECT ?v0 WHERE { ?v0 ?v1 ?v2 }");
    }

    #[test]
    fn renaming_normalizes_sigil() {
        // ?x and $x are the same variable.
        assert_eq!(
            canonicalize_renamed("SELECT ?x { $x ?y ?x }"),
            "SELECT ?v0 { ?v0 ?v1 ?v0 }"
        );
    }

    #[test]
    fn string_literal_var_is_data() {
        // A `?x` inside a string is NOT a variable — must survive verbatim and not
        // consume a rename slot.
        let q = r#"SELECT ?x { ?x ?p "has a ?y inside" }"#;
        let r = canonicalize_renamed(q);
        // The literal's `?y` is untouched; only ?x and ?p are renamed.
        assert!(r.contains(r#""has a ?y inside""#));
        assert!(r.contains("?v0")); // ?x
        assert!(r.contains("?v1")); // ?p
        assert!(!r.contains("?v2")); // ?y inside the string did NOT allocate a slot
    }

    #[test]
    fn comment_is_dropped() {
        let a = canonicalize("SELECT ?x # a comment with ?y\n WHERE { ?x ?p ?o }");
        let b = canonicalize("SELECT ?x WHERE { ?x ?p ?o }");
        assert_eq!(a, b);
    }

    #[test]
    fn long_string_literal_preserved() {
        let q = "SELECT ?x { ?x ?p \"\"\"multi\nline ?y\"\"\" }";
        let r = canonicalize_renamed(q);
        assert!(r.contains("\"\"\"multi\nline ?y\"\"\""));
    }

    #[test]
    fn escaped_quote_in_literal() {
        let q = r#"ASK { ?s ?p "a \" b" }"#;
        let r = canonicalize(q);
        assert!(r.contains(r#""a \" b""#));
    }

    #[test]
    fn canonicalize_is_idempotent() {
        for query in IDEMPOTENCE_QUERIES {
            let once = canonicalize(query);
            assert_eq!(canonicalize(&once), once, "query: {query:?}");
        }
    }

    #[test]
    fn canonicalize_renamed_is_idempotent() {
        for query in IDEMPOTENCE_QUERIES {
            let once = canonicalize_renamed(query);
            assert_eq!(canonicalize_renamed(&once), once, "query: {query:?}");
        }
    }
}
