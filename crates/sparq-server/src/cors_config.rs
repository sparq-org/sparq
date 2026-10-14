//! First-party CORS origin allowlist — server configuration. (sq-o7o0)
//!
//! ASVS V14.5.3 wants a deliberate, documented CORS decision. `sparq-server` is a
//! SPARQL *data API* (it serves SPARQL-results JSON/XML/CSV/TSV and RDF, never an
//! HTML document it asks a browser to render), so its **safe default is to emit NO
//! CORS headers at all** — a response with no `Access-Control-Allow-Origin` is, by
//! the Same-Origin Policy, unreadable to a cross-origin browser script. That default
//! is exactly the historical behaviour and is the correct posture for a public,
//! unauthenticated endpoint: adding a permissive CORS header would *widen* the
//! browser-reachable surface, the opposite of hardening.
//!
//! ## Why an opt-in allowlist at all
//!
//! Some operators run a **first-party browser app** (a dashboard, a query UI) on a
//! different origin from the endpoint and legitimately need that one origin to read
//! responses from a browser `fetch`. For them — and ONLY them — this provides an
//! explicit allowlist: list the exact origins your own front-end is served from, and
//! the server reflects each listed `Origin` back in `Access-Control-Allow-Origin`
//! (plus `Vary: Origin`) and answers the `OPTIONS` preflight. Empty (the default)
//! means **no CORS headers** — unchanged from before this option existed.
//!
//! ## Deliberately conservative
//!
//!   * **No `*` wildcard origin.** We never emit `Access-Control-Allow-Origin: *`.
//!     Only an exact origin from the allowlist is reflected; an un-listed origin gets
//!     no CORS header at all (the browser then blocks the cross-origin read). This
//!     keeps the allowlist a true first-party list, not an open door.
//!   * **No `Access-Control-Allow-Credentials`.** The combination of credentialed
//!     CORS with a reflected origin is a classic footgun; this layer is for a
//!     first-party app reading PUBLIC query results, so credentials are never opted
//!     in. (The endpoint's own Bearer-token gate is orthogonal and unaffected — a
//!     browser still sends/omits it per its own `fetch` options; we simply never set
//!     `Allow-Credentials`, so the browser will not expose a credentialed response.)
//!   * **Allowlisting an origin does NOT relax any other guard.** Auth, the bind
//!     posture, body limits, the SERVICE egress allowlist and the row caps all still
//!     apply to a CORS request exactly as to any other.
//!
//! ## Origin syntax
//!
//! Each entry is one **origin** in the RFC 6454 serialization the browser sends in
//! the `Origin` header: `scheme "://" host [ ":" port ]`, e.g.
//! `https://app.example.org`, `http://localhost:8080`. The match is **exact** (the
//! reflected value is byte-for-byte the request `Origin`), case-folded only on the
//! scheme + host (per RFC 6454 origins are compared case-insensitively on those, but
//! a browser already lower-cases them, so in practice the entries are compared
//! verbatim against the header). A default port is NOT inferred: list
//! `https://app.example.org` and `https://app.example.org:443` separately if you
//! somehow receive both (browsers omit the default port, so the bare form is the one
//! you want).
//!
//! Entries come from (union of all three, deduplicated):
//!   * the repeatable `--cors-allow-origin <origin>` CLI flag;
//!   * `--cors-allow-origin-file <path>` (one entry per line; `#` comments + blanks
//!     ignored);
//!   * the `SPARQ_CORS_ALLOW_ORIGIN` env var (comma- and/or whitespace-separated).
//!
//! Precedence mirrors the SERVICE allowlist: env establishes a baseline, the file +
//! CLI flags ADD to it (the union — an allowlist is additive, so the CLI only ever
//! widens what env granted, never silently removes an origin).

use std::collections::BTreeSet;

/// The configured first-party CORS origin allowlist. Empty = emit NO CORS headers
/// (the default — historical behaviour).
///
/// Stored as a normalised, deduplicated, sorted set so two configs built from the
/// same inputs in any order compare equal (handy for tests) and the startup log is
/// deterministic. Each stored entry is already validated as a well-formed origin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CorsAllowlist {
    /// Allowed origins in their `scheme://host[:port]` serialization (scheme + host
    /// lower-cased; no trailing slash).
    origins: BTreeSet<String>,
}

impl CorsAllowlist {
    /// True when no origin is allowlisted — i.e. NO CORS headers are emitted (the
    /// default; a cross-origin browser read is blocked by the Same-Origin Policy).
    pub fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    /// Number of distinct allowlisted origins.
    pub fn len(&self) -> usize {
        self.origins.len()
    }

