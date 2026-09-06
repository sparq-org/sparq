//! [SONNET-4.6] XPath differential harness — PROOF M1 (`sq-3x7dl.14.2`).
//!
//! Generates a Noir test file asserting that each `noir_XPath` primitive returns
//! EXACTLY what sparq's own trusted Rust SPARQL/XSD scalar evaluator returns, over a
//! unicode-aware corpus. The generated file is compiled against the released
//! `sparq-org/noir_XPath` face repo by `zk/xpath/scripts/run_differential_harness.sh`.
//!
//! ## The oracle
//!
//! `sparq_engine::query` — the same evaluator that answers a real SPARQL `BIND`. Every
//! expected value in the generated file is READ BACK from a `BIND(<expr> AS ?out)` over a
//! one-row graph, never hand-written. Two secondary cross-checks run over each oracle
//! answer and HARD-FAIL generation on disagreement, so a lossy or wrong oracle answer can
//! never be silently pinned into the circuit's test suite:
//!
//! * **IEEE cross-check** — every `xs:double` result is recomputed with native Rust `f64`
//!   (the hardware IEEE-754 reference) and compared BIT-for-BIT.
//! * **F&O cross-check** — every `fn:substring` result is recomputed with an explicit
//!   XPath F&O 3.1 §5.4.3 `fn:substring` window reference implemented here.
//!
//! That F&O reference is itself cross-checked, in this file's own unit tests, against
//! **CPython string slicing** — an out-of-repo, codepoint-indexed statement of the same
//! window — so the multibyte rows do not rest on two in-repo references agreeing with
//! each other. See `tests::fo_substring_agrees_with_cpython_slicing`.
//!
//! Where a cross-check fires, the case is NOT silently dropped or downgraded: it is
//! recorded in [`DIVERGENCES`] and emitted as a **LIVE** assertion carrying the
//! SPEC-correct value, labelled a SPEC-REFERENCE row rather than a sparq-differential one.
//! The spec value is precisely what `noir_XPath` implements, so asserting it exercises the
//! circuit instead of enshrining the engine bug — and a `noir_XPath` regression on one of
//! these edges reds the run. A unit test here pins each such row live, and a second pins
//! that the divergence still reproduces, so the workaround expires the day sparq-engine is
//! fixed.
//!
//! ## HONEST TCB STATEMENT
//!
//! This harness is **VERIFICATION, not proof**. Its trusted computing base is:
//!
//! 1. **sparq's Rust XSD evaluator** — itself UNAUDITED; it is the repo's reference
//!    semantics, not a proven-correct implementation. Two live divergences from XPath F&O
//!    are already recorded in [`DIVERGENCES`].
//! 2. **The SAMPLE** — the corpus is hand-picked edge cases, not exhaustive. A wrong
//!    answer on an unsampled input is not caught.
//! 3. **The Noir → ACIR → Barretenberg lowering** — entirely untrusted-but-unchecked here.
//!    `nargo test` exercises witness generation only; nothing below ACIR is covered.
//!
//! No soundness or privacy claim is made or implied by a green run of this harness. The
//! ZK estate remains research-grade and NOT externally audited (`sq-qhy4`).
//!
//! ## Usage
//!
//! ```text
//! sparq-xpath-differential --output PATH                   # generate the oracle Noir test file
//! sparq-xpath-differential --inject-fault PATH             # every test corrupted, one value each
//! sparq-xpath-differential --inject-fault-in TEST_FN PATH  # exactly ONE test corrupted
//! sparq-xpath-differential --list-fault-sites              # the faultable test fns, one per line
//! ```
//!
//! Fault injection is the non-vacuity self-test: `nargo test` on a corrupted file MUST
//! fail. If it passes, the harness is not wired to the circuit and every green run is
//! meaningless. The driver script proves that **per test function** — it runs one
//! `--inject-fault-in` variant per name from `--list-fault-sites` and requires each to
//! fail — because a single all-faults run only ever proves that SOME test failed. See
//! [`FaultMode`].

use sparq_core::Graph;
use std::fmt::Write as FmtWrite;

// ---------------------------------------------------------------------------
// Oracle — sparq's own Rust SPARQL/XSD scalar evaluator
// ---------------------------------------------------------------------------

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// The one-row dataset every probe evaluates over. `BIND` must neither add nor drop a
/// row, so a row count other than 1 is a harness bug, not a value difference.
fn oracle_graph() -> Graph {
    Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "ntriples")
        .expect("the one-row oracle graph must parse")
}

/// Evaluate `BIND(<expr> AS ?out)` and return the single row's `?out` as a term string
/// (`None` when the expression raised an error, i.e. `?out` is unbound).
fn oracle_term(g: &Graph, expr: &str) -> Option<String> {
    let q = format!(
        "PREFIX xsd: <{XSD}>\nSELECT ?out WHERE {{ ?s <http://ex/p> ?o BIND({expr} AS ?out) }}"
    );
    let r = sparq_engine::query(g, &q)
        .unwrap_or_else(|e| panic!("oracle query failed for `{expr}`: {e}"));
    assert_eq!(r.rows.len(), 1, "BIND must keep exactly one row for `{expr}`");
    r.rows[0][0].as_ref().map(|t| t.to_string())
}

/// Strip `"lex"^^<datatype>` / `"lex"` down to the lexical form, asserting the datatype.
/// `dt` is the XSD local name, or `""` for a plain (untyped) literal.
fn lexical(term: &str, dt: &str, expr: &str) -> String {
    let suffix = if dt.is_empty() { String::new() } else { format!("^^<{XSD}{dt}>") };
    let body = term
        .strip_suffix(&suffix)
        .unwrap_or_else(|| panic!("oracle `{expr}` returned {term}, expected datatype xsd:{dt}"));
    let inner = body
        .strip_prefix('"')
        .and_then(|b| b.strip_suffix('"'))
        .unwrap_or_else(|| panic!("oracle `{expr}` returned a non-literal term {term}"));
    // The term writer escapes `\` and `"`; the corpus avoids both, so a surviving
    // backslash means the corpus grew a case this un-escaper cannot honestly decode.
    assert!(!inner.contains('\\'), "oracle `{expr}` returned an escaped literal {term}");
    inner.to_string()
}

fn oracle_bool(g: &Graph, expr: &str) -> bool {
    let t = oracle_term(g, expr).unwrap_or_else(|| panic!("oracle `{expr}` errored, wanted a boolean"));
    lexical(&t, "boolean", expr) == "true"
}

fn oracle_int(g: &Graph, expr: &str) -> i64 {
    let t = oracle_term(g, expr).unwrap_or_else(|| panic!("oracle `{expr}` errored, wanted an integer"));
    lexical(&t, "integer", expr).parse().unwrap_or_else(|e| panic!("oracle `{expr}`: {e}"))
}

fn oracle_plain_string(g: &Graph, expr: &str) -> String {
    let t = oracle_term(g, expr).unwrap_or_else(|| panic!("oracle `{expr}` errored, wanted a string"));
    lexical(&t, "", expr)
}

/// An `xs:double` oracle answer, as IEEE-754 bits.
///
/// CROSS-CHECK: `expect_ieee` is the same value recomputed with native Rust `f64`. A
/// mismatch means either sparq's evaluator or its double serializer lost the value, so
/// the case must NOT be pinned as-is — generation aborts and the caller is told to record
/// the case in [`DIVERGENCES`] instead.
fn oracle_double_bits(g: &Graph, expr: &str, expect_ieee: f64) -> u64 {
    let t = oracle_term(g, expr).unwrap_or_else(|| panic!("oracle `{expr}` errored, wanted a double"));
    let lex = lexical(&t, "double", expr);
    let parsed: f64 = lex.parse().unwrap_or_else(|e| panic!("oracle `{expr}` lexical {lex:?}: {e}"));
    assert_eq!(
        parsed.to_bits(),
        expect_ieee.to_bits(),
        "IEEE CROSS-CHECK FAILED for `{expr}`: sparq-engine says {lex:?} \
         (bits 0x{:016x}), native Rust f64 says {expect_ieee:?} (bits 0x{:016x}). \
         Do not pin this case — record it in DIVERGENCES with a filed follow-up.",
        parsed.to_bits(),
        expect_ieee.to_bits()
    );
    parsed.to_bits()
}

// ---------------------------------------------------------------------------
// Spec references used as cross-checks
// ---------------------------------------------------------------------------

/// XPath F&O 3.1 §5.4.3 `fn:substring` — `fn:substring($s, $start, $length)` over CODEPOINTS.
///
/// The window is `round(start) <= p < round(start) + round(length)` on 1-based positions
/// and is NOT shifted when `start < 1`: a start below 1 CONSUMES part of the length
/// budget. `substring("12345", 0, 3)` is therefore `"12"`, not `"123"`.
fn fo_substring(s: &str, start: i64, length: i64) -> String {
    let end = start.saturating_add(length); // exclusive
    s.chars()
        .enumerate()
        .filter_map(|(i, c)| {
            let pos = i as i64 + 1;
            (pos >= start && pos < end).then_some(c)
        })
        .collect()
}

/// The CODEPOINT → BYTE position conversion for `fn:substring` (`sq-hjvte`).
///
/// `noir_XPath`'s `substring` windows **byte** positions in the logical content (its own
/// documented caveat), while `fn:substring` / SPARQL `SUBSTR` — and the `string_length`
/// that `sq-3x7dl.6` made codepoint-counting — window **codepoint** positions. Composing
/// the two, e.g. `substring(s, i, string_length(s))`, is therefore wrong for any
/// multibyte input. This is the *convert positions at the boundary* half of that bead:
/// given the F&O CODEPOINT window parameters, return the equivalent BYTE window
/// `(start, length)`, already NORMALIZED (`start >= 1`, `length >= 0`) so neither side
/// has to clamp again.
///
/// The window rule is [`fo_substring`]'s, unchanged — this only re-expresses the SAME
/// window in byte units, which is why the two are pinned against each other by
/// [`tests::codepoint_window_to_byte_window_reproduces_the_fo_window`].
///
/// SCOPE, honestly: this converts on the HOST, which requires the host to know the
/// string's bytes. It does NOT make the in-circuit primitive codepoint-positional — for a
/// PRIVATE (witness) string the conversion has to happen in-circuit, and that stays a
/// `noir_XPath` change under `sq-hjvte`. What it buys here is (a) an executable,
/// oracle-checked statement of the conversion rule, and (b) multibyte coverage of the
/// byte-window primitive, which the corpus previously had none of by construction.
fn codepoint_window_to_byte_window(s: &str, start: i64, length: i64) -> (i64, i64) {
    let end = start.saturating_add(length); // exclusive, in CODEPOINTS
    // The F&O window is contiguous, so the selected bytes are exactly
    // [first selected char's offset, last selected char's end offset).
    let mut lo: Option<usize> = None;
    let mut hi = 0usize;
    for (i, (offset, ch)) in s.char_indices().enumerate() {
        let pos = i as i64 + 1;
        if pos >= start && pos < end {
            lo.get_or_insert(offset);
            hi = offset + ch.len_utf8();
        }
    }
    match lo {
        // Byte positions are 1-based, exactly like the codepoint ones.
        Some(lo) => ((lo + 1) as i64, (hi - lo) as i64),
        // Empty window — any normalized zero-length window will do; pick the canonical one.
        None => (1, 0),
    }
}

