//! The verifiable transpiler skeleton (design §3/§6, Phase 1).
//!
//! [`terse_to_sparql`] is the crate's single public contract. It takes a *terse* query
//! string and returns an [`Expansion`] whose [`Expansion::canonical_sparql`] is **always**
//! standard, conformant SPARQL — the exact text the engine will run and the agent can
//! inspect. There is no path where the engine executes something the agent cannot read
//! (design §6.1, "echo the canonical expanded query, always").
//!
//! Two invariants this skeleton enforces mechanically — the *silent-rewrite canary*
//! (design §6.7):
//!
//! 1. **Conformance.** The emitted `canonical_sparql` is re-parsed under the unmodified
//!    vendored `spargebra` parser. If it does not parse, [`terse_to_sparql`] returns
//!    [`TerseError::CanaryFailed`] rather than handing the agent invalid SPARQL — the
//!    transpiler can only ever emit a query the engine could actually run.
//! 2. **Pass-through identity.** A query with no terse constructs is emitted *byte-for-byte
//!    unchanged*. The transpiler never silently rewrites canonical SPARQL.
//!
//! The one terse construct this crate defines is `V("phrase")` concept resolution
//! ([`crate::resolve`], design §3.3). Detection of `V(...)` is feature-independent so the
//! *default* (no-`vectors`) build fails **loudly** with [`TerseError::FeatureRequired`]
//! instead of passing an un-expanded `V(...)` through to the parser (which would be a
//! confusing downstream parse error). The actual resolution lives behind the `vectors`
//! feature.

use crate::error::TerseError;

/// The result of transpiling a terse query: the canonical SPARQL to execute, plus the
/// verification surface (resolutions + warnings) the agent inspects.
///
/// `canonical_sparql` is the *whole* contract: it is standard SPARQL, it is what runs, and
/// it is what the agent must be shown. `resolutions` and `warnings` let the agent verify
/// *how* any terse construct was expanded (design §6.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Expansion {
    /// The canonical, conformant SPARQL the engine will run. Re-parsed under `spargebra`
    /// before return (the canary), so it is guaranteed to parse.
    pub canonical_sparql: String,
    /// One entry per terse concept-resolution (`V("phrase")`) that was expanded, echoing
    /// the resolved IRI, its score, the runner-up, the confidence and which method fired
    /// (design §6.2). Empty for a pure pass-through. The type is always present (so callers
    /// compile in both feature states); it is only ever populated behind the `vectors`
    /// feature.
    pub resolutions: Vec<crate::resolve::Resolution>,
    /// One entry per `K:<name>` keyword (lever 1, design §3.1) that was expanded, echoing the
    /// keyword, the absolute IRI it became, and the frozen legend version (design §3.1 (ii) —
    /// "the expansion is always echoed"). Empty for input with no keyword tokens. Populated in
    /// **both** feature states (the keyword layer is in the lean default build).
    pub keywords: Vec<crate::keyword::KeywordExpansion>,
    /// Non-fatal advisories surfaced to the agent (e.g. a close runner-up flagging
    /// ambiguity). Never hides a rewrite — a rewrite that *changes intent* is a hard error,
    /// not a warning.
    pub warnings: Vec<String>,
}

