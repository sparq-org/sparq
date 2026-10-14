//! RSP-QL surface-syntax parser (sq-9u1, extended sq-2n1q3.2).
//!
//! The crate's public API is *programmatic*: you build a [`WindowSpec`] in Rust
//! and register plain SPARQL ([`ContinuousQuery`](crate::ContinuousQuery) etc.).
//! This module adds the **RSP-QL textual query language** — the streaming SPARQL
//! extension that names streams, declares windows over them, and references those
//! windows inside the `WHERE` clause — parsing it into the crate's existing
//! continuous-query / window representation. It is the front end for
//! [`ContinuousMultiQuery`](crate::ContinuousMultiQuery) (multi-window joins).
//!
//! # The supported subset
//!
//! ```text
//! REGISTER [RSTREAM|ISTREAM|DSTREAM] <output-stream-iri> AS
//! SELECT ...                       -- any spargebra-parseable SELECT projection
//! FROM NAMED WINDOW <w1> ON <s1> [RANGE <dur> STEP <dur> [T0 <dur>] [MAXDELAY <dur>]]
//! FROM NAMED WINDOW <w2> ON <s2> [TUMBLING RANGE <dur> [T0 <dur>] [MAXDELAY <dur>]]
//! FROM NAMED WINDOW <w3> ON <s3> [SLIDING RANGE <dur> STEP <dur> [T0 <dur>] [MAXDELAY <dur>]]
//! WHERE {
//!   WINDOW <w1> { ?s :p ?o }       -- a graph pattern scoped to window <w1>
//!   WINDOW <w2> { ?s :q ?x }       -- joined with <w2>'s pattern (shared vars)
//!   ...                            -- (plus any default-graph BGP / FILTER / …)
//! }
//! ```
//!
//! ## What is parsed
//!
//! * **`REGISTER [R2S] <out> AS`** — the registration header. The optional
//!   relation-to-stream operator (`RSTREAM` default / `ISTREAM` / `DSTREAM` /
//!   `STREAM`) maps onto [`R2S`](crate::R2S); the output-stream IRI is retained as
//!   metadata ([`RspqlQuery::output_stream`]).
//! * **`FROM NAMED WINDOW <w> ON <stream> [<window-op>]`** — one *physical*
//!   window declaration. Three textual window-operator forms are accepted:
//!   - `RANGE r [STEP s]` — the W3C-community form; `STEP` defaults to `r`
//!     (tumbling) when omitted.
//!   - `TUMBLING RANGE r` — explicit tumbling-window keyword (equivalent to
//!     `RANGE r`, i.e. `step = range`). Brackets optional: `[TUMBLING RANGE r]`.
//!   - `SLIDING RANGE r STEP s` — explicit sliding-window keyword; `STEP` is
//!     required (rejected with a clear error if omitted). Brackets optional.
//!
//!   Durations are ISO-8601 (`PT10S`, `PT1M30S`, `PT2H`) or a bare integer
//!   (logical ticks — the crate's native `u64` timestamp scale). Every window is
//!   bound to its `<stream>` so a push routes to the windows over that stream.
//!   With the default-off `window-origin` feature, time-window declarations may
//!   append `T0 <dur>` and then `MAXDELAY <dur>` in that fixed order. These set
//!   the window origin and lateness tolerance; either clause may be omitted.
//! * **`WHERE { WINDOW <w> { … } … }`** — the window graph patterns. Each
//!   `WINDOW <w>` is rewritten to a standard SPARQL `GRAPH <w>` pattern; the
//!   engine then evaluates it against the per-window materialised named graph
//!   (see [`ContinuousMultiQuery`](crate::ContinuousMultiQuery)). Everything
//!   *inside* the braces — BGPs, FILTERs, OPTIONAL, nested patterns — is left
//!   untouched for spargebra to parse, so the full embedded SPARQL algebra is
//!   supported without re-implementing a SPARQL parser here.
//!
//! ## Scoped out — fail closed with a clear error (NOT silently dropped)
//!
//! * **`ROWS n`** — count-window textual form: rejected with
//!   `"ROWS (count) windows are not supported in the RSP-QL textual surface
//!   grammar; use the programmatic WindowSpec::count builder"`. The programmatic
//!   [`WindowSpec::count`](crate::WindowSpec::count) builder is the supported path.
//! * **`NOW`-relative window bounds** (`NOW-PT10S TO NOW` etc.) — rejected with
//!   `"NOW-relative window bounds … are not supported; use RANGE/STEP"`.
//! * **`SLIDING RANGE r` without a `STEP`** — rejected with
//!   `"SLIDING window declaration requires a STEP duration"`.
//! * **`TUMBLING RANGE r STEP s`** — rejected with a clear error: `TUMBLING`
//!   implies `step == range`; a contradictory `STEP` clause is not allowed.
//!   Use `SLIDING RANGE r STEP s` for an explicit sliding step.
//! * **Named-window *variables*** (`FROM NAMED WINDOW ?w …`, `WINDOW ?w { … }`)
//!   — RSP-QL permits a window variable ranging over declared windows; we
//!   require a concrete window IRI (the common case). A `WINDOW ?w` is rejected
//!   with a clear error. (The window/stream IRIs may be written as `<…>` IRIs OR
//!   as prefixed names — `ex:w1`, `:w1` — resolved against the body's `PREFIX` /
//!   `BASE` declarations, so the compact W3C-example forms parse.)
//! * **Stream-graph `GRAPH`** vs **`WINDOW`**: a literal `GRAPH` clause in the
//!   `WHERE` is passed through to spargebra unchanged (it is ordinary SPARQL).
//!   Only `WINDOW` is RSP-QL.

