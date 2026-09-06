//! [FABLE-5] sq-w42s3 (epic sq-hmd7l) — W3C JSON-LD 1.1 SELF-conformance
//! pass-rate SCOREBOARD runner for the NATIVE `sparq-jsonld` pipeline.
//!
//! The cross-engine conformance record
//! (`research/gap-conformance-cross-engine-2026-07.md`, bead sq-hmd7l.22) found
//! JSON-LD 1.1 **BEHIND** the peers' published EARL results. That table's peer
//! cells come from pinned published artifacts; the SPARQ row must come from a
//! durable, re-runnable self-scoreboard — this example. It runs the pinned
//! W3C JSON-LD 1.1 API + framing suites through the crate's PUBLIC native
//! entry points and emits a per-operation pass-rate scoreboard:
//!
//! * a human table (per-operation `pass / fail / skip / total` + truncated,
//!   never-rounded-up pass-rate),
//! * an HONEST per-test failure listing (every failing test id + reason —
//!   nothing is elided), and
//! * a machine-readable JSON document (`--json`) that
//!   `research/gap-jsonld-conformance-2026-07.md`, the cross-engine table and
//!   the site propagation consume as the sparq-side row.
//!
//! ## Operations covered (the native document-level pipeline)
//!
//! | operation | native entry                     | oracle                                        |
//! |-----------|----------------------------------|-----------------------------------------------|
//! | expand    | [`sparq_jsonld::expand`]         | `json_ld_equal` vs the normative expected doc |
//! | compact   | [`sparq_jsonld::compact::compact`] | `json_ld_equal` vs the normative expected doc |
//! | flatten   | [`sparq_jsonld::flatten`]        | `json_ld_equal` vs the normative expected doc |
//! | frame     | [`sparq_jsonld::frame::frame`]   | `json_ld_equal` + exact negative error codes  |
//! | fromRdf   | [`sparq_jsonld::from_rdf::from_rdf`] | `json_ld_equal` + exact negative error codes |
//! | toRdf     | — NOT IMPLEMENTED natively       | reported as an explicit gap row, never a fake |
//!
//! **toRdf honesty note:** `sparq_jsonld::to_rdf` is still the sq-oy1f.23
//! scaffold (the Deserialization-to-RDF algorithm lands with bead sq-oy1f.30).
//! The ENGINE's toRdf ingest (via `oxjsonld`) is ratcheted separately in
//! `sparq-conformance` (`floors::to_rdf`); this native scoreboard reports the
//! toRdf row as `implemented: false` rather than borrowing the engine number.
//!
//! ## Relationship to the `sparq-conformance` ratchet lane (no drift)
//!
//! The CI ratchet of record for these lanes is
//! `crates/sparq-conformance/tests/jsonld_suite.rs` asserting the LIB-SIDE
//! floors in `sparq_conformance::floors::<lane>` (sq-oy1f.40). This example
//! cannot import those consts (the dependency arrow points the other way:
//! sparq-conformance depends on sparq-jsonld), so the `*_FLOOR` consts below
//! are documented MIRRORS calibrated at the same suite pin; the full run
//! asserts `pass >= FLOOR` (floor semantics — a ratchet rise upstream never
//! breaks this runner, and a regression below the shared floor fails loudly).
//! The `fromRdf` oracle here is DOCUMENT-LEVEL ONLY (the sparq-conformance
//! lane additionally runs a scoped oxjsonld round-trip leg this dependency-free
//! example cannot express), so its floor is calibrated independently and
//! documented as the weaker oracle — see `FROMRDF_FLOOR`.
//!
//! ## Suite data (pinned, fetched, never committed)
//!
//! Fixtures are the two pinned W3C checkouts the repo's fetch scripts vendor
//! into the gitignored `tests/w3c/` tree:
//!
//! * `scripts/fetch-jsonld-tests.sh`        → `tests/w3c/json-ld-api`     (pin [`API_PIN`])
//! * `scripts/fetch-jsonld-framing-tests.sh`→ `tests/w3c/json-ld-framing` (pin [`FRAMING_PIN`])
//!
//! When a suite is absent the runner prints a SKIP notice and exits 0 (the
//! repo-wide offline-green convention) — it never fabricates a row.
//!
//! ## Usage
//!
//! ```text
//! cargo run -p sparq-jsonld --release --example jsonld_conformance             # full scoreboard
//! cargo run -p sparq-jsonld --release --example jsonld_conformance -- --json   # machine-readable only
//! cargo run -p sparq-jsonld --release --example jsonld_conformance -- --smoke  # pinned smoke subset
//! ```
//!
//! `--smoke` runs the FIRST [`SMOKE_N`] manifest entries of every operation
//! (deterministic at the suite pin) and asserts the EXACT pinned
//! pass/fail/skip counts in [`SMOKE_PINS`] — the bead's acceptance gate. A
//! count drift (suite bump, algorithm fix, oracle change) fails loudly so the
//! pins are re-calibrated deliberately, never silently.
//!
//! INVARIANT (bead sq-w42s3): every reported rate comes ONLY from this
//! runner's output over the pinned suites; every failing test is listed;
//! pass-rates are truncated (never rounded up); the BEHIND verdict is
//! preserved honestly.