/// Transpiles a terse query into canonical SPARQL (design §3, Phase 1 + lever-1 contract).
///
/// In the *default* build (no `vectors` feature) this is a verifiable transpiler over two
/// surfaces:
///
/// - **The `K:<name>` keyword layer (lever 1, design §3.1).** Each `K:<name>` token expands
///   pre-parse to the canonical `<iri>` from the frozen, versioned legend
///   ([`crate::legend`]); every expansion is echoed in [`Expansion::keywords`]. An
///   unknown keyword is [`TerseError::UnknownKeyword`] and a `K:<name>` colliding with a real
///   `PREFIX K:` is [`TerseError::KeywordPrefixCollision`] — never a silent guess. This layer
///   is in the lean default build (no model, no engine dependency).
/// - **Identity pass-through** for canonical SPARQL with no terse token, emitted unchanged
///   after the conformance canary. A `V("phrase")` construct is rejected with
///   [`TerseError::FeatureRequired`] (it needs the `vectors` feature + a resolution context).
///
/// With the `vectors` feature plus a resolution context (`crate::resolve::ResolveCtx`), use
/// `terse_to_sparql_with` instead; that entry point runs the same keyword layer **and** then
/// resolves `V("phrase")`.
///
/// # Errors
/// - [`TerseError::UnknownKeyword`] for a `K:<name>` not in the frozen legend.
/// - [`TerseError::KeywordPrefixCollision`] if `K:<name>` is used and the query also declares
///   a real `PREFIX K:`.
/// - [`TerseError::FeatureRequired`] if the input uses `V(...)` but no resolution context
///   was supplied (this entry point) or the `vectors` feature is off.
/// - [`TerseError::CanaryFailed`] if the (otherwise canonical) input does not parse under
///   `spargebra` — surfaced loudly rather than handing back invalid SPARQL.
pub fn terse_to_sparql(src: &str) -> Result<Expansion, TerseError> {
    // Lever 1 first: expand every K:<name> keyword (loud on unknown / prefix collision).
    let (expanded, keywords) = expand_keywords(src)?;
    // The remaining terse construct is V(...): without a resolution context there is nothing
    // to expand, so this is a strict pass-through. Detect V(...) and fail loudly (design §6.6:
    // loud-fail beats silent-wrong) rather than letting an un-expanded `V(...)` reach the
    // parser as a baffling syntax error.
    if let Some(phrase) = find_first_v_phrase(&expanded) {
        return Err(TerseError::FeatureRequired {
            phrase,
            why: "V(\"...\") concept resolution requires the `vectors` feature and a \
                  resolution context (terse_to_sparql_with)"
                .to_string(),
        });
    }
    canary(&expanded)?;
    Ok(Expansion {
        canonical_sparql: expanded,
        resolutions: Vec::new(),
        keywords,
        warnings: Vec::new(),
    })
}

/// Transpiles a terse query, resolving every `V("phrase")` construct against a
/// [`crate::resolve::ResolveCtx`] (design §3.3 / §6, Phase 2). Each `V("phrase")` is
/// replaced by the canonical `<iri>` it resolves to (lexical-first, confidence-gated,
/// staleness-guarded), and the resulting canonical SPARQL is run through the silent-rewrite
/// canary before return. Every resolution is echoed in [`Expansion::resolutions`] so the
/// agent can audit each bind (design §6.2). A close runner-up is surfaced as a warning.
///
/// `embed` is an optional embedder for the vector FALLBACK: it is called *only* for a phrase
/// that fails lexical linking **and** only when the fallback can actually run — a vector store
/// is attached and passes the §6.5 staleness guard — to produce the query vector the vector
/// search needs. A stale store is [`TerseError::StaleStore`] with no embedder call. Pass a
/// closure that returns `None` (or use [`terse_to_sparql`] for canonical-only input) to stay
/// on the no-model, lexical-only path; the design (§6/§9 Q5) keeps the embedding model the
/// caller's explicit, opt-in dependency rather than forcing one into this crate.
///
/// # Errors
/// - [`TerseError::Unresolved`] if any `V("phrase")` cannot be confidently resolved (below
///   the score floor, ambiguous, or no candidate) — the bind is refused loudly, never
///   guessed (design §6.3).
/// - [`TerseError::StaleStore`] if the vector fallback's store is stale vs the graph
///   (design §6.5).
/// - [`TerseError::CanaryFailed`] if the expanded SPARQL does not parse — surfaced rather
///   than executed.
#[cfg(feature = "vectors")]
pub fn terse_to_sparql_with(
    src: &str,
    ctx: &crate::resolve::ResolveCtx<'_>,
    mut embed: impl FnMut(&str) -> Option<Vec<f32>>,
) -> Result<Expansion, TerseError> {
    // Lever 1 first: expand every K:<name> keyword (loud on unknown / prefix collision), then
    // run V() resolution over the keyword-expanded source so both surfaces compose.
    let (src, keywords) = expand_keywords(src)?;
    let src = src.as_str();
    let spans = v_spans(src);
    if spans.is_empty() {
        // No V() construct: emit the (possibly keyword-expanded) source, still canary-checked.
        canary(src)?;
        return Ok(Expansion {
            canonical_sparql: src.to_string(),
            resolutions: Vec::new(),
            keywords,
            warnings: Vec::new(),
        });
    }
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    let mut resolutions = Vec::with_capacity(spans.len());
    let mut warnings = Vec::new();
    for span in &spans {
        // Copy the source up to this V(...) verbatim.
        out.push_str(&src[cursor..span.start]);
        // Resolve: lexical-first. The embedder is handed to the ctx as a THUNK, so it is
        // invoked only when lexical returns nothing and a usable (non-stale) vector store is
        // attached — a lexically-bound phrase, or one whose fallback cannot run at all, never
        // pays the model/network cost (design §6.4/§6.5/§9 Q5).
        let res = ctx.resolve_lazy(&span.phrase, |p| embed(p))?;
        // Splice the canonical <iri> in place of the V(...) construct.
        out.push('<');
        out.push_str(&res.iri);
        out.push('>');
        // Surface a close runner-up as a visible ambiguity warning (design §6.2).
        if let (Some(runner), Some(rs)) = (&res.runner_up, res.runner_up_score) {
            warnings.push(format!(
                "V(\"{}\") -> <{}> (score {:.3}, confidence {:.3}) with a runner-up <{}> \
                 (score {:.3}); verify the bind",
                span.phrase, res.iri, res.score, res.confidence, runner, rs
            ));
        }
        resolutions.push(res);
        cursor = span.end;
    }
    out.push_str(&src[cursor..]);
    canary(&out)?;
    Ok(Expansion {
        canonical_sparql: out,
        resolutions,
        keywords,
        warnings,
    })
}