use std::collections::HashMap;

use oxrdf::NamedNode;

use crate::query::R2S;
use crate::window::WindowSpec;

/// The `PREFIX p: <iri>` / `BASE <iri>` declarations harvested from the query
/// body, used to resolve prefixed names (`ex:w1`) and the empty prefix (`:w1`)
/// in the RSP-QL extension clauses (`FROM NAMED WINDOW`, `REGISTER`). spargebra
/// resolves them for the embedded SPARQL itself; we mirror them here so the
/// window/stream IRIs accept the same compact forms.
#[derive(Debug, Default)]
struct Prefixes {
    map: HashMap<String, String>,
    base: Option<String>,
}

impl Prefixes {
    /// Harvests every `PREFIX <pname>: <iri>` and `BASE <iri>` declaration from
    /// the query text (case-insensitive keywords). Later declarations win, as in
    /// SPARQL.
    fn harvest(text: &str) -> Prefixes {
        let mut p = Prefixes::default();
        let mut rest = text;
        while !rest.is_empty() {
            rest = rest.trim_start();
            if let Some(after) = keyword_prefix(rest, "PREFIX") {
                let after = after.trim_start();
                // `pname:` up to the colon (may be empty for the default prefix).
                if let Some(colon) = after.find(':') {
                    let pname = after[..colon].trim().to_owned();
                    if let Some((iri, tail)) = take_plain_iri(after[colon + 1..].trim_start()) {
                        p.map.insert(pname, iri);
                        rest = tail;
                        continue;
                    }
                }
            } else if let Some(after) = keyword_prefix(rest, "BASE") {
                if let Some((iri, tail)) = take_plain_iri(after.trim_start()) {
                    p.base = Some(iri);
                    rest = tail;
                    continue;
                }
            }
            // Advance one char and keep scanning for the next declaration.
            match rest.chars().next() {
                Some(c) => rest = &rest[c.len_utf8()..],
                None => break,
            }
        }
        p
    }

    /// Resolves `prefix:local` against the harvested prefixes. `None` if the
    /// prefix is undeclared.
    fn resolve(&self, prefix: &str, local: &str) -> Option<String> {
        self.map.get(prefix).map(|ns| format!("{ns}{local}"))
    }
}

/// One parsed RSP-QL window declaration: its IRI, the stream it reads, and the
/// [`WindowSpec`] (S2R operator) it applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDecl {
    /// The window IRI (`FROM NAMED WINDOW <this> …`); also the named-graph key
    /// the `WHERE`'s `WINDOW <this> { … }` resolves against.
    pub window: NamedNode,
    /// The stream IRI this window reads (`… ON <this>`). Pushes tagged with this
    /// stream feed this window.
    pub stream: NamedNode,
    /// The S2R window operator (`RANGE … STEP …`).
    pub spec: WindowSpec,
}

/// A parsed RSP-QL query: the registration metadata, the window declarations,
/// and a standard SPARQL string (with `WINDOW <w>` rewritten to `GRAPH <w>`)
/// ready for [`PreparedQuery`](sparq_engine::PreparedQuery).
#[derive(Debug, Clone)]
pub struct RspqlQuery {
    /// The `REGISTER <R2S> <output-stream> AS` output-stream IRI, if present.
    pub output_stream: Option<NamedNode>,
    /// The relation-to-stream operator from the `REGISTER` header (default
    /// [`R2S::RStream`]).
    pub r2s: R2S,
    /// Every `FROM NAMED WINDOW … ON … [RANGE … STEP …]` declaration, in source
    /// order.
    pub windows: Vec<WindowDecl>,
    /// The embedded SPARQL query with the RSP-QL extensions removed: the
    /// `REGISTER` header and `FROM NAMED WINDOW` clauses are stripped and every
    /// `WINDOW <w> { … }` is rewritten to `GRAPH <w> { … }`. Parseable by
    /// spargebra as-is.
    pub sparql: String,
}

