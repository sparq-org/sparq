//! [OPUS-4.8] (sq-7d3dj.18) 🤖 SPARQ agent — a prefix-memoized IRI-validation fast path that
//! sits *in front of* the full [`oxiri`] RFC-3987 automaton in the ingest lane.
//!
//! # Why
//!
//! Re-validating every IRI *occurrence* against the full RFC-3987 grammar is one of the
//! largest aggregate ingest costs, yet real RDF dumps reuse a handful of uniform
//! `scheme://authority/` prefixes and are overwhelmingly ASCII in the path/query/fragment.
//! This module skips the full parse for that common case with two stacked ACCEPT-shortcuts,
//! falling back to `oxiri` on *any* uncertainty:
//!
//! 1. **Prefix memo** — the last-N `scheme://authority/` prefixes that `oxiri` has already
//!    accepted are remembered. An incoming IRI that byte-matches a memoized prefix has its
//!    authority *already proven valid*, so only its suffix needs checking.
//! 2. **ASCII suffix pre-scan** — a single vectorizable pass over the suffix. If every byte
//!    is in the position-independent "always-valid after the authority `/`" ASCII class
//!    ([`is_safe_suffix_byte`]), the whole IRI is provably a valid absolute IRI. A non-ASCII
//!    byte, a `%`-escape, a `#`, or any other anomaly drops straight to `oxiri`.
//!
//! # The load-bearing invariant (differential-fuzz gated)
//!
//! The fast path is an **ACCEPT-shortcut only, never a reject-shortcut**:
//!
//! > `fast_accept(s)` ⇒ `oxiri::Iri::parse(s).is_ok()`
//!
//! and therefore [`IriPrefixMemo::validate`] accepts *exactly* the set `oxiri` accepts (an
//! absolute IRI, RFC-3987). This equivalence is proven mechanically by the mandatory
//! differential-fuzz gate in this module's tests (`fast_accept ⇒ oxiri`, and
//! `validate == oxiri`) over a committed adversarial corpus plus a deterministic
//! IRI-structured generator — a wrong accept would corrupt ingest, so the gate is a hard
//! per-PR replay. See bead sq-7d3dj.18 and `research/engine-performance-review.md` §1.2.
//!
//! Because the fallback branch defers to `oxiri` verbatim, correctness never depends on the
//! memo's contents: the memo is a pure cache of *already-validated* prefixes, so its state
//! (or staleness across unrelated parses) can only change *speed*, never the answer.

/// [OPUS-4.8] (sq-7d3dj.18) True iff `b` is an ASCII byte that is valid in **every** position
/// of an IRI *after* the authority-terminating `/` — i.e. the intersection of the classes
/// legal in `ipath-abempty`, `iquery`, and `ifragment` that needs no context to accept.
///
/// This is `iunreserved` (ASCII subset) ∪ `sub-delims` ∪ `{ ':' , '@' }` (the extra `ipchar`
/// members) ∪ `{ '/' , '?' }` (the path/query separators). It deliberately EXCLUDES:
///
/// * `'#'` — a fragment cannot itself contain `'#'`, so a second `'#'` would be invalid;
/// * `'%'` — percent-encoding needs two following hex digits, which this scan does not check;
/// * `'['` / `']'` — IP-literal brackets belong to the authority, not the suffix;
/// * every non-ASCII byte — `ucschar`/`iprivate` are position-dependent (e.g. a private-use
///   codepoint is legal only in `iquery`), so they always fall back to `oxiri`.
///
/// Any excluded byte drops the IRI to the full `oxiri` automaton, preserving equivalence.
#[inline]
#[must_use]
pub const fn is_safe_suffix_byte(b: u8) -> bool {
    matches!(b,
        // iunreserved (ASCII subset): ALPHA / DIGIT / "-" / "." / "_" / "~"
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
        // sub-delims: "!" / "$" / "&" / "'" / "(" / ")" / "*" / "+" / "," / ";" / "="
        | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
        // remaining ipchar members
        | b':' | b'@'
        // path/query separators (never re-open the authority, which the memoized '/' closed)
        | b'/' | b'?'
    )
}

/// Position-independent lookup table for [`is_safe_suffix_byte`], so the suffix pre-scan is a
/// branch-light one-pass load per byte (autovectorizable). [OPUS-4.8]
const SUFFIX_SAFE: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0usize;
    while i < 256 {
        t[i] = is_safe_suffix_byte(i as u8);
        i += 1;
    }
    t
};

