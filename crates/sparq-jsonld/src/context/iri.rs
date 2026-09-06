//! IRI Expansion (JSON-LD 1.1 API §5.2) and RFC 3986 reference resolution.
//!
//! [OPUS-4.8] (sq-oy1f.24) IRI Expansion resolves a term / compact IRI / relative
//! reference against an [`ActiveContext`], honouring keyword
//! aliases, `@vocab`, `@base`, and on-demand term creation from a local context. The
//! compaction-side companions (IRI Compaction, Term Selection, inverse-context
//! construction) are deferred to a follow-on bead — see the module docs.

use std::collections::BTreeMap;

use super::process::{create_term_definition, Env};
use super::{has_keyword_form, is_keyword, ActiveContext};
use crate::error::JsonLdError;
use crate::json::Json;

/// True iff `s` is a blank node identifier (`_:` prefix).
pub(crate) fn is_blank_node(s: &str) -> bool {
    s.starts_with("_:")
}

/// True iff `s` is an absolute IRI per the RFC 3987 §2.2 `IRI` production:
///
/// ```text
/// IRI = scheme ":" ihier-part [ "?" iquery ] [ "#" ifragment ]
/// ```
///
/// [OPUS-5] sq-gzsky — this is the "Processors MUST validate datatype IRIs" obligation
/// behind the `invalid typed value` negative (W3C expand/0123 pins
/// `"http://example.com/baz z"`, a scheme-valid string carrying a SPACE). Scheme validity
/// alone accepted it.
///
/// [SONNET-4.6] (PR #4610 review) This grew in three rounds. The first cut was a character
/// *denylist* with no escape validation (`http://example.com/%ZZ` passed); round one made
/// it a complete code-point allowlist plus `pct-encoded` validation; round two — this one —
/// makes it the **structural** grammar, so a string that is character-legal but
/// component-malformed no longer passes. The witnesses that motivated it: `http://[` (an
/// unterminated IP-literal) and `http://example.com:bad-port` (`port = *DIGIT`).
///
/// What is now enforced beyond the code-point set:
/// * authority shape — `[ iuserinfo "@" ] ihost [ ":" port ]`, with `ihost` one of an
///   IP-literal (`"[" ( IPv6address / IPvFuture ) "]"`), an IPv4address, or an `ireg-name`;
/// * `[` and `]` inside the authority only as IP-literal delimiters, so an unterminated
///   `http://[` or a bracketed reg-name is rejected;
/// * `port = *DIGIT`;
/// * `iprivate` in the **query only** — `ipchar` admits `ucschar` but not `iprivate`;
/// * component split — the fragment opens at the first `#` and the query at the first `?`
///   before it, so a stray second `#` is rejected.
///
/// **The one deliberate deviation, with its evidence.** RFC 3987 excludes the `gen-delims`
/// `[` and `]` from `ipchar`, so a strict reading rejects them in a path, query, or
/// fragment. The W3C JSON-LD suite requires the opposite: `compact/p004` ("Compact IRIs
/// using simple terms ending with gen-delim") is a *PositiveEvaluationTest* whose context
/// maps `lbracket` to `http://example.org/[` and whose input carries
/// `http://example.org/[foo`, and those values must survive term-definition validation as
/// IRI mappings. Enforcing `ipchar` literally in those three components costs that case.
/// So outside the authority the check stays at the code-point level and admits the
/// bracket gen-delims; structural validation applies to the authority, where the
/// malformed-but-character-legal strings actually arise and where nothing in the suite
/// conflicts. This is a bounded, test-pinned deviation, not an unexamined gap.
pub(crate) fn is_absolute_iri(s: &str) -> bool {
    let Some(colon) = s.find(':') else {
        return false;
    };
    if !is_valid_scheme(&s[..colon]) {
        return false;
    }
    // Peel the trailing components off in grammar order: the fragment opens at the first
    // `#`, the query at the first `?` preceding it. A second `#` therefore lands *inside*
    // the fragment, where it is not an `ifragment` character — correctly rejected.
    let (rest, fragment) = split_at_first(&s[colon + 1..], '#');
    let (hier, query) = split_at_first(rest, '?');

    is_valid_ihier_part(hier)
        && query.is_none_or(|q| scan_pct(q, is_iquery_char))
        && fragment.is_none_or(|f| scan_pct(f, is_ifragment_char))
}

/// Splits `s` at the first `delim`, returning the part before it and (if present) the part
/// after. The delimiter itself is dropped.
fn split_at_first(s: &str, delim: char) -> (&str, Option<&str>) {
    match s.find(delim) {
        Some(i) => (&s[..i], Some(&s[i + delim.len_utf8()..])),
        None => (s, None),
    }
}

/// Scans `s` left to right, requiring every `%` to open a well-formed `pct-encoded` triplet
/// (`"%" HEXDIG HEXDIG`) and every other code point to satisfy `allowed`.
///
/// Escape validation lives here rather than in each character class because `pct-encoded`
/// is an alternative of every one of them (`iuserinfo`, `ireg-name`, `ipchar`).
fn scan_pct(s: &str, allowed: fn(char) -> bool) -> bool {
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            // A bare (`%`), truncated (`%A`) or non-hex (`%ZZ`) escape is not an IRI.
            let (Some(h1), Some(h2)) = (chars.next(), chars.next()) else {
                return false;
            };
            if !h1.is_ascii_hexdigit() || !h2.is_ascii_hexdigit() {
                return false;
            }
        } else if !allowed(c) {
            return false;
        }
    }
    true
}

/// `ihier-part = "//" iauthority ipath-abempty / ipath-absolute / ipath-rootless /
/// ipath-empty` (RFC 3987 §2.2).
fn is_valid_ihier_part(hier: &str) -> bool {
    match hier.strip_prefix("//") {
        Some(after) => {
            // The authority runs to the first `/`, which opens `ipath-abempty`.
            let end = after.find('/').unwrap_or(after.len());
            is_valid_iauthority(&after[..end]) && scan_pct(&after[end..], is_ipath_char)
        }
        // `ipath-absolute` / `ipath-rootless` / `ipath-empty` differ only in whether the
        // first segment is empty or absent, and a leading `//` is impossible here — so the
        // character check is the whole obligation.
        None => scan_pct(hier, is_ipath_char),
    }
}