impl RspqlQuery {
    /// Parses an RSP-QL surface-syntax query into its structured form. See the
    /// [`RspqlQuery`] type docs for the supported grammar and the scoped-out
    /// constructs. Errors describe the offending construct.
    pub fn parse(text: &str) -> Result<RspqlQuery, String> {
        // Harvest PREFIX/BASE up front so the RSP-QL clauses can use the same
        // compact IRI forms (`ex:w1`, `:w1`) the embedded SPARQL does.
        let prefixes = Prefixes::harvest(text);
        let (output_stream, r2s, after_register) = parse_register(text, &prefixes)?;
        let (windows, body) = parse_window_decls(after_register, &prefixes)?;
        if windows.is_empty() {
            return Err("RSP-QL query declares no FROM NAMED WINDOW".into());
        }
        // Reject duplicate window IRIs: two declarations of the same window
        // would make `WINDOW <w>` ambiguous (and clash as a named-graph key).
        for (i, w) in windows.iter().enumerate() {
            if windows[..i].iter().any(|p| p.window == w.window) {
                return Err(format!(
                    "window <{}> declared more than once",
                    w.window.as_str()
                ));
            }
        }
        let sparql = rewrite_window_to_graph(&body)?;
        Ok(RspqlQuery {
            output_stream,
            r2s,
            windows,
            sparql,
        })
    }
}

/// Parses the optional `REGISTER [R2S] <out> AS` header, returning the output
/// stream, the R2S operator, and the remaining text (the SPARQL body, possibly
/// still carrying `FROM NAMED WINDOW` clauses). A query with no `REGISTER`
/// header is accepted (the whole text is the body) with the defaults.
fn parse_register<'a>(
    text: &'a str,
    prefixes: &Prefixes,
) -> Result<(Option<NamedNode>, R2S, &'a str), String> {
    let trimmed = text.trim_start();
    let mut rest = match keyword_prefix(trimmed, "REGISTER") {
        Some(r) => r,
        // No REGISTER header: a bare RSP-QL body (the FROM NAMED WINDOW form).
        None => return Ok((None, R2S::RStream, trimmed)),
    };
    rest = rest.trim_start();
    // Optional R2S operator. The W3C-community grammar writes `REGISTER STREAM`
    // (the default RSTREAM); we also accept `REGISTER RSTREAM/ISTREAM/DSTREAM`.
    let mut r2s = R2S::RStream;
    for (kw, op) in [
        ("STREAM", R2S::RStream),
        ("RSTREAM", R2S::RStream),
        ("ISTREAM", R2S::IStream),
        ("DSTREAM", R2S::DStream),
    ] {
        if let Some(after) = keyword_prefix(rest, kw) {
            r2s = op;
            rest = after.trim_start();
            break;
        }
    }
    // The output-stream IRI.
    let (iri, after_iri) = take_iri(rest, prefixes)
        .ok_or("REGISTER must name an output stream IRI: `REGISTER [R2S] <iri> AS …`")?;
    rest = after_iri.trim_start();
    rest = keyword_prefix(rest, "AS").ok_or("expected `AS` after the REGISTER output stream")?;
    Ok((Some(iri), r2s, rest))
}