/// XPath F&O 3.1 §4.4.4 `fn:round` for `xs:double`: round to nearest, TIES TOWARD +∞
/// (`0.5 -> 1`, `-0.5 -> -0.0`, `-1.5 -> -1`), preserving the sign of a zero result.
fn fo_round_double(v: f64) -> f64 {
    if v.is_nan() || v.is_infinite() || v == 0.0 {
        return v;
    }
    let r = (v + 0.5).floor();
    // A negative input rounding to zero yields NEGATIVE zero per F&O.
    if r == 0.0 && v.is_sign_negative() {
        -0.0
    } else {
        r
    }
}

// ---------------------------------------------------------------------------
// Recorded oracle divergences (sparq-engine vs XPath F&O)
// ---------------------------------------------------------------------------

/// A case where sparq's evaluator — the ORACLE — is itself wrong against XPath F&O, which
/// is what `noir_XPath` implements. Asserting the oracle's answer here would enshrine the
/// engine bug and paint the (correct) circuit red, so these cases are emitted as LIVE
/// assertions against the F&O value instead, labelled SPEC-REFERENCE (their expected value
/// comes from the F&O reference in this file, not from the oracle), and the divergence is
/// stated in the generated header. Keeping them live is what makes a `noir_XPath`
/// regression on an edge it already fixed fail the run. Each is pinned by two unit tests
/// below: one asserting the row is emitted live, one asserting the divergence still
/// reproduces, so the entry expires when the engine is fixed.
struct Divergence {
    /// The SPARQL expression, as evaluated against the oracle.
    expr: &'static str,
    /// What sparq-engine returns today.
    sparq: &'static str,
    /// What XPath F&O requires (and `noir_XPath` implements).
    spec: &'static str,
    /// One line of why.
    why: &'static str,
}

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        expr: "SUBSTR(s, start, len) for start < 1",
        sparq: "clamps start to 1 and then takes `len` characters",
        spec: "the window [start, start+len) is NOT shifted, so a start below 1 consumes length",
        why: "sparq-engine exec.rs SubStr uses `start.max(1) - 1` then `.take(len)`; F&O 3.1 \
              §5.4.3 `fn:substring` keeps the window. e.g. SUBSTR(\"12345\", 0, 3) is \"12\", \
              not \"123\".",
    },
    Divergence {
        expr: "ROUND(xsd:double(\"-0.5\"))",
        sparq: "+0.0 (lexical \"0\")",
        spec: "-0.0E0 — a negative argument in [-0.5, 0) rounds to NEGATIVE zero",
        why: "sparq-engine's ROUND loses the sign of a zero result; the double serializer \
              itself is fine (xsd:double(\"-0.0\") does render as \"-0E0\").",
    },
];

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// One corpus string: the logical value, plus the Noir `str<CAP>` capacity it is stored
/// in. `cap > value.len()` exercises the NUL-padding path (Noir strings are fixed-width
/// byte buffers whose logical content ends at the first NUL).
struct S {
    value: &'static str,
    cap: usize,
}

const fn s(value: &'static str, cap: usize) -> S {
    S { value, cap }
}

/// Unicode-aware string corpus. Byte length != codepoint count for every entry after the
/// first three — which is exactly the STRLEN edge the qt3 corpus does not cover.
fn string_corpus() -> Vec<S> {
    vec![
        s("hello", 5),          // ASCII, exact capacity
        s("", 0),               // empty
        s("hello", 10),         // NUL-PADDED: logical length 5 in a str<10> buffer
        s("é", 2),              // U+00E9 — 2 bytes, 1 codepoint
        s("aé", 3),             // mixed ASCII + 2-byte
        s("naïve", 6),          // 6 bytes, 5 codepoints
        s("日本語", 9),          // 3-byte codepoints
        s("𝄞", 4),              // U+1D11E — 4-byte (astral) codepoint
        s("e\u{301}", 3),       // combining acute: 2 codepoints, 1 grapheme
        s("ﬀ", 3),              // U+FB00 ligature — 3 bytes, 1 codepoint
        s("日本語", 15),         // multibyte AND NUL-padded
    ]
}

/// (haystack, needle) pairs for CONTAINS / STRSTARTS / STRENDS. UTF-8 is
/// self-synchronizing, so byte-substring matching and codepoint-substring matching agree
/// — these are safe to sample with multibyte content (unlike SUBSTR, see below).
fn pair_corpus() -> Vec<(S, S)> {
    vec![
        (s("hello", 5), s("ell", 3)),
        (s("hello", 5), s("hel", 3)),
        (s("hello", 5), s("llo", 3)),
        (s("hello", 5), s("", 0)),          // empty needle
        (s("hello", 5), s("zzz", 3)),       // absent
        (s("hello", 10), s("llo", 3)),      // NUL-padded haystack
        (s("hello", 5), s("llo", 6)),       // NUL-padded needle (sq-0ylr9)
        (s("hello", 10), s("hell", 4)),     // prefix, not suffix
        (s("naïve", 6), s("ï", 2)),         // multibyte needle, interior
        (s("naïve", 6), s("na", 2)),        // ASCII prefix of multibyte haystack
        (s("naïve", 6), s("ve", 2)),        // ASCII suffix of multibyte haystack
        (s("日本語", 9), s("本", 3)),        // 3-byte needle, interior
        (s("日本語", 9), s("日", 3)),        // 3-byte prefix
        (s("日本語", 9), s("語", 3)),        // 3-byte suffix
        (s("𝄞", 4), s("𝄞", 4)),            // astral, whole
        (s("lo", 10), s("hello", 5)),       // needle longer than logical content
    ]
}

/// `fn:substring` cases whose positions are passed to the circuit **VERBATIM**.
///
/// **ASCII ONLY, deliberately.** `noir_XPath`'s `substring` indexes BYTE positions in the
/// logical content (documented caveat in its `string.nr`; codepoint-positional substring
/// is bead `sq-hjvte`), while SPARQL SUBSTR is codepoint-indexed. The two agree exactly on
/// single-byte content and only there — sampling multibyte HERE would pin a divergence
/// that is a KNOWN, beaded gap rather than a regression. Multibyte content is covered by
/// [`substring_multibyte_corpus`] instead, through the boundary conversion.
///
/// This section is what exercises the primitive's OWN F&O window arithmetic (`start < 1`
/// consuming the length budget, `sq-3x7dl.6`), because only here does the circuit receive
/// an un-normalized window.
///
/// `start < 1` cases carry the F&O window semantics that `sq-3x7dl.6` fixed in the
/// circuit; sparq-engine still shifts (see [`DIVERGENCES`]), so those rows are emitted
/// commented-out against the spec value.
fn substring_corpus() -> Vec<(&'static str, usize, i64, i64)> {
    vec![
        // (value, cap, start, length)
        ("hello", 5, 2, 3),
        ("hello", 5, 1, 5),
        ("hello", 5, 1, 100), // length past the end clamps
        ("hello", 5, 3, 0),   // zero length
        ("hello", 5, 7, 3),   // start past the end
        ("hello", 5, 5, 1),   // last character
        ("hello", 10, 2, 3),  // NUL-padded buffer
        ("12345", 5, 0, 3),   // start < 1  (F&O window: "12")
        ("hello", 5, -2, 4),  // start < 1  (F&O window: "h")
        ("12345", 5, -3, 5),  // start < 1  (F&O window: "1")
        ("hello", 5, -100, 3), // window entirely before the string
    ]
}

/// `fn:substring` cases with **MULTIBYTE** content, driven through
/// [`codepoint_window_to_byte_window`] (`sq-hjvte`).
///
/// The positions below are CODEPOINT positions — what SPARQL `SUBSTR` and `fn:substring`
/// take, and what the oracle is asked for. The generator converts each to the equivalent
/// BYTE window before emitting the call, so the byte-positional primitive is held to the
/// CODEPOINT answer. Every entry has `chars().count() != len()`, which is exactly the
/// regime [`substring_corpus`] cannot reach.
///
/// What this catches that the ASCII section cannot: a byte window that lands mid-codepoint
/// or truncates a trailing continuation byte, a logical-length (NUL) scan that misreads a
/// `>= 0x80` byte, and any per-byte handling that is only correct for 7-bit content. What
/// it deliberately does NOT claim: that the primitive is codepoint-positional. It is not —
/// see the scope note on [`codepoint_window_to_byte_window`].
fn substring_multibyte_corpus() -> Vec<(&'static str, usize, i64, i64)> {
    vec![
        // (value, cap, CODEPOINT start, CODEPOINT length)
        ("naïve", 6, 3, 1),     // the 2-byte codepoint, alone
        ("naïve", 6, 1, 3),     // window ENDING on the 2-byte codepoint
        ("naïve", 6, 3, 100),   // multibyte start, length past the end
        ("naïve", 6, 1, 5),     // whole string: 5 codepoints, 6 bytes
        ("naïve", 6, 4, 2),     // window STARTING after the multibyte codepoint
        ("日本語", 9, 2, 1),     // interior 3-byte codepoint
        ("日本語", 9, 2, 2),     // two 3-byte codepoints
        ("日本語", 15, 2, 2),    // ... in a NUL-PADDED buffer
        ("𝄞", 4, 1, 1),         // astral (4-byte) codepoint
        ("aé", 3, 2, 1),        // ASCII then 2-byte
        ("e\u{301}", 3, 2, 1),  // combining acute: codepoint 2 of a one-grapheme string
        ("ﬀ", 3, 1, 1),         // 3-byte ligature
        ("日本語", 9, 3, 0),     // zero length at a multibyte position
        ("日本語", 9, 4, 2),     // start past the CODEPOINT end (byte length would not be)
        ("日本語", 9, 0, 2),     // start < 1: the F&O window consumes length
    ]
}

/// `op:numeric-divide` on two `xs:integer` operands. F&O 3.1 §4.2.4 `op:numeric-divide`
/// makes this the DECIMAL quotient (`7 div 2 = 3.5`); `noir_XPath` implements the
/// documented double approximation, so the oracle is asked for the double quotient of the
/// promoted operands. Non-exact quotients are the cases the qt3 corpus never recorded.
const DIVIDE_CORPUS: &[(i64, i64)] =
    &[(7, 2), (1, 3), (10, 5), (-7, 2), (7, -2), (-7, -2), (1, 7), (2, 3), (100, 7), (0, 5)];