use sparq_jsonld::frame::{frame as jsonld_frame, FrameOptions};
use sparq_jsonld::from_rdf::{from_rdf, FromRdfOptions, RdfQuad, RdfTerm};
use sparq_jsonld::{
    compact::compact as jsonld_compact, expand as jsonld_expand, flatten as jsonld_flatten,
    FsLoader, Json, JsonLdError, JsonLdOptions, NoopLoader, ProcessingMode, RdfDirection,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Pinned `w3c/json-ld-api` commit — MUST match `scripts/fetch-jsonld-tests.sh`.
const API_PIN: &str = "8654ac22b6cf4f441d2fee915ae634d36b5a8067";
/// Pinned `w3c/json-ld-framing` commit — MUST match `scripts/fetch-jsonld-framing-tests.sh`.
const FRAMING_PIN: &str = "3bf782ba9a40dd1b143435abe386d38df64f2b47";

/// The json-ld-api suite's declared base IRI (resolves each test's input path).
const SUITE_BASE: &str = "https://w3c.github.io/json-ld-api/tests/";
/// The json-ld-framing suite's declared base IRI.
const FRAME_SUITE_BASE: &str = "https://w3c.github.io/json-ld-framing/tests/";

/// `--smoke` runs the first `SMOKE_N` manifest entries per operation.
const SMOKE_N: usize = 30;

// ── Full-run ratchet floors (pass >= FLOOR) ─────────────────────────────────
//
// Documented MIRRORS of `sparq_conformance::floors::<lane>::FLOOR` at suite pin
// 8654ac2 (see the module docs for why the const cannot be imported here). The
// assertion is `>=`, so an upstream ratchet RISE never breaks this runner.
const EXPAND_FLOOR: usize = 381; // mirrors floors::expand::FLOOR
const COMPACT_FLOOR: usize = 243; // mirrors floors::compact::FLOOR
const FLATTEN_FLOOR: usize = 53; // mirrors floors::flatten::FLOOR
const FRAME_FLOOR: usize = 92; // mirrors floors::frame::FLOOR
/// fromRdf floor for THIS runner's document-level-only oracle (calibrated at
/// the pin by this example, NOT a mirror: the sparq-conformance lane adds a
/// scoped oxjsonld round-trip leg, so its floor of record is the STRICTER
/// oracle — this document-level count could legitimately sit higher, and at
/// the current pin it MEASURES 52/53, coinciding with the ratchet lane's 52).
const FROMRDF_FLOOR: usize = 52;

/// Exact pinned (pass, fail, skip) per operation for `--smoke` (first
/// [`SMOKE_N`] manifest entries at the suite pins above). Calibrated by
/// running the smoke subset; a drift means the suite pin, an algorithm, or an
/// oracle changed — re-calibrate DELIBERATELY and say so in the PR.
const SMOKE_PINS: &[(&str, usize, usize, usize)] = &[
    ("expand", 30, 0, 0),
    ("compact", 30, 0, 0),
    ("flatten", 28, 0, 2),
    ("frame", 30, 0, 0),
    ("fromRdf", 29, 0, 1),
];

// ── Scoreboard plumbing ──────────────────────────────────────────────────────

/// One operation's tally. Every failure carries its test id + reason so the
/// listing is complete (the honesty invariant: never elide a failing test).
#[derive(Default)]
struct Score {
    pass: usize,
    fail: usize,
    skip: usize,
    failures: Vec<(String, String)>,
}

impl Score {
    fn pass(&mut self) {
        self.pass += 1;
    }
    fn fail(&mut self, id: &str, why: String) {
        self.fail += 1;
        self.failures.push((id.to_string(), why));
    }
    fn skip(&mut self) {
        self.skip += 1;
    }
    fn total(&self) -> usize {
        self.pass + self.fail + self.skip
    }
    /// Pass-rate over ALL manifest entries, ×10 and TRUNCATED (never rounded
    /// up): `716` means 71.6%.
    fn pass_rate_tenths(&self) -> usize {
        if self.total() == 0 {
            0
        } else {
            self.pass * 1000 / self.total()
        }
    }
}

/// Renders a tenths-of-a-percent value as `NN.N` (truncation already applied).
fn fmt_tenths(t: usize) -> String {
    format!("{}.{}", t / 10, t % 10)
}

// ── Json helpers (accessors over the crate's own AST — no serde) ────────────

fn as_bool(j: &Json) -> Option<bool> {
    match j {
        Json::Raw(t) if t == "true" => Some(true),
        Json::Raw(t) if t == "false" => Some(false),
        _ => None,
    }
}

fn get_str(obj: &Json, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// `option.<key>` accessor.
fn opt_member<'a>(entry: &'a Json, key: &str) -> Option<&'a Json> {
    entry.get("option").and_then(|o| o.get(key))
}

// ── The JSON-LD document-level equality comparator ──────────────────────────
//
// A port of the pinned sq-kk1mq comparator (sparq-conformance's
// `json_ld_equal`) onto the crate's own `Json` AST:
//   • object key order insignificant;
//   • array order SIGNIFICANT only for the direct value of a "@list" key
//     (every other array is a multiset);
//   • numbers: integral tokens compared exactly (so 2^53 and 2^53+1 stay
//     distinct), mixed/non-integral fall back to f64 (so `1` ≡ `1.0`);
//   • strings / booleans / null: exact.

fn json_ld_equal(a: &Json, b: &Json) -> bool {
    json_ld_equal_inner(a, b, false)
}

fn json_ld_equal_inner(a: &Json, b: &Json, in_list: bool) -> bool {
    match (a, b) {
        (Json::Str(x), Json::Str(y)) => x == y,
        (Json::Raw(x), Json::Raw(y)) => raw_token_equal(x, y),
        (Json::Arr(xs), Json::Arr(ys)) => {
            if xs.len() != ys.len() {
                return false;
            }
            if in_list {
                xs.iter()
                    .zip(ys.iter())
                    .all(|(x, y)| json_ld_equal_inner(x, y, false))
            } else {
                json_ld_array_equal_unordered(xs, ys)
            }
        }
        (Json::Obj(xa), Json::Obj(ya)) => {
            if xa.len() != ya.len() {
                return false;
            }
            xa.iter()
                .all(|(k, va)| match ya.iter().find(|(ky, _)| ky == k) {
                    Some((_, vb)) => json_ld_equal_inner(va, vb, k == "@list"),
                    None => false,
                })
        }
        _ => false,
    }
}