/// Splits the SELECT/ASK/CONSTRUCT-prefixed body into the `FROM NAMED WINDOW`
/// declarations and the residual SPARQL (header + WHERE) with those clauses
/// removed. Window declarations may appear in their SPARQL-standard position
/// (between the projection and the `WHERE`).
fn parse_window_decls(
    body: &str,
    prefixes: &Prefixes,
) -> Result<(Vec<WindowDecl>, String), String> {
    let mut windows = Vec::new();
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    loop {
        match find_keyword(rest, "FROM") {
            Some(idx) => {
                // A `FROM` — but only `FROM NAMED WINDOW` is ours; a plain `FROM`
                // / `FROM NAMED` dataset clause is passed through to spargebra.
                let after_from = &rest[idx + "FROM".len()..];
                let after_from_ws = after_from.trim_start();
                let named = keyword_prefix(after_from_ws, "NAMED");
                let window_after = named.and_then(|a| keyword_prefix(a.trim_start(), "WINDOW"));
                match window_after {
                    Some(window_clause) => {
                        // Emit everything up to the FROM, then consume the clause.
                        out.push_str(&rest[..idx]);
                        out.push(' ');
                        let (decl, after_decl) = parse_one_window_decl(window_clause, prefixes)?;
                        windows.push(decl);
                        rest = after_decl;
                    }
                    None => {
                        // Not a window clause: copy through `FROM` and continue
                        // scanning after it.
                        out.push_str(&rest[..idx + "FROM".len()]);
                        rest = after_from;
                    }
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    Ok((windows, out))
}

/// Parses one `<w> ON <stream> [RANGE r STEP s]` declaration (the `FROM NAMED
/// WINDOW` keywords already consumed), returning it and the text after it. The
/// window/stream IRIs may be `<…>` IRIs or prefixed names (`ex:w1`, `:w1`).
fn parse_one_window_decl<'a>(
    clause: &'a str,
    prefixes: &Prefixes,
) -> Result<(WindowDecl, &'a str), String> {
    let clause = clause.trim_start();
    if clause.starts_with('?') || clause.starts_with('$') {
        return Err(
            "window VARIABLES (FROM NAMED WINDOW ?w …) are not supported; \
                    name a concrete window IRI"
                .into(),
        );
    }
    let (window, rest) =
        take_iri(clause, prefixes).ok_or("FROM NAMED WINDOW must be followed by a window IRI")?;
    let rest = keyword_prefix(rest.trim_start(), "ON")
        .ok_or("expected `ON <stream>` after the window IRI")?;
    let (stream, rest) = take_iri(rest.trim_start(), prefixes)
        .ok_or("FROM NAMED WINDOW <w> ON must be followed by a stream IRI")?;
    let (spec, rest) = parse_window_spec(rest.trim_start())?;
    Ok((
        WindowDecl {
            window,
            stream,
            spec,
        },
        rest,
    ))
}

/// Parses the window operator. Accepted forms (brackets optional in all cases):
///
/// - `[RANGE r [STEP s] [T0 t0] [MAXDELAY d]]` — W3C-community form; `STEP`
///   defaults to `r` (tumbling).
/// - `[TUMBLING RANGE r [T0 t0] [MAXDELAY d]]` — explicit tumbling keyword;
///   equivalent to `RANGE r`.
/// - `[SLIDING RANGE r STEP s [T0 t0] [MAXDELAY d]]` — explicit sliding
///   keyword; `STEP` is required.
///
/// `T0` and `MAXDELAY` require the default-off `window-origin` feature and must
/// occur in that order. Both use the same duration lexer as `RANGE` and `STEP`.
///
/// Rejected with a clear error (fail-closed, never silently mis-parsed):
///
/// - `ROWS n` — count-window textual form; use the programmatic
///   `WindowSpec::count` builder instead.
/// - `NOW …` — NOW-relative window bounds; use `RANGE/STEP`.
/// - `SLIDING RANGE r` without `STEP` — `STEP` is required for `SLIDING`.
/// - `TUMBLING RANGE r STEP s` — `STEP` is contradictory with `TUMBLING`
///   (tumbling implies `step == range`); use `SLIDING RANGE r STEP s` instead.
///
/// Returns the parsed [`WindowSpec`] and the text after the operator.
fn parse_window_spec(text: &str) -> Result<(WindowSpec, &str), String> {
    let text = text.trim_start();
    // Optional opening bracket.
    let (bracketed, text) = match text.strip_prefix('[') {
        Some(inner) => (true, inner.trim_start()),
        None => (false, text),
    };

    // ROWS n — count-window textual form: not supported in the surface grammar.
    // Fail closed so callers get a diagnostic, not a mis-parse.
    if keyword_prefix(text, "ROWS").is_some() {
        #[cfg(feature = "window-origin")]
        if let Some(after_rows) = keyword_prefix(text, "ROWS") {
            if let Ok((_, after_count)) = take_duration_for(after_rows.trim_start(), "ROWS") {
                let after_count = after_count.trim_start();
                if keyword_prefix(after_count, "T0").is_some()
                    || keyword_prefix(after_count, "MAXDELAY").is_some()
                {
                    return Err(
                        "T0 and MAXDELAY apply to time windows only; they cannot be attached to a ROWS count window"
                            .into(),
                    );
                }
            }
        }
        return Err(
            "ROWS (count) windows are not supported in the RSP-QL textual surface grammar; \
             use the programmatic `WindowSpec::count` builder instead"
                .into(),
        );
    }

    // NOW-relative window bounds — not supported; fail closed with a clear hint.
    if keyword_prefix(text, "NOW").is_some() {
        return Err(
            "NOW-relative window bounds (e.g. NOW-PT10S TO NOW) are not supported in this parser; \
             express windows as `RANGE <dur> [STEP <dur>]`"
                .into(),
        );
    }

    // TUMBLING RANGE r — explicit tumbling keyword (step == range).
    // SLIDING RANGE r STEP s — explicit sliding keyword (STEP required).
    // Track both flags: explicit_tumbling lets us reject TUMBLING + STEP.
    let (explicit_tumbling, explicit_sliding, text) =
        if let Some(after) = keyword_prefix(text, "TUMBLING") {
            // TUMBLING: semantically identical to plain RANGE r (step = range).
            // An explicit STEP is contradictory and must be rejected (see below).
            (true, false, after.trim_start())
        } else if let Some(after) = keyword_prefix(text, "SLIDING") {
            // SLIDING: a STEP is mandatory — reject without it (see below).
            (false, true, after.trim_start())
        } else {
            (false, false, text)
        };

    let rest = keyword_prefix(text, "RANGE").ok_or(
        "window declaration needs a `[RANGE <dur> [STEP <dur>]]`, \
         `[TUMBLING RANGE <dur>]`, or `[SLIDING RANGE <dur> STEP <dur>]` operator",
    )?;
    let (range, rest) = take_duration(rest.trim_start())?;
    let rest_ws = rest.trim_start();
    let (spec, rest) = if let Some(after_step) = keyword_prefix(rest_ws, "STEP") {
        // TUMBLING with an explicit STEP is contradictory: tumbling
        // means step == range by definition. Reject rather than silently accepting
        // an inconsistent spec.
        if explicit_tumbling {
            return Err("TUMBLING windows have step == range implicitly; \
                 a STEP clause is not allowed with TUMBLING — \
                 use SLIDING RANGE <dur> STEP <dur> for an explicit step"
                .into());
        }
        let (step, rest) = take_duration(after_step.trim_start())?;
        (WindowSpec::time(range, step), rest)
    } else if explicit_sliding {
        // SLIDING without STEP is a user error — STEP defines the slide interval.
        return Err("SLIDING window declaration requires a STEP duration: \
             `SLIDING RANGE <dur> STEP <dur>`"
            .into());
    } else {
        // No STEP and not a SLIDING keyword: tumbling (step == range).
        (WindowSpec::time(range, range), rest_ws)
    };
    // sq-ayn0i: this call is compiled out entirely by default so the
    // established parser's returned spec, residual text, and errors stay exact.
    #[cfg(feature = "window-origin")]
    let (spec, rest) = parse_window_origin_clauses(spec, rest)?;
    // Consume the matching closing bracket if we opened one.
    let rest = rest.trim_start();
    if bracketed {
        let rest = rest
            .strip_prefix(']')
            .ok_or("unterminated window operator: expected a closing `]`")?;
        Ok((spec, rest))
    } else {
        Ok((spec, rest))
    }
}

/// Parses the default-off time-window origin and lateness clauses. Clauses are
/// optional but ordered: `T0` must precede `MAXDELAY` when both are present.
#[cfg(feature = "window-origin")]
fn parse_window_origin_clauses(
    mut spec: WindowSpec,
    rest: &str,
) -> Result<(WindowSpec, &str), String> {
    let mut rest = rest.trim_start();
    let mut saw_t0 = false;
    let mut saw_max_delay = false;

    if let Some(after_t0) = keyword_prefix(rest, "T0") {
        let (t0, after_t0) = take_duration_for(after_t0.trim_start(), "T0")?;
        spec = spec.with_t0(t0);
        saw_t0 = true;
        rest = after_t0.trim_start();
    }

    if let Some(after_max_delay) = keyword_prefix(rest, "MAXDELAY") {
        let (max_delay, after_max_delay) =
            take_duration_for(after_max_delay.trim_start(), "MAXDELAY")?;
        spec = spec.with_max_delay(max_delay);
        saw_max_delay = true;
        rest = after_max_delay.trim_start();
    }

    if keyword_prefix(rest, "T0").is_some() {
        return Err(if saw_t0 {
            "duplicate T0 clause in window declaration".into()
        } else {
            "T0 must precede MAXDELAY in a window declaration".into()
        });
    }
    if keyword_prefix(rest, "MAXDELAY").is_some() {
        return Err(if saw_max_delay {
            "duplicate MAXDELAY clause in window declaration".into()
        } else {
            "MAXDELAY clause is out of order in window declaration".into()
        });
    }

    Ok((spec, rest))
}

/// Rewrites every `WINDOW <iri>` token in the SPARQL body to `GRAPH <iri>` so
/// spargebra parses the per-window pattern as a named-graph pattern. Operates on
/// whitespace-delimited tokens outside IRIs/strings; a `WINDOW ?var` is
/// rejected (window variables are out of scope).
fn rewrite_window_to_graph(body: &str) -> Result<String, String> {
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // Copy IRIs (<…>) and quoted strings verbatim — a `WINDOW` substring
        // inside one is data, not a keyword.
        if b == b'<' {
            let end = body[i..]
                .find('>')
                .map(|j| i + j + 1)
                .unwrap_or(bytes.len());
            out.push_str(&body[i..end]);
            i = end;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let end = scan_string(body, i, b as char);
            out.push_str(&body[i..end]);
            i = end;
            continue;
        }
        // Try to match the WINDOW keyword at a token boundary.
        if (b == b'W' || b == b'w') && is_token_start(body, i) {
            if let Some(after) = keyword_prefix(&body[i..], "WINDOW") {
                if after.trim_start().starts_with('?') || after.trim_start().starts_with('$') {
                    return Err("WINDOW variables (WINDOW ?w { … }) are not supported; \
                                reference a concrete window IRI"
                        .into());
                }
                // Replace just the keyword; `after` keeps the original
                // whitespace + window IRI, which spargebra parses unchanged.
                out.push_str("GRAPH");
                i = body.len() - after.len(); // resume at the byte after WINDOW
                continue;
            }
        }
        // Copy one whole UTF-8 char (the body may carry unicode in prefixed
        // names / local parts outside IRIs and strings) and advance past it.
        let char_len = utf8_char_len(b);
        out.push_str(&body[i..i + char_len]);
        i += char_len;
    }
    Ok(out)
}

/// The byte length of the UTF-8 sequence whose lead byte is `b` (1–4). All other
/// scan helpers slice by byte ranges, which are char-boundary-safe because they
/// only cut at ASCII delimiters (`<`, `>`, quotes, whitespace).
fn utf8_char_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

// ----------------------------------------------------------------- tokenizing

/// `true` if byte offset `i` begins a token (start of input or preceded by a
/// non-identifier byte), so a keyword match is not the tail of a longer word.
fn is_token_start(s: &str, i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = s.as_bytes()[i - 1];
    !(prev.is_ascii_alphanumeric() || prev == b'_')
}

/// If `s` starts with `kw` (ASCII case-insensitive) at a word boundary, returns
/// the remainder after the keyword; otherwise `None`. A trailing identifier
/// byte means it is a longer word, not the keyword.
fn keyword_prefix<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    if s.len() < kw.len() || !s.as_bytes()[..kw.len()].eq_ignore_ascii_case(kw.as_bytes()) {
        return None;
    }
    let after = &s[kw.len()..];
    match after.bytes().next() {
        Some(b) if b.is_ascii_alphanumeric() || b == b'_' => None,
        _ => Some(after),
    }
}

/// Finds the byte offset of the next standalone occurrence of `kw` (case
/// insensitive, word-boundaried) OUTSIDE any `<…>` IRI or quoted string.
fn find_keyword(s: &str, kw: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'<' {
            i = s[i..].find('>').map(|j| i + j + 1).unwrap_or(bytes.len());
            continue;
        }
        if b == b'"' || b == b'\'' {
            i = scan_string(s, i, b as char);
            continue;
        }
        if is_token_start(s, i) && keyword_prefix(&s[i..], kw).is_some() {
            return Some(i);
        }
        // Advance one whole UTF-8 char so `&s[i..]` always cuts on a boundary.
        i += utf8_char_len(b);
    }
    None
}