/// `iauthority = [ iuserinfo "@" ] ihost [ ":" port ]` (RFC 3987 §2.2).
fn is_valid_iauthority(auth: &str) -> bool {
    // `iuserinfo` admits no `@`, so the last `@` is unambiguously the delimiter.
    let host_port = match auth.rfind('@') {
        Some(i) => {
            if !scan_pct(&auth[..i], is_iuserinfo_char) {
                return false;
            }
            &auth[i + 1..]
        }
        None => auth,
    };

    // `IP-literal = "[" ( IPv6address / IPvFuture ) "]"` — the only place `[`/`]` are legal.
    if let Some(rest) = host_port.strip_prefix('[') {
        let Some(close) = rest.find(']') else {
            return false;
        };
        return is_ip_literal(&rest[..close]) && is_valid_port_suffix(&rest[close + 1..]);
    }

    // `ireg-name` (which subsumes `IPv4address` — digits and `.` are `iunreserved`) admits
    // no `:`, so the first `:` is the port delimiter.
    let (name, port_suffix) = match host_port.find(':') {
        Some(i) => (&host_port[..i], &host_port[i..]),
        None => (host_port, ""),
    };
    scan_pct(name, is_ireg_name_char) && is_valid_port_suffix(port_suffix)
}

/// True iff `s` is the empty string or `":" port` with `port = *DIGIT` (RFC 3986 §3.2.3).
fn is_valid_port_suffix(s: &str) -> bool {
    match s.strip_prefix(':') {
        Some(port) => port.bytes().all(|b| b.is_ascii_digit()),
        None => s.is_empty(),
    }
}

/// The inside of an `IP-literal` — `IPv6address / IPvFuture` (RFC 3986 §3.2.2).
fn is_ip_literal(inner: &str) -> bool {
    // IPvFuture = "v" 1*HEXDIG "." 1*( unreserved / sub-delims / ":" )
    if let Some(v) = inner.strip_prefix(['v', 'V']) {
        let Some((hex, tail)) = v.split_once('.') else {
            return false;
        };
        return !hex.is_empty()
            && hex.bytes().all(|b| b.is_ascii_hexdigit())
            && !tail.is_empty()
            && tail.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '-' | '.' | '_' | '~' | ':')
                    || is_sub_delim(c)
            });
    }
    is_ipv6(inner)
}

/// `IPv6address` (RFC 3986 §3.2.2), normalised through the `::` zero-run elision: at most
/// one `::`, eight 16-bit groups in total, and a trailing dotted-quad counts as two.
fn is_ipv6(s: &str) -> bool {
    match s.split_once("::") {
        // No elision: exactly eight groups, all written out.
        None => count_h16_groups(s, true) == Some(8),
        Some((left, right)) => {
            // A second `::` is ambiguous and therefore not an address.
            if right.contains("::") {
                return false;
            }
            let l = if left.is_empty() {
                Some(0)
            } else {
                count_h16_groups(left, false)
            };
            let r = if right.is_empty() {
                Some(0)
            } else {
                count_h16_groups(right, true)
            };
            // `::` stands for at least one elided zero group, so the written groups must
            // leave room for it.
            matches!((l, r), (Some(l), Some(r)) if l + r <= 7)
        }
    }
}

/// Counts the 16-bit groups in a colon-separated run, or `None` if any is malformed. With
/// `ipv4_tail`, a final dotted-quad (`ls32`'s `IPv4address` alternative) counts as two.
fn count_h16_groups(s: &str, ipv4_tail: bool) -> Option<usize> {
    let mut groups = 0;
    let mut parts = s.split(':').peekable();
    while let Some(part) = parts.next() {
        if ipv4_tail && parts.peek().is_none() && part.contains('.') {
            if !is_ipv4(part) {
                return None;
            }
            groups += 2;
        } else {
            if !is_h16(part) {
                return None;
            }
            groups += 1;
        }
    }
    Some(groups)
}