/// Equality of two pre-rendered scalar tokens (`true` / `false` / `null` / a
/// JSON number) under the comparator's numeric rules.
fn raw_token_equal(x: &str, y: &str) -> bool {
    if x == y {
        return true; // identical tokens (covers true/false/null and equal numerals)
    }
    // Both integral (no '.', 'e', 'E'): exact big-integer comparison via i128
    // so 9007199254740992 ≢ 9007199254740993 (the f64-collapse guard).
    if let (Ok(xi), Ok(yi)) = (x.parse::<i128>(), y.parse::<i128>()) {
        return xi == yi;
    }
    // Mixed / non-integral numerals: f64 fallback so `1` ≡ `1.0` ≡ `1e0`.
    if let (Ok(xf), Ok(yf)) = (x.parse::<f64>(), y.parse::<f64>()) {
        return xf == yf;
    }
    false
}

/// Multiset equality for arrays outside `@list` (order-insensitive).
fn json_ld_array_equal_unordered(xs: &[Json], ys: &[Json]) -> bool {
    let mut used = vec![false; ys.len()];
    'outer: for x in xs {
        for (j, y) in ys.iter().enumerate() {
            if !used[j] && json_ld_equal_inner(x, y, false) {
                used[j] = true;
                continue 'outer;
            }
        }
        return false;
    }
    true
}

// ── Manifest reading ─────────────────────────────────────────────────────────

/// One manifest entry's salient fields (the cross-lane superset).
#[derive(Default)]
struct Entry {
    id: String,
    is_negative: bool,
    input: String,
    expect: Option<String>,
    expect_error_code: Option<String>,
    base_override: Option<String>,
    requires: Option<String>,
    context: Option<String>,
    spec_version: Option<String>,
    processing_mode: Option<String>,
    frame: Option<String>,
    expand_context_path: Option<String>,
    compact_arrays: Option<bool>,
    compact_to_relative: Option<bool>,
    omit_graph: Option<bool>,
    ordered: Option<bool>,
    use_native_types: bool,
    use_rdf_type: bool,
    rdf_direction: RdfDirection,
}

/// Reads `<cat>-manifest.jsonld` and returns its `sequence` entries.
fn read_manifest(root: &Path, cat: &str) -> Result<Vec<Entry>, String> {
    let path = root.join(format!("{cat}-manifest.jsonld"));
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let v = Json::parse(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let Some(Json::Arr(seq)) = v.get("sequence") else {
        return Err(format!(
            "{}: manifest has no sequence array",
            path.display()
        ));
    };
    let mut out = Vec::new();
    for e in seq {
        let types: Vec<&str> = match e.get("@type") {
            Some(Json::Arr(a)) => a.iter().filter_map(Json::as_str).collect(),
            Some(Json::Str(s)) => vec![s.as_str()],
            _ => vec![],
        };
        let rdf_direction = match opt_member(e, "rdfDirection").and_then(Json::as_str) {
            Some("i18n-datatype") => RdfDirection::I18nDatatype,
            Some("compound-literal") => RdfDirection::CompoundLiteral,
            _ => RdfDirection::None,
        };
        out.push(Entry {
            id: get_str(e, "@id").unwrap_or_else(|| "?".to_string()),
            is_negative: types.iter().any(|t| t.contains("NegativeEvaluationTest")),
            input: get_str(e, "input").unwrap_or_default(),
            expect: get_str(e, "expect"),
            expect_error_code: get_str(e, "expectErrorCode"),
            base_override: opt_member(e, "base")
                .and_then(Json::as_str)
                .map(str::to_string),
            requires: get_str(e, "requires"),
            context: get_str(e, "context"),
            spec_version: opt_member(e, "specVersion")
                .and_then(Json::as_str)
                .map(str::to_string),
            processing_mode: opt_member(e, "processingMode")
                .and_then(Json::as_str)
                .map(str::to_string),
            frame: get_str(e, "frame"),
            expand_context_path: opt_member(e, "expandContext")
                .and_then(Json::as_str)
                .map(str::to_string),
            compact_arrays: opt_member(e, "compactArrays").and_then(as_bool),
            compact_to_relative: opt_member(e, "compactToRelative").and_then(as_bool),
            omit_graph: opt_member(e, "omitGraph").and_then(as_bool),
            ordered: opt_member(e, "ordered").and_then(as_bool),
            use_native_types: opt_member(e, "useNativeTypes")
                .and_then(as_bool)
                .unwrap_or(false),
            use_rdf_type: opt_member(e, "useRdfType")
                .and_then(as_bool)
                .unwrap_or(false),
            rdf_direction,
        });
    }
    Ok(out)
}

fn processing_mode(e: &Entry) -> ProcessingMode {
    match (e.processing_mode.as_deref(), e.spec_version.as_deref()) {
        (Some("json-ld-1.0"), _) | (_, Some("json-ld-1.0")) => ProcessingMode::JsonLd10,
        _ => ProcessingMode::JsonLd11,
    }
}

/// The document IRI / base for a json-ld-api test.
fn doc_base(e: &Entry) -> String {
    e.base_override
        .clone()
        .unwrap_or_else(|| format!("{}{}", SUITE_BASE, e.input))
}

fn read_json(root: &Path, rel: &str) -> Result<Json, String> {
    let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
    Json::parse(&text).map_err(|e| format!("parse {rel}: {e}"))
}

fn is_remote(rel: &str) -> bool {
    rel.starts_with("http://") || rel.starts_with("https://")
}

// ── A minimal N-Quads reader (fromRdf inputs; dependency-free) ───────────────

/// Parses a well-formed W3C-suite N-Quads document into the crate's native
/// term model, PRESERVING blank-node labels (§8.1 is label-faithful).
fn parse_nquads(text: &str) -> Result<Vec<RdfQuad>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut c = Cursor {
            s: line.as_bytes(),
            i: 0,
            line: n + 1,
        };
        let subject = c.term()?;
        let predicate = c.term()?;
        let object = c.term()?;
        c.ws();
        let graph = if c.peek() == Some(b'.') {
            None
        } else {
            Some(c.term()?)
        };
        c.ws();
        if c.peek() != Some(b'.') {
            return Err(format!("line {}: missing terminating '.'", c.line));
        }
        out.push(RdfQuad::new(subject, predicate, object, graph));
    }
    Ok(out)
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.i += 1;
        }
    }
    fn err(&self, why: &str) -> String {
        format!("line {}: {}", self.line, why)
    }
    /// Reads one term: `<iri>`, `_:label`, or `"lex"(^^<dt> | @lang)?`.
    fn term(&mut self) -> Result<RdfTerm, String> {
        self.ws();
        match self.peek() {
            Some(b'<') => {
                self.i += 1;
                let raw = self.until(b'>')?;
                Ok(RdfTerm::iri(unescape(&raw, self.line)?))
            }
            Some(b'_') => {
                if self.s.get(self.i + 1) != Some(&b':') {
                    return Err(self.err("expected _:label"));
                }
                self.i += 2;
                let start = self.i;
                while self
                    .peek()
                    .is_some_and(|b| !matches!(b, b' ' | b'\t' | b'.'))
                {
                    self.i += 1;
                }
                let label = std::str::from_utf8(&self.s[start..self.i])
                    .map_err(|e| self.err(&e.to_string()))?;
                Ok(RdfTerm::blank(label))
            }
            Some(b'"') => {
                self.i += 1;
                let mut raw = String::new();
                loop {
                    match self.peek() {
                        Some(b'"') => {
                            self.i += 1;
                            break;
                        }
                        Some(b'\\') => {
                            raw.push(self.s[self.i] as char);
                            self.i += 1;
                            if let Some(b) = self.peek() {
                                raw.push(b as char);
                                self.i += 1;
                            }
                        }
                        Some(_) => {
                            // Copy the full UTF-8 sequence byte-faithfully.
                            let start = self.i;
                            self.i += 1;
                            while self.i < self.s.len() && (self.s[self.i] & 0xC0) == 0x80 {
                                self.i += 1;
                            }
                            raw.push_str(
                                std::str::from_utf8(&self.s[start..self.i])
                                    .map_err(|e| self.err(&e.to_string()))?,
                            );
                        }
                        None => return Err(self.err("unterminated literal")),
                    }
                }
                let lex = unescape(&raw, self.line)?;
                match self.peek() {
                    Some(b'@') => {
                        self.i += 1;
                        let start = self.i;
                        while self
                            .peek()
                            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-')
                        {
                            self.i += 1;
                        }
                        let lang = std::str::from_utf8(&self.s[start..self.i])
                            .map_err(|e| self.err(&e.to_string()))?;
                        Ok(RdfTerm::lang_literal(lex, lang))
                    }
                    Some(b'^') => {
                        if self.s.get(self.i + 1) != Some(&b'^')
                            || self.s.get(self.i + 2) != Some(&b'<')
                        {
                            return Err(self.err("expected ^^<datatype>"));
                        }
                        self.i += 3;
                        let dt = self.until(b'>')?;
                        Ok(RdfTerm::typed_literal(lex, unescape(&dt, self.line)?))
                    }
                    _ => Ok(RdfTerm::literal(lex)),
                }
            }
            _ => Err(self.err("expected a term")),
        }
    }
    /// Consumes up to (and including) the delimiter, returning the raw slice.
    fn until(&mut self, delim: u8) -> Result<String, String> {
        let start = self.i;
        while let Some(b) = self.peek() {
            if b == delim {
                let raw = std::str::from_utf8(&self.s[start..self.i])
                    .map_err(|e| self.err(&e.to_string()))?
                    .to_string();
                self.i += 1;
                return Ok(raw);
            }
            self.i += 1;
        }
        Err(self.err("unterminated token"))
    }
}