/// Returns the offset just past a quoted string starting at `start` (the quote
/// char `q`), honouring backslash escapes. Used to skip strings while scanning.
fn scan_string(s: &str, start: usize, q: char) -> usize {
    let bytes = s.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '\\' {
            i += 2;
            continue;
        }
        if c == q {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// Parses a leading IRI token — either a `<iri>` or a prefixed name
/// `prefix:local` / `:local` resolved against `prefixes` — returning the
/// [`NamedNode`] and the remaining text. `None` if neither form is present (or a
/// prefixed name's prefix is undeclared / the IRI is invalid).
fn take_iri<'a>(s: &'a str, prefixes: &Prefixes) -> Option<(NamedNode, &'a str)> {
    let s = s.trim_start();
    if s.starts_with('<') {
        let (iri, rest) = take_plain_iri(s)?;
        // Resolve a relative IRI against BASE if one was declared.
        let resolved = match &prefixes.base {
            Some(base) if NamedNode::new(&iri).is_err() => format!("{base}{iri}"),
            _ => iri,
        };
        return Some((NamedNode::new(resolved).ok()?, rest));
    }
    // A prefixed name: `pname:local` or `:local` (empty prefix). The local part
    // runs to the next delimiter (whitespace, `{`, `[`, `]`, or end).
    let colon = s.find(':')?;
    // Guard: a `:` only counts as a prefix separator if what precedes it is a
    // valid (possibly empty) prefix label, not e.g. part of a keyword.
    let prefix = &s[..colon];
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let after = &s[colon + 1..];
    let end = after
        .find(|c: char| c.is_whitespace() || c == '{' || c == '[' || c == ']' || c == '<')
        .unwrap_or(after.len());
    let local = &after[..end];
    let iri = prefixes.resolve(prefix, local)?;
    Some((NamedNode::new(iri).ok()?, &after[end..]))
}

/// Parses a leading `<iri>` token only (no prefixed-name resolution), returning
/// the raw IRI string and the text after the closing `>`. Used both by
/// [`take_iri`] and by PREFIX/BASE harvesting.
fn take_plain_iri(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    if !s.starts_with('<') {
        return None;
    }
    let close = s.find('>')?;
    Some((s[1..close].to_owned(), &s[close + 1..]))
}

/// Parses a leading duration token — ISO-8601 (`PT10S`, `PT1M30S`, `PT2H`,
/// `P1D`) or a bare unsigned integer (logical ticks) — into a `u64` count on
/// the stream's timestamp scale (ISO-8601 resolves to SECONDS). Returns the
/// value and the remaining text.
fn take_duration(s: &str) -> Result<(u64, &str), String> {
    take_duration_for(s, "RANGE/STEP")
}

/// Shared duration-literal lexer for all window clauses.
fn take_duration_for<'a>(s: &'a str, clause: &str) -> Result<(u64, &'a str), String> {
    let s = s.trim_start();
    let end = s
        .find(|c: char| c.is_whitespace() || c == '{' || c == ']' || c == '[')
        .unwrap_or(s.len());
    let (tok, rest) = s.split_at(end);
    if tok.is_empty() {
        return Err(format!("expected a duration after {clause}"));
    }
    let value = parse_duration_token(tok)?;
    Ok((value, rest))
}