/// `h16 = 1*4HEXDIG` — one 16-bit group of an IPv6 address (RFC 3986 §3.2.2).
fn is_h16(s: &str) -> bool {
    matches!(s.len(), 1..=4) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `IPv4address = dec-octet "." dec-octet "." dec-octet "." dec-octet` (RFC 3986 §3.2.2).
fn is_ipv4(s: &str) -> bool {
    let mut parts = s.split('.');
    let four = (0..4).all(|_| parts.next().is_some_and(is_dec_octet));
    four && parts.next().is_none()
}

/// `dec-octet` — 0-255 with no redundant leading zero (RFC 3986 §3.2.2).
fn is_dec_octet(s: &str) -> bool {
    matches!(s.len(), 1..=3)
        && s.bytes().all(|b| b.is_ascii_digit())
        && (s.len() == 1 || !s.starts_with('0'))
        && s.parse::<u32>().is_ok_and(|v| v <= 255)
}

/// `sub-delims = "!" / "$" / "&" / "'" / "(" / ")" / "*" / "+" / "," / ";" / "="`.
fn is_sub_delim(c: char) -> bool {
    matches!(
        c,
        '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '='
    )
}

/// `iunreserved = ALPHA / DIGIT / "-" / "." / "_" / "~" / ucschar`.
fn is_iunreserved(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~') || is_ucschar(c)
}

/// `ipchar = iunreserved / pct-encoded / sub-delims / ":" / "@"` (the `pct-encoded`
/// alternative is handled by [`scan_pct`]).
fn is_ipchar(c: char) -> bool {
    is_iunreserved(c) || is_sub_delim(c) || matches!(c, ':' | '@')
}

/// The bracket `gen-delims`. RFC 3987 admits them only as IP-literal delimiters in the
/// authority, but the JSON-LD suite pins them as ordinary characters elsewhere — see the
/// deviation documented on [`is_absolute_iri`] (W3C `compact/p004`).
fn is_tolerated_gen_delim(c: char) -> bool {
    matches!(c, '[' | ']')
}

/// A path code point: `ipchar` plus the `/` segment separator (plus the tolerated bracket
/// gen-delims).
fn is_ipath_char(c: char) -> bool {
    is_ipchar(c) || c == '/' || is_tolerated_gen_delim(c)
}

/// `iquery = *( ipchar / iprivate / "/" / "?" )` — the **only** component in which
/// `iprivate` is legal.
fn is_iquery_char(c: char) -> bool {
    is_ipchar(c) || is_iprivate(c) || matches!(c, '/' | '?') || is_tolerated_gen_delim(c)
}

/// `ifragment = *( ipchar / "/" / "?" )` — note the absence of `iprivate`.
fn is_ifragment_char(c: char) -> bool {
    is_ipchar(c) || matches!(c, '/' | '?') || is_tolerated_gen_delim(c)
}

/// `iuserinfo = *( iunreserved / pct-encoded / sub-delims / ":" )`.
fn is_iuserinfo_char(c: char) -> bool {
    is_iunreserved(c) || is_sub_delim(c) || c == ':'
}

/// `ireg-name = *( iunreserved / pct-encoded / sub-delims )`.
fn is_ireg_name_char(c: char) -> bool {
    is_iunreserved(c) || is_sub_delim(c)
}

/// True iff `c` is in RFC 3987 §2.2 `ucschar` — the non-ASCII code points admitted
/// throughout an IRI.
///
/// ```text
/// ucschar = %xA0-D7FF / %xF900-FDCF / %xFDF0-FFEF / %x10000-1FFFD / %x20000-2FFFD
///         / %x30000-3FFFD / %x40000-4FFFD / %x50000-5FFFD / %x60000-6FFFD
///         / %x70000-7FFFD / %x80000-8FFFD / %x90000-9FFFD / %xA0000-AFFFD
///         / %xB0000-BFFFD / %xC0000-CFFFD / %xD0000-DFFFD / %xE1000-EFFFD
/// ```
///
/// Surrogates need no handling: a Rust `char` cannot be one.
fn is_ucschar(c: char) -> bool {
    let cp = c as u32;
    match cp {
        // BMP portion; the gaps are the C1 controls (<0xA0), the surrogate and private-use
        // blocks, and the noncharacters 0xFDD0-0xFDEF / 0xFFF0-0xFFFF.
        0xA0..=0xD7FF | 0xF900..=0xFDCF | 0xFDF0..=0xFFEF => true,
        // Supplementary planes 1-13: each admits 0x0000-0xFFFD of its plane.
        0x10000..=0xDFFFD => cp & 0xFFFF <= 0xFFFD,
        // Plane 14: starts at 0xE1000, so the tag / variation-selector block
        // 0xE0000-0xE0FFF is excluded.
        0xE1000..=0xEFFFD => true,
        _ => false,
    }
}

/// True iff `c` is in RFC 3987 §2.2 `iprivate` — the private-use code points, legal in the
/// **query component only** (see [`is_iquery_char`]).
///
/// ```text
/// iprivate = %xE000-F8FF / %xF0000-FFFFD / %x100000-10FFFD
/// ```
fn is_iprivate(c: char) -> bool {
    let cp = c as u32;
    match cp {
        // BMP private-use area.
        0xE000..=0xF8FF => true,
        // Supplementary planes 15-16.
        0xF0000..=0x10FFFD => cp & 0xFFFF <= 0xFFFD,
        _ => false,
    }
}

/// True iff `s` is a syntactically valid URI scheme: `ALPHA *( ALPHA / DIGIT / "+" / "-" /
/// "." )` (RFC 3986 §3.1).
fn is_valid_scheme(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_alphabetic()
        && b.iter()
            .all(|&c| c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.')
}

impl ActiveContext {
    /// IRI Expansion (JSON-LD 1.1 API §5.2) — read-only.
    ///
    /// Expands `value` (a term, compact IRI, or relative reference) against this active
    /// context. With `vocab` true, `value` is resolved as a *vocabulary* reference (against
    /// term definitions and `@vocab`); with `document_relative` true, an otherwise
    /// unresolved reference is resolved against the base IRI. Returns `None` when `value`
    /// expands to null (a keyword-shaped non-keyword, or a term bound to null).
    ///
    /// This is the entry point once the active context is fully built. During Context
    /// Processing, expansion may need to define terms on demand; that variant lives in
    /// the crate-internal `expand_iri_in_context`.
    pub fn expand_iri(&self, value: &str, document_relative: bool, vocab: bool) -> Option<String> {
        self.expand_iri_readonly(value, document_relative, vocab)
    }

    /// The read-only core of IRI Expansion — no on-demand term creation (steps that would
    /// invoke Create Term Definition are skipped; the active context is assumed complete).
    pub(crate) fn expand_iri_readonly(
        &self,
        value: &str,
        document_relative: bool,
        vocab: bool,
    ) -> Option<String> {
        // §5.2 step 1: a keyword (or null) expands to itself.
        if is_keyword(value) {
            return Some(value.to_string());
        }
        // step 2: a keyword-shaped token that is not a recognised keyword expands to null.
        if has_keyword_form(value) {
            return None;
        }
        // step 4: a term whose IRI mapping is a keyword expands to that keyword.
        if let Some(def) = self.term_definitions.get(value) {
            if let Some(iri) = &def.iri {
                if is_keyword(iri) {
                    return Some(iri.clone());
                }
            }
        }
        // step 5: a vocabulary reference that is a defined term expands to its IRI mapping
        // (which may be null — e.g. an explicit `@id: null`, in which case the term drops).
        if vocab {
            if let Some(def) = self.term_definitions.get(value) {
                return def.iri.clone();
            }
        }
        // step 6: a compact IRI or absolute IRI.
        if let Some(colon) = value.find(':') {
            if colon > 0 {
                let prefix = &value[..colon];
                let suffix = &value[colon + 1..];
                // A blank node identifier or an IRI with an authority (`scheme://…`) is
                // returned unchanged.
                if prefix == "_" || suffix.starts_with("//") {
                    return Some(value.to_string());
                }
                // [SONNET-4.6] sq-oy1f.45 — a colon in a fragment (e.g. `#Test:2`) does
                // NOT mark a compact-IRI prefix; the prefix must be a valid RFC 3986 URI
                // scheme (ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )).  A non-scheme
                // prefix (starting with `#`, containing non-scheme chars, etc.) means the
                // value is a relative reference — fall through to base resolution (step 8).
                // W3C expand/0109: `#Test:2` must resolve against the base, not be returned
                // as-is as if it were an absolute IRI.
                if !is_valid_scheme(prefix) {
                    // Not a compact IRI (prefix is not a valid scheme); fall through to
                    // vocab / base resolution.
                } else {
                    // If the prefix is a term flagged `@prefix`, concatenate.
                    if let Some(pdef) = self.term_definitions.get(prefix) {
                        if pdef.prefix {
                            if let Some(piri) = &pdef.iri {
                                return Some(format!("{}{}", piri, suffix));
                            }
                        }
                    }
                    // Otherwise the value already contains a valid scheme: treat as absolute IRI.
                    return Some(value.to_string());
                }
            }
        }
        // step 7: a vocabulary reference with a `@vocab` mapping.
        if vocab {
            if let Some(vocab_iri) = &self.vocabulary_mapping {
                return Some(format!("{}{}", vocab_iri, value));
            }
        }
        // step 8: a document-relative reference resolved against the base IRI.
        if document_relative {
            if let Some(base) = &self.base_iri {
                return Some(resolve_iri(base, value));
            }
        }
        // step 9: return unchanged.
        Some(value.to_string())
    }
}

/// IRI Expansion with on-demand term creation (JSON-LD 1.1 API §5.2 steps 3 and 6.3):
/// while a local context is being processed, expanding a value may require the referenced
/// term — or the prefix of a compact IRI — to be defined first. Used by Create Term
/// Definition; delegates the actual lookup to [`ActiveContext::expand_iri_readonly`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn expand_iri_in_context(
    active: &mut ActiveContext,
    value: &str,
    document_relative: bool,
    vocab: bool,
    local: &Json,
    defined: &mut BTreeMap<String, bool>,
    env: &mut Env,
) -> Result<Option<String>, JsonLdError> {
    // Keyword / keyword-shaped tokens short-circuit before any term is created (§5.2 1–2).
    if is_keyword(value) {
        return Ok(Some(value.to_string()));
    }
    if has_keyword_form(value) {
        return Ok(None);
    }
    // step 3: if the local context defines `value` and it is not yet built, build it.
    ensure_defined(active, value, local, defined, env)?;
    // step 6.3: for a compact IRI, ensure its prefix term is built too.
    if let Some(colon) = value.find(':') {
        if colon > 0 {
            let prefix = &value[..colon];
            let suffix = &value[colon + 1..];
            if prefix != "_" && !suffix.starts_with("//") {
                ensure_defined(active, prefix, local, defined, env)?;
            }
        }
    }
    Ok(active.expand_iri_readonly(value, document_relative, vocab))
}

/// If `term` is defined by the local context and has not yet been built into `active`,
/// invoke Create Term Definition for it.
fn ensure_defined(
    active: &mut ActiveContext,
    term: &str,
    local: &Json,
    defined: &mut BTreeMap<String, bool>,
    env: &mut Env,
) -> Result<(), JsonLdError> {
    if local.get(term).is_some() && defined.get(term) != Some(&true) {
        create_term_definition(active, local, term, defined, false, false, env)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// RFC 3986 §5 reference resolution (used for `@base`, remote-context URLs, and
// document-relative IRI expansion). Kept dependency-free and self-contained.
// ---------------------------------------------------------------------------------------

/// The five components of a URI reference (RFC 3986 §3).
struct UriRef<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

/// Splits a URI reference into its components (RFC 3986 §3.2 / Appendix B).
fn split_uri(input: &str) -> UriRef<'_> {
    let (rest, fragment) = match input.find('#') {
        Some(i) => (&input[..i], Some(&input[i + 1..])),
        None => (input, None),
    };
    let (rest, query) = match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    let (scheme, rest) = match rest.find(':') {
        Some(i) if is_valid_scheme(&rest[..i]) => (Some(&rest[..i]), &rest[i + 1..]),
        _ => (None, rest),
    };
    let (authority, path) = if let Some(after) = rest.strip_prefix("//") {
        let end = after.find('/').unwrap_or(after.len());
        (Some(&after[..end]), &after[end..])
    } else {
        (None, rest)
    };
    UriRef {
        scheme,
        authority,
        path,
        query,
        fragment,
    }
}

/// Removes `.` and `..` complete path segments (RFC 3986 §5.2.4).
fn remove_dot_segments(path: &str) -> String {
    let mut input = path.to_string();
    let mut output = String::new();
    while !input.is_empty() {
        if let Some(r) = input.strip_prefix("../") {
            input = r.to_string();
        } else if let Some(r) = input.strip_prefix("./") {
            input = r.to_string();
        } else if let Some(r) = input.strip_prefix("/./") {
            input = format!("/{}", r);
        } else if input == "/." {
            input = "/".to_string();
        } else if let Some(r) = input.strip_prefix("/../") {
            input = format!("/{}", r);
            remove_last_segment(&mut output);
        } else if input == "/.." {
            input = "/".to_string();
            remove_last_segment(&mut output);
        } else if input == "." || input == ".." {
            input.clear();
        } else {
            let start = usize::from(input.starts_with('/'));
            let next = match input[start..].find('/') {
                Some(i) => start + i,
                None => input.len(),
            };
            output.push_str(&input[..next]);
            input = input[next..].to_string();
        }
    }
    output
}

/// Removes the last `/`-delimited segment (and its preceding `/`) from `output`.
fn remove_last_segment(output: &mut String) {
    match output.rfind('/') {
        Some(i) => output.truncate(i),
        None => output.clear(),
    }
}

/// Merges a relative reference path with a base's path (RFC 3986 §5.2.3).
fn merge(base: &UriRef<'_>, ref_path: &str) -> String {
    if base.authority.is_some() && base.path.is_empty() {
        format!("/{}", ref_path)
    } else {
        match base.path.rfind('/') {
            Some(i) => format!("{}{}", &base.path[..=i], ref_path),
            None => ref_path.to_string(),
        }
    }
}

/// Recomposes URI components into a string (RFC 3986 §5.3).
fn recompose(
    scheme: Option<&str>,
    authority: Option<&str>,
    path: &str,
    query: Option<&str>,
    fragment: Option<&str>,
) -> String {
    let mut s = String::new();
    if let Some(sc) = scheme {
        s.push_str(sc);
        s.push(':');
    }
    if let Some(a) = authority {
        s.push_str("//");
        s.push_str(a);
    }
    s.push_str(path);
    if let Some(q) = query {
        s.push('?');
        s.push_str(q);
    }
    if let Some(f) = fragment {
        s.push('#');
        s.push_str(f);
    }
    s
}

/// Resolves a URI `reference` against a `base` URI (RFC 3986 §5.2.2). Used for `@base`,
/// remote-context URL resolution, and document-relative IRI expansion.
pub(crate) fn resolve_iri(base: &str, reference: &str) -> String {
    let r = split_uri(reference);
    let b = split_uri(base);

    let (t_scheme, t_authority, t_path, t_query);
    if r.scheme.is_some() {
        t_scheme = r.scheme;
        t_authority = r.authority;
        t_path = remove_dot_segments(r.path);
        t_query = r.query;
    } else {
        if r.authority.is_some() {
            t_authority = r.authority;
            t_path = remove_dot_segments(r.path);
            t_query = r.query;
        } else if r.path.is_empty() {
            t_path = b.path.to_string();
            t_query = if r.query.is_some() { r.query } else { b.query };
            t_authority = b.authority;
        } else {
            if r.path.starts_with('/') {
                t_path = remove_dot_segments(r.path);
            } else {
                t_path = remove_dot_segments(&merge(&b, r.path));
            }
            t_query = r.query;
            t_authority = b.authority;
        }
        t_scheme = b.scheme;
    }
    recompose(t_scheme, t_authority, &t_path, t_query, r.fragment)
}

/// Generates a relative-reference from `base` to `iri` — the inverse of
/// [`resolve_iri`] (RFC 3986 §5.3 recomposition in reverse).
///
/// Used by IRI Compaction (JSON-LD 1.1 API §7.1 step 6) to produce a
/// base-relative compact form instead of the literal `strip_prefix` fallback.
/// The output satisfies the round-trip property:
/// `resolve_iri(base, relativize_iri(base, iri).unwrap()) == iri`.
///
/// Returns `None` when `iri` cannot be expressed relative to `base`:
/// - different URI scheme
/// - different authority (host/port)
///
/// [SONNET-4.6] (sq-90mu3) §7.1 step 6 defect fix: the prior
/// `iri.strip_prefix(base)` only handled the literal-prefix case
/// (`http://ex/a/b` → `c` when base is `http://ex/a/b` → ∅); this helper
/// covers same-directory, child-path, parent-traversal, and
/// query/fragment preservation.
pub(crate) fn relativize_iri(base: &str, iri: &str) -> Option<String> {
    let b = split_uri(base);
    let r = split_uri(iri);

    // Scheme and authority must agree — otherwise the IRI cannot be expressed
    // as a relative reference and the caller should fall through to the
    // absolute form.
    if b.scheme != r.scheme || b.authority != r.authority {
        return None;
    }

    // [FABLE-5] (sq-oy1f.27) Same-document references (RFC 3986 §4.4): when the
    // paths agree, prefer a fragment-only reference (queries also agreeing) or a
    // query reference over re-stating the last path segment — the forms the W3C
    // compact suite expects for `#fragment` / `?query` targets.
    if b.path == r.path && !r.path.is_empty() {
        if b.query == r.query {
            if let Some(f) = r.fragment {
                return Some(format!("#{}", f));
            }
        }
        if let Some(q) = r.query {
            let mut s = format!("?{}", q);
            if let Some(f) = r.fragment {
                s.push('#');
                s.push_str(f);
            }
            return Some(s);
        }
        // Target has no query: fall through to the path-based form (a bare
        // fragment reference would wrongly inherit the base's query).
    }

    // Base "directory": path up to and including the last '/'.
    // e.g. "/a/b" → "/a/"   "/a/" → "/a/"   "/" → "/"
    let base_dir_end = b.path.rfind('/').map(|i| i + 1).unwrap_or(0);
    let base_dir = &b.path[..base_dir_end];
    let base_segs: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();

    // IRI directory and filename:
    // "/a/c"  → dir="/a/"  file="c"
    // "/a/c/" → dir="/a/c/" file="" (directory IRI)
    let iri_dir_end = r.path.rfind('/').map(|i| i + 1).unwrap_or(0);
    let iri_dir = &r.path[..iri_dir_end];
    let iri_file = &r.path[iri_dir_end..];
    let iri_dir_segs: Vec<&str> = iri_dir.split('/').filter(|s| !s.is_empty()).collect();

    // Common directory-segment prefix length.
    let common = base_segs
        .iter()
        .zip(iri_dir_segs.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Number of `../` hops needed to reach the common ancestor.
    let up = base_segs.len() - common;
    // IRI-unique directory segments after the common prefix.
    let tail_dir = &iri_dir_segs[common..];

    // Build the relative path.
    let mut rel = String::new();
    for _ in 0..up {
        rel.push_str("../");
    }
    for seg in tail_dir {
        rel.push_str(seg);
        rel.push('/');
    }
    // If iri ends with '/' and the relative path so far is empty, we need
    // "./" so it resolves to the directory rather than the current document.
    if iri_file.is_empty() && rel.is_empty() {
        rel.push_str("./");
    } else {
        rel.push_str(iri_file);
    }

    // Preserve query and fragment from the target IRI.
    let mut result = rel;
    if let Some(q) = r.query {
        result.push('?');
        result.push_str(q);
    }
    if let Some(f) = r.fragment {
        result.push('#');
        result.push_str(f);
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_and_absolute_iri_predicates() {
        assert!(is_valid_scheme("http"));
        assert!(is_valid_scheme("urn"));
        assert!(!is_valid_scheme("1http")); // must start with ALPHA
        assert!(!is_valid_scheme(""));
        assert!(is_absolute_iri("http://example.org/a"));
        assert!(is_absolute_iri("mailto:a@example.org"));
        assert!(!is_absolute_iri("example.org/a"));
        assert!(!is_absolute_iri("_:b0"));
        assert!(is_blank_node("_:b0"));
        assert!(!is_blank_node("http://x"));
    }

    /// [SONNET-4.6] (PR #4610 review) `pct-encoded = "%" HEXDIG HEXDIG` — a malformed
    /// escape makes the string a non-IRI, so it must not pass as a datatype IRI. The
    /// original character-denylist check accepted all of these.
    #[test]
    fn malformed_percent_escapes_are_not_absolute_iris() {
        assert!(!is_absolute_iri("http://example.com/%ZZ")); // non-hex digits
        assert!(!is_absolute_iri("http://example.com/%")); // bare, at end
        assert!(!is_absolute_iri("http://example.com/%A")); // truncated, at end
        assert!(!is_absolute_iri("http://example.com/%2")); // truncated, at end
        assert!(!is_absolute_iri("http://example.com/%A/b")); // truncated, mid-string
        assert!(!is_absolute_iri("http://example.com/%%20")); // `%` opening `%2` + `0`
        assert!(!is_absolute_iri("http://example.com/%2G")); // second digit non-hex
        assert!(!is_absolute_iri("http://example.com/%G2")); // first digit non-hex
    }

    /// The complement of the above: well-formed escapes must keep passing — over-rejection
    /// here would silently cost conformance passes at every `is_absolute_iri` call site.
    #[test]
    fn well_formed_percent_escapes_are_absolute_iris() {
        assert!(is_absolute_iri("http://example.com/a%20b"));
        assert!(is_absolute_iri("http://example.com/%C3%A9"));
        assert!(is_absolute_iri("http://example.com/%ff%FF%aB")); // case-insensitive HEXDIG
        assert!(is_absolute_iri("http://example.com/p?q=%2F#%2F"));
    }

    /// RFC 3987 `ucschar`: a native Unicode IRI is well-formed and must not be rejected,
    /// while the code points outside the range (C1 controls, noncharacters, the plane-14
    /// tag block) are not IRI characters.
    #[test]
    fn unicode_iri_code_points_follow_ucschar() {
        // Positive: ucschar in the BMP, in the CJK compatibility range, and in plane 1.
        assert!(is_absolute_iri("http://example.com/na\u{ef}ve"));
        assert!(is_absolute_iri("http://\u{4f8b}\u{3048}.jp/\u{30d1}\u{30b9}"));
        assert!(is_absolute_iri("http://example.com/\u{f900}"));
        assert!(is_absolute_iri("http://example.com/\u{10000}"));

        // Negative: C1 control, BMP noncharacters, and the excluded plane-14 tag block.
        assert!(!is_absolute_iri("http://example.com/\u{80}"));
        assert!(!is_absolute_iri("http://example.com/\u{9f}"));
        assert!(!is_absolute_iri("http://example.com/\u{fdd0}"));
        assert!(!is_absolute_iri("http://example.com/\u{fffe}"));
        assert!(!is_absolute_iri("http://example.com/\u{1fffe}"));
        assert!(!is_absolute_iri("http://example.com/\u{e0001}"));
    }

    /// [SONNET-4.6] (PR #4610 review round 2) `iprivate` is an alternative of `iquery`
    /// ALONE — `ipchar` admits `ucschar` but not `iprivate`, so a private-use code point is
    /// legal in the query and nowhere else. The round-one code-point check admitted it
    /// everywhere; this is the placement rule the structural grammar adds.
    #[test]
    fn iprivate_is_legal_in_the_query_component_only() {
        assert!(is_absolute_iri("http://example.com/p?q=\u{e000}"));
        assert!(is_absolute_iri("http://example.com/p?\u{f0000}"));
        assert!(is_absolute_iri("http://example.com/p?\u{10fffd}"));

        assert!(!is_absolute_iri("http://example.com/\u{e000}")); // path
        assert!(!is_absolute_iri("http://example.com/p#\u{e000}")); // fragment
        assert!(!is_absolute_iri("http://\u{e000}.example.com/")); // host
        assert!(!is_absolute_iri("http://example.com/p?\u{fffff}")); // plane-15 noncharacter
    }

    /// [SONNET-4.6] (PR #4610 review round 2) The cited witnesses: strings built entirely
    /// from admitted code points whose *component structure* is malformed. `has_only_iri_chars`
    /// accepted every one of these, so `@type` never raised `invalid typed value` for them.
    #[test]
    fn structurally_malformed_authorities_are_not_absolute_iris() {
        // Unterminated IP-literal, and brackets outside an IP-literal.
        assert!(!is_absolute_iri("http://["));
        assert!(!is_absolute_iri("http://[::1"));
        assert!(!is_absolute_iri("http://]/"));
        assert!(!is_absolute_iri("http://a[b]c/"));
        // port = *DIGIT
        assert!(!is_absolute_iri("http://example.com:bad-port"));
        assert!(!is_absolute_iri("http://example.com:80a/"));
        assert!(!is_absolute_iri("http://example.com:8 0/"));
        // A second `#` lands inside the fragment, where it is not an ifragment character.
        assert!(!is_absolute_iri("http://example.com/p#a#b"));
        // Malformed IP-literal contents.
        assert!(!is_absolute_iri("http://[:::1]/"));
        assert!(!is_absolute_iri("http://[12345::1]/"));
        assert!(!is_absolute_iri("http://[1:2:3:4:5:6:7]/"));
        assert!(!is_absolute_iri("http://[1::2::3]/"));
        assert!(!is_absolute_iri("http://[::ffff:999.1.1.1]/"));
        assert!(!is_absolute_iri("http://[gggg::1]/"));
        assert!(!is_absolute_iri("http://[v.a]/")); // IPvFuture needs 1*HEXDIG
        assert!(!is_absolute_iri("http://[vF.]/")); // ...and a non-empty tail
    }

    /// The complement: authority forms that ARE well-formed must keep passing. Over-rejection
    /// here silently costs conformance passes at every `is_absolute_iri` call site, so each
    /// alternative of `ihost` and each optional component gets a witness.
    #[test]
    fn well_formed_authorities_are_absolute_iris() {
        // IP-literal: IPv6, elided runs, dotted-quad ls32, and IPvFuture.
        assert!(is_absolute_iri("http://[::1]/"));
        assert!(is_absolute_iri("http://[::]/"));
        assert!(is_absolute_iri("http://[2001:db8::1]:8080/p"));
        assert!(is_absolute_iri("http://[1:2:3:4:5:6:7:8]/"));
        assert!(is_absolute_iri("http://[::ffff:192.168.0.1]/"));
        assert!(is_absolute_iri("http://[v7.host:name]/"));
        // IPv4 and reg-name hosts, with and without userinfo / port.
        assert!(is_absolute_iri("http://192.168.0.1:80/p"));
        assert!(is_absolute_iri("http://user:pass@example.com:8080/p?q#f"));
        assert!(is_absolute_iri("http://example.com:/p")); // port = *DIGIT admits empty
        assert!(is_absolute_iri("http://example.com"));
        assert!(is_absolute_iri("http://"));
        assert!(is_absolute_iri("file:///etc/hosts"));
        // Authority-less schemes: ipath-rootless, ipath-absolute, ipath-empty.
        assert!(is_absolute_iri("urn:uuid:6e8bc430-9c3a-11d9-9669-0800200c9a66"));
        assert!(is_absolute_iri("did:example:123#key-1"));
        assert!(is_absolute_iri("mailto:a@example.org"));
        assert!(is_absolute_iri("tag:example.com,2026:a/b"));
        assert!(is_absolute_iri("about:blank"));
        assert!(is_absolute_iri("foo:/absolute/path"));
        assert!(is_absolute_iri("foo:"));
        // sub-delims are legal throughout.
        assert!(is_absolute_iri("http://example.com/a!$&'()*+,;=:@/b"));
    }

    /// [SONNET-4.6] (PR #4610 review round 2) The documented deviation from a literal
    /// `ipchar` reading, pinned to the W3C case that forces it: `compact/p004` is a
    /// PositiveEvaluationTest mapping `lbracket` to `http://example.org/[` and carrying
    /// `http://example.org/[foo` as a property IRI. Both must pass term-definition
    /// validation, so the bracket gen-delims stay admitted outside the authority. If this
    /// test is ever "tightened" to assert rejection, `compact/p004` fails and the
    /// conformance ratchet reds — check that before changing it.
    #[test]
    fn bracket_gen_delims_are_tolerated_outside_the_authority() {
        assert!(is_absolute_iri("http://example.org/["));
        assert!(is_absolute_iri("http://example.org/]"));
        assert!(is_absolute_iri("http://example.org/[foo"));
        assert!(is_absolute_iri("http://example.com/a[b]"));
        assert!(is_absolute_iri("http://example.com/p?q=[1]"));
        assert!(is_absolute_iri("http://example.com/p#[1]"));
        // ...but they remain structural inside the authority, so these stay rejected.
        assert!(!is_absolute_iri("http://[/"));
        assert!(!is_absolute_iri("http://a[b]c/"));
    }

    /// The other gen-delims the suite exercises alongside the brackets in `compact/p004`
    /// ("All simple terms ending with gen-delim are suitable for compaction"). `?` and `#`
    /// are the component delimiters themselves, so these pin the split, not a character set.
    #[test]
    fn terms_ending_with_a_gen_delim_are_absolute_iris() {
        for iri in [
            "http://example.com/",
            "http://example.org/:",
            "http://example.org/?",
            "http://example.org/#",
            "http://example.org/[",
            "http://example.org/]",
            "http://example.org/@",
        ] {
            assert!(is_absolute_iri(iri), "should accept {:?}", iri);
        }
    }

    /// The excluded-delimiter and control classes the original check already covered stay
    /// rejected (a regression tripwire for the rewrite into an allowlist).
    #[test]
    fn excluded_ascii_delimiters_are_not_absolute_iris() {
        assert!(!is_absolute_iri("http://example.com/baz z"));
        for bad in ['<', '>', '"', '{', '}', '|', '\\', '^', '`'] {
            let s = format!("http://example.com/{}", bad);
            assert!(!is_absolute_iri(&s), "should reject {:?}", bad);
        }
        assert!(!is_absolute_iri("http://example.com/\u{7f}")); // DEL
        assert!(!is_absolute_iri("http://example.com/a\tb"));
    }

    // RFC 3986 §5.4 normal-example reference-resolution vectors.
    #[test]
    fn rfc3986_normal_examples() {
        let base = "http://a/b/c/d;p?q";
        assert_eq!(resolve_iri(base, "g"), "http://a/b/c/g");
        assert_eq!(resolve_iri(base, "./g"), "http://a/b/c/g");
        assert_eq!(resolve_iri(base, "g/"), "http://a/b/c/g/");
        assert_eq!(resolve_iri(base, "/g"), "http://a/g");
        assert_eq!(resolve_iri(base, "?y"), "http://a/b/c/d;p?y");
        assert_eq!(resolve_iri(base, "g?y"), "http://a/b/c/g?y");
        assert_eq!(resolve_iri(base, "#s"), "http://a/b/c/d;p?q#s");
        assert_eq!(resolve_iri(base, "g#s"), "http://a/b/c/g#s");
        assert_eq!(resolve_iri(base, ""), "http://a/b/c/d;p?q");
        assert_eq!(resolve_iri(base, "."), "http://a/b/c/");
        assert_eq!(resolve_iri(base, ".."), "http://a/b/");
        assert_eq!(resolve_iri(base, "../.."), "http://a/");
        assert_eq!(resolve_iri(base, "../../g"), "http://a/g");
    }

    // RFC 3986 §5.4.2 abnormal dot-segment examples.
    #[test]
    fn rfc3986_abnormal_dot_segments() {
        let base = "http://a/b/c/d;p?q";
        assert_eq!(resolve_iri(base, "../../../g"), "http://a/g");
        assert_eq!(resolve_iri(base, "/./g"), "http://a/g");
        assert_eq!(resolve_iri(base, "g."), "http://a/b/c/g.");
        assert_eq!(resolve_iri(base, ".g"), "http://a/b/c/.g");
        assert_eq!(resolve_iri(base, "g/./h"), "http://a/b/c/g/h");
        assert_eq!(resolve_iri(base, "g/../h"), "http://a/b/c/h");
    }

    #[test]
    fn absolute_reference_keeps_its_own_scheme() {
        assert_eq!(
            resolve_iri("http://a/b", "https://c/d?x#y"),
            "https://c/d?x#y"
        );
    }

    // [SONNET-4.6] Issue #3714 — mutation tripwire for the dependency-free RFC
    // 3986 implementation. oxiri remains test-only so the crate's zero-mandatory-
    // dependency design is preserved.
    #[test]
    fn resolve_iri_matches_oxiri_on_shared_corpus() {
        let cases = [
            // RFC 3986 §5.4 normal examples.
            ("http://a/b/c/d;p?q", "g:h"),
            ("http://a/b/c/d;p?q", "g"),
            ("http://a/b/c/d;p?q", "./g"),
            ("http://a/b/c/d;p?q", "g/"),
            ("http://a/b/c/d;p?q", "/g"),
            ("http://a/b/c/d;p?q", "//g"),
            ("http://a/b/c/d;p?q", "?y"),
            ("http://a/b/c/d;p?q", "g?y"),
            ("http://a/b/c/d;p?q", "#s"),
            ("http://a/b/c/d;p?q", "g#s"),
            ("http://a/b/c/d;p?q", "g?y#s"),
            ("http://a/b/c/d;p?q", ";x"),
            ("http://a/b/c/d;p?q", "g;x"),
            ("http://a/b/c/d;p?q", "g;x?y#s"),
            ("http://a/b/c/d;p?q", ""),
            ("http://a/b/c/d;p?q", "."),
            ("http://a/b/c/d;p?q", "./"),
            ("http://a/b/c/d;p?q", ".."),
            ("http://a/b/c/d;p?q", "../"),
            ("http://a/b/c/d;p?q", "../g"),
            ("http://a/b/c/d;p?q", "../.."),
            ("http://a/b/c/d;p?q", "../../"),
            ("http://a/b/c/d;p?q", "../../g"),
            // RFC 3986 §5.4.2 abnormal examples.
            ("http://a/b/c/d;p?q", "../../../g"),
            ("http://a/b/c/d;p?q", "../../../../g"),
            ("http://a/b/c/d;p?q", "/./g"),
            ("http://a/b/c/d;p?q", "/../g"),
            ("http://a/b/c/d;p?q", "g."),
            ("http://a/b/c/d;p?q", ".g"),
            ("http://a/b/c/d;p?q", "g.."),
            ("http://a/b/c/d;p?q", "..g"),
            ("http://a/b/c/d;p?q", "./../g"),
            ("http://a/b/c/d;p?q", "./g/."),
            ("http://a/b/c/d;p?q", "g/./h"),
            ("http://a/b/c/d;p?q", "g/../h"),
            ("http://a/b/c/d;p?q", "g;x=1/./y"),
            ("http://a/b/c/d;p?q", "g;x=1/../y"),
            ("http://a/b/c/d;p?q", "g?y/./x"),
            ("http://a/b/c/d;p?q", "g?y/../x"),
            ("http://a/b/c/d;p?q", "g#s/./x"),
            ("http://a/b/c/d;p?q", "g#s/../x"),
            ("http://a/b/c/d;p?q", "http:g"),
            // Existing resolver cases and JSON-LD @base/document-relative shapes.
            ("http://a/b", "https://c/d?x#y"),
            ("https://example.com/doc", "#node"),
            ("https://example.com/a/context.jsonld", "../vocab/term"),
            ("https://example.com/a/", "child"),
            ("https://example.com", "relative"),
            ("https://example.com/a?old=1", "?new=2"),
            ("https://example.com/a?old=1", "#fragment"),
            ("urn:example:base", "next"),
        ];

        for (base, reference) in cases {
            let oxiri_base = oxiri::Iri::parse(base.to_string()).unwrap();
            let Ok(expected) = oxiri_base.resolve(reference) else {
                continue;
            };
            assert_eq!(
                resolve_iri(base, reference),
                expected.into_inner(),
                "base={base:?}, reference={reference:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // relativize_iri — round-trip correctness matrix
    // [SONNET-4.6] (sq-90mu3) Fix for §7.1 step 6 base-relative compaction.
    // Each case asserts: resolve_iri(base, relativize_iri(base, iri)) == iri
    // -----------------------------------------------------------------------

    fn round_trip(base: &str, iri: &str) -> String {
        let rel = relativize_iri(base, iri)
            .unwrap_or_else(|| panic!("expected Some for base={base:?} iri={iri:?}"));
        let resolved = resolve_iri(base, &rel);
        assert_eq!(
            resolved, iri,
            "round-trip failed: base={base:?} iri={iri:?} rel={rel:?}"
        );
        rel
    }

    /// Same directory: shared parent, different filename.
    #[test]
    fn relativize_same_directory() {
        let rel = round_trip("http://ex/a/b", "http://ex/a/c");
        assert_eq!(rel, "c");
    }

    /// Child path: iri is deeper than base.
    #[test]
    fn relativize_child_path() {
        let rel = round_trip("http://ex/a/b", "http://ex/a/b/c/d");
        assert_eq!(rel, "b/c/d");
    }

    /// Parent traversal: iri is in a sibling directory.
    #[test]
    fn relativize_parent_traversal() {
        let rel = round_trip("http://ex/a/b/c", "http://ex/a/d");
        assert_eq!(rel, "../d");
    }

    /// Query preservation: relative reference keeps the query string.
    #[test]
    fn relativize_query_preservation() {
        let rel = round_trip("http://ex/a/b", "http://ex/a/c?q=1");
        assert_eq!(rel, "c?q=1");
    }

    /// Fragment preservation: relative reference keeps the fragment.
    #[test]
    fn relativize_fragment_preservation() {
        let rel = round_trip("http://ex/a/b", "http://ex/a/c#sec");
        assert_eq!(rel, "c#sec");
    }

    /// Query and fragment together.
    #[test]
    fn relativize_query_and_fragment() {
        let rel = round_trip("http://ex/a/b", "http://ex/a/c?q=1#sec");
        assert_eq!(rel, "c?q=1#sec");
    }

    /// Same-file reference (iri == base): resolves back to the same resource.
    #[test]
    fn relativize_same_file() {
        let rel = round_trip("http://ex/a/b", "http://ex/a/b");
        // The relative ref must resolve back; "b" is the canonical form here.
        assert_eq!(rel, "b");
    }

    /// Scheme mismatch: returns None (cannot relativize across schemes).
    #[test]
    fn relativize_scheme_mismatch_returns_none() {
        assert!(relativize_iri("http://ex/a", "https://ex/a").is_none());
    }

    /// Authority mismatch: returns None (cannot relativize across hosts).
    #[test]
    fn relativize_authority_mismatch_returns_none() {
        assert!(relativize_iri("http://ex/a", "http://other.ex/a").is_none());
    }

    /// Root-relative base: iri one level deep from the server root.
    #[test]
    fn relativize_root_base() {
        let rel = round_trip("http://ex/", "http://ex/foo");
        assert_eq!(rel, "foo");
    }

    /// Deep parent traversal: two levels up.
    #[test]
    fn relativize_two_levels_up() {
        let rel = round_trip("http://ex/a/b/c/d", "http://ex/e");
        assert_eq!(rel, "../../../e");
    }
}