/// Mixed `xs:integer` ↔ `xs:double` comparison operands. Every integer here is OUTSIDE
/// `[-128, 127]` — the i8 range the pre-`sq-3x7dl.5` circuit silently truncated to, which
/// made `256 = 0.0` evaluate TRUE.
const MIXED_CORPUS: &[(i64, &str)] = &[
    (256, "0"),                              // the sq-3x7dl.5 witness: i8-wrap made this true
    (256, "256"),
    (-129, "-129"),
    (128, "128"),
    (384, "128"),                            // 384 wraps to 128 in i8
    (9007199254740993, "9007199254740992"),  // 2^53 + 1 collapses onto 2^53 under promotion
    (1000000, "999999.5"),
    (-1000000, "0"),
];

/// `xs:double -> xs:integer` casts (`fn:round` is NOT applied — the cast truncates toward
/// zero per F&O 3.1 §19.1.2.4 "Casting to xs:integer").
const CAST_CORPUS: &[&str] = &["3.9", "-3.9", "0", "-0.5", "2.5", "9007199254740992", "-2.5"];

/// `fn:round` on `xs:double`.
const ROUND_CORPUS: &[&str] = &["2.5", "-2.5", "0.5", "1.5", "-1.5", "2.4999", "-0.5", "0", "-3"];

/// Pre-1970 (and epoch-straddling) `xs:dateTime` instants. The negative epoch is the
/// `sq-3x7dl.7` edge: a `u64` cast of the two's-complement epoch corrupts every component.
const DATETIME_CORPUS: &[(&str, i64, u8, u8, u8, u8, u8)] = &[
    // (lexical, year, month, day, hour, minute, second)
    ("1969-07-20T20:17:40Z", 1969, 7, 20, 20, 17, 40),
    ("1901-12-13T20:45:52Z", 1901, 12, 13, 20, 45, 52),
    ("1969-12-31T23:59:59Z", 1969, 12, 31, 23, 59, 59),
    ("1970-01-01T00:00:00Z", 1970, 1, 1, 0, 0, 0),
    ("1970-01-01T00:00:01Z", 1970, 1, 1, 0, 0, 1),
    ("1968-02-29T12:00:00Z", 1968, 2, 29, 12, 0, 0),
    ("2024-06-15T08:09:10Z", 2024, 6, 15, 8, 9, 10),
];

// ---------------------------------------------------------------------------
// Noir literal rendering
// ---------------------------------------------------------------------------

/// Render `value` as a Noir `str<cap>` literal, NUL-padding to the declared capacity.
/// Multibyte UTF-8 is emitted verbatim (Noir string literals are byte buffers, and its
/// own test suite pins `let s: str<2> = "é";`).
fn noir_str(value: &str, cap: usize) -> String {
    assert!(
        value.len() <= cap,
        "corpus entry {value:?} is {} bytes but declares capacity {cap}",
        value.len()
    );
    assert!(
        !value.contains('"') && !value.contains('\\') && !value.contains('\0'),
        "corpus entry {value:?} needs escaping this renderer deliberately does not do"
    );
    let mut out = String::from("\"");
    out.push_str(value);
    for _ in value.len()..cap {
        out.push_str("\\0");
    }
    out.push('"');
    out
}

/// A SPARQL string literal for the oracle query. Same no-escaping contract as `noir_str`.
fn sparql_str(value: &str) -> String {
    assert!(!value.contains('"') && !value.contains('\\'));
    format!("\"{value}\"")
}

fn hex64(bits: u64) -> String {
    format!("0x{bits:016x} as u64")
}

/// Rewrite ONE comment line so it holds only ASCII.
///
/// `noirc` (1.0.0-beta.21) hard-errors with `Non-ASCII character in comment` on any
/// multibyte byte inside `//` or `///`, while happily accepting multibyte STRING
/// literals — which this corpus exists to exercise. So the comments are transliterated
/// on the way out instead of the prose being written in a degraded style: typographic
/// punctuation maps to its ASCII spelling, and anything else becomes an explicit
/// `\u{...}` escape. For the unicode corpus rows that escape is strictly more precise
/// than the glyph, because the assertion underneath is about CODEPOINTS.
fn ascii_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        match ch {
            c if c.is_ascii() => out.push(c),
            '\u{2014}' => out.push_str("--"),        // em dash
            '\u{2013}' => out.push('-'),             // en dash
            '\u{2192}' => out.push_str("->"),        // rightwards arrow
            '\u{2194}' => out.push_str("<->"),       // left right arrow
            '\u{221e}' => out.push_str("infinity"),  // infinity
            '\u{00a7}' => out.push_str("sec. "),     // section sign
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201c}' | '\u{201d}' => out.push('"'),
            other => out.push_str(&format!("\\u{{{:x}}}", other as u32)),
        }
    }
    out
}

/// Post-pass over the whole generated file: transliterate every comment line, leave code
/// (and its multibyte string literals) byte-for-byte alone, and PROVE the result satisfies
/// noirc's rule — so a future prose edit cannot silently re-red the `nargo` lane.
fn asciify_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let is_comment = line.trim_start().starts_with("//");
        // Every comment this generator emits sits on its own line, which is what makes the
        // line-based split above exact. A trailing comment (or a corpus value containing
        // `//`) would need this pass to understand string literals — fail loudly rather
        // than emit a file noirc will reject.
        assert!(
            is_comment || !line.contains("//"),
            "generated code line carries an unsanitized trailing comment; teach \
             asciify_comments about it: {}",
            line
        );
        let rewritten = if is_comment { ascii_comment(line) } else { line.to_string() };
        assert!(
            !is_comment || rewritten.is_ascii(),
            "comment line is still non-ASCII after transliteration; add the character to \
             ascii_comment: {}",
            rewritten
        );
        out.push_str(&rewritten);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// One line fault injection corrupts, plus its corrupted form and the generated Noir
/// `#[test]` it lives in.
struct FaultSite {
    /// The `fn differential_oracle_*` this site sits inside. One site per test function.
    test_fn: String,
    original: String,
    faulty: String,
}

/// Which of the registered fault sites [`generate_noir_file`] actually corrupts.
///
/// The self-test's whole claim is "a deliberately wrong expected value makes `nargo test`
/// fail, therefore the generated file is really wired to the circuit". Corrupting the
/// whole file at once cannot establish that per test function: a `nargo test` run reports
/// ONE exit status for the whole file, and nonzero there says only that SOME test failed,
/// so nine sections that had drifted into something incapable of failing (a corpus that
/// emptied, an expected value computed from the very call it asserts) would hide behind
/// the one section that still works.
///
/// [`FaultMode::Only`] is what makes the claim per-function. A variant carrying exactly
/// one fault is byte-identical to the oracle file everywhere else, and that oracle file
/// has just been run green — so if the variant's `nargo test` fails, it failed *because
/// of that one corrupted value*. Running one variant per site is therefore a proof
/// obligation discharged once per test function.
#[derive(Clone, Debug, PartialEq)]
enum FaultMode {
    /// No corruption: the real oracle file.
    None,
    /// One expected value corrupted in EVERY generated test (`--inject-fault`). Useful
    /// for eyeballing the whole fault set in one file; NOT sufficient on its own as the
    /// non-vacuity proof, for the reason above.
    All,
    /// One expected value corrupted in exactly the named test (`--inject-fault-in`).
    Only(String),
}

/// The fault sites available for injection — **one per generated Noir `#[test]`**.
#[derive(Default)]
struct Faults {
    sites: Vec<FaultSite>,
}

impl Faults {
    /// Offer a candidate site for `test_fn`; the FIRST offer per test function wins, so a
    /// generator loop can offer on every row without special-casing its first iteration.
    fn offer(&mut self, test_fn: &str, original: &str, faulty: String) {
        assert_ne!(original, faulty, "fault site for {test_fn} is not actually corrupted");
        if self.sites.iter().any(|s| s.test_fn == test_fn) {
            return;
        }
        self.sites.push(FaultSite {
            test_fn: test_fn.to_string(),
            original: original.to_string(),
            faulty,
        });
    }
}

/// The `fn differential_oracle_*` names actually emitted into `src`, in file order.
fn emitted_test_fns(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|l| l.strip_prefix("fn ")?.strip_suffix("() {"))
        .map(str::to_string)
        .collect()
}

/// The LIVE assertion lines of `src`, keyed by the generated `#[test]` they sit in.
/// Used by the non-vacuity guards, which are the only consumers that need the grouping.
#[cfg(test)]
fn live_assertions_by_test(src: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in src.lines() {
        if let Some(name) = line.strip_prefix("fn ").and_then(|l| l.strip_suffix("() {")) {
            out.push((name.to_string(), Vec::new()));
        } else if line.trim_start().starts_with("assert(") {
            out.last_mut()
                .expect("an assertion was emitted outside any test function")
                .1
                .push(line.trim().to_string());
        }
    }
    out
}

/// Running assertion count, so the generated file can state its own coverage and an
/// accidentally-empty section cannot pass silently.
#[derive(Default)]
struct Counts {
    /// Every LIVE assertion in the generated file, `spec_reference` rows included.
    assertions: usize,
    /// The subset of `assertions` whose expected value came from the F&O reference in this
    /// file rather than from the oracle, because the oracle diverges (see [`DIVERGENCES`]).
    spec_reference: usize,
}