    /// Whether the given request `Origin` header value is allowlisted (and therefore
    /// safe to reflect into `Access-Control-Allow-Origin`). The comparison is exact
    /// against a normalised origin: the input is normalised (scheme + host
    /// lower-cased, trailing slash stripped) and looked up. Returns `false` for an
    /// un-listed origin, for a malformed `Origin`, and always for the literal `null`
    /// origin (sandboxed/`file:`/redirected contexts — we never reflect `null`).
    pub fn allows(&self, origin: &str) -> bool {
        match Self::normalize_origin(origin) {
            Some(o) => self.origins.contains(&o),
            None => false,
        }
    }

    /// Adds one raw entry (CLI flag value, file line, or env token). Returns an error
    /// string for a malformed origin so the caller can fail fast at startup rather
    /// than silently dropping an origin the operator meant to allow (a fail-closed
    /// CORS list that *looks* configured but never matches would be a confusing
    /// footgun).
    ///
    /// Accepted form: a single RFC 6454 origin `scheme://host[:port]`. A `*` (the
    /// wildcard origin) is REJECTED on purpose — this is a first-party allowlist, not
    /// an open-CORS switch; reflecting only exact listed origins is the whole point.
    pub fn add(&mut self, raw: &str) -> Result<(), String> {
        let e = raw.trim();
        if e.is_empty() {
            return Ok(()); // blank line / empty token — nothing to add
        }
        if e == "*" || e.contains('*') {
            return Err(format!(
                "invalid CORS origin {raw:?}: a '*' wildcard origin is not supported \
                 (list each exact first-party origin instead — we never emit \
                 'Access-Control-Allow-Origin: *')"
            ));
        }
        match Self::normalize_origin(e) {
            Some(o) => {
                self.origins.insert(o);
                Ok(())
            }
            None => Err(format!(
                "invalid CORS origin {raw:?}: expected an origin 'scheme://host[:port]' \
                 (e.g. https://app.example.org)"
            )),
        }
    }

    /// Adds every comma- and/or whitespace-separated token in `s` (the
    /// `SPARQ_CORS_ALLOW_ORIGIN` env-var grammar).
    pub fn add_many(&mut self, s: &str) -> Result<(), String> {
        for tok in s.split([',', ' ', '\t', '\n', '\r']) {
            self.add(tok)?;
        }
        Ok(())
    }