/// Runs the silent-rewrite canary (design §6.7): re-parse the canonical output under the
/// unmodified `spargebra` parser. The transpiler must only ever emit SPARQL the engine
/// could run. We accept either a query or an update (the surface transpiles both).
pub(crate) fn canary(canonical_sparql: &str) -> Result<(), TerseError> {
    use spargebra::SparqlParser;
    // The surface transpiles both queries and updates, so the emission is conformant if it
    // parses as EITHER. Capture the query parse error (the common case) for the diagnostic.
    let query_err = match SparqlParser::new().parse_query(canonical_sparql) {
        Ok(_) => return Ok(()),
        Err(e) => e.to_string(),
    };
    if SparqlParser::new().parse_update(canonical_sparql).is_ok() {
        return Ok(());
    }
    Err(TerseError::CanaryFailed {
        sparql: canonical_sparql.to_string(),
        parse_error: query_err,
    })
}

/// Where a `K:<name>` keyword token sits in the source: the keyword name and the byte range
/// `[start, end)` of the whole `K:<name>` span (so the expander can splice `<iri>` in).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KSpan {
    name: String,
    start: usize,
    end: usize,
}

/// Expands every `K:<name>` keyword token in `src` to its canonical `<iri>` from the frozen
/// legend (lever 1, design §3.1), returning the expanded SPARQL **and** the echoed
/// [`crate::KeywordExpansion`] list. A pure pass-through (no `K:` token) returns the
/// source unchanged and an empty list.
///
/// Guardrails (design §3.1):
/// - If the query declares a **real** `PREFIX K:` (a prefix literally named `K`) *and* uses a
///   `K:<name>` token, the token is genuinely ambiguous between keyword- and prefix-expansion:
///   [`TerseError::KeywordPrefixCollision`] (never a silent guess).
/// - A `K:<name>` whose `<name>` is not in the frozen legend is [`TerseError::UnknownKeyword`]
///   (with the legend version + did-you-mean suggestions) — never an invented expansion.
///
/// `K:<name>` tokens inside string literals, IRIs and comments are *not* keywords (the same
/// lexical-context discipline as the `V(...)` scanner) and pass through untouched.
pub(crate) fn expand_keywords(src: &str) -> Result<(String, Vec<crate::keyword::KeywordExpansion>), TerseError> {
    let spans = k_spans(src);
    if spans.is_empty() {
        return Ok((src.to_string(), Vec::new()));
    }
    // Guardrail: a real `PREFIX K:` declaration makes `K:<name>` ambiguous. We only need to
    // check this once, and only when there is at least one K:<name> token to expand.
    if declares_prefix_k(src) {
        return Err(TerseError::KeywordPrefixCollision {
            keyword: spans[0].name.clone(),
        });
    }
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    let mut expansions = Vec::with_capacity(spans.len());
    for span in &spans {
        out.push_str(&src[cursor..span.start]);
        match crate::keyword::lookup(&span.name) {
            Some(iri) => {
                out.push('<');
                out.push_str(&iri);
                out.push('>');
                expansions.push(crate::keyword::KeywordExpansion {
                    keyword: span.name.clone(),
                    iri,
                    legend_version: crate::keyword::LEGEND_VERSION.to_string(),
                });
            }
            None => {
                return Err(TerseError::UnknownKeyword {
                    keyword: span.name.clone(),
                    legend_version: crate::keyword::LEGEND_VERSION.to_string(),
                    suggestions: crate::keyword::suggestions(&span.name, 3)
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                });
            }
        }
        cursor = span.end;
    }
    out.push_str(&src[cursor..]);
    Ok((out, expansions))
}