/// True iff **every** byte of `suffix` is [`is_safe_suffix_byte`] — one vectorizable pass.
///
/// An empty suffix is safe (a memoized `scheme://authority/` is itself a valid IRI). [OPUS-4.8]
#[inline]
#[must_use]
fn suffix_all_safe(suffix: &[u8]) -> bool {
    suffix.iter().all(|&b| SUFFIX_SAFE[b as usize])
}

/// Length (in bytes) of the leading `scheme://authority/` of `s` — the index one *past* the
/// first `/` that terminates the authority — or `None` when `s` has no such `/`-terminated
/// hierarchical prefix worth memoizing.
///
/// Only ever called on a string `oxiri` has ALREADY accepted, so the scheme (`ALPHA
/// *(ALPHA / DIGIT / "+" / "-" / ".")`) contains no `:` and the FIRST `:` is the scheme
/// separator. Returns `None` for the non-`//` forms (`urn:…`, `mailto:…`) and for an authority
/// not terminated by a `/` (`http://ex`, `http://ex?q`) — there is no path-prefix to reuse, so
/// those simply never seed the memo. [OPUS-4.8]
#[inline]
#[must_use]
fn scheme_authority_prefix_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let colon = b.iter().position(|&c| c == b':')?;
    // Require the "scheme://" hierarchical form.
    if b.get(colon + 1) != Some(&b'/') || b.get(colon + 2) != Some(&b'/') {
        return None;
    }
    // The authority runs from just past "://" to the first '/', '?', or '#' (or end of input).
    // Only a '/'-terminated authority yields a reusable `scheme://authority/` prefix.
    let mut i = colon + 3;
    while i < b.len() {
        match b[i] {
            b'/' => return Some(i + 1),
            b'?' | b'#' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Default number of `scheme://authority/` prefixes an [`IriPrefixMemo`] remembers. Real dumps
/// reuse only a handful of prefixes, so a small ring captures the win while a miss costs one
/// short `starts_with` per entry. [OPUS-4.8]
pub const DEFAULT_MEMO_CAPACITY: usize = 8;

/// [OPUS-4.8] (sq-7d3dj.18) A bounded ring of recently `oxiri`-validated `scheme://authority/`
/// prefixes, used to shortcut re-validation of IRIs that share a validated prefix.
///
/// Construct with [`IriPrefixMemo::new`] (or [`IriPrefixMemo::with_capacity`]) and call
/// [`IriPrefixMemo::validate`] once per IRI occurrence. The memo is a pure *speed* cache: every
/// stored prefix was accepted by `oxiri`, and any accept/reject the memo cannot *prove* is
/// deferred to `oxiri` verbatim, so the accepted language is byte-for-byte `oxiri`'s.
#[derive(Debug, Clone)]
pub struct IriPrefixMemo {
    /// Validated `scheme://authority/` prefixes, oldest first (FIFO eviction at `cap`).
    prefixes: Vec<Box<str>>,
    cap: usize,
}

impl Default for IriPrefixMemo {
    fn default() -> Self {
        Self::new()
    }
}

impl IriPrefixMemo {
    /// A memo with the [`DEFAULT_MEMO_CAPACITY`] ring size.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MEMO_CAPACITY)
    }

    /// A memo that remembers the last `cap` validated prefixes (a `cap` of 0 disables the
    /// prefix memo — [`validate`](Self::validate) then always falls back to `oxiri`, which
    /// is still correct, just unaccelerated). [OPUS-4.8]
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self { prefixes: Vec::with_capacity(cap), cap }
    }

    /// Number of memoized prefixes currently held (for tests/introspection). [OPUS-4.8]
    #[must_use]
    pub fn len(&self) -> usize {
        self.prefixes.len()
    }

    /// Whether the memo currently holds no prefixes. [OPUS-4.8]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty()
    }

    /// The ACCEPT-shortcut: `true` iff a memoized prefix + ASCII suffix pre-scan *proves* `s`
    /// is a valid absolute IRI. A `false` result means "unknown — ask `oxiri`", NOT "invalid".
    ///
    /// Soundness (`fast_accept(s)` ⇒ `oxiri` accepts `s`) is the differential-fuzz gate's
    /// load-bearing assertion. [OPUS-4.8]
    #[must_use]
    fn fast_accept(&self, s: &str) -> bool {
        let sb = s.as_bytes();
        for p in &self.prefixes {
            let pb = p.as_bytes();
            if sb.len() >= pb.len() && sb.starts_with(pb) && suffix_all_safe(&sb[pb.len()..]) {
                return true;
            }
        }
        false
    }

    /// Seed the ring from a string `oxiri` has just accepted (extract & remember its
    /// `scheme://authority/` prefix; FIFO-evict the oldest at capacity; no duplicates). [OPUS-4.8]
    fn record(&mut self, s: &str) {
        if self.cap == 0 {
            return;
        }
        let Some(plen) = scheme_authority_prefix_len(s) else { return };
        let prefix = &s[..plen];
        if self.prefixes.iter().any(|p| p.as_ref() == prefix) {
            return;
        }
        if self.prefixes.len() >= self.cap {
            self.prefixes.remove(0);
        }
        self.prefixes.push(prefix.into());
    }

    /// Validate `s` as an absolute IRI (RFC-3987), accepting *exactly* the set
    /// `oxiri::Iri::parse(s).is_ok()` accepts.
    ///
    /// Tries the memoized fast path first; on a miss, runs the full `oxiri` automaton and, when
    /// it accepts, seeds the ring so future IRIs sharing that prefix take the fast path. [OPUS-4.8]
    #[must_use]
    pub fn validate(&mut self, s: &str) -> bool {
        if self.fast_accept(s) {
            return true;
        }
        // Authoritative fallback — this is the only place the accepted language is defined.
        let ok = oxiri::Iri::parse(s).is_ok();
        if ok {
            self.record(s);
        }
        ok
    }
}