    /// Validate + normalise an origin string to the canonical `scheme://host[:port]`
    /// form we store and compare against. Returns `None` for anything that is not a
    /// well-formed origin (so a malformed `Origin` header can never match, and a
    /// malformed config entry is rejected at startup).
    ///
    /// Normalisation: trim; strip a single trailing `/`; lower-case the scheme and
    /// host (RFC 6454 §4 — origins are scheme/host case-insensitive); keep the port
    /// verbatim. We require an explicit scheme and a non-empty host, reject any path
    /// / query / fragment / userinfo (an origin has none), and reject the `null`
    /// keyword (we never reflect `null`).
    fn normalize_origin(raw: &str) -> Option<String> {
        let s = raw.trim().strip_suffix('/').unwrap_or(raw.trim());
        if s.is_empty() || s.eq_ignore_ascii_case("null") {
            return None;
        }
        let (scheme, rest) = s.split_once("://")?;
        if scheme.is_empty()
            || !scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
            || !scheme.as_bytes()[0].is_ascii_alphabetic()
        {
            return None;
        }
        // `rest` must be exactly the authority: host[:port]. No path/query/fragment,
        // no userinfo (`@`), no embedded slash, no whitespace.
        if rest.is_empty()
            || rest.contains('/')
            || rest.contains('?')
            || rest.contains('#')
            || rest.contains('@')
            || rest.contains(char::is_whitespace)
        {
            return None;
        }
        let scheme_lc = scheme.to_ascii_lowercase();
        // Split off an optional :port. Bracketed IPv6 (`[::1]` / `[::1]:8080`) is
        // handled so the colon inside the address is not mistaken for the port sep.
        let (host, port) = if let Some(after_bracket) = rest.strip_prefix('[') {
            let (inner, tail) = after_bracket.split_once(']')?;
            if inner.is_empty() {
                return None;
            }
            let port = match tail {
                "" => None,
                t => Some(t.strip_prefix(':')?),
            };
            (format!("[{}]", inner.to_ascii_lowercase()), port)
        } else if let Some((h, p)) = rest.rsplit_once(':') {
            // Only treat the trailing colon group as a port when it is all digits;
            // otherwise it is part of an (unbracketed, so invalid) IPv6 literal, which
            // the host check below rejects via its stray-colon test.
            if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                (h.to_ascii_lowercase(), Some(p))
            } else {
                (rest.to_ascii_lowercase(), None)
            }
        } else {
            (rest.to_ascii_lowercase(), None)
        };
        // Host must be non-empty and must not itself contain a stray colon (a bare,
        // unbracketed IPv6 literal is not a valid Origin authority).
        if host.is_empty() || (!host.starts_with('[') && host.contains(':')) {
            return None;
        }
        match port {
            Some(p) => Some(format!("{scheme_lc}://{host}:{p}")),
            None => Some(format!("{scheme_lc}://{host}")),
        }
    }

    /// A stable, human-readable rendering for the startup log.
    pub fn display(&self) -> String {
        if self.is_empty() {
            return "off (no CORS headers emitted)".to_string();
        }
        self.origins.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_no_cors() {
        let a = CorsAllowlist::default();
        assert!(a.is_empty());
        assert_eq!(a.len(), 0);
        assert!(!a.allows("https://app.example.org"));
        assert_eq!(a.display(), "off (no CORS headers emitted)");
    }

    #[test]
    fn exact_origin_allowed_others_not() {
        let mut a = CorsAllowlist::default();
        a.add("https://app.example.org").unwrap();
        assert!(a.allows("https://app.example.org"));
        // Different scheme / host / port are all distinct origins.
        assert!(!a.allows("http://app.example.org"));
        assert!(!a.allows("https://evil.example.org"));
        assert!(!a.allows("https://app.example.org:8443"));
    }

    #[test]
    fn scheme_and_host_lowercased_trailing_slash_stripped() {
        let mut a = CorsAllowlist::default();
        a.add("HTTPS://App.Example.ORG/").unwrap();
        assert_eq!(a.len(), 1);
        // A browser always sends the lower-cased, slash-free origin.
        assert!(a.allows("https://app.example.org"));
        assert!(a.allows("HTTPS://APP.EXAMPLE.ORG")); // normalised on lookup too
    }

    #[test]
    fn port_preserved_and_distinct() {
        let mut a = CorsAllowlist::default();
        a.add("http://localhost:8080").unwrap();
        assert!(a.allows("http://localhost:8080"));
        assert!(!a.allows("http://localhost")); // no default-port inference
        assert!(!a.allows("http://localhost:8081"));
    }

    #[test]
    fn ipv6_origin_supported() {
        let mut a = CorsAllowlist::default();
        a.add("http://[::1]:3000").unwrap();
        assert!(a.allows("http://[::1]:3000"));
        assert!(!a.allows("http://[::1]"));
        let mut b = CorsAllowlist::default();
        b.add("http://[::1]").unwrap();
        assert!(b.allows("http://[::1]"));
    }

    #[test]
    fn wildcard_rejected() {
        let mut a = CorsAllowlist::default();
        assert!(a.add("*").is_err());
        assert!(a.add("https://*.example.org").is_err());
        assert!(a.is_empty());
    }

    #[test]
    fn null_origin_never_allowed() {
        let mut a = CorsAllowlist::default();
        // "null" is not a valid allowlist entry...
        assert!(a.add("null").is_err());
        // ...and even a lookup of `null` against a configured list is refused.
        a.add("https://app.example.org").unwrap();
        assert!(!a.allows("null"));
        assert!(!a.allows("NULL"));
    }

    #[test]
    fn malformed_origins_rejected() {
        let mut a = CorsAllowlist::default();
        assert!(a.add("app.example.org").is_err()); // no scheme
        assert!(a.add("https://").is_err()); // no host
        assert!(a.add("https://app.example.org/path").is_err()); // has a path
        assert!(a.add("https://app.example.org?x=1").is_err()); // has a query
        assert!(a.add("https://user@app.example.org").is_err()); // userinfo
        assert!(a.add("https://app.example.org:notaport").is_err()); // bad port
        assert!(a.add("ht tps://app.example.org").is_err()); // space in scheme
        assert!(a.is_empty());
    }

    #[test]
    fn blank_entries_ignored() {
        let mut a = CorsAllowlist::default();
        a.add("   ").unwrap();
        a.add("").unwrap();
        assert!(a.is_empty());
    }

    #[test]
    fn add_many_splits_and_dedups() {
        let mut a = CorsAllowlist::default();
        a.add_many(
            "https://a.example.org, https://b.example.org  https://a.example.org\nhttps://c.example.org",
        )
        .unwrap();
        assert_eq!(a.len(), 3); // a, b, c — the duplicate a is collapsed
        assert!(a.allows("https://a.example.org"));
        assert!(a.allows("https://b.example.org"));
        assert!(a.allows("https://c.example.org"));
    }

    #[test]
    fn display_is_sorted_and_stable() {
        let mut a = CorsAllowlist::default();
        a.add("https://b.example.org").unwrap();
        a.add("https://a.example.org").unwrap();
        assert_eq!(a.display(), "https://a.example.org, https://b.example.org");
    }
}