/// ISO-8601 duration (seconds resolution) or a bare integer. Kept deliberately
/// small: the crate's timestamps are application `u64`s, so ISO durations are
/// only a convenience that resolves to a second count.
fn parse_duration_token(tok: &str) -> Result<u64, String> {
    // Bare integer: logical ticks on whatever scale the embedder pushes.
    if let Ok(n) = tok.parse::<u64>() {
        return Ok(n);
    }
    let upper = tok.to_ascii_uppercase();
    let body = upper.strip_prefix('P').ok_or_else(|| {
        format!("not a duration: `{tok}` (want ISO-8601 like PT10S or an integer)")
    })?;
    // Split into the date part (before 'T') and the time part (after).
    let (date_part, time_part) = match body.split_once('T') {
        Some((d, t)) => (d, t),
        None => (body, ""),
    };
    let mut total: u64 = 0;
    let mut num = String::new();
    // Date part: only whole DAYS are supported (D); Y/M/W are calendar-relative
    // and have no fixed second count, so they are rejected.
    for c in date_part.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: u64 = num
                .parse()
                .map_err(|_| format!("bad duration component in `{tok}`"))?;
            num.clear();
            match c {
                'D' => total += n * 86_400,
                'Y' | 'M' | 'W' => {
                    return Err(format!(
                        "duration `{tok}`: years/months/weeks have no fixed second \
                         count — use days (D), hours/minutes/seconds, or a bare integer"
                    ))
                }
                _ => return Err(format!("bad duration unit `{c}` in `{tok}`")),
            }
        }
    }
    if !num.is_empty() {
        return Err(format!("dangling number in duration `{tok}`"));
    }
    // Time part: H / M / S.
    for c in time_part.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: u64 = num
                .parse()
                .map_err(|_| format!("bad duration component in `{tok}`"))?;
            num.clear();
            match c {
                'H' => total += n * 3_600,
                'M' => total += n * 60,
                'S' => total += n,
                _ => return Err(format!("bad time-duration unit `{c}` in `{tok}`")),
            }
        }
    }
    if !num.is_empty() {
        return Err(format!("dangling number in duration `{tok}`"));
    }
    Ok(total)
}