/// Validate `s` as an absolute IRI (RFC-3987) — a convenience for one-shot callers that
/// validate a single IRI and keep no state. Calls the `oxiri` automaton DIRECTLY (no memo
/// fast path). Accepts exactly `oxiri::Iri::parse(s).is_ok()`. [OPUS-4.8]
#[must_use]
pub fn is_valid_iri(s: &str) -> bool {
    // This one-shot entry point does NOT run the prefix-memo fast path or the ASCII suffix
    // pre-scan — it calls the `oxiri` automaton directly. Those fast paths need caller-held
    // ring state, so they live on the stateful validator `IriPrefixMemo::validate`, which the
    // byte-level N-Triples/N-Quads loader (see `nt.rs`) drives per worker thread.
    oxiri::Iri::parse(s).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground truth: the full `oxiri` absolute-IRI automaton.
    fn oxiri_ok(s: &str) -> bool {
        oxiri::Iri::parse(s).is_ok()
    }

    // ---- Unit tests for the building blocks -------------------------------------------------

    #[test]
    fn safe_suffix_byte_table_matches_predicate() {
        for b in 0u8..=255 {
            assert_eq!(SUFFIX_SAFE[b as usize], is_safe_suffix_byte(b), "byte {}", b);
        }
        // Spot-check the boundary of what is / isn't safe.
        assert!(is_safe_suffix_byte(b'a') && is_safe_suffix_byte(b'/') && is_safe_suffix_byte(b'?'));
        assert!(is_safe_suffix_byte(b':') && is_safe_suffix_byte(b'@') && is_safe_suffix_byte(b'~'));
        for &bad in &[b'#', b'%', b'[', b']', b' ', b'<', b'>', b'"', b'\\', 0u8, 0x7f] {
            assert!(!is_safe_suffix_byte(bad), "byte {} must fall back", bad);
        }
        // No non-ASCII byte is ever "safe".
        for b in 128u8..=255 {
            assert!(!is_safe_suffix_byte(b));
        }
    }

    #[test]
    fn prefix_extraction_covers_the_forms() {
        assert_eq!(scheme_authority_prefix_len("http://ex/foo"), Some("http://ex/".len()));
        assert_eq!(scheme_authority_prefix_len("https://a.b.c/x/y"), Some("https://a.b.c/".len()));
        assert_eq!(scheme_authority_prefix_len("http://[::1]:80/p"), Some("http://[::1]:80/".len()));
        // No '/'-terminated authority -> nothing to memo.
        assert_eq!(scheme_authority_prefix_len("http://ex"), None);
        assert_eq!(scheme_authority_prefix_len("http://ex?q=1"), None);
        assert_eq!(scheme_authority_prefix_len("http://ex#f"), None);
        // Non-hierarchical schemes.
        assert_eq!(scheme_authority_prefix_len("urn:isbn:12345"), None);
        assert_eq!(scheme_authority_prefix_len("mailto:a@b.c"), None);
    }

    #[test]
    fn memo_fast_path_fires_on_repeat_prefix() {
        let mut memo = IriPrefixMemo::new();
        assert!(memo.is_empty());
        // First occurrence: oxiri fallback seeds the prefix.
        assert!(memo.validate("http://example.org/a"));
        assert_eq!(memo.len(), 1);
        // A subsequent IRI sharing the prefix must be provable by the fast path alone.
        assert!(memo.fast_accept("http://example.org/b/c?d=e"));
        assert!(memo.validate("http://example.org/b/c?d=e"));
        // A '#' in the suffix is not fast-provable, but validate still agrees with oxiri.
        assert!(!memo.fast_accept("http://example.org/a#frag"));
        assert_eq!(memo.validate("http://example.org/a#frag"), oxiri_ok("http://example.org/a#frag"));
    }

    #[test]
    fn memo_capacity_is_a_bounded_fifo_ring() {
        let mut memo = IriPrefixMemo::with_capacity(2);
        assert!(memo.validate("http://a/x"));
        assert!(memo.validate("http://b/x"));
        assert!(memo.validate("http://c/x")); // evicts http://a/
        assert_eq!(memo.len(), 2);
        // http://a/ was evicted, so its suffix is no longer fast-provable...
        assert!(!memo.fast_accept("http://a/y"));
        // ...but the two most-recent prefixes still are.
        assert!(memo.fast_accept("http://b/y"));
        assert!(memo.fast_accept("http://c/y"));
        // Cap 0 disables the ring entirely (still correct via fallback).
        let mut off = IriPrefixMemo::with_capacity(0);
        assert!(off.validate("http://a/x"));
        assert_eq!(off.len(), 0);
        assert!(!off.fast_accept("http://a/y"));
    }

    #[test]
    fn is_valid_iri_matches_oxiri() {
        for s in ["http://ex/a", "urn:x:y", "not an iri", "", "http://ex/a b", "mailto:a@b.c"] {
            assert_eq!(is_valid_iri(s), oxiri_ok(s), "{:?}", s);
        }
    }

    // ---- The MANDATORY differential-fuzz equivalence gate (sq-7d3dj.18) ---------------------

    /// A committed adversarial corpus — unicode boundaries, percent-encoding, empty authority,
    /// IPv6 literals, bad schemes, and `iunreserved`/`iprivate` edges — replayed deterministically
    /// every run alongside the generated cases. [OPUS-4.8]
    const ADVERSARIAL_CORPUS: &[&str] = &[
        // valid, common
        "http://example.org/foo",
        "http://example.org/foo#frag",
        "http://ex/a?q=1&r=2",
        "https://a.b.c/p/q?x=y#z",
        "urn:isbn:0451450523",
        "mailto:a@b.c",
        "tag:example.com,2024:x",
        "a:b",
        // authority / host edges
        "http://[::1]/x",
        "http://[::1]:8080/x",
        "http://[v1.fe80::a]/x",
        "http://",          // empty authority, empty path
        "http:///x",        // empty authority, absolute path
        "http://u:p@h:1/x", // userinfo + port
        // percent-encoding edges
        "http://ex/%20",
        "http://ex/%2F",
        "http://ex/%GG",    // bad hex
        "http://ex/%2",     // truncated
        "http://ex/%",      // lone percent
        "http://ex/a%zzb",
        // non-ASCII / iunreserved / iprivate boundaries
        "http://ex/\u{00e9}",       // é (ucschar) — valid in path
        "http://ex/\u{00a0}",       // NBSP — NOT ucschar
        "http://ex/\u{d7ff}",       // last BMP before surrogates (ucschar)
        "http://ex/\u{e000}",       // private-use start — iprivate, invalid in PATH
        "http://ex/?\u{e000}",      // iprivate is valid in QUERY
        "http://ex/\u{10ffff}",     // max codepoint — not ucschar
        "http://ex/\u{fdd0}",       // noncharacter
        // raw-illegal / control / whitespace
        "http://ex/a<b",
        "http://ex/a>b",
        "http://ex/a\"b",
        "http://ex/a\\b",
        "http://ex/a`b",
        "http://ex/a{b}",
        "http://ex/a|b",
        "http://ex/a^b",
        "http://ex/a b",
        "http://ex/a\tb",
        "http://ex/a\u{0000}",
        "http://ex/a\u{007f}",
        // bad / relative / empty
        "1http://x",
        ":x",
        "foo",
        "/abs/path",
        "//host/path",
        "",
        "HTTP://EX/x",
        "h+t-t.p://ex/x",
        // suffix structure edges the fast path must reason about
        "http://example.org/a#b#c", // second '#' invalid
        "http://example.org/a?b?c", // second '?' fine (in query)
        "http://example.org/:@/?",  // ipchar extras + separators
        "http://example.org/a;p=1,2",
    ];

    /// Deterministic, allocation-cheap xorshift64* PRNG (no external `rand` dependency, so the
    /// fuzz gate replays byte-identically per PR). [OPUS-4.8]
    struct Rng(u64);
    impl Rng {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// The character alphabet the generator draws from — a deliberate mix of clearly-safe ASCII,
    /// structural delimiters, percent-encoding bytes, and non-ASCII boundary codepoints — so the
    /// generated IRIs straddle the accept/reject frontier where a fast-path bug would hide.
    const ALPHABET: &[char] = &[
        'a', 'Z', '0', '9', '-', '.', '_', '~', // iunreserved
        '!', '$', '&', '\'', '(', ')', '*', '+', ',', ';', '=', // sub-delims
        ':', '@', '/', '?', '#', // separators / ipchar extras
        '%', '2', 'F', 'G', // percent-encoding (valid + invalid)
        '[', ']', ' ', '<', '>', '\\', // illegal-ish / brackets
        '\u{00e9}', '\u{00a0}', '\u{e000}', '\u{10ffff}', // ucschar / non-ucschar / iprivate / max
    ];

    fn gen_iri(rng: &mut Rng, out: &mut String) {
        out.clear();
        // Half the time, prefix with a realistic uniform `scheme://authority/` so the memo path
        // is exercised heavily; otherwise build a fully-random string.
        if rng.below(2) == 0 {
            let prefixes = ["http://ex/", "https://a.b/", "http://[::1]:80/", "ftp://h.i.j/", "x://y/"];
            out.push_str(prefixes[rng.below(prefixes.len())]);
        } else {
            // Random scheme-ish head.
            let head_len = rng.below(6);
            for _ in 0..head_len {
                out.push(ALPHABET[rng.below(ALPHABET.len())]);
            }
            if rng.below(2) == 0 {
                out.push_str("://");
            } else {
                out.push(':');
            }
        }
        let body_len = rng.below(24);
        for _ in 0..body_len {
            out.push(ALPHABET[rng.below(ALPHABET.len())]);
        }
    }

    #[test]
    fn differential_fuzz_fast_path_equivalent_to_oxiri() {
        // 1) The committed adversarial corpus (deterministic replay), each checked against a
        //    FRESH memo AND against the shared memo below.
        let mut shared = IriPrefixMemo::new();
        for &s in ADVERSARIAL_CORPUS {
            let expected = oxiri_ok(s);
            // Fresh-memo equivalence.
            let mut fresh = IriPrefixMemo::new();
            assert_eq!(fresh.validate(s), expected, "fresh validate != oxiri for {:?}", s);
            // Accept-shortcut soundness on a memo primed with a matching prefix: prime, then
            // assert a fast accept is never a wrong accept.
            if let Some(plen) = scheme_authority_prefix_len(s) {
                let mut primed = IriPrefixMemo::new();
                // Seed the prefix by validating a trivially-safe sibling under it.
                let seed = format!("{}seed", &s[..plen]);
                let _ = primed.validate(&seed);
                if primed.fast_accept(s) {
                    assert!(expected, "fast_accept wrongly accepted {:?} (oxiri rejects)", s);
                }
            }
            // Shared-memo equivalence (state accumulates across the corpus).
            assert_eq!(shared.validate(s), expected, "shared validate != oxiri for {:?}", s);
        }

        // 2) The deterministic IRI-structured generator: many randomized cases, each asserting
        //    both the soundness of the accept-shortcut and full equivalence. Two seeds, a shared
        //    memo threaded through so memo hits actually occur.
        for seed in [0x9E37_79B9_7F4A_7C15u64, 0xD1B5_4A32_D192_ED03u64] {
            let mut rng = Rng(seed);
            let mut memo = IriPrefixMemo::new();
            let mut s = String::new();
            for _ in 0..60_000 {
                gen_iri(&mut rng, &mut s);
                let expected = oxiri_ok(&s);
                // Soundness: a standalone fast-accept must never over-accept.
                if memo.fast_accept(&s) {
                    assert!(expected, "fast_accept over-accepted {:?} (oxiri rejects)", s);
                }
                // Full equivalence of the wired validator against oxiri, memo state and all.
                assert_eq!(memo.validate(&s), expected, "validate != oxiri for {:?}", s);
            }
        }
    }
}