fn header(out: &mut String, mode: &FaultMode) {
    writeln!(out, "// [SONNET-4.6] GENERATED by zk/xpath/differential/src/main.rs — DO NOT EDIT BY HAND.").unwrap();
    writeln!(out, "// Regenerate: bash zk/xpath/scripts/run_differential_harness.sh --update-committed").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// PROOF M1 (sq-3x7dl.14.2): no expected value below is hand-written. Each was READ").unwrap();
    writeln!(out, "// BACK from sparq's own Rust SPARQL/XSD scalar evaluator (`sparq_engine::query`, a").unwrap();
    writeln!(out, "// real BIND over a one-row graph), EXCEPT the rows explicitly labelled").unwrap();
    writeln!(out, "// SPEC-REFERENCE, whose value comes from the generator's XPath F&O reference").unwrap();
    writeln!(out, "// because the oracle itself diverges there (see RECORDED ORACLE DIVERGENCES).").unwrap();
    writeln!(out, "// Doubles are additionally cross-checked BIT-for-BIT against native Rust f64, and").unwrap();
    writeln!(out, "// fn:substring against an explicit XPath F&O 3.1 sec. 5.4.3 fn:substring window reference;").unwrap();
    writeln!(out, "// an UNRECORDED cross-check mismatch ABORTS generation.").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// TCB, honestly: (1) the sparq Rust XSD evaluator is itself UNAUDITED — it is the").unwrap();
    writeln!(out, "// repo's reference semantics, not a proven implementation; (2) coverage is a").unwrap();
    writeln!(out, "// hand-picked SAMPLE, not exhaustive; (3) the Noir -> ACIR -> Barretenberg").unwrap();
    writeln!(out, "// lowering is NOT covered — `nargo test` exercises witness generation only.").unwrap();
    writeln!(out, "// This is VERIFICATION, not proof. No soundness or privacy claim is made or").unwrap();
    writeln!(out, "// implied; the ZK estate stays research-grade and NOT externally audited (sq-qhy4).").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// RECORDED ORACLE DIVERGENCES (sparq-engine vs XPath F&O, which noir_XPath").unwrap();
    writeln!(out, "// implements). These cases are still asserted LIVE, but against the SPEC value and").unwrap();
    writeln!(out, "// labelled SPEC-REFERENCE at the row: asserting the ORACLE's answer would enshrine").unwrap();
    writeln!(out, "// an engine bug and red a correct circuit, whereas asserting the F&O value keeps").unwrap();
    writeln!(out, "// the edge EXECUTABLE, so a noir_XPath regression on it fails this file. Read a").unwrap();
    writeln!(out, "// SPEC-REFERENCE row as `noir_XPath == XPath F&O`, not as `noir_XPath == sparq`.").unwrap();
    writeln!(out, "// Each is pinned by unit tests in the generator so the entry expires when the").unwrap();
    writeln!(out, "// engine is fixed:").unwrap();
    for d in DIVERGENCES {
        writeln!(out, "//   * {}", d.expr).unwrap();
        writeln!(out, "//       sparq-engine: {}", d.sparq).unwrap();
        writeln!(out, "//       XPath F&O:    {}", d.spec).unwrap();
        writeln!(out, "//       {}", d.why).unwrap();
    }
    writeln!(out, "//").unwrap();
    writeln!(out, "// KNOWN SCOPE LIMIT (sq-hjvte): noir_XPath's `substring` indexes BYTE positions in").unwrap();
    writeln!(out, "// the logical content (its own documented caveat), while SPARQL SUBSTR / fn:substring").unwrap();
    writeln!(out, "// index CODEPOINTS -- and `string_length` counts CODEPOINTS too (sq-3x7dl.6), so").unwrap();
    writeln!(out, "// `substring(s, i, string_length(s))` is WRONG for multibyte s. The two units agree").unwrap();
    writeln!(out, "// exactly on single-byte content and only there. Both regimes are covered here:").unwrap();
    writeln!(out, "//   * differential_oracle_substring -- ASCII, positions passed VERBATIM, so it holds").unwrap();
    writeln!(out, "//     noir_XPath to the F&O window arithmetic itself (the start < 1 budget rule).").unwrap();
    writeln!(out, "//   * differential_oracle_substring_multibyte -- multibyte, with each CODEPOINT window").unwrap();
    writeln!(out, "//     converted to the equivalent BYTE window by the generator before the call, so the").unwrap();
    writeln!(out, "//     byte-positional primitive is held to the CODEPOINT answer.").unwrap();
    writeln!(out, "// That conversion is HOST-side and needs the string's bytes. The in-circuit primitive").unwrap();
    writeln!(out, "// is still byte-positional; making it codepoint-positional for a PRIVATE string stays").unwrap();
    writeln!(out, "// open as sq-hjvte in the noir_XPath face repo.").unwrap();
    match mode {
        FaultMode::None => {}
        FaultMode::All => {
            writeln!(out, "//").unwrap();
            writeln!(out, "// !! INJECT-FAULT BUILD: ONE expected value in EVERY test below has been").unwrap();
            writeln!(out, "// !! deliberately corrupted. `nargo test` on this file MUST FAIL -- but a").unwrap();
            writeln!(out, "// !! single failure only proves SOME test can fail, which is why the driver").unwrap();
            writeln!(out, "// !! script runs one --inject-fault-in variant per test function instead.").unwrap();
        }
        FaultMode::Only(name) => {
            writeln!(out, "//").unwrap();
            writeln!(out, "// !! INJECT-FAULT VARIANT for `{}`: ONE expected value in that test has", name).unwrap();
            writeln!(out, "// !! been deliberately corrupted; every other test below is the real oracle,").unwrap();
            writeln!(out, "// !! which has already run green. `nargo test` on this file MUST FAIL, and it").unwrap();
            writeln!(out, "// !! can only fail because of that one value.").unwrap();
        }
    }
    writeln!(out).unwrap();
    writeln!(out, "use xpath::{{").unwrap();
    writeln!(out, "    cast_double_to_integer, compare_double_int_eq, compare_double_int_ge,").unwrap();
    writeln!(out, "    compare_double_int_gt, compare_double_int_le, compare_double_int_lt,").unwrap();
    writeln!(out, "    compare_int_double_eq, compare_int_double_ge, compare_int_double_gt,").unwrap();
    writeln!(out, "    compare_int_double_le, compare_int_double_lt, contains, datetime_from_components,").unwrap();
    writeln!(out, "    datetime_less_than, day_from_datetime, ends_with, hours_from_datetime,").unwrap();
    writeln!(out, "    minutes_from_datetime, month_from_datetime, numeric_divide_int,").unwrap();
    writeln!(out, "    numeric_divide_int_as_double, round_double, seconds_from_datetime, starts_with,").unwrap();
    writeln!(out, "    string_length, substring, XsdDouble, year_from_datetime,").unwrap();
    writeln!(out, "}};").unwrap();
    writeln!(out).unwrap();
}