/// Returns every `K:<name>` keyword token in `src`, in source order, as [`KSpan`]s — a
/// standalone `K` (sigil) immediately followed by `:` and a SPARQL local-name, OUTSIDE any
/// string/IRI/comment. `K:` with no name, or `K` glued to a leading identifier char, is not a
/// keyword token.
fn k_spans(src: &str) -> Vec<KSpan> {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < n {
        let c = bytes[i];
        match c {
            b'<' => {
                let mut j = i + 1;
                while j < n && bytes[j] != b'>' && bytes[j] != b'\n' {
                    j += 1;
                }
                i = if j < n { j + 1 } else { n };
            }
            b'#' => {
                let mut j = i + 1;
                while j < n && bytes[j] != b'\n' {
                    j += 1;
                }
                i = j;
            }
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
            }
            b'K' => {
                // Standalone sigil: the preceding char (if any) must not be an identifier
                // char (so `?xK:type` or `fooK:type` are NOT the keyword token).
                let prev_ok = !preceded_by_name(bytes, i);
                if prev_ok && i + 1 < n && bytes[i + 1] == b':' {
                    if let Some((name, end)) = parse_k_name(bytes, i + 2) {
                        out.push(KSpan { name, start: i, end });
                        i = end;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// At `bytes[start]` being the first char after `K:`, reads a SPARQL-ish local name
/// (`[A-Za-z0-9_-]+`, must start with a letter/underscore), returning `(name, end)` where
/// `end` is the byte index just past the name. `None` if there is no valid name.
fn parse_k_name(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let n = bytes.len();
    if start >= n {
        return None;
    }
    let first = bytes[start];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut j = start + 1;
    while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-') {
        j += 1;
    }
    let name = std::str::from_utf8(&bytes[start..j]).ok()?.to_string();
    Some((name, j))
}

/// `true` if `src` declares a SPARQL/Turtle prefix literally named `K` — `PREFIX K: <...>`
/// (SPARQL) or `@prefix K: <...>` (Turtle-style, tolerated). Case-insensitive on the
/// `PREFIX`/`@prefix` keyword; the prefix label `K` is case-sensitive (it must match the `K`
/// sigil to collide). Scans only outside string/IRI/comment contexts.
fn declares_prefix_k(src: &str) -> bool {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        let c = bytes[i];
        match c {
            b'<' => {
                let mut j = i + 1;
                while j < n && bytes[j] != b'>' && bytes[j] != b'\n' {
                    j += 1;
                }
                i = if j < n { j + 1 } else { n };
            }
            b'#' => {
                let mut j = i + 1;
                while j < n && bytes[j] != b'\n' {
                    j += 1;
                }
                i = j;
            }
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
            }
            _ => {
                // At a token boundary, try to match `PREFIX`/`@prefix` then `K` `:`.
                let boundary = !preceded_by_name(bytes, i);
                if boundary {
                    if let Some(after) = match_prefix_kw(bytes, i) {
                        let k = skip_ws(bytes, after);
                        // Need `K` then `:` for the label to be the sigil `K`.
                        if k + 1 < n && bytes[k] == b'K' && bytes[k + 1] == b':' {
                            return true;
                        }
                    }
                }
                i += 1;
            }
        }
    }
    false
}

/// If `bytes[start..]` begins with a `PREFIX` (SPARQL, case-insensitive) or `@prefix`
/// (Turtle) keyword at a token boundary, returns the index just past the keyword; else `None`.
fn match_prefix_kw(bytes: &[u8], start: usize) -> Option<usize> {
    // SPARQL `PREFIX` (case-insensitive).
    if let Some(end) = match_word_ci(bytes, start, b"PREFIX") {
        return Some(end);
    }
    // Turtle `@prefix` (the `@` then `prefix`, case-insensitive on the word).
    if bytes.get(start) == Some(&b'@') {
        if let Some(end) = match_word_ci(bytes, start + 1, b"prefix") {
            return Some(end);
        }
    }
    None
}

/// Case-insensitive match of ASCII `word` at `bytes[start]`, requiring a non-identifier char
/// (or end) immediately after. Returns the index just past the word on success.
fn match_word_ci(bytes: &[u8], start: usize, word: &[u8]) -> Option<usize> {
    let end = start + word.len();
    if end > bytes.len() {
        return None;
    }
    for (k, w) in word.iter().enumerate() {
        if !bytes[start + k].eq_ignore_ascii_case(w) {
            return None;
        }
    }
    // Must be followed by a non-identifier byte (or end of input).
    if end < bytes.len() && is_name_byte(bytes[end]) {
        return None;
    }
    Some(end)
}

/// Scans `src` for the first `V("phrase")` (or `V('phrase')`) construct OUTSIDE of any
/// SPARQL string/IRI literal, returning the inner phrase. Returns `None` if there is no
/// such construct. This is the feature-independent *detector*: it is deliberately a tiny,
/// conservative lexical scan (the design forbids touching the vendored grammar), and it
/// only recognises a single, fixed shape — `V` (case-sensitive) immediately followed by
/// `(` then a quoted string then `)`. Anything else passes through untouched.
///
/// We skip over SPARQL string literals (`'...'`, `"..."`, `'''...'''`, `"""..."""`) and
/// IRIs (`<...>`) so a `V(` appearing *inside* a literal/IRI is never mistaken for the
/// construct — the construct is a top-level token, not text inside a value.
pub(crate) fn find_first_v_phrase(src: &str) -> Option<String> {
    scan_v_constructs(src, |phrase, _start, _end| Some(phrase.to_string())).into_iter().next()
}

/// Where a `V("phrase")` construct sits in the source: the inner phrase and the byte range
/// `[start, end)` of the whole `V(...)` span (so a caller can splice a replacement in).
/// Only used by the `vectors`-gated [`terse_to_sparql_with`] expansion path.
#[cfg(feature = "vectors")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VSpan {
    pub phrase: String,
    pub start: usize,
    pub end: usize,
}

/// Returns every `V("phrase")` construct in `src`, in source order, as [`VSpan`]s.
#[cfg(feature = "vectors")]
pub(crate) fn v_spans(src: &str) -> Vec<VSpan> {
    scan_v_constructs(src, |phrase, start, end| {
        Some(VSpan {
            phrase: phrase.to_string(),
            start,
            end,
        })
    })
}

/// The shared lexer for `V("phrase")` constructs. Walks `src` char-by-char, tracking
/// whether we are inside a string literal or IRI (where a `V(` must be ignored), and for
/// each top-level `V(` immediately followed by a single quoted string and a `)`, invokes
/// `f(phrase, start, end)`; non-`None` results are collected in source order.
fn scan_v_constructs<T>(
    src: &str,
    mut f: impl FnMut(&str, usize, usize) -> Option<T>,
) -> Vec<T> {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < n {
        let c = bytes[i];
        match c {
            b'<' => {
                // Skip an IRI ref <...> (no nested '<', '>' ends it; newline cancels it,
                // matching SPARQL's IRIREF lexing where these are illegal inside).
                let mut j = i + 1;
                while j < n && bytes[j] != b'>' && bytes[j] != b'\n' {
                    j += 1;
                }
                i = if j < n { j + 1 } else { n };
            }
            b'#' => {
                // Line comment to end-of-line.
                let mut j = i + 1;
                while j < n && bytes[j] != b'\n' {
                    j += 1;
                }
                i = j;
            }
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
            }
            b'V' => {
                // Must be a standalone `V` token: the preceding char (if any) must not be
                // part of an identifier (so `?fooV(` or `abcV(` are NOT the construct).
                let prev_ok = !preceded_by_name(bytes, i);
                if prev_ok {
                    if let Some((phrase, end)) = parse_v_call(bytes, i) {
                        if let Some(t) = f(&phrase, i, end) {
                            out.push(t);
                        }
                        i = end;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// `true` if `b` can appear inside a SPARQL identifier / prefixed-name local part (so a `V`
/// glued to one is not a standalone token).
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'?' || b == b'$'
}

/// `true` if `b` can be part of a SPARQL name: an ASCII identifier byte, or any
/// non-ASCII byte. Outside strings, IRIs and comments (which the scanners skip),
/// a non-ASCII character can only be a `PN_CHARS` name character — SPARQL
/// whitespace and punctuation are all ASCII — so every UTF-8 lead/continuation
/// byte counts as a name byte. GitHub #4662.
fn is_name_byte(b: u8) -> bool {
    is_ident_char(b) || !b.is_ascii()
}

/// `true` if the token starting at `bytes[i]` is glued to a preceding name, so
/// it is NOT a standalone `K` / `V` / `PREFIX` token: the previous byte is a
/// name byte ([`is_name_byte`]), or the previous two bytes are a `PN_LOCAL_ESC`
/// (`\` + one of `_~.-!$&'()*+,;=/?#@%`), which continues a local name.
/// GitHub #4662.
fn preceded_by_name(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    if is_name_byte(bytes[i - 1]) {
        return true;
    }
    i >= 2 && bytes[i - 2] == b'\\' && b"_~.-!$&'()*+,;=/?#@%".contains(&bytes[i - 1])
}

/// At `bytes[start] == b'V'`, tries to parse `V("phrase")` / `V('phrase')` allowing
/// whitespace around the parens and the string. On success returns `(phrase, end)` where
/// `end` is the byte index just past the closing `)`. The phrase is the *unescaped* string
/// content (we honour `\"`, `\'`, `\\`, `\n`, `\t`, `\r` — the subset SPARQL strings use).
fn parse_v_call(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let n = bytes.len();
    let mut i = start + 1; // past 'V'
    i = skip_ws(bytes, i);
    if i >= n || bytes[i] != b'(' {
        return None;
    }
    i += 1;
    i = skip_ws(bytes, i);
    if i >= n || (bytes[i] != b'"' && bytes[i] != b'\'') {
        return None;
    }
    let quote = bytes[i];
    i += 1;
    let mut phrase = String::new();
    while i < n {
        let b = bytes[i];
        if b == b'\\' && i + 1 < n {
            let esc = bytes[i + 1];
            match esc {
                b'"' => phrase.push('"'),
                b'\'' => phrase.push('\''),
                b'\\' => phrase.push('\\'),
                b'n' => phrase.push('\n'),
                b't' => phrase.push('\t'),
                b'r' => phrase.push('\r'),
                other => {
                    phrase.push('\\');
                    phrase.push(other as char);
                }
            }
            i += 2;
            continue;
        }
        if b == quote {
            i += 1;
            break;
        }
        if b == b'\n' {
            // Unterminated single-line string: not a valid V() call.
            return None;
        }
        // Push the raw byte; for multi-byte UTF-8 we copy the full sequence below.
        let ch_len = utf8_len(b);
        if i + ch_len > n {
            return None;
        }
        let s = std::str::from_utf8(&bytes[i..i + ch_len]).ok()?;
        phrase.push_str(s);
        i += ch_len;
    }
    i = skip_ws(bytes, i);
    if i >= n || bytes[i] != b')' {
        return None;
    }
    Some((phrase, i + 1))
}

/// Skips ASCII whitespace, returning the index of the first non-whitespace byte.
fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// At `bytes[start]` being a quote, skips a SPARQL string literal (single- or
/// triple-quoted, honouring backslash escapes), returning the index just past it.
fn skip_string(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let quote = bytes[start];
    // Triple-quoted?
    let triple = start + 2 < n && bytes[start + 1] == quote && bytes[start + 2] == quote;
    if triple {
        let mut i = start + 3;
        while i + 2 < n {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
                return i + 3;
            }
            i += 1;
        }
        return n;
    }
    let mut i = start + 1;
    while i < n {
        if bytes[i] == b'\\' && i + 1 < n {
            i += 2;
            continue;
        }
        if bytes[i] == quote || bytes[i] == b'\n' {
            return i + 1;
        }
        i += 1;
    }
    n
}

/// Byte-length of the UTF-8 sequence whose lead byte is `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_is_byte_identical() {
        let q = "SELECT ?s WHERE { ?s <http://ex/p> ?o }";
        let e = terse_to_sparql(q).expect("canonical SPARQL passes through");
        assert_eq!(e.canonical_sparql, q, "pass-through must be byte-identical");
        assert!(e.resolutions.is_empty());
        assert!(e.warnings.is_empty());
    }

    /// GitHub #4662: a `K` that ends a Unicode variable name (`?aéK`) is not the
    /// keyword sigil — the byte before it is a UTF-8 continuation byte, which is
    /// part of the name, not a delimiter. Valid SPARQL must pass through unchanged.
    #[test]
    fn k_after_unicode_name_char_is_not_a_sigil() {
        let q = "PREFIX : <http://ex/> SELECT ?s WHERE { ?aéK:x ?s }";
        let e = terse_to_sparql(q).expect("valid SPARQL passes through");
        assert_eq!(e.canonical_sparql, q);
        assert!(e.keywords.is_empty());
    }

    /// GitHub #4662: a `K` after a `PN_LOCAL_ESC` (`\-`) continues the local
    /// name (`ex:a\-K:x` is the local name `a-K:x`), so it is not the sigil.
    #[test]
    fn k_after_pn_local_esc_is_not_a_sigil() {
        let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ex:a\\-K:x }";
        let e = terse_to_sparql(q).expect("valid SPARQL passes through");
        assert_eq!(e.canonical_sparql, q);
        assert!(e.keywords.is_empty());
    }

    /// GitHub #4662: the `V(` scanner shares the boundary predicate, so a `V`
    /// glued to a Unicode name is not the construct either.
    #[test]
    fn v_after_unicode_name_char_is_not_a_construct() {
        let q = "SELECT ?s WHERE { ?s <http://ex/p> ?aéV(\"x\") }";
        let err = terse_to_sparql(q).expect_err("not valid SPARQL");
        assert!(matches!(err, TerseError::CanaryFailed { .. }), "{err:?}");
    }

    #[test]
    fn canary_rejects_invalid_sparql() {
        let bad = "SELECT ?s WHERE { ?s ?p"; // unbalanced — does not parse
        let err = terse_to_sparql(bad).expect_err("invalid SPARQL must fail the canary");
        assert!(matches!(err, TerseError::CanaryFailed { .. }));
    }

    #[test]
    fn canary_accepts_update() {
        let u = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";
        let e = terse_to_sparql(u).expect("a valid UPDATE passes the canary");
        assert_eq!(e.canonical_sparql, u);
    }

    #[test]
    fn v_construct_detected_and_rejected_without_context() {
        let q = "SELECT ?f WHERE { ?f <http://ex/about> V(\"cardinality estimation\") }";
        let err = terse_to_sparql(q).expect_err("V() needs the vectors feature + a context");
        match err {
            TerseError::FeatureRequired { phrase, .. } => {
                assert_eq!(phrase, "cardinality estimation");
            }
            other => panic!("expected FeatureRequired, got {other:?}"),
        }
    }

    #[test]
    fn v_inside_a_string_literal_is_not_a_construct() {
        // A V( appearing inside a SPARQL string is data, not the construct.
        let q = "SELECT ?s WHERE { ?s <http://ex/p> \"V(\\\"x\\\")\" }";
        let e = terse_to_sparql(q).expect("V inside a literal is passed through");
        assert_eq!(e.canonical_sparql, q);
    }

    #[test]
    fn v_inside_an_iri_is_not_a_construct() {
        let q = "SELECT ?s WHERE { ?s <http://ex/V(x)> ?o }";
        let e = terse_to_sparql(q).expect("V inside an IRI is passed through");
        assert_eq!(e.canonical_sparql, q);
    }

    #[test]
    fn glued_v_is_not_a_construct() {
        // `?fooV("x")` — the V is glued to an identifier, not a standalone token. There is
        // no V() construct here; the text is left for the parser (which will reject it, but
        // via the canary, not a false V-detection).
        assert!(find_first_v_phrase("?fooV(\"x\")").is_none());
    }

    #[cfg(feature = "vectors")]
    #[test]
    fn v_spans_finds_all_in_order() {
        let q = "{ ?a <p> V(\"one\") . ?b <p> V('two') }";
        let spans = v_spans(q);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].phrase, "one");
        assert_eq!(spans[1].phrase, "two");
        // The spans must point at the actual V(...) text.
        assert_eq!(&q[spans[0].start..spans[0].end], "V(\"one\")");
        assert_eq!(&q[spans[1].start..spans[1].end], "V('two')");
    }

    #[test]
    fn v_escapes_are_unescaped() {
        let phrase = find_first_v_phrase("V(\"a\\\"b\")").unwrap();
        assert_eq!(phrase, "a\"b");
    }

    #[test]
    fn v_with_whitespace_around_parens() {
        let phrase = find_first_v_phrase("V ( \"spaced\" )").unwrap();
        assert_eq!(phrase, "spaced");
    }

    #[test]
    fn v_in_a_comment_is_ignored() {
        let q = "SELECT ?s WHERE { # V(\"commented\")\n ?s <http://ex/p> ?o }";
        let e = terse_to_sparql(q).expect("V in a comment is not a construct");
        assert_eq!(e.canonical_sparql, q);
    }

    // ---- lever 1: the K:<name> keyword layer (design §3.1, sq-vfeme) ----

    #[test]
    fn keyword_expands_to_canonical_iri_and_echoes() {
        // K:type and K:derivedFrom expand to their frozen IRIs; the output is canonical
        // SPARQL (it passes the canary) and every expansion is echoed for the agent.
        let q = "SELECT ?f WHERE { ?f K:type K:Finding ; K:derivedFrom ?s }";
        let e = terse_to_sparql(q).expect("known keywords expand");
        assert!(
            e.canonical_sparql
                .contains("<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>"),
            "got: {}",
            e.canonical_sparql
        );
        assert!(
            e.canonical_sparql
                .contains("<http://www.w3.org/ns/prov#wasDerivedFrom>"),
            "got: {}",
            e.canonical_sparql
        );
        assert!(
            e.canonical_sparql.contains("<https://sparq.dev/ns/pkg#Finding>"),
            "got: {}",
            e.canonical_sparql
        );
        assert!(!e.canonical_sparql.contains("K:"), "no K: token must remain");
        // Echoed, in source order, with the legend version.
        let kws: Vec<&str> = e.keywords.iter().map(|k| k.keyword.as_str()).collect();
        assert_eq!(kws, vec!["type", "Finding", "derivedFrom"]);
        assert!(e.keywords.iter().all(|k| k.legend_version == crate::keyword::LEGEND_VERSION));
    }

    #[test]
    fn unknown_keyword_is_a_loud_error_with_suggestions() {
        // A keyword not in the frozen legend is rejected loudly (never an invented IRI), with
        // a did-you-mean list — a near-miss surfaces the intended keyword.
        let q = "SELECT ?f WHERE { ?f K:derived ?s }";
        let err = terse_to_sparql(q).expect_err("an unknown keyword must fail loudly");
        match err {
            TerseError::UnknownKeyword { keyword, legend_version, suggestions } => {
                assert_eq!(keyword, "derived");
                assert_eq!(legend_version, crate::keyword::LEGEND_VERSION);
                assert!(suggestions.contains(&"derivedFrom".to_string()), "got {suggestions:?}");
            }
            other => panic!("expected UnknownKeyword, got {other:?}"),
        }
    }

    #[test]
    fn keyword_colliding_with_real_prefix_k_is_a_hard_error() {
        // If the query declares a real `PREFIX K:`, a K:<name> token is ambiguous between
        // keyword- and prefix-expansion: a hard error, never a silent guess (design §3.1).
        let q = "PREFIX K: <http://ex/> SELECT ?s WHERE { ?s K:type ?o }";
        let err = terse_to_sparql(q).expect_err("a real PREFIX K: collides with the sigil");
        assert!(matches!(err, TerseError::KeywordPrefixCollision { .. }), "got {err:?}");
    }

    #[test]
    fn unrelated_prefix_does_not_collide() {
        // A `PREFIX ex:` is fine — only a prefix literally named `K` collides.
        let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s K:type ex:Thing }";
        let e = terse_to_sparql(q).expect("an unrelated prefix does not collide");
        assert!(e.canonical_sparql.contains("<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>"));
        assert!(e.canonical_sparql.contains("ex:Thing"), "ordinary prefixed names pass through");
    }

    #[test]
    fn keyword_inside_string_iri_or_comment_is_not_expanded() {
        // K:<name> inside a literal / IRI / comment is data, not a keyword token.
        let q = "SELECT ?s WHERE { ?s <http://ex/K:type> \"K:label\" . # K:about\n }";
        let e = terse_to_sparql(q).expect("K: in non-code contexts passes through");
        assert_eq!(e.canonical_sparql, q, "must be byte-identical");
        assert!(e.keywords.is_empty());
    }

    #[test]
    fn glued_k_is_not_a_keyword_token() {
        // `?xK:type` — the K is glued to an identifier; not a standalone sigil.
        let spans = k_spans("?xK:type");
        assert!(spans.is_empty(), "glued K is not a keyword token, got {spans:?}");
    }

    #[test]
    fn bare_k_without_colon_or_name_passes_through() {
        // A lone `K` (e.g. a variable suffix scenario) or `K:` with no name is not a token.
        assert!(k_spans("SELECT ?K WHERE { ?K ?p ?o }").is_empty());
        assert!(k_spans("?s K: ?o").is_empty());
    }

    #[test]
    fn keyword_and_canary_compose() {
        // A keyword that expands to a predicate yields canonical SPARQL the canary accepts;
        // and a keyword expansion that would still leave broken SPARQL fails the canary
        // (loud, never executed).
        let ok = terse_to_sparql("ASK { ?s K:type ?o }").expect("valid after expansion");
        assert!(ok.canonical_sparql.starts_with("ASK"));
        let bad = terse_to_sparql("ASK { ?s K:type"); // unbalanced even after expansion
        assert!(matches!(bad, Err(TerseError::CanaryFailed { .. })), "got {bad:?}");
    }
}