/// N-Triples string/IRI escape decoding (`\t \b \n \r \f \" \' \\ \uXXXX \UXXXXXXXX`).
fn unescape(raw: &str, line: usize) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut it = raw.chars();
    while let Some(ch) = it.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('b') => out.push('\u{8}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            Some(u @ ('u' | 'U')) => {
                let n = if u == 'u' { 4 } else { 8 };
                let hex: String = it.by_ref().take(n).collect();
                if hex.len() != n {
                    return Err(format!("line {line}: truncated \\{u} escape"));
                }
                let cp = u32::from_str_radix(&hex, 16)
                    .map_err(|e| format!("line {line}: bad \\{u} escape: {e}"))?;
                out.push(
                    char::from_u32(cp).ok_or(format!("line {line}: invalid code point {cp:#x}"))?,
                );
            }
            other => return Err(format!("line {line}: unknown escape \\{other:?}")),
        }
    }
    Ok(out)
}

// ── Per-operation lane runners (ported oracles; see the module docs) ─────────

/// `expand`: native `sparq_jsonld::expand()` vs the normative expected doc.
/// SKIPs (honest, per the ratchet lane): `requires` optional features, remote inputs,
/// no-expect.
///
/// [OPUS-5] sq-gzsky — NegativeEvaluationTests are RUN, not skipped (mirroring the ratchet
/// lane): the case passes iff `expand()` raises EXACTLY the manifest's `expectErrorCode`.
fn run_expand(root: &Path, entries: &[Entry]) -> Score {
    let mut s = Score::default();
    let loader = FsLoader::new().map_prefix(SUITE_BASE, root);
    for e in entries {
        if e.requires.is_some() || is_remote(&e.input) {
            s.skip();
            continue;
        }
        let is_negative = e.is_negative || e.expect_error_code.is_some();
        if !is_negative && e.expect.is_none() {
            s.skip();
            continue;
        }
        let input = match read_json(root, &e.input) {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, why);
                continue;
            }
        };
        let mut opts = JsonLdOptions::default();
        opts.base = Some(doc_base(e));
        opts.processing_mode = processing_mode(e);
        if let Some(ctx_rel) = &e.expand_context_path {
            match read_json(root, ctx_rel) {
                Ok(ctx) => opts.expand_context = Some(ctx),
                Err(why) => {
                    s.fail(&e.id, why);
                    continue;
                }
            }
        }
        let outcome = jsonld_expand(&input, &opts, &loader);
        if is_negative {
            check_expected_error(&mut s, e, outcome.as_ref().err());
            continue;
        }
        let expect_rel = e.expect.as_deref().expect("positive cases carry an expect doc");
        match outcome {
            Ok(got) => compare_expected(&mut s, root, e, expect_rel, &got, "expand"),
            Err(why) => s.fail(&e.id, format!("expand() error: {why}")),
        }
    }
    s
}