#[cfg(all(test, feature = "window-origin"))]
mod window_origin_tests {
    use super::*;

    fn query(operator: &str) -> String {
        format!(
            "SELECT * WHERE {{ WINDOW <http://ex/w> {{ ?s ?p ?o }} }}\n\
             FROM NAMED WINDOW <http://ex/w> ON <http://ex/s> {operator}"
        )
    }

    #[test]
    fn parses_t0_and_maxdelay_exactly() {
        let parsed = RspqlQuery::parse(&query("RANGE 10 STEP 5 T0 100 MAXDELAY 3")).unwrap();
        assert_eq!(
            parsed.windows[0].spec,
            WindowSpec::time(10, 5).with_t0(100).with_max_delay(3)
        );
    }

    #[test]
    fn parses_t0_without_maxdelay() {
        let parsed = RspqlQuery::parse(&query("RANGE PT10S STEP PT5S T0 PT1M")).unwrap();
        assert_eq!(parsed.windows[0].spec, WindowSpec::time(10, 5).with_t0(60));
    }

    #[test]
    fn parses_maxdelay_without_t0() {
        let parsed = RspqlQuery::parse(&query("RANGE 10 STEP 5 MAXDELAY 3")).unwrap();
        assert_eq!(
            parsed.windows[0].spec,
            WindowSpec::time(10, 5).with_max_delay(3)
        );
    }