fn gen_string_length(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// fn:string-length — CODEPOINTS, not bytes, and stopping at the NUL terminator.").unwrap();
    writeln!(out, "/// Oracle: SPARQL `STRLEN(<literal>)`.").unwrap();
    writeln!(out, "#[test]").unwrap();
    writeln!(out, "fn differential_oracle_string_length() {{").unwrap();
    for (i, item) in string_corpus().iter().enumerate() {
        let expr = format!("STRLEN({})", sparql_str(item.value));
        let expected = oracle_int(g, &expr);
        writeln!(out, "    // {expr} -> {expected}").unwrap();
        writeln!(out, "    let s{i}: str<{}> = {};", item.cap, noir_str(item.value, item.cap)).unwrap();
        let line = format!("    assert(string_length::<{}>(s{i}) == {expected});", item.cap);
        // The first non-zero-length case is the fault target: +1 makes it provably wrong.
        if expected > 0 {
            f.offer(
                "differential_oracle_string_length",
                &line,
                format!("    assert(string_length::<{}>(s{i}) == {});", item.cap, expected + 1),
            );
        }
        writeln!(out, "{line}").unwrap();
        c.assertions += 1;
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn gen_string_predicates(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    // (Noir fn, SPARQL builtin)
    for (nfn, builtin) in [("starts_with", "STRSTARTS"), ("ends_with", "STRENDS"), ("contains", "CONTAINS")] {
        writeln!(out, "/// fn:{} — oracle: SPARQL `{builtin}`.", nfn.replace('_', "-")).unwrap();
        writeln!(out, "#[test]").unwrap();
        writeln!(out, "fn differential_oracle_{nfn}() {{").unwrap();
        for (i, (hay, needle)) in pair_corpus().iter().enumerate() {
            let expr = format!("{builtin}({}, {})", sparql_str(hay.value), sparql_str(needle.value));
            let expected = oracle_bool(g, &expr);
            writeln!(out, "    // {expr} -> {expected}").unwrap();
            writeln!(out, "    let h{i}: str<{}> = {};", hay.cap, noir_str(hay.value, hay.cap)).unwrap();
            writeln!(out, "    let n{i}: str<{}> = {};", needle.cap, noir_str(needle.value, needle.cap)).unwrap();
            let line =
                format!("    assert({nfn}::<{}, {}>(h{i}, n{i}) == {expected});", hay.cap, needle.cap);
            // Flipping the expected boolean is provably wrong whichever way it went.
            f.offer(
                &format!("differential_oracle_{nfn}"),
                &line,
                format!("    assert({nfn}::<{}, {}>(h{i}, n{i}) == {});", hay.cap, needle.cap, !expected),
            );
            writeln!(out, "{line}").unwrap();
            c.assertions += 1;
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

/// Which position UNIT the generated `substring` call passes to `noir_XPath`.
#[derive(Clone, Copy, PartialEq)]
enum Positions {
    /// Pass the F&O positions straight through. Sound only for ASCII content, and the
    /// only way the primitive's OWN window arithmetic (`start < 1`) gets exercised.
    Verbatim,
    /// Convert the CODEPOINT window to the equivalent BYTE window
    /// ([`codepoint_window_to_byte_window`]) before the call — the host-side half of
    /// `sq-hjvte`, and what makes multibyte content samplable at all.
    ConvertedToBytes,
}

fn gen_substring(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// fn:substring — ASCII-only (see the byte-vs-codepoint scope limit above).").unwrap();
    writeln!(out, "/// Oracle: SPARQL `SUBSTR`, cross-checked against the F&O 3.1 sec. 5.4.3 fn:substring").unwrap();
    writeln!(out, "/// window. The `start < 1` rows are SPEC-REFERENCE (see the header): sparq-engine shifts").unwrap();
    writeln!(out, "/// the window, so their expected value is F&O's, and they assert noir_XPath against the SPEC.").unwrap();
    writeln!(out, "///").unwrap();
    writeln!(out, "/// Positions reach the circuit VERBATIM here, which is what holds noir_XPath to the F&O").unwrap();
    writeln!(out, "/// window arithmetic itself (sq-3x7dl.6). Multibyte content is covered separately, by").unwrap();
    writeln!(out, "/// differential_oracle_substring_multibyte.").unwrap();
    substring_section(
        out,
        g,
        c,
        f,
        &SubstringSection {
            test_fn: "differential_oracle_substring",
            var: "",
            corpus: substring_corpus(),
            positions: Positions::Verbatim,
        },
    );
}

fn gen_substring_multibyte(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// fn:substring over MULTIBYTE content (sq-hjvte). Oracle: SPARQL `SUBSTR`, which is").unwrap();
    writeln!(out, "/// CODEPOINT-indexed, cross-checked against the F&O 3.1 sec. 5.4.3 window reference (and,").unwrap();
    writeln!(out, "/// in the generator's own unit tests, against CPython's codepoint slicing).").unwrap();
    writeln!(out, "///").unwrap();
    writeln!(out, "/// noir_XPath's `substring` windows BYTE positions, so each row's CODEPOINT window is").unwrap();
    writeln!(out, "/// converted to the equivalent BYTE window BY THE GENERATOR and the byte window is what").unwrap();
    writeln!(out, "/// the call below carries -- the row comment shows both. The expected value is still the").unwrap();
    writeln!(out, "/// CODEPOINT answer, so these rows hold the primitive to fn:substring on content where").unwrap();
    writeln!(out, "/// byte and codepoint positions DISAGREE.").unwrap();
    writeln!(out, "///").unwrap();
    writeln!(out, "/// HONEST SCOPE: that conversion happens on the HOST, which knows the string's bytes. It").unwrap();
    writeln!(out, "/// does NOT make the primitive codepoint-positional -- for a PRIVATE (witness) string the").unwrap();
    writeln!(out, "/// conversion has to happen in-circuit, which is still open as sq-hjvte in the noir_XPath").unwrap();
    writeln!(out, "/// face repo. Until then `substring(s, i, string_length(s))` remains WRONG for multibyte s.").unwrap();
    substring_section(
        out,
        g,
        c,
        f,
        &SubstringSection {
            test_fn: "differential_oracle_substring_multibyte",
            var: "m",
            corpus: substring_multibyte_corpus(),
            positions: Positions::ConvertedToBytes,
        },
    );
}

/// One generated `fn:substring` test function: which corpus it walks, which position unit
/// its calls carry, and how its generated bindings are named.
struct SubstringSection {
    test_fn: &'static str,
    /// Prefixes the generated bindings so two sections can share row indices without
    /// either colliding with the other's fault site (matched by unique line text).
    var: &'static str,
    corpus: Vec<(&'static str, usize, i64, i64)>,
    positions: Positions,
}

/// Emit one `fn:substring` test function.
fn substring_section(
    out: &mut String,
    g: &Graph,
    c: &mut Counts,
    f: &mut Faults,
    section: &SubstringSection,
) {
    let SubstringSection { test_fn, var, positions, .. } = *section;
    writeln!(out, "#[test]").unwrap();
    writeln!(out, "fn {test_fn}() {{").unwrap();
    for (i, &(value, cap, start, length)) in section.corpus.iter().enumerate() {
        match positions {
            Positions::Verbatim => assert!(
                value.is_ascii(),
                "a verbatim-position substring corpus must stay ASCII: {value:?}"
            ),
            // A single-byte row here would be indistinguishable from a Verbatim one and
            // would silently stop this section from covering what it exists to cover.
            Positions::ConvertedToBytes => assert!(
                value.chars().count() != value.len(),
                "the multibyte substring corpus must not carry ASCII: {value:?}"
            ),
        }
        let expr = format!("SUBSTR({}, {start}, {length})", sparql_str(value));
        let sparq_answer = oracle_plain_string(g, &expr);
        let spec_answer = fo_substring(value, start, length);
        let diverges = sparq_answer != spec_answer;
        // A divergence is only ever expected for the recorded start < 1 window bug.
        assert!(
            !diverges || start < 1,
            "UNRECORDED oracle divergence on `{expr}`: sparq-engine {sparq_answer:?} vs \
             F&O {spec_answer:?}. Investigate before pinning — do not widen DIVERGENCES blindly."
        );
        if diverges {
            writeln!(out, "    // SPEC-REFERENCE (start < 1): sparq-engine says {sparq_answer:?}, F&O 3.1 sec. 5.4.3").unwrap();
            writeln!(out, "    // fn:substring says {spec_answer:?}. noir_XPath is correct here (sq-3x7dl.6) and the").unwrap();
            writeln!(out, "    // oracle is not, so the row asserts the SPEC value LIVE — it holds noir_XPath to").unwrap();
            writeln!(out, "    // F&O, not to sparq.").unwrap();
            writeln!(out, "    // Drop the special-casing (not the assertion) when the engine is fixed.").unwrap();
            // One length assertion plus one per expected byte.
            c.spec_reference += 1 + spec_answer.len();
        }
        c.assertions += 1 + spec_answer.len();
        // An empty expected result leaves the byte buffer unread; bind it to `_out` so
        // the generated file does not trip Noir's unused-variable warning.
        let bind =
            if spec_answer.is_empty() { format!("_{var}out{i}") } else { format!("{var}out{i}") };
        // The window the CALL carries. Verbatim rows pass the F&O positions through;
        // converted rows carry the byte window, and say so, so the row stays readable as
        // "codepoint question, byte call, codepoint answer".
        let (call_start, call_length) = match positions {
            Positions::Verbatim => (start, length.max(0)),
            Positions::ConvertedToBytes => codepoint_window_to_byte_window(value, start, length),
        };
        writeln!(out, "    // {expr} -> {spec_answer:?}").unwrap();
        if positions == Positions::ConvertedToBytes {
            writeln!(
                out,
                "    // codepoint window ({start}, {length}) -> byte window ({call_start}, {call_length})"
            )
            .unwrap();
        }
        writeln!(out, "    let {var}ss{i}: str<{cap}> = {};", noir_str(value, cap)).unwrap();
        writeln!(
            out,
            "    let ({bind}, {var}len{i}) = substring::<{cap}, {cap}>({var}ss{i}, {call_start}, {call_length});"
        )
        .unwrap();
        let len_line = format!("    assert({var}len{i} == {});", spec_answer.len());
        // The returned length is asserted on every row, empty ones included, so it is
        // always available as this section's fault target.
        f.offer(
            test_fn,
            &len_line,
            format!("    assert({var}len{i} == {});", spec_answer.len() + 1),
        );
        writeln!(out, "{len_line}").unwrap();
        for (j, byte) in spec_answer.as_bytes().iter().enumerate() {
            writeln!(out, "    assert({var}out{i}[{j}] == {byte});").unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn gen_divide(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// op:numeric-divide on two xs:integer operands — the DECIMAL quotient").unwrap();
    writeln!(out, "/// (F&O 3.1 sec. 4.2.4 op:numeric-divide), implemented as the documented double").unwrap();
    writeln!(out, "/// approximation, and DISTINCT from op:numeric-integer-divide (sq-3x7dl.4").unwrap();
    writeln!(out, "/// de-aliasing). Oracle: SPARQL double division of the promoted operands,").unwrap();
    writeln!(out, "/// bit-cross-checked against native f64.").unwrap();
    writeln!(out, "///").unwrap();
    writeln!(out, "/// The `numeric_divide_int` (idiv) rows are the DE-ALIASING control and are NOT").unwrap();
    writeln!(out, "/// oracle-derived — SPARQL exposes no `idiv` builtin, so their reference is Rust's").unwrap();
    writeln!(out, "/// integer division, which truncates toward zero exactly as F&O 3.1 sec. 4.2.5").unwrap();
    writeln!(out, "/// op:numeric-integer-divide requires.").unwrap();
    writeln!(out, "#[test]").unwrap();
    writeln!(out, "fn differential_oracle_numeric_divide() {{").unwrap();
    for &(a, b) in DIVIDE_CORPUS {
        assert_ne!(b, 0, "the circuit fail-closes on a zero divisor; that is a should_fail case, not a value case");
        let expr = format!("xsd:double(\"{a}\") / xsd:double(\"{b}\")");
        let bits = oracle_double_bits(g, &expr, (a as f64) / (b as f64));
        writeln!(out, "    // {expr} -> bits {}", hex64(bits)).unwrap();
        let line =
            format!("    assert(numeric_divide_int_as_double({a}, {b}).to_bits() == {});", hex64(bits));
        // Flip the low mantissa bit: still a valid double literal, never the right one.
        f.offer(
            "differential_oracle_numeric_divide",
            &line,
            format!(
                "    assert(numeric_divide_int_as_double({a}, {b}).to_bits() == {});",
                hex64(bits ^ 1)
            ),
        );
        writeln!(out, "{line}").unwrap();
        c.assertions += 1;
        // idiv MUST stay a distinct, truncating-toward-zero path.
        writeln!(out, "    assert(numeric_divide_int({a}, {b}) == {});", a / b).unwrap();
        c.assertions += 1;
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn gen_round(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// fn:round for xs:double — ties toward +INFINITY (F&O 3.1 sec. 4.4.4 fn:round), sign").unwrap();
    writeln!(out, "/// of zero preserved. Oracle: SPARQL `ROUND`, bit-cross-checked against the F&O").unwrap();
    writeln!(out, "/// reference. The -0.5 row is SPEC-REFERENCE (see the header): sparq-engine loses the").unwrap();
    writeln!(out, "/// sign of a zero result, so its expected value is F&O's and it asserts noir_XPath").unwrap();
    writeln!(out, "/// against the SPEC.").unwrap();
    writeln!(out, "#[test]").unwrap();
    writeln!(out, "fn differential_oracle_round_double() {{").unwrap();
    for lex in ROUND_CORPUS {
        let input: f64 = lex.parse().unwrap();
        let spec = fo_round_double(input);
        let expr = format!("ROUND(xsd:double(\"{lex}\"))");
        let sparq_bits = {
            let t = oracle_term(g, &expr).unwrap_or_else(|| panic!("oracle `{expr}` errored"));
            lexical(&t, "double", &expr).parse::<f64>().unwrap().to_bits()
        };
        let diverges = sparq_bits != spec.to_bits();
        // Only the recorded negative-zero case may diverge.
        assert!(
            !diverges || (spec == 0.0 && spec.is_sign_negative()),
            "UNRECORDED oracle divergence on `{expr}`: sparq-engine bits 0x{sparq_bits:016x} vs \
             F&O bits 0x{:016x}. Investigate before pinning.",
            spec.to_bits()
        );
        if diverges {
            writeln!(out, "    // SPEC-REFERENCE: sparq-engine returns +0.0 (bits 0x{sparq_bits:016x}); F&O 3.1").unwrap();
            writeln!(out, "    // sec. 4.4.4 fn:round requires NEGATIVE zero for an argument in [-0.5, 0). The").unwrap();
            writeln!(out, "    // row asserts the SPEC value LIVE — it holds noir_XPath to F&O, not to sparq.").unwrap();
            writeln!(out, "    // Drop the special-casing (not the assertion) when the engine is fixed.").unwrap();
            c.spec_reference += 1;
        }
        c.assertions += 1;
        writeln!(out, "    // {expr} -> {spec:?}").unwrap();
        let line = format!(
            "    assert(round_double(XsdDouble::from_bits({})).to_bits() == {});",
            hex64(input.to_bits()),
            hex64(spec.to_bits())
        );
        f.offer(
            "differential_oracle_round_double",
            &line,
            format!(
                "    assert(round_double(XsdDouble::from_bits({})).to_bits() == {});",
                hex64(input.to_bits()),
                hex64(spec.to_bits() ^ 1)
            ),
        );
        writeln!(out, "{line}").unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn gen_mixed_compare(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// Mixed xs:integer <-> xs:double comparison. Every integer here is OUTSIDE the i8").unwrap();
    writeln!(out, "/// range the pre-sq-3x7dl.5 circuit truncated to (which made `256 = 0.0` TRUE — a").unwrap();
    writeln!(out, "/// wrong FILTER verdict an adversarial prover could satisfy). Oracle: SPARQL").unwrap();
    writeln!(out, "/// relational operators, which promote the integer to double per XPath B.1.").unwrap();
    writeln!(out, "#[test]").unwrap();
    writeln!(out, "fn differential_oracle_mixed_compare() {{").unwrap();
    for (i, &(n, d)) in MIXED_CORPUS.iter().enumerate() {
        let dbits = {
            let expr = format!("xsd:double(\"{d}\")");
            oracle_double_bits(g, &expr, d.parse::<f64>().unwrap())
        };
        writeln!(out, "    let d{i} = XsdDouble::from_bits({});", hex64(dbits)).unwrap();
        // int OP double, then double OP int — both operand orders.
        for (nfn, op) in [("eq", "="), ("lt", "<"), ("gt", ">"), ("le", "<="), ("ge", ">=")] {
            let fwd = oracle_bool(g, &format!("xsd:integer(\"{n}\") {op} xsd:double(\"{d}\")"));
            writeln!(out, "    // {n} {op} {d} -> {fwd}").unwrap();
            let line = format!("    assert(compare_int_double_{nfn}({n}, d{i}) == {fwd});");
            f.offer(
                "differential_oracle_mixed_compare",
                &line,
                format!("    assert(compare_int_double_{nfn}({n}, d{i}) == {});", !fwd),
            );
            writeln!(out, "{line}").unwrap();
            c.assertions += 1;

            let rev = oracle_bool(g, &format!("xsd:double(\"{d}\") {op} xsd:integer(\"{n}\")"));
            writeln!(out, "    // {d} {op} {n} -> {rev}").unwrap();
            writeln!(out, "    assert(compare_double_int_{nfn}(d{i}, {n}) == {rev});").unwrap();
            c.assertions += 1;
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn gen_cast(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// xs:integer(xs:double) — TRUNCATES toward zero (F&O 3.1 sec. 19.1.2.4 \"Casting to").unwrap();
    writeln!(out, "/// xs:integer\"); it does NOT round.").unwrap();
    writeln!(out, "/// Oracle: SPARQL `xsd:integer(xsd:double(...))`.").unwrap();
    writeln!(out, "#[test]").unwrap();
    writeln!(out, "fn differential_oracle_double_to_integer_cast() {{").unwrap();
    for lex in CAST_CORPUS {
        let input: f64 = lex.parse().unwrap();
        let expr = format!("xsd:integer(xsd:double(\"{lex}\"))");
        let expected = oracle_int(g, &expr);
        assert_eq!(
            expected,
            input.trunc() as i64,
            "IEEE CROSS-CHECK FAILED for `{expr}`: sparq-engine says {expected}, truncation says {}",
            input.trunc()
        );
        writeln!(out, "    // {expr} -> {expected}").unwrap();
        let line = format!(
            "    assert(cast_double_to_integer(XsdDouble::from_bits({})).unwrap() == {expected});",
            hex64(input.to_bits())
        );
        f.offer(
            "differential_oracle_double_to_integer_cast",
            &line,
            format!(
                "    assert(cast_double_to_integer(XsdDouble::from_bits({})).unwrap() == {});",
                hex64(input.to_bits()),
                expected + 1
            ),
        );
        writeln!(out, "{line}").unwrap();
        c.assertions += 1;
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn gen_datetime(out: &mut String, g: &Graph, c: &mut Counts, f: &mut Faults) {
    writeln!(out, "/// Pre-1970 xs:dateTime. The epoch is a SIGNED microsecond count; the pre-sq-3x7dl.7").unwrap();
    writeln!(out, "/// circuit cast it through u64 and corrupted every extracted component. Oracle:").unwrap();
    writeln!(out, "/// SPARQL YEAR/MONTH/DAY/HOURS/MINUTES/SECONDS over the parsed lexical.").unwrap();
    writeln!(out, "#[test]").unwrap();
    writeln!(out, "fn differential_oracle_datetime_pre_1970() {{").unwrap();
    for (i, &(lex, year, month, day, hour, minute, second)) in DATETIME_CORPUS.iter().enumerate() {
        let dt = format!("xsd:dateTime(\"{lex}\")");
        // The corpus components are declared, then CONFIRMED against the oracle: the
        // circuit is fed the components and must reproduce what the evaluator extracts.
        let checks: [(&str, &str, i64); 6] = [
            ("YEAR", "year_from_datetime", year),
            ("MONTH", "month_from_datetime", month as i64),
            ("DAY", "day_from_datetime", day as i64),
            ("HOURS", "hours_from_datetime", hour as i64),
            ("MINUTES", "minutes_from_datetime", minute as i64),
            ("SECONDS", "seconds_from_datetime", second as i64),
        ];
        writeln!(out, "    // {dt}").unwrap();
        writeln!(
            out,
            "    let dt{i} = datetime_from_components({year}, {month}, {day}, {hour}, {minute}, {second}, 0);"
        )
        .unwrap();
        for (builtin, nfn, declared) in checks {
            // SECONDS is xs:decimal in SPARQL; the corpus keeps whole seconds so the
            // integer-valued comparison below is exact.
            let expr = format!("{builtin}({dt})");
            let got = if builtin == "SECONDS" {
                let t = oracle_term(g, &expr).unwrap_or_else(|| panic!("oracle `{expr}` errored"));
                lexical(&t, "decimal", &expr).parse::<f64>().unwrap() as i64
            } else {
                oracle_int(g, &expr)
            };
            assert_eq!(got, declared, "corpus component for `{expr}` disagrees with the oracle");
            let line = format!("    assert({nfn}(dt{i}) == {got});");
            f.offer(
                "differential_oracle_datetime_pre_1970",
                &line,
                format!("    assert({nfn}(dt{i}) == {});", got + 1),
            );
            writeln!(out, "{line}").unwrap();
            c.assertions += 1;
        }
    }
    // Signed ordering across the epoch boundary — the other half of sq-3x7dl.7.
    writeln!(out, "    // Signed ordering across the 1970 epoch boundary.").unwrap();
    let ordered = oracle_bool(
        g,
        "xsd:dateTime(\"1969-07-20T20:17:40Z\") < xsd:dateTime(\"1970-01-01T00:00:00Z\")",
    );
    writeln!(out, "    assert(datetime_less_than(dt0, dt3) == {ordered});").unwrap();
    c.assertions += 1;
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Build the complete Noir test file, corrupting the sites `mode` selects.
fn generate_noir_file(mode: &FaultMode) -> (String, Counts) {
    let g = oracle_graph();
    let mut out = String::with_capacity(64 * 1024);
    let mut counts = Counts::default();
    let mut faults = Faults::default();

    header(&mut out, mode);
    gen_string_length(&mut out, &g, &mut counts, &mut faults);
    gen_string_predicates(&mut out, &g, &mut counts, &mut faults);
    gen_substring(&mut out, &g, &mut counts, &mut faults);
    gen_substring_multibyte(&mut out, &g, &mut counts, &mut faults);
    gen_divide(&mut out, &g, &mut counts, &mut faults);
    gen_round(&mut out, &g, &mut counts, &mut faults);
    gen_mixed_compare(&mut out, &g, &mut counts, &mut faults);
    gen_cast(&mut out, &g, &mut counts, &mut faults);
    gen_datetime(&mut out, &g, &mut counts, &mut faults);

    writeln!(
        out,
        "// Coverage: {} live assertions, {} of them SPEC-REFERENCE rows whose expected value is\n\
         // XPath F&O's rather than the oracle's (recorded oracle divergences; 0 commented out).",
        counts.assertions, counts.spec_reference
    )
    .unwrap();

    // noirc rejects non-ASCII inside comments (but not inside string literals), so the
    // comment prose is transliterated before the file is handed to nargo. Runs BEFORE
    // fault injection so the site match below is against the final text.
    out = asciify_comments(&out);

    // EVERY generated `#[test]` must have offered a site, or the self-test would silently
    // stop covering a whole section — exactly the vacuity it exists to rule out.
    let emitted = emitted_test_fns(&out);
    for name in &emitted {
        assert!(
            faults.sites.iter().any(|s| &s.test_fn == name),
            "generated test `{name}` offered no fault site: it could never be proved non-vacuous"
        );
    }
    assert_eq!(
        faults.sites.len(),
        emitted.len(),
        "a fault site was registered for a test function that is not in the generated file"
    );

    let selected: Vec<&FaultSite> = match mode {
        FaultMode::None => Vec::new(),
        FaultMode::All => faults.sites.iter().collect(),
        // An unknown name is a hard error, never an uncorrupted file: the driver script
        // reads `nargo test` PASSING as "this test function is vacuous", so silently
        // injecting nothing would report the wrong verdict from the wrong evidence.
        FaultMode::Only(name) => {
            let site = faults.sites.iter().find(|s| &s.test_fn == name).unwrap_or_else(|| {
                let known: Vec<&str> = faults.sites.iter().map(|s| s.test_fn.as_str()).collect();
                panic!("no fault site for test `{}` — known sites: {}", name, known.join(", "))
            });
            vec![site]
        }
    };

    for site in selected {
        assert_eq!(
            out.matches(&site.original).count(),
            1,
            "fault site for {} is not a unique line; injection would corrupt the wrong row",
            site.test_fn
        );
        out = out.replacen(&site.original, &site.faulty, 1);
        eprintln!(
            "inject-fault: corrupted one expected value in {}\n  was: {}\n  now: {}",
            site.test_fn,
            site.original.trim(),
            site.faulty.trim()
        );
    }

    (out, counts)
}

/// The generated `#[test]`s that carry a fault site, in file order — the list the driver
/// script loops over. Taking it from the generator (rather than hard-coding it in the
/// script) means a renamed or dropped section changes the loop instead of quietly
/// shrinking the set of test functions the self-test proves anything about.
fn fault_site_names() -> Vec<String> {
    let (src, _) = generate_noir_file(&FaultMode::None);
    emitted_test_fns(&src)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

const USAGE: &str = "usage: sparq-xpath-differential (--output PATH | --inject-fault PATH \
                     | --inject-fault-in TEST_FN PATH | --list-fault-sites)";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut path: Option<String> = None;
    let mut mode = FaultMode::None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                path = Some(args.get(i).expect("--output needs a PATH").clone());
            }
            "--inject-fault" => {
                i += 1;
                mode = FaultMode::All;
                path = Some(args.get(i).expect("--inject-fault needs a PATH").clone());
            }
            "--inject-fault-in" => {
                i += 1;
                let name = args.get(i).expect("--inject-fault-in needs TEST_FN PATH").clone();
                mode = FaultMode::Only(name);
                i += 1;
                path = Some(args.get(i).expect("--inject-fault-in needs TEST_FN PATH").clone());
            }
            // The driver script's loop bound: printed to stdout, one name per line, so the
            // per-test-function self-test covers exactly what the generator emitted.
            "--list-fault-sites" => {
                for name in fault_site_names() {
                    println!("{}", name);
                }
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("{}", USAGE);
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let path = path.unwrap_or_else(|| {
        eprintln!("{}", USAGE);
        std::process::exit(2);
    });

    let (content, counts) = generate_noir_file(&mode);
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent).expect("create output directory");
    }
    std::fs::write(&path, &content).unwrap_or_else(|e| panic!("write {path}: {e}"));
    eprintln!(
        "wrote {path}: {} live assertions, {} of them SPEC-REFERENCE (recorded oracle divergences)",
        counts.assertions, counts.spec_reference
    );
}

// ---------------------------------------------------------------------------
// Unit tests — the guards that keep the harness honest
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The F&O window reference, pinned against the spec's own worked examples.
    #[test]
    fn fo_substring_matches_the_spec_examples() {
        assert_eq!(fo_substring("motor car", 6, 3), " ca");
        assert_eq!(fo_substring("metadata", 4, 3), "ada");
        assert_eq!(fo_substring("12345", 0, 3), "12");
        assert_eq!(fo_substring("12345", -3, 5), "1");
        assert_eq!(fo_substring("12345", 5, -3), "");
        assert_eq!(fo_substring("hello", -100, 3), "");
        // Codepoint-indexed, not byte-indexed.
        assert_eq!(fo_substring("naïve", 3, 1), "ï");
    }

    /// The `sq-hjvte` boundary conversion is EXACTLY the F&O window re-expressed in byte
    /// units: taking `length` bytes from `start` (1-based, in bytes) out of the SAME string
    /// must reproduce [`fo_substring`] byte-for-byte. Checked over the whole multibyte
    /// corpus AND over every window in a small exhaustive sweep, so an off-by-one at a
    /// codepoint boundary cannot hide behind a hand-picked case.
    ///
    /// This is the property the generated multibyte rows rest on: it is what makes
    /// "call the BYTE-positional primitive with the converted window" equivalent to
    /// "call a CODEPOINT-positional primitive with the original window".
    #[test]
    fn codepoint_window_to_byte_window_reproduces_the_fo_window() {
        // Taking the converted window out of `s` — the byte-positional read the circuit
        // performs, modelled here so the equivalence is asserted rather than assumed.
        fn byte_window(s: &str, start: i64, length: i64) -> &[u8] {
            let lo = (start - 1) as usize;
            &s.as_bytes()[lo..lo + length as usize]
        }

        let mut checked = 0usize;
        let mut multibyte_windows = 0usize;
        for &(value, _, start, length) in substring_multibyte_corpus().iter() {
            let (b_start, b_len) = codepoint_window_to_byte_window(value, start, length);
            assert!(b_start >= 1 && b_len >= 0, "the converted window must be normalized");
            assert_eq!(
                byte_window(value, b_start, b_len),
                fo_substring(value, start, length).as_bytes(),
                "conversion diverges from the F&O window for ({value:?}, {start}, {length})"
            );
            checked += 1;
        }
        assert!(checked >= 10, "the multibyte corpus shrank to {checked} rows");

        // Exhaustive sweep over every window a small corpus admits, negative starts and
        // lengths included. `fo_substring` is the reference; the conversion must never
        // disagree with it, at any offset, on any string.
        for value in ["naïve", "日本語", "𝄞a", "e\u{301}x", "ﬀ", "aé日", "ascii", ""] {
            for start in -4i64..=8 {
                for length in -2i64..=8 {
                    let (b_start, b_len) = codepoint_window_to_byte_window(value, start, length);
                    assert_eq!(
                        byte_window(value, b_start, b_len),
                        fo_substring(value, start, length).as_bytes(),
                        "conversion diverges for ({value:?}, {start}, {length})"
                    );
                    if !value.is_ascii() && b_len > 0 {
                        multibyte_windows += 1;
                    }
                }
            }
        }
        // A sweep that never produced a non-empty window on multibyte content would pass
        // while proving nothing.
        assert!(multibyte_windows > 100, "the sweep hit only {multibyte_windows} live windows");
    }

    /// THIRD, out-of-repo reference for the CODEPOINT window: **CPython string slicing**.
    ///
    /// [`fo_substring`] is this file's F&O reference and sparq-engine is the oracle — both
    /// live in this repository, so agreeing with each other is weaker evidence than it
    /// looks. CPython's `str` is a sequence of CODEPOINTS, so `s[start-1 : start+length-1]`
    /// (both ends floored at 0) is an independent statement of the same window, written by
    /// someone else, in another language. Every substring case the generator emits — ASCII
    /// and multibyte — is checked against it, and a disagreement FAILS: it would mean the
    /// reference the multibyte rows are pinned to is itself wrong.
    ///
    /// If `python3` is absent the check cannot run. That is reported loudly and skipped
    /// rather than quietly passed — on such a box this test is not evidence of anything.
    #[test]
    fn fo_substring_agrees_with_cpython_slicing() {
        let cases: Vec<(&str, i64, i64)> = substring_corpus()
            .iter()
            .chain(substring_multibyte_corpus().iter())
            .map(|&(v, _, start, length)| (v, start, length))
            .collect();

        // Every character goes out as an explicit \U000xxxxx escape, so the program text
        // is pure ASCII and no quoting, encoding or normalization step sits between this
        // corpus and CPython's view of it.
        let mut prog = String::from("cases = [\n");
        for &(value, start, length) in &cases {
            prog.push_str("    (\"");
            for ch in value.chars() {
                prog.push_str(&format!("\\U{:08X}", ch as u32));
            }
            prog.push_str(&format!("\", {start}, {length}),\n"));
        }
        prog.push_str(
            "]\n\
             for s, start, length in cases:\n\
             \x20   # XPath F&O 3.1 sec. 5.4.3 fn:substring: the 1-based window\n\
             \x20   # [start, start + length) over CODEPOINTS, as a Python slice.\n\
             \x20   out = s[max(start - 1, 0):max(start + length - 1, 0)]\n\
             \x20   print(','.join(str(ord(ch)) for ch in out))\n",
        );

        let run = std::process::Command::new("python3").arg("-c").arg(&prog).output();
        let out = match run {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!(
                    "SKIPPED fo_substring_agrees_with_cpython_slicing: python3 is not installed, \
                     so the CPython cross-check verified NOTHING on this machine."
                );
                return;
            }
            Err(e) => panic!("running python3 failed: {e}"),
        };
        assert!(
            out.status.success(),
            "the CPython reference program failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let stdout = String::from_utf8(out.stdout).expect("python3 emitted non-UTF-8");
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), cases.len(), "CPython returned the wrong number of answers");
        for ((value, start, length), line) in cases.iter().zip(lines) {
            let ours: String = fo_substring(value, *start, *length)
                .chars()
                .map(|c| (c as u32).to_string())
                .collect::<Vec<_>>()
                .join(",");
            assert_eq!(
                line, ours,
                "CPython slicing and the F&O reference disagree on ({value:?}, {start}, {length}): \
                 python {line:?} vs harness {ours:?}"
            );
        }
    }

    /// `fn:round` ties go toward +INFINITY and a negative argument rounding to zero keeps
    /// its sign — the two properties the circuit's `round_double` implements.
    #[test]
    fn fo_round_ties_toward_positive_infinity() {
        assert_eq!(fo_round_double(2.5), 3.0);
        assert_eq!(fo_round_double(-2.5), -2.0);
        assert_eq!(fo_round_double(0.5), 1.0);
        assert_eq!(fo_round_double(-1.5), -1.0);
        assert_eq!(fo_round_double(2.4999), 2.0);
        assert!(fo_round_double(-0.5).is_sign_negative() && fo_round_double(-0.5) == 0.0);
        assert!(fo_round_double(f64::NAN).is_nan());
    }

    /// The two circuit edges `noir_XPath` FIXED — the F&O window for `start < 1`
    /// (`sq-3x7dl.6`) and negative zero out of `fn:round` — must reach the circuit as LIVE
    /// assertions. They are the cases the oracle gets wrong, so they are asserted against
    /// the F&O reference instead; if they were ever emitted commented out, a `noir_XPath`
    /// regression on an edge it advertises as fixed would go completely undetected while
    /// every other check stayed green. This test is that guard.
    #[test]
    fn spec_reference_edges_reach_the_circuit_as_live_assertions() {
        let (src, counts) = generate_noir_file(&FaultMode::None);

        // No assertion may be emitted commented out, in any section.
        let smothered: Vec<&str> =
            src.lines().map(str::trim).filter(|l| l.starts_with("// assert(")).collect();
        assert!(
            smothered.is_empty(),
            "assertions emitted COMMENTED OUT (they cannot fail, so they verify nothing): {:#?}",
            smothered
        );
        assert!(counts.spec_reference > 0, "no SPEC-REFERENCE row was emitted at all");

        let live: Vec<&str> =
            src.lines().map(str::trim).filter(|l| l.starts_with("assert(")).collect();
        let require = |want: &str| {
            assert!(live.contains(&want), "missing LIVE assertion `{}`", want);
        };

        // substring("12345", 0, 3) == "12" — the F&O window, NOT the oracle's "123".
        // Located by corpus content so a corpus edit cannot silently orphan this guard.
        let i = substring_corpus()
            .iter()
            .position(|&(v, _, start, len)| (v, start, len) == ("12345", 0, 3))
            .expect("the substring corpus must keep the (\"12345\", 0, 3) F&O window case");
        require(&format!("assert(len{} == 2);", i));
        require(&format!("assert(out{}[0] == 49);", i)); // b'1'
        require(&format!("assert(out{}[1] == 50);", i)); // b'2'

        // round_double(-0.5) == -0.0 — the F&O sign of zero, NOT the oracle's +0.0. Built
        // with the generator's own renderers so the two cannot drift apart.
        assert!(ROUND_CORPUS.contains(&"-0.5"), "the round corpus must keep the -0.5 case");
        require(&format!(
            "assert(round_double(XsdDouble::from_bits({})).to_bits() == {});",
            hex64((-0.5f64).to_bits()),
            hex64((-0.0f64).to_bits())
        ));
    }

    /// SELF-EXPIRING GUARD #1. `DIVERGENCES[0]` claims sparq-engine shifts the
    /// `fn:substring` window when `start < 1`. When that engine bug is fixed this test
    /// goes RED — the signal to delete the entry and stop special-casing those rows (the
    /// assertions themselves already run; they just stop needing the F&O expected value).
    #[test]
    fn recorded_divergence_substring_start_below_one_still_reproduces() {
        let g = oracle_graph();
        assert_eq!(oracle_plain_string(&g, "SUBSTR(\"12345\", 0, 3)"), "123");
        assert_eq!(fo_substring("12345", 0, 3), "12");
        assert_eq!(oracle_plain_string(&g, "SUBSTR(\"hello\", -2, 4)"), "hell");
        assert_eq!(fo_substring("hello", -2, 4), "h");
    }

    /// SELF-EXPIRING GUARD #2. `DIVERGENCES[1]` claims sparq-engine's `ROUND` loses the
    /// sign of a negative zero result. Goes RED when the engine is fixed, at which point
    /// the row stops being SPEC-REFERENCE and becomes an ordinary oracle-derived one.
    #[test]
    fn recorded_divergence_round_negative_zero_still_reproduces() {
        let g = oracle_graph();
        let t = oracle_term(&g, "ROUND(xsd:double(\"-0.5\"))").expect("ROUND must not error");
        let bits = lexical(&t, "double", "ROUND").parse::<f64>().unwrap().to_bits();
        assert_eq!(bits, 0.0f64.to_bits(), "engine now returns something other than +0.0");
        assert_eq!(fo_round_double(-0.5).to_bits(), (-0.0f64).to_bits());
        // The serializer itself is NOT the culprit — it renders -0.0 correctly.
        assert!(oracle_term(&g, "xsd:double(\"-0.0\")").unwrap().starts_with("\"-0E0\""));
    }

    /// Every recorded divergence must carry all four fields — an empty `why` would make
    /// the generated header's honesty claim hollow.
    #[test]
    fn divergences_are_fully_documented() {
        assert!(!DIVERGENCES.is_empty());
        for d in DIVERGENCES {
            assert!(!d.expr.is_empty() && !d.sparq.is_empty() && !d.spec.is_empty() && !d.why.is_empty());
        }
    }

    /// The corpus must actually exercise the edges the bead names, otherwise a green run
    /// proves nothing about them.
    #[test]
    fn corpus_covers_the_beaded_edges() {
        // Multibyte and NUL-padding in the string corpus.
        let corpus = string_corpus();
        assert!(corpus.iter().any(|s| !s.value.is_ascii()), "no multibyte STRLEN case");
        assert!(corpus.iter().any(|s| s.cap > s.value.len()), "no NUL-padded case");
        assert!(corpus.iter().any(|s| s.value.chars().count() == 1 && s.value.len() == 4), "no astral case");
        // start < 1 in the substring corpus (sq-3x7dl.6).
        assert!(substring_corpus().iter().any(|&(_, _, start, _)| start < 1));
        // Multibyte substring coverage through the boundary conversion (sq-hjvte). The
        // whole point of that section is content where the byte and codepoint units
        // DISAGREE, so a row whose window happens to be byte-identical proves nothing on
        // its own — require the corpus to carry rows where the converted window actually
        // moved, an astral codepoint, and a NUL-padded multibyte buffer.
        let multibyte = substring_multibyte_corpus();
        assert!(
            multibyte.iter().all(|&(v, _, _, _)| v.chars().count() != v.len()),
            "an ASCII row in the multibyte substring corpus covers nothing new"
        );
        assert!(
            multibyte
                .iter()
                .filter(|&&(v, _, start, length)| codepoint_window_to_byte_window(v, start, length)
                    != (start, length.max(0)))
                .count()
                >= 5,
            "too few rows where the byte window actually differs from the codepoint window"
        );
        assert!(
            multibyte.iter().any(|&(v, _, _, _)| v.chars().count() == 1 && v.len() == 4),
            "no astral substring case"
        );
        assert!(
            multibyte.iter().any(|&(v, cap, _, _)| cap > v.len()),
            "no NUL-padded multibyte substring case"
        );
        assert!(
            multibyte.iter().any(|&(_, _, start, _)| start < 1),
            "no start < 1 multibyte substring case"
        );
        // A non-exact quotient (sq-3x7dl.4).
        assert!(DIVIDE_CORPUS.iter().any(|&(a, b)| a % b != 0));
        // Every mixed-comparison integer is out of i8 range (sq-3x7dl.5).
        assert!(MIXED_CORPUS.iter().all(|&(n, _)| n < -128 || n > 127));
        // A pre-1970 dateTime (sq-3x7dl.7).
        assert!(DATETIME_CORPUS.iter().any(|&(_, y, ..)| y < 1970));
    }

    /// Generation is DETERMINISTIC: same oracle, same corpus, same bytes. The committed
    /// golden file's drift guard depends on this.
    #[test]
    fn generation_is_deterministic() {
        let (a, _) = generate_noir_file(&FaultMode::None);
        let (b, _) = generate_noir_file(&FaultMode::None);
        assert_eq!(a, b);
    }

    /// noirc hard-errors on a non-ASCII byte inside a comment, so the generated file's
    /// comments must be pure ASCII — while the multibyte STRING literals that are the whole
    /// point of the unicode corpus must survive untouched. Both halves are pinned here,
    /// because a violation only shows up in the (toolchain-gated) `nargo` leg otherwise.
    #[test]
    fn generated_comments_are_ascii_but_literals_keep_their_multibyte() {
        for mode in [
            FaultMode::None,
            FaultMode::All,
            FaultMode::Only("differential_oracle_substring".to_string()),
        ] {
            let (src, _) = generate_noir_file(&mode);
            for line in src.lines() {
                if line.trim_start().starts_with("//") {
                    assert!(line.is_ascii(), "noirc rejects this comment: {}", line);
                }
            }
            assert!(
                src.contains("= \"\u{65e5}\u{672c}\u{8a9e}"),
                "the multibyte corpus literal was mangled by the comment pass"
            );
        }
        // The transliteration itself: prose punctuation becomes its ASCII spelling, and an
        // arbitrary codepoint becomes an explicit escape rather than being dropped.
        assert_eq!(ascii_comment("// a \u{2014} b \u{00a7}5.4.3"), "// a -- b sec. 5.4.3");
        assert_eq!(ascii_comment("// STRLEN(\"\u{e9}\")"), "// STRLEN(\"\\u{e9}\")");
    }

    /// `--inject-fault` must change exactly one LIVE assertion in EVERY generated
    /// `#[test]`, and never a commented-out one — i.e. every test function has a fault
    /// site, so [`fault_site_names`] can hand the driver script a variant for each.
    ///
    /// This test is about the fault SITES, not about the non-vacuity claim: a corrupted
    /// assertion is only textual evidence that the line changed, not that the change
    /// reaches the circuit. The per-function claim is discharged by the script's
    /// per-variant runs, whose isolation is pinned here by
    /// [`inject_fault_in_corrupts_only_the_named_test`].
    #[test]
    fn inject_fault_corrupts_one_live_assertion_in_every_generated_test() {
        let (clean, counts) = generate_noir_file(&FaultMode::None);
        let (faulty, _) = generate_noir_file(&FaultMode::All);
        assert_ne!(clean, faulty);
        assert!(counts.assertions > 100, "corpus shrank unexpectedly: {}", counts.assertions);

        // Compare only LIVE assertions, grouped by test function: the fault-injected
        // header carries two extra warning lines, so a positional whole-file diff would
        // report every line.
        let (a, b) = (live_assertions_by_test(&clean), live_assertions_by_test(&faulty));
        assert_eq!(a.len(), 11, "the generated file must still carry 11 test functions");
        assert_eq!(
            a.iter().map(|(_, l)| l.len()).sum::<usize>(),
            counts.assertions,
            "every live assertion must be countable"
        );
        assert_eq!(
            a.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            b.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            "fault injection must not add, drop or reorder a test function"
        );

        for ((name, before), (_, after)) in a.iter().zip(b.iter()) {
            assert_eq!(
                before.len(),
                after.len(),
                "fault injection must not add or drop an assertion in {name}"
            );
            let differing = before.iter().zip(after.iter()).filter(|(x, y)| x != y).count();
            assert_eq!(differing, 1, "expected exactly one corrupted assertion in {name}");
        }
    }

    /// No fault may be hidden in a comment: a corrupted comment cannot fail `nargo test`,
    /// so the self-test would go green on the fault run and be read as "the harness is
    /// vacuous" — the wrong verdict, from the wrong evidence.
    #[test]
    fn every_fault_site_is_a_live_assertion() {
        let (faulty, _) = generate_noir_file(&FaultMode::All);
        let live: Vec<String> =
            live_assertions_by_test(&faulty).into_iter().flat_map(|(_, l)| l).collect();
        let (g, mut counts, mut faults) = (oracle_graph(), Counts::default(), Faults::default());
        let mut sink = String::new();
        gen_string_length(&mut sink, &g, &mut counts, &mut faults);
        gen_string_predicates(&mut sink, &g, &mut counts, &mut faults);
        gen_substring(&mut sink, &g, &mut counts, &mut faults);
        gen_substring_multibyte(&mut sink, &g, &mut counts, &mut faults);
        gen_divide(&mut sink, &g, &mut counts, &mut faults);
        gen_round(&mut sink, &g, &mut counts, &mut faults);
        gen_mixed_compare(&mut sink, &g, &mut counts, &mut faults);
        gen_cast(&mut sink, &g, &mut counts, &mut faults);
        gen_datetime(&mut sink, &g, &mut counts, &mut faults);

        assert_eq!(faults.sites.len(), 11, "one fault site per generated test function");
        for site in &faults.sites {
            assert!(
                site.original.trim_start().starts_with("assert("),
                "fault site for {} is not an assertion: {}",
                site.test_fn,
                site.original
            );
            assert!(
                live.contains(&site.faulty.trim().to_string()),
                "the corrupted line for {} never reached the generated file",
                site.test_fn
            );
        }
    }

    /// `--list-fault-sites` is the driver script's loop bound, so it must name EVERY
    /// generated `#[test]`. A name missing here is a test function the script would never
    /// build a variant for — it would run green forever with nothing proving it can go red.
    #[test]
    fn fault_site_names_lists_every_generated_test() {
        let (clean, _) = generate_noir_file(&FaultMode::None);
        let names = fault_site_names();
        assert!(!names.is_empty(), "an empty list makes the self-test loop a no-op");
        assert_eq!(names, emitted_test_fns(&clean), "the loop bound must be the emitted tests");
    }

    /// `--inject-fault-in NAME` must corrupt exactly one live assertion, inside NAME, and
    /// leave every other test function byte-identical to the oracle file.
    ///
    /// That isolation is what makes the script's per-variant run a proof about NAME: the
    /// oracle file has just run green, the variant differs from it in one expected value
    /// inside NAME, so a failing `nargo test` on the variant can only be NAME failing. An
    /// all-faults run cannot support that step — its nonzero exit says only that SOME test
    /// failed, which nine vacuous sections would satisfy just as well as ten live ones.
    #[test]
    fn inject_fault_in_corrupts_only_the_named_test() {
        let (clean, _) = generate_noir_file(&FaultMode::None);
        let baseline = live_assertions_by_test(&clean);
        let names = fault_site_names();

        for name in &names {
            let (variant, _) = generate_noir_file(&FaultMode::Only(name.clone()));
            let after = live_assertions_by_test(&variant);
            assert_eq!(
                baseline.iter().map(|(n, _)| n).collect::<Vec<_>>(),
                after.iter().map(|(n, _)| n).collect::<Vec<_>>(),
                "injecting into {} changed the set of test functions",
                name
            );

            for ((fn_name, before), (_, now)) in baseline.iter().zip(after.iter()) {
                assert_eq!(
                    before.len(),
                    now.len(),
                    "injecting into {} added or dropped an assertion in {}",
                    name,
                    fn_name
                );
                let differing = before.iter().zip(now.iter()).filter(|(x, y)| x != y).count();
                let expected = usize::from(fn_name == name);
                assert_eq!(
                    differing, expected,
                    "injecting into {} changed {} assertions in {}",
                    name, differing, fn_name
                );
            }
        }
    }

    /// A name the generator does not emit must be a hard error, never a silently clean
    /// file: the script reads a PASSING `nargo test` on a variant as "that test function
    /// is vacuous", so an uncorrupted variant would report a false failure of the harness.
    #[test]
    #[should_panic(expected = "no fault site for test")]
    fn inject_fault_in_rejects_an_unknown_test() {
        generate_noir_file(&FaultMode::Only("differential_oracle_not_a_test".to_string()));
    }
}