/// Score a NegativeEvaluationTest: it passes iff the algorithm raised EXACTLY the
/// manifest's `expectErrorCode`. A wrong code is a FAIL, not a pass — the error registry
/// is the machine-checkable part of the negative lane ([`sparq_jsonld::JsonLdErrorCode`]).
fn check_expected_error(s: &mut Score, e: &Entry, err: Option<&JsonLdError>) {
    let want = e.expect_error_code.as_deref().unwrap_or("");
    match err {
        Some(err) if err.code().as_str() == want => s.pass(),
        Some(err) => s.fail(
            &e.id,
            format!("expected error {want:?}, got {:?}", err.code().as_str()),
        ),
        None => s.fail(&e.id, format!("expected error {want:?}, got success")),
    }
}

/// `compact`: native Compaction Algorithm vs the normative expected doc.
/// ONE narrowly-pinned honest SKIP (bead sq-uzdw7): exactly `#t0038`, whose
/// `specVersion: json-ld-1.0` expectation (compact-IRI creation through an
/// expanded term definition) contradicts what `#tp001` pins for 1.0 processing
/// mode under the 1.1 REC — one implementation cannot pass both. The pin is by
/// exact manifest id (NOT a blanket `specVersion` match, mirroring the ratchet
/// lane's `is_pinned_1_0_only_skip` + its scope test): `processingMode:
/// json-ld-1.0` cases still RUN, and a future 1.0-only positive at a suite-pin
/// bump RUNS rather than being silently skipped.
fn run_compact(root: &Path, entries: &[Entry]) -> Score {
    let mut s = Score::default();
    let loader = FsLoader::new().map_prefix(SUITE_BASE, root);
    for e in entries {
        if e.requires.is_some()
            || is_remote(&e.input)
            || (e.id == "#t0038" && e.spec_version.as_deref() == Some("json-ld-1.0"))
        {
            s.skip();
            continue;
        }
        let is_negative = e.is_negative || e.expect_error_code.is_some();
        let Some(ctx_rel) = &e.context else {
            s.skip();
            continue;
        };
        if !is_negative && e.expect.is_none() {
            s.skip();
            continue;
        }
        let (input, ctx) = match (read_json(root, &e.input), read_json(root, ctx_rel)) {
            (Ok(i), Ok(c)) => (i, c),
            (Err(why), _) | (_, Err(why)) => {
                s.fail(&e.id, why);
                continue;
            }
        };
        let mut opts = JsonLdOptions::default();
        opts.base = Some(doc_base(e));
        opts.processing_mode = processing_mode(e);
        if let Some(ca) = e.compact_arrays {
            opts.compact_arrays = ca;
        }
        if let Some(cr) = e.compact_to_relative {
            opts.compact_to_relative = cr;
        }
        let outcome = jsonld_compact(&input, &ctx, &opts, &loader);
        if is_negative {
            check_expected_error(&mut s, e, outcome.as_ref().err());
            continue;
        }
        let expect_rel = e.expect.as_deref().expect("positive cases carry an expect doc");
        match outcome {
            Ok(got) => compare_expected(&mut s, root, e, expect_rel, &got, "compact"),
            Err(why) => s.fail(&e.id, format!("compact() error: {why}")),
        }
    }
    s
}

/// `flatten`: native `sparq_jsonld::flatten()` vs the normative expected doc.
/// Extra honest SKIPs (per the ratchet lane): JSON-LD-1.0-only positives and
/// compaction-`context` cases (composition tracked by the ratchet lane too).
fn run_flatten(root: &Path, entries: &[Entry]) -> Score {
    let mut s = Score::default();
    for e in entries {
        if e.requires.is_some()
            || e.is_negative
            || is_remote(&e.input)
            || e.spec_version.as_deref() == Some("json-ld-1.0")
            || e.processing_mode.as_deref() == Some("json-ld-1.0")
            || e.context.is_some()
        {
            s.skip();
            continue;
        }
        let Some(expect_rel) = &e.expect else {
            s.skip();
            continue;
        };
        let input = match read_json(root, &e.input) {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, why);
                continue;
            }
        };
        let mut opts = JsonLdOptions::default();
        opts.base = Some(doc_base(e));
        opts.processing_mode = processing_mode(e);
        match jsonld_flatten(&input, &opts, &NoopLoader) {
            Ok(got) => compare_expected(&mut s, root, e, expect_rel, &got, "flatten"),
            Err(why) => s.fail(&e.id, format!("flatten() error: {why}")),
        }
    }
    s
}