    #[test]
    fn parses_sliding_and_bracketed_clauses() {
        let parsed =
            RspqlQuery::parse(&query("[SLIDING RANGE 10 STEP 5 T0 100 MAXDELAY 3]")).unwrap();
        assert_eq!(
            parsed.windows[0].spec,
            WindowSpec::time(10, 5).with_t0(100).with_max_delay(3)
        );
    }

    #[test]
    fn declaration_without_new_clauses_is_unchanged() {
        let parsed = RspqlQuery::parse(&query("RANGE 10 STEP 5")).unwrap();
        assert_eq!(parsed.windows[0].spec, WindowSpec::time(10, 5));
    }

    #[test]
    fn rejects_origin_clauses_on_rows_without_panicking() {
        for operator in ["ROWS 10 T0 100", "ROWS 10 MAXDELAY 3"] {
            let error = RspqlQuery::parse(&query(operator)).unwrap_err();
            assert_eq!(
                error,
                "T0 and MAXDELAY apply to time windows only; they cannot be attached to a ROWS count window"
            );
        }
    }

    #[test]
    fn rejects_duplicate_clauses() {
        let t0_error = RspqlQuery::parse(&query("RANGE 10 STEP 5 T0 100 T0 200")).unwrap_err();
        assert_eq!(t0_error, "duplicate T0 clause in window declaration");

        let max_delay_error =
            RspqlQuery::parse(&query("RANGE 10 STEP 5 MAXDELAY 3 MAXDELAY 4")).unwrap_err();
        assert_eq!(
            max_delay_error,
            "duplicate MAXDELAY clause in window declaration"
        );
    }

    #[test]
    fn rejects_reversed_clause_order() {
        let error = RspqlQuery::parse(&query("RANGE 10 STEP 5 MAXDELAY 3 T0 100")).unwrap_err();
        assert_eq!(error, "T0 must precede MAXDELAY in a window declaration");
    }

    #[test]
    fn rejects_malformed_clause_durations() {
        let t0_error = RspqlQuery::parse(&query("RANGE 10 STEP 5 T0 bogus")).unwrap_err();
        assert!(
            t0_error.contains("not a duration: `bogus`"),
            "got: {t0_error}"
        );

        let max_delay_error =
            RspqlQuery::parse(&query("RANGE 10 STEP 5 MAXDELAY PT1X")).unwrap_err();
        assert!(
            max_delay_error.contains("bad time-duration unit `X`"),
            "got: {max_delay_error}"
        );
    }
}

#[cfg(all(test, not(feature = "window-origin")))]
mod window_origin_default_tests {
    use super::*;

    #[test]
    fn new_unbracketed_clauses_preserve_the_existing_residual_sparql() {
        let query = "SELECT * WHERE { WINDOW <http://ex/w> { ?s ?p ?o } }\n\
                     FROM NAMED WINDOW <http://ex/w> ON <http://ex/s> \
                     RANGE 10 STEP 5 T0 100 MAXDELAY 3";
        let parsed = RspqlQuery::parse(query).unwrap();
        assert_eq!(parsed.windows[0].spec, WindowSpec::time(10, 5));
        assert!(parsed.sparql.contains("T0 100 MAXDELAY 3"));
        let downstream_error = sparq_engine::PreparedQuery::parse(&parsed.sparql).unwrap_err();
        assert_eq!(
            downstream_error,
            "error at 2:8: expected one of HAVING, OFFSET, VALUES"
        );
    }

    #[test]
    fn bracketed_new_clauses_keep_the_existing_exact_error() {
        let query = "SELECT * WHERE { WINDOW <http://ex/w> { ?s ?p ?o } }\n\
                     FROM NAMED WINDOW <http://ex/w> ON <http://ex/s> \
                     [RANGE 10 STEP 5 T0 100 MAXDELAY 3]";
        assert_eq!(
            RspqlQuery::parse(query).unwrap_err(),
            "unterminated window operator: expected a closing `]`"
        );
    }
}