/// `frame`: native Framing Algorithm over the SEPARATE framing suite;
/// NegativeEvaluationTests are RUN (exact spec error code must be raised).
fn run_frame(root: &Path, entries: &[Entry]) -> Score {
    let mut s = Score::default();
    for e in entries {
        if e.requires.is_some() {
            s.skip();
            continue;
        }
        let Some(frame_rel) = &e.frame else {
            s.skip();
            continue;
        };
        if is_remote(&e.input) || is_remote(frame_rel) {
            s.skip();
            continue;
        }
        let (input, frame_doc) = match (read_json(root, &e.input), read_json(root, frame_rel)) {
            (Ok(i), Ok(f)) => (i, f),
            (Err(why), _) | (_, Err(why)) => {
                s.fail(&e.id, why);
                continue;
            }
        };
        let mut opts = JsonLdOptions::default();
        opts.base = Some(
            e.base_override
                .clone()
                .unwrap_or_else(|| format!("{}{}", FRAME_SUITE_BASE, e.input)),
        );
        opts.processing_mode = processing_mode(e);
        opts.ordered = e.ordered.unwrap_or(false);
        let mut fopts = FrameOptions::default();
        fopts.omit_graph = e.omit_graph;
        let framed = jsonld_frame(&input, &frame_doc, &opts, &fopts, &NoopLoader);
        if e.is_negative || e.expect_error_code.is_some() {
            let want = e.expect_error_code.as_deref().unwrap_or("");
            match &framed {
                Err(err) if err.code().as_str() == want => s.pass(),
                Err(err) => s.fail(
                    &e.id,
                    format!("expected error {want:?}, got {:?}", err.code().as_str()),
                ),
                Ok(_) => s.fail(&e.id, format!("expected error {want:?}, got success")),
            }
            continue;
        }
        let Some(expect_rel) = &e.expect else {
            s.skip();
            continue;
        };
        match framed {
            Ok(got) => compare_expected(&mut s, root, e, expect_rel, &got, "frame"),
            Err(why) => s.fail(&e.id, format!("frame() error: {why}")),
        }
    }
    s
}

/// `fromRdf`: native §8.1 serialization, DOCUMENT-LEVEL oracle (see the
/// module docs for the deliberate round-trip-leg difference vs the ratchet
/// lane). NegativeEvaluationTests are RUN (exact spec error code).
fn run_fromrdf(root: &Path, entries: &[Entry]) -> Score {
    let mut s = Score::default();
    for e in entries {
        if e.requires.is_some() || e.spec_version.as_deref() == Some("json-ld-1.0") {
            s.skip();
            continue;
        }
        let nq_text = match std::fs::read_to_string(root.join(&e.input)) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read input: {why}"));
                continue;
            }
        };
        let quads = match parse_nquads(&nq_text) {
            Ok(q) => q,
            Err(why) => {
                s.fail(&e.id, format!("parse input nq: {why}"));
                continue;
            }
        };
        let mut opts = FromRdfOptions::default();
        opts.use_native_types = e.use_native_types;
        opts.use_rdf_type = e.use_rdf_type;
        opts.rdf_direction = e.rdf_direction;
        opts.ordered = true;
        let outcome = from_rdf(&quads, &opts);
        if e.is_negative {
            match (&outcome, e.expect_error_code.as_deref()) {
                (Err(err), Some(want)) if err.code().as_str() == want => s.pass(),
                (Err(err), want) => s.fail(
                    &e.id,
                    format!("raised {:?}, expected {:?}", err.code().as_str(), want),
                ),
                (Ok(_), want) => s.fail(&e.id, format!("expected error {want:?}, got a document")),
            }
            continue;
        }
        let Some(expect_rel) = &e.expect else {
            s.skip();
            continue;
        };
        match outcome {
            Ok(got) => compare_expected(&mut s, root, e, expect_rel, &got, "fromRdf"),
            Err(why) => s.fail(&e.id, format!("from_rdf error: {why}")),
        }
    }
    s
}

/// Shared positive-case tail: read + parse the expected document, compare via
/// `json_ld_equal`, record an honest failure with truncated got/want dumps.
fn compare_expected(s: &mut Score, root: &Path, e: &Entry, expect_rel: &str, got: &Json, op: &str) {
    let want = match read_json(root, expect_rel) {
        Ok(j) => j,
        Err(why) => {
            s.fail(&e.id, why);
            return;
        }
    };
    if json_ld_equal(got, &want) {
        s.pass();
    } else {
        let mut g = String::new();
        got.write(&mut g);
        let mut w = String::new();
        want.write(&mut w);
        g.truncate(200);
        w.truncate(200);
        s.fail(
            &e.id,
            format!("{op} document mismatch\n      got:  {g}\n      want: {w}"),
        );
    }
}

// ── Scoreboard assembly + output ─────────────────────────────────────────────

struct Row {
    op: &'static str,
    score: Score,
    floor: usize,
}

/// Machine-readable scoreboard, built with the crate's own `Json` writer so
/// the output is deterministic (object member order = insertion order).
fn scoreboard_json(rows: &[Row], mode: &str, frame_suite_present: bool) -> String {
    let mut pins = Json::obj();
    pins.set("json-ld-api", Json::Str(API_PIN.to_string()));
    pins.set("json-ld-framing", Json::Str(FRAMING_PIN.to_string()));
    let mut ops = Json::obj();
    for r in rows {
        let mut o = Json::obj();
        o.set("pass", Json::Raw(r.score.pass.to_string()));
        o.set("fail", Json::Raw(r.score.fail.to_string()));
        o.set("skip", Json::Raw(r.score.skip.to_string()));
        o.set("total", Json::Raw(r.score.total().to_string()));
        o.set(
            "pass_rate_pct_truncated",
            Json::Raw(fmt_tenths(r.score.pass_rate_tenths())),
        );
        o.set("floor", Json::Raw(r.floor.to_string()));
        let mut fails = Vec::new();
        for (id, why) in &r.score.failures {
            let mut f = Json::obj();
            f.set("id", Json::Str(id.clone()));
            f.set("reason", Json::Str(why.replace('\n', " ")));
            fails.push(f);
        }
        o.set("failures", Json::Arr(fails));
        ops.set(r.op, o);
    }
    // toRdf: the explicit NOT-IMPLEMENTED-natively gap row (honesty invariant).
    let mut tordf = Json::obj();
    tordf.set("implemented", Json::Raw("false".to_string()));
    tordf.set(
        "note",
        Json::Str(
            "native Deserialization-to-RDF is the sq-oy1f.30 scaffold; the engine-path toRdf \
             (oxjsonld) is ratcheted in sparq-conformance floors::to_rdf"
                .to_string(),
        ),
    );
    ops.set("toRdf", tordf);
    let mut doc = Json::obj();
    doc.set(
        "runner",
        Json::Str("sparq-jsonld/examples/jsonld_conformance.rs".to_string()),
    );
    doc.set("mode", Json::Str(mode.to_string()));
    doc.set("suite_pins", pins);
    doc.set(
        "frame_suite_present",
        Json::Raw(frame_suite_present.to_string()),
    );
    doc.set("operations", ops);
    let mut out = String::new();
    doc.write(&mut out);
    out
}

fn print_human(rows: &[Row], mode: &str) {
    println!("W3C JSON-LD 1.1 self-conformance scoreboard — native sparq-jsonld pipeline ({mode})");
    println!("suite pins: json-ld-api {API_PIN} / json-ld-framing {FRAMING_PIN}");
    println!(
        "{:<10} {:>6} {:>6} {:>6} {:>7} {:>8}  floor",
        "op", "pass", "fail", "skip", "total", "rate%"
    );
    for r in rows {
        println!(
            "{:<10} {:>6} {:>6} {:>6} {:>7} {:>8}  {}",
            r.op,
            r.score.pass,
            r.score.fail,
            r.score.skip,
            r.score.total(),
            fmt_tenths(r.score.pass_rate_tenths()),
            r.floor,
        );
    }
    println!(
        "toRdf      NOT IMPLEMENTED natively (sq-oy1f.30 scaffold); engine-path toRdf is \
         ratcheted in sparq-conformance"
    );
    let failing: usize = rows.iter().map(|r| r.score.failures.len()).sum();
    println!("\nfailing tests ({failing} — complete listing, never elided):");
    for r in rows {
        for (id, why) in &r.score.failures {
            println!("  [{}] {} — {}", r.op, id, why);
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--smoke");
    let json_only = args.iter().any(|a| a == "--json");
    if let Some(bad) = args.iter().find(|a| *a != "--smoke" && *a != "--json") {
        eprintln!("unknown argument {bad:?} (supported: --smoke --json)");
        return ExitCode::FAILURE;
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let api_root = manifest_dir.join("../../tests/w3c/json-ld-api/tests");
    let frame_root = manifest_dir.join("../../tests/w3c/json-ld-framing/tests");
    if !api_root.exists() {
        // Offline-green convention: never fabricate a row, never fail a fresh
        // checkout — say exactly what is missing and exit 0.
        println!(
            "SKIP: W3C json-ld-api suite not present at {} — run scripts/fetch-jsonld-tests.sh \
             (and scripts/fetch-jsonld-framing-tests.sh) first.",
            api_root.display()
        );
        return ExitCode::SUCCESS;
    }
    let frame_present = frame_root.exists();

    let mode = if smoke { "smoke" } else { "full" };
    let cap = if smoke { SMOKE_N } else { usize::MAX };
    let mut rows = Vec::new();
    type Lane<'a> = (
        &'static str,
        &'a dyn Fn(&Path, &[Entry]) -> Score,
        &'a PathBuf,
        usize,
    );
    let lanes: [Lane<'_>; 5] = [
        ("expand", &run_expand, &api_root, EXPAND_FLOOR),
        ("compact", &run_compact, &api_root, COMPACT_FLOOR),
        ("flatten", &run_flatten, &api_root, FLATTEN_FLOOR),
        ("frame", &run_frame, &frame_root, FRAME_FLOOR),
        ("fromRdf", &run_fromrdf, &api_root, FROMRDF_FLOOR),
    ];
    for (op, run, root, floor) in lanes {
        if op == "frame" && !frame_present {
            eprintln!(
                "SKIP frame: json-ld-framing suite not present at {} — run \
                 scripts/fetch-jsonld-framing-tests.sh",
                frame_root.display()
            );
            continue;
        }
        let cat = if op == "fromRdf" { "fromRdf" } else { op };
        let entries = match read_manifest(root, cat) {
            Ok(mut e) => {
                e.truncate(cap);
                e
            }
            Err(why) => {
                eprintln!("ERROR: {op}: {why}");
                return ExitCode::FAILURE;
            }
        };
        rows.push(Row {
            op,
            score: run(root, &entries),
            floor,
        });
    }

    if json_only {
        println!("{}", scoreboard_json(&rows, mode, frame_present));
    } else {
        print_human(&rows, mode);
        println!(
            "\nmachine-readable scoreboard:\n{}",
            scoreboard_json(&rows, mode, frame_present)
        );
    }

    // Self-assertions: exact pins in smoke mode, ratchet floors in full mode.
    let mut ok = true;
    if smoke {
        for r in &rows {
            let Some((_, p, f, k)) = SMOKE_PINS.iter().find(|(op, ..)| *op == r.op) else {
                eprintln!("ASSERT FAIL: no smoke pin for {}", r.op);
                ok = false;
                continue;
            };
            if (r.score.pass, r.score.fail, r.score.skip) != (*p, *f, *k) {
                eprintln!(
                    "ASSERT FAIL: {} smoke counts (pass,fail,skip)=({},{},{}) != pinned ({p},{f},{k}) \
                     — suite pin / algorithm / oracle changed; re-calibrate DELIBERATELY",
                    r.op, r.score.pass, r.score.fail, r.score.skip
                );
                ok = false;
            }
        }
    } else {
        for r in &rows {
            if r.score.pass < r.floor {
                eprintln!(
                    "ASSERT FAIL: {} pass count {} regressed below floor {}",
                    r.op, r.score.pass, r.floor
                );
                ok = false;
            }
        }
    }
    if ok {
        // In --json mode stdout carries ONLY the JSON document (machine
        // consumers parse it directly) — the status note goes to stderr.
        if json_only {
            eprintln!("self-assertions OK ({mode} mode)");
        } else {
            println!("\nself-assertions OK ({mode} mode)");
        }
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

// ── Hermetic unit tests (run by `cargo test -p sparq-jsonld` via the
//    `[[example]] test = true` target; no suite checkout required) ───────────

#[cfg(test)]
mod tests {
    use super::*;

    fn j(s: &str) -> Json {
        Json::parse(s).expect("valid JSON fixture")
    }

    /// Arrays outside @list are unordered; inside @list order is significant.
    #[test]
    fn comparator_list_order_semantics() {
        assert!(json_ld_equal(&j("[1,2,3]"), &j("[3,1,2]")));
        assert!(!json_ld_equal(&j("[1,2,3]"), &j("[1,2,4]")));
        assert!(!json_ld_equal(
            &j(r#"{"@list":[1,2]}"#),
            &j(r#"{"@list":[2,1]}"#)
        ));
        assert!(json_ld_equal(
            &j(r#"{"@list":[1,2]}"#),
            &j(r#"{"@list":[1,2]}"#)
        ));
    }

    /// Object key order insignificant; missing/extra keys unequal.
    #[test]
    fn comparator_object_semantics() {
        assert!(json_ld_equal(
            &j(r#"{"a":1,"b":2}"#),
            &j(r#"{"b":2,"a":1}"#)
        ));
        assert!(!json_ld_equal(&j(r#"{"a":1}"#), &j(r#"{"a":1,"b":2}"#)));
    }

    /// Numeric guard: `1` ≡ `1.0`, but 2^53 and 2^53+1 stay distinct.
    #[test]
    fn comparator_numeric_guard() {
        assert!(json_ld_equal(&j("1"), &j("1.0")));
        assert!(json_ld_equal(&j("1"), &j("1e0")));
        assert!(!json_ld_equal(
            &j("9007199254740992"),
            &j("9007199254740993")
        ));
        assert!(!json_ld_equal(&j("true"), &j("1")));
    }

    /// Multiset semantics for repeated elements outside @list.
    #[test]
    fn comparator_multiset_duplicates() {
        assert!(json_ld_equal(&j("[1,1,2]"), &j("[1,2,1]")));
        assert!(!json_ld_equal(&j("[1,1,2]"), &j("[1,2,2]")));
    }

    /// N-Quads reader: IRI / bnode / plain / lang / typed terms + graph label.
    #[test]
    fn nquads_reader_terms() {
        let text = concat!(
            "<http://ex/s> <http://ex/p> \"a b\\nc\" .\n",
            "# comment\n",
            "_:b0 <http://ex/p> \"x\"@en <http://ex/g> .\n",
            "<http://ex/s> <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> _:g1 .\n",
            "<http://ex/s> <http://ex/p> <http://ex/o\\u00E9> .\n",
        );
        let quads = parse_nquads(text).expect("well-formed N-Quads");
        assert_eq!(quads.len(), 4);
        assert_eq!(quads[0].object, RdfTerm::literal("a b\nc"));
        assert_eq!(quads[0].graph, None);
        assert_eq!(quads[1].subject, RdfTerm::blank("b0"));
        assert_eq!(quads[1].object, RdfTerm::lang_literal("x", "en"));
        assert_eq!(quads[1].graph, Some(RdfTerm::iri("http://ex/g")));
        assert_eq!(
            quads[2].object,
            RdfTerm::typed_literal("1", "http://www.w3.org/2001/XMLSchema#integer")
        );
        assert_eq!(quads[2].graph, Some(RdfTerm::blank("g1")));
        assert_eq!(quads[3].object, RdfTerm::iri("http://ex/o\u{e9}"));
    }

    /// Malformed N-Quads are an error, not a silent drop.
    #[test]
    fn nquads_reader_rejects_malformed() {
        assert!(parse_nquads("<http://ex/s> <http://ex/p> \"unterminated").is_err());
        assert!(parse_nquads("<http://ex/s> <http://ex/p> \"x\"").is_err());
    }

    /// Pass-rates are TRUNCATED, never rounded up (2/3 → 66.6, not 66.7).
    #[test]
    fn pass_rate_truncates_never_rounds_up() {
        let mut s = Score::default();
        s.pass();
        s.pass();
        s.fail("t1", "why".to_string());
        assert_eq!(s.total(), 3);
        assert_eq!(fmt_tenths(s.pass_rate_tenths()), "66.6");
        let empty = Score::default();
        assert_eq!(fmt_tenths(empty.pass_rate_tenths()), "0.0");
    }

    /// The manifest reader surfaces the fields the lanes depend on.
    #[test]
    fn manifest_reader_fields() {
        let dir = std::env::temp_dir().join(format!("sq-w42s3-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("expand-manifest.jsonld"),
            r##"{"sequence":[
                {"@id":"#t1","@type":["jld:PositiveEvaluationTest"],"input":"expand/0001-in.jsonld",
                 "expect":"expand/0001-out.jsonld",
                 "option":{"specVersion":"json-ld-1.1","expandContext":"expand/0001-ctx.jsonld"}},
                {"@id":"#t2","@type":["jld:NegativeEvaluationTest"],"input":"expand/0002-in.jsonld",
                 "expectErrorCode":"invalid local context","option":{"base":"http://ex/base"}}
            ]}"##,
        )
        .unwrap();
        let entries = read_manifest(&dir, "expand").expect("manifest parses");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "#t1");
        assert!(!entries[0].is_negative);
        assert_eq!(
            entries[0].expand_context_path.as_deref(),
            Some("expand/0001-ctx.jsonld")
        );
        assert_eq!(
            doc_base(&entries[0]),
            format!("{SUITE_BASE}expand/0001-in.jsonld")
        );
        assert!(entries[1].is_negative);
        assert_eq!(
            entries[1].expect_error_code.as_deref(),
            Some("invalid local context")
        );
        assert_eq!(doc_base(&entries[1]), "http://ex/base");
    }
}
