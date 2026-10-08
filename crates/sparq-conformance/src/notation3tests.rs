//! Runner logic for the community **notation3tests** suite
//! (<https://codeberg.org/phochste/notation3tests>, issue #6467) — a corpus of
//! self-checking N3 documents collected across N3 reasoners (EYE, eyeling, cwm,
//! jen3, ...), complementary to the w3c/N3 community-group manifests that
//! `sparq-inference-conformance` already runs.
//!
//! **Layout.** Every case is one `.n3` document under the checkout's `tests/`
//! directory (`tests/static/**`, `tests/generated/<ns>/<builtin>/**`). The rest
//! of the checkout (`HELLO.n3`, `lib/`, `extra/`, reports, the JS harness) is
//! support material, not cases; [`discover`] reads `tests/` only and errors when
//! it is missing.
//!
//! **Expectation.** Each document names its case(s) with `:test :contains :<name>`
//! (`:` = `http://example.org/`), and the case name's prefix carries the expected
//! outcome — there is no separate manifest:
//!
//! - `success-*` — the reasoner must derive `:result :has :<name>` or
//!   `:test :is true`. Deriving `:test :is false` is NONCONFORM; deriving none of
//!   them is INCOMPLETE.
//! - `fail-*` (negative) — correct behaviour derives NONE of `:result :has :<name>`,
//!   `:test :is true`, `:test :is false`; deriving any of them is NONCONFORM. A
//!   closure with no boolean verdict is therefore the PASS case here.
//! - `crash-*` (file name) — the reasoner must REJECT the document (a syntax
//!   error, or an inconsistency/fuse report). A parse or reasoning error, an empty
//!   closure, or a closure without `:test :contains` is OK; a closure that still
//!   reaches `:test :contains` means the bad input was accepted (NONCONFORM).
//! - Any other case name (e.g. a misspelt prefix) is INCOMPLETE.
//! - A document that derives no `:test :contains` at all (and is not `crash-*`)
//!   is INCOMPLETE; a parse/reasoning error on a non-`crash-*` document is CRASHED.
//!
//! Source of this rule: the suite's own reference harness (`npm run test:<reasoner>`
//! in the codeberg checkout), which reports exactly these per-case outcomes as
//! `TOTAL [COUNT:n] OK: INCOMPLETE: NONCONFORM: CRASHED:` (the line eyeling's
//! `test/run.js` parses). codeberg.org was not reachable when this was written, so
//! the decision table above is taken from eyeron's Rust port of that harness,
//! `tests/notation3_conformance.rs::run_one` in
//! <https://github.com/eyereasoner/eyeron> (commit a355eb4), and checked against
//! the fixtures themselves (every `success-*` document derives `:test :is true`
//! on success; every `fail-*` / `crash-*` document derives `:test :is false` only
//! when the invalid operation or input is accepted). Two deliberate deviations,
//! both stricter: a `success-*` closure that derives `:test :is false` alongside
//! the pass signal is NONCONFORM (no suite case derives both), and `xsd:boolean`
//! verdicts are read by value (`"1"`/`"0"` as well as `true`/`false`). Timeouts
//! (not in the reference) get their own bucket.
//!
//! The suite is FETCHED at run time (never vendored);
//! `.github/workflows/notation3tests.yml` fetches it and runs
//! `sparq-notation3tests`. This module is pure logic (discovery, verdict, report)
//! so it can be pinned by hand-written cases in the suite's format without the
//! suite present; the binary adds per-test process isolation for the timeout.

use sparq_reason::n3::Term;
use std::path::{Path, PathBuf};

const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
/// The suite's `:` namespace (`@prefix : <http://example.org/>` in every case).
const EX: &str = "http://example.org/";

/// Outcome of one test document, in the reference harness's categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// OK: the case met its expectation.
    Pass,
    /// NONCONFORM: the closure contradicts the expectation (a `success-*` case
    /// derived `:test :is false`, a `fail-*` case derived any verdict, a
    /// `crash-*` document was accepted).
    Nonconform,
    /// INCOMPLETE: the closure lacks the expected signal (no `:test :contains`,
    /// a `success-*` case without its pass signal, or an unrecognised case name).
    Incomplete,
    /// CRASHED: a parse or reasoning error on a document that must be accepted.
    Crashed(String),
    /// Exceeded the per-test timeout.
    Timeout,
    /// Unmeasured: the document asked `log:content` / `log:semantics` for an
    /// external resource the offline runner cannot serve (the IRI is kept), and
    /// its outcome is not established without it (anything but a positive
    /// pass). A harness limitation, reported apart from engine failures.
    Unavailable(String),
}

impl Verdict {
    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Pass)
    }

    /// A single line, used as the child-process protocol and in the report.
    pub fn to_line(&self) -> String {
        match self {
            Verdict::Pass => "PASS".into(),
            Verdict::Nonconform => "FAIL nonconform".into(),
            Verdict::Incomplete => "FAIL incomplete".into(),
            Verdict::Timeout => "FAIL timeout".into(),
            Verdict::Crashed(e) => format!("FAIL crashed {}", e.replace(['\n', '\r'], " ")),
            Verdict::Unavailable(iri) => {
                format!("SKIP unavailable {}", iri.replace(['\n', '\r'], " "))
            }
        }
    }

    /// Inverse of [`Verdict::to_line`] (unknown lines become `Crashed`).
    pub fn from_line(line: &str) -> Verdict {
        let line = line.trim();
        match line {
            "PASS" => Verdict::Pass,
            "FAIL nonconform" => Verdict::Nonconform,
            "FAIL incomplete" => Verdict::Incomplete,
            "FAIL timeout" => Verdict::Timeout,
            _ => {
                if let Some(e) = line.strip_prefix("FAIL crashed ") {
                    Verdict::Crashed(e.to_string())
                } else if let Some(iri) = line.strip_prefix("SKIP unavailable ") {
                    Verdict::Unavailable(iri.to_string())
                } else {
                    Verdict::Crashed(format!("unrecognised child output: {line:?}"))
                }
            }
        }
    }

    fn bucket(&self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Nonconform => "nonconform",
            Verdict::Incomplete => "incomplete",
            Verdict::Crashed(_) => "crashed",
            Verdict::Timeout => "timeout",
            Verdict::Unavailable(_) => "unavailable",
        }
    }

    /// Severity for combining several cases in one document (worst wins).
    fn rank(&self) -> u8 {
        match self {
            Verdict::Pass => 0,
            Verdict::Unavailable(_) => 1,
            Verdict::Incomplete => 1,
            Verdict::Nonconform => 2,
            Verdict::Timeout => 3,
            Verdict::Crashed(_) => 4,
        }
    }
}

/// Report buckets, in display order.
const BUCKETS: [&str; 6] = [
    "pass",
    "nonconform",
    "incomplete",
    "crashed",
    "timeout",
    "unavailable",
];

/// Whether a test file is a `crash-*` case: the input must be rejected.
pub fn expects_rejection(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("crash"))
}

/// The local name of an IRI: the part after the last `#` or `/`.
fn local_name(iri: &str) -> &str {
    iri.rsplit(['#', '/']).next().unwrap_or(iri)
}

/// The value of an `xsd:boolean` literal, by any of its four valid lexical
/// forms (`true`/`1`, `false`/`0`; whitespace collapsed). `None` for anything
/// else, including an invalid `xsd:boolean` spelling.
fn boolean_value(t: &Term) -> Option<bool> {
    match t {
        Term::Lit(lex, dt, None) if dt == XSD_BOOLEAN => match lex.trim() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn is_ex(t: &Term, local: &str) -> bool {
    matches!(t, Term::Iri(i) if i.strip_prefix(EX) == Some(local))
}

/// Decide the verdict from a closure's facts. `expect_rejection` is true for a
/// `crash-*` document (see the module docs for the full decision table).
/// Whether the closure announces any negative (`fail-*`) case.
fn has_negative_case(facts: &[[Term; 3]]) -> bool {
    facts.iter().any(|[s, p, o]| {
        is_ex(s, "test")
            && is_ex(p, "contains")
            && matches!(o, Term::Iri(i) if local_name(i).starts_with("fail"))
    })
}

pub fn verdict_from_facts(facts: &[[Term; 3]], expect_rejection: bool) -> Verdict {
    let mut cases: Vec<&Term> = facts
        .iter()
        .filter(|[s, p, _]| is_ex(s, "test") && is_ex(p, "contains"))
        .map(|[_, _, o]| o)
        .collect();
    cases.sort_by_key(|t| format!("{t:?}"));
    cases.dedup();
    if expect_rejection {
        return if cases.is_empty() {
            Verdict::Pass
        } else {
            Verdict::Nonconform
        };
    }
    if cases.is_empty() {
        return Verdict::Incomplete;
    }
    let (mut t, mut f) = (false, false);
    for [s, p, o] in facts {
        if is_ex(s, "test") && is_ex(p, "is") {
            match boolean_value(o) {
                Some(true) => t = true,
                Some(false) => f = true,
                None => {}
            }
        }
    }
    let result_has = |case: &Term| {
        facts
            .iter()
            .any(|[s, p, o]| is_ex(s, "result") && is_ex(p, "has") && o == case)
    };
    let mut worst = Verdict::Pass;
    for case in cases {
        let name = match case {
            Term::Iri(i) => local_name(i),
            _ => "",
        };
        let v = if name.starts_with("fail") {
            if result_has(case) || t || f {
                Verdict::Nonconform
            } else {
                Verdict::Pass
            }
        } else if name.starts_with("success") {
            if f {
                Verdict::Nonconform
            } else if result_has(case) || t {
                Verdict::Pass
            } else {
                Verdict::Incomplete
            }
        } else {
            Verdict::Incomplete
        };
        if v.rank() > worst.rank() {
            worst = v;
        }
    }
    worst
}

/// `file://` IRI for a local path (used as the document base). Every byte outside
/// the RFC 3986 unreserved set (and `/`) is percent-encoded, so a `#`, `?`, `%` or
/// space in the checkout path cannot turn into a fragment, query or escape.
fn file_iri(path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut iri = String::from("file://");
    for &b in abs.to_string_lossy().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            iri.push(b as char);
        } else {
            iri.push_str(&format!("%{b:02X}"));
        }
    }
    iri
}

/// The filesystem path a `file://` IRI denotes: query and fragment dropped, then
/// percent-decoded. `None` for a malformed escape or a non-UTF-8 result.
fn file_iri_path(iri: &str) -> Option<PathBuf> {
    let p = iri.strip_prefix("file://")?;
    let p = p.split(['#', '?']).next().unwrap_or(p).as_bytes();
    let mut out = Vec::with_capacity(p.len());
    let mut i = 0;
    while i < p.len() {
        if p[i] == b'%' {
            let hex = std::str::from_utf8(p.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(p[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

/// Where the suite publishes its own files: cases fetch fixtures such as
/// `HELLO.n3` from `<SUITE_RAW>/<branch>/<path>` (or `raw/commit/<sha>/`).
const SUITE_RAW: [&str; 2] = [
    "https://codeberg.org/phochste/notation3tests/raw/branch/",
    "https://codeberg.org/phochste/notation3tests/raw/commit/",
];

/// The checkout-relative path a canonical suite fixture URL denotes (the
/// branch or commit segment dropped, then percent-decoded like a `file://`
/// path). `None` when the IRI is not one of the suite's raw URLs.
fn suite_fixture_path(iri: &str) -> Option<PathBuf> {
    let rest = SUITE_RAW.iter().find_map(|p| iri.strip_prefix(p))?;
    let (_rev, rel) = rest.split_once('/')?;
    let rel = file_iri_path(&format!("file://{rel}"))?;
    (!rel.as_os_str().is_empty()).then_some(rel)
}

/// Resolve a `log:content` / `log:semantics` IRI to a file inside `root`: a
/// `file://` IRI by its path, a canonical suite fixture URL by its path in the
/// checkout. Either way the canonical result must stay inside `root`.
/// `Ok(None)`: no such document (a case may probe for one on purpose).
/// `Err(())`: the document exists but lies outside `root`, so the runner
/// refused it.
fn resolve_in_suite(iri: &str, root: &Path) -> Result<Option<PathBuf>, ()> {
    let p = match suite_fixture_path(iri) {
        Some(rel) => root.join(rel),
        None => match file_iri_path(iri) {
            Some(p) => p,
            None => return Ok(None),
        },
    };
    let Ok(p) = std::fs::canonicalize(p) else {
        return Ok(None);
    };
    if p.starts_with(root) {
        Ok(Some(p))
    } else {
        Err(())
    }
}

/// Run sparq's N3 reasoner over one test document. `log:semantics` /
/// `log:content` may read `file://` documents and the suite's own published
/// fixtures (`https://codeberg.org/phochste/notation3tests/raw/...`, mapped into
/// the checkout), but only inside `suite_root` (strictly offline; nothing
/// outside the fetched suite is readable). The
/// test document itself is held to the same rule: it is canonicalized (which
/// resolves symlinks) and refused, without being read, unless it lies inside
/// the canonical `suite_root`. Whether the document must be rejected comes
/// from its file name ([`expects_rejection`]).
pub fn run_one(path: &Path, suite_root: &Path) -> Verdict {
    let (Ok(canon), Ok(root)) = (
        std::fs::canonicalize(path),
        std::fs::canonicalize(suite_root),
    ) else {
        return Verdict::Crashed(format!("cannot resolve {}", path.display()));
    };
    if !canon.starts_with(&root) {
        return Verdict::Crashed(format!(
            "refusing {}: resolves outside the suite root",
            path.display()
        ));
    }
    let src = match std::fs::read_to_string(&canon) {
        Ok(s) => s,
        Err(e) => return Verdict::Crashed(format!("read {}: {e}", path.display())),
    };
    run_source(&src, &file_iri(path), suite_root, expects_rejection(path))
}

/// As [`run_one`], over in-memory source with an explicit base IRI and
/// expectation. With `expect_rejection` (a `crash-*` case) a parse or reasoning
/// error is the PASS outcome; otherwise it is CRASHED.
pub fn run_source(src: &str, base: &str, suite_root: &Path, expect_rejection: bool) -> Verdict {
    // Fail closed: with no canonical root (missing, or an empty path) every
    // local lookup is refused, never confined to a prefix that matches all.
    let root = std::fs::canonicalize(suite_root).ok();
    // The first IRI the runner refused: external (non-file, non-suite), or an
    // existing file outside the checkout.
    // (`Resolver` is `'static`, so the slot is shared rather than borrowed.)
    let unavailable = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
    let seen = std::rc::Rc::clone(&unavailable);
    let resolver = move |iri: &str| -> Option<String> {
        let refuse = || {
            seen.borrow_mut().get_or_insert_with(|| iri.to_string());
            None
        };
        if !iri.starts_with("file://") && suite_fixture_path(iri).is_none() {
            return refuse();
        }
        let Some(root) = root.as_deref() else {
            return refuse();
        };
        match resolve_in_suite(iri, root) {
            Ok(p) => std::fs::read_to_string(p?).ok(),
            Err(()) => refuse(),
        }
    };
    let (verdict, positive_pass) =
        match sparq_reason::n3::reason_n3_terms_with_resolver(src, Some(base), Some(&resolver)) {
            Ok(c) => {
                let v = verdict_from_facts(&c.facts, expect_rejection);
                let positive = v.is_pass() && !expect_rejection && !has_negative_case(&c.facts);
                (v, positive)
            }
            Err(_) if expect_rejection => (Verdict::Pass, false),
            Err(e) => (Verdict::Crashed(e), false),
        };
    // A pass that rests on something *not* being derived (a `fail-*` or
    // `crash-*` case) is not established when a lookup was refused: the missing
    // resource alone can explain it. Only a derived pass signal stands.
    let unavailable = unavailable.borrow_mut().take();
    match unavailable {
        Some(iri) if !positive_pass => Verdict::Unavailable(iri),
        _ => verdict,
    }
}

/// Every `.n3` case under `<root>/tests` (the suite's case directory; the rest
/// of the checkout is support material): recursive, sorted, hidden dirs skipped.
/// Errors when `<root>/tests` is not a directory, so a moved or renamed case
/// directory is reported instead of silently scanning the whole checkout.
/// Symlinks (to files or directories) are never followed or returned, so a
/// fetched suite cannot point the runner at files outside it.
pub fn discover(root: &Path) -> Result<Vec<PathBuf>, String> {
    let base = root.join("tests");
    if !base.is_dir() {
        return Err(format!(
            "notation3tests case directory {} not found (expected the suite checkout's tests/)",
            base.display()
        ));
    }
    let mut out = Vec::new();
    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let hidden = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if hidden {
                continue;
            }
            // `DirEntry::file_type` does not follow symlinks.
            let Ok(ft) = e.file_type() else {
                continue;
            };
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && p.extension().is_some_and(|x| x == "n3") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// One row of the report.
pub struct Row {
    pub test: String,
    pub verdict: Verdict,
}

/// Markdown report: summary line, per-bucket counts, and every non-pass row.
pub fn report_markdown(rows: &[Row], suite_rev: &str) -> String {
    let total = rows.len();
    let pass = rows.iter().filter(|r| r.verdict.is_pass()).count();
    let pct = if total == 0 {
        0.0
    } else {
        100.0 * pass as f64 / total as f64
    };
    let mut s = String::new();
    s.push_str("# notation3tests report\n\n");
    s.push_str(&format!(
        "- Suite: https://codeberg.org/phochste/notation3tests @ `{suite_rev}`\n"
    ));
    s.push_str("- Engine: `sparq_reason::n3::reason_n3_terms_with_resolver` (forward closure)\n");
    s.push_str(
        "- Pass criterion: the suite's per-case expectation (`success-*` derives its pass signal, \
         `fail-*` derives no verdict, `crash-*` input is rejected); see `notation3tests.rs`\n\n",
    );
    s.push_str(&format!("**{pass} / {total} pass ({pct:.1}%)**\n\n"));
    s.push_str("| bucket | count |\n|---|---|\n");
    for b in BUCKETS {
        let n = rows.iter().filter(|r| r.verdict.bucket() == b).count();
        s.push_str(&format!("| {b} | {n} |\n"));
    }
    let fails: Vec<&Row> = rows.iter().filter(|r| !r.verdict.is_pass()).collect();
    if !fails.is_empty() {
        s.push_str("\n## Failing tests\n\n| test | result |\n|---|---|\n");
        for r in fails {
            let mut line = r.verdict.to_line().replace('|', "\\|");
            if line.len() > 200 {
                let mut cut = 200;
                while !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                line.truncate(cut);
                line.push('…');
            }
            s.push_str(&format!("| `{}` | {line} |\n", r.test));
        }
    }
    s
}

/// Machine-readable summary (`{"total":..,"pass":..,"buckets":{..},"results":[..]}`).
pub fn report_json(rows: &[Row], suite_rev: &str) -> String {
    let mut buckets = serde_json::Map::new();
    for b in BUCKETS {
        let n = rows.iter().filter(|r| r.verdict.bucket() == b).count();
        buckets.insert(b.into(), n.into());
    }
    let results: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| serde_json::json!({"test": r.test, "result": r.verdict.to_line()}))
        .collect();
    serde_json::json!({
        "suite": "https://codeberg.org/phochste/notation3tests",
        "suite_rev": suite_rev,
        "total": rows.len(),
        "pass": rows.iter().filter(|r| r.verdict.is_pass()).count(),
        "buckets": buckets,
        "results": results,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PFX: &str = "@prefix : <http://example.org/>.\n\
                       @prefix xsd: <http://www.w3.org/2001/XMLSchema#>.\n\
                       @prefix math: <http://www.w3.org/2000/10/swap/math#>.\n\
                       @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n";

    /// A document in the suite's shape: `premise => :result :has :<name>`,
    /// `{} => :test :contains :<name>`, `:result :has :<name> => :test :is <on_result>`.
    fn case(name: &str, premise: &str, on_result: &str) -> String {
        format!(
            "{PFX}{{ {premise} }} => {{ :result :has :{name} }}.\n\
             {{}} => {{ :test :contains :{name} }}.\n\
             {{ :result :has :{name} }} => {{ :test :is {on_result} }}.\n"
        )
    }

    fn run(src: &str) -> Verdict {
        run_source(
            src,
            "file:///nonexistent/t.n3",
            Path::new("/nonexistent"),
            false,
        )
    }

    fn run_rejecting(src: &str) -> Verdict {
        run_source(
            src,
            "file:///nonexistent/t.n3",
            Path::new("/nonexistent"),
            true,
        )
    }

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("n3t-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn success_case_deriving_true_passes() {
        let src = format!(
            "{}:a :b :c.\n",
            case("success-literal-1", ":a :b :c", "true")
        );
        assert_eq!(run(&src), Verdict::Pass);
    }

    #[test]
    fn hash_namespace_and_builtin_pass() {
        let src = case("success-literal-1", "(1 2) math:sum 3", "true");
        assert_eq!(run(&src), Verdict::Pass);
    }

    #[test]
    fn success_case_without_its_signal_is_incomplete() {
        let src = case("success-literal-1", "(1 2) math:sum 4", "true");
        assert_eq!(run(&src), Verdict::Incomplete);
    }

    #[test]
    fn success_case_deriving_false_is_nonconform_even_alongside_true() {
        let src = format!("{PFX}{{}} => {{ :test :contains :success-x }}.\n:test :is true.\n{{ :test :is true }} => {{ :test :is false }}.\n");
        assert_eq!(run(&src), Verdict::Nonconform);
    }

    /// The reviewer's example: tests/generated/list/append/fail-literal-1.n3.
    /// Correct behaviour (the invalid append does not succeed) derives no boolean
    /// verdict at all, and that is a PASS for a `fail-*` case.
    #[test]
    fn negative_case_with_no_verdict_passes() {
        let src = case("fail-literal-1", "(1 2) list:append (1 2)", "false");
        assert_eq!(run(&src), Verdict::Pass);
    }

    #[test]
    fn negative_case_deriving_false_or_result_is_nonconform() {
        // The invalid operation "succeeds", so the document derives :test :is false.
        let src = case("fail-literal-1", "(1 2) math:sum 3", "false");
        assert_eq!(run(&src), Verdict::Nonconform);
        // :result :has :<name> alone (no boolean) is already nonconform.
        let src = format!(
            "{PFX}{{ (1 2) math:sum 3 }} => {{ :result :has :fail-literal-1 }}.\n\
             {{}} => {{ :test :contains :fail-literal-1 }}.\n"
        );
        assert_eq!(run(&src), Verdict::Nonconform);
        // So is a stray :test :is true.
        let src = format!("{PFX}{{}} => {{ :test :contains :fail-x }}.\n:test :is true.\n");
        assert_eq!(run(&src), Verdict::Nonconform);
    }

    /// tests/static/crash-syntax-8.n3 in shape: the IRI escape is illegal, so a
    /// conforming parser rejects the document, which is the PASS outcome.
    #[test]
    fn expected_parse_failure_passes_when_rejected() {
        let src = format!(
            "<http://bad\\u0020example.org/> a :BadExample.\n{}",
            case("crash-syntax-8", ":subject :predicate ?X", "false")
        );
        assert_eq!(run_rejecting(&src), Verdict::Pass);
        // The same parse error on a document that must be accepted is CRASHED.
        assert!(matches!(run(&src), Verdict::Crashed(_)), "{:?}", run(&src));
    }

    #[test]
    fn crash_case_that_is_accepted_is_nonconform() {
        let src = format!(
            "{PFX}{{}} => {{ :test :contains :crash-fuse-1 }}.\n{{}} => {{ :test :is false }}.\n"
        );
        assert_eq!(run_rejecting(&src), Verdict::Nonconform);
    }

    /// The expectation comes from the file name: `crash-*` must be rejected.
    #[test]
    fn run_one_reads_the_expectation_from_the_file_name() {
        let dir = tmp("expect");
        let bad = format!(
            "<http://bad\\u0020example.org/> a :B.\n{}",
            case("crash-syntax-8", ":s :p ?X", "false")
        );
        std::fs::write(dir.join("crash-syntax-8.n3"), &bad).unwrap();
        std::fs::write(dir.join("success-literal-1.n3"), &bad).unwrap();
        assert!(expects_rejection(&dir.join("crash-syntax-8.n3")));
        assert!(!expects_rejection(&dir.join("success-literal-1.n3")));
        assert_eq!(run_one(&dir.join("crash-syntax-8.n3"), &dir), Verdict::Pass);
        assert!(matches!(
            run_one(&dir.join("success-literal-1.n3"), &dir),
            Verdict::Crashed(_)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_test_contains_or_unknown_name_is_incomplete() {
        let src = format!("{PFX}:test :is true.\n");
        assert_eq!(run(&src), Verdict::Incomplete);
        let src = format!("{PFX}{{}} => {{ :test :contains :mystery-1 }}.\n:test :is true.\n");
        assert_eq!(run(&src), Verdict::Incomplete);
    }

    #[test]
    fn string_true_is_not_a_boolean_verdict() {
        let src = format!("{PFX}{{}} => {{ :test :contains :success-x }}.\n:test :is \"true\".\n");
        assert_eq!(run(&src), Verdict::Incomplete);
    }

    #[test]
    fn parse_error_is_crashed() {
        assert!(matches!(
            run("@prefix : <urn:example:> .\n:a :b"),
            Verdict::Crashed(_)
        ));
    }

    #[test]
    fn verdict_line_round_trips() {
        for v in [
            Verdict::Pass,
            Verdict::Nonconform,
            Verdict::Incomplete,
            Verdict::Timeout,
            Verdict::Crashed("bad\nthing".into()),
        ] {
            let back = Verdict::from_line(&v.to_line());
            match (&v, &back) {
                (Verdict::Crashed(_), Verdict::Crashed(b)) => assert_eq!(b, "bad thing"),
                _ => assert_eq!(v, back),
            }
        }
    }

    /// The real checkout keeps support documents (HELLO.n3, lib/, extra/) beside
    /// tests/; only tests/ holds cases.
    #[test]
    fn discover_reads_tests_dir_only_and_skips_hidden() {
        let dir = tmp("discover");
        for d in [
            "lib",
            "extra",
            "tests/static/math",
            "tests/generated/list",
            "tests/.git",
        ] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in [
            "HELLO.n3",
            "lib/query.n3",
            "extra/stratification-check-1.n3",
            "tests/static/success-literal-1.n3",
            "tests/static/math/fail-literal-1.n3",
            "tests/generated/list/crash-x.n3",
            "tests/static/notes.ttl",
            "tests/.git/d.n3",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let found: Vec<String> = discover(&dir)
            .unwrap()
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().display().to_string())
            .collect();
        assert_eq!(
            found,
            [
                "tests/generated/list/crash-x.n3",
                "tests/static/math/fail-literal-1.n3",
                "tests/static/success-literal-1.n3",
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn discover_errors_when_tests_dir_is_missing() {
        let dir = tmp("discover-missing");
        std::fs::create_dir_all(dir.join("test-cases")).unwrap();
        std::fs::write(dir.join("HELLO.n3"), "").unwrap();
        std::fs::write(dir.join("test-cases/a.n3"), "").unwrap();
        let err = discover(&dir).unwrap_err();
        assert!(err.contains("tests") && err.contains("not found"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A checkout path with `#`, `%` and a space still resolves documents inside the
    /// suite: the base IRI is percent-encoded and the resolver decodes it.
    #[test]
    fn checkout_path_with_reserved_characters_resolves() {
        let dir = std::env::temp_dir().join(format!("n3t-iri#1 %41-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("suite")).unwrap();
        std::fs::write(
            dir.join("suite/data.n3"),
            "@prefix : <urn:example:>.\n:a :b :c.\n",
        )
        .unwrap();
        let suite = dir.join("suite");
        let src = format!(
            "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}",
            case(
                "success-semantics-1",
                "<data.n3> log:semantics ?f. ?f log:includes { <urn:example:a> <urn:example:b> <urn:example:c> }",
                "true"
            )
        );
        let base = file_iri(&suite.join("t.n3"));
        assert!(!base.contains('#') && !base.contains(' '), "{base}");
        assert_eq!(
            file_iri_path(&base).unwrap(),
            std::fs::canonicalize(&suite).unwrap().join("t.n3")
        );
        assert_eq!(run_source(&src, &base, &suite, false), Verdict::Pass);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolver_reads_inside_suite_only() {
        let dir = tmp("resolve");
        std::fs::create_dir_all(dir.join("suite")).unwrap();
        let data = "@prefix : <urn:example:>.\n:a :b :c.\n";
        std::fs::write(dir.join("suite/data.n3"), data).unwrap();
        std::fs::write(dir.join("outside.n3"), data).unwrap();
        let suite = dir.join("suite");
        let doc = |target: &Path| {
            format!(
                "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}",
                case(
                    "success-semantics-1",
                    &format!(
                        "<{}> log:semantics ?f. ?f log:includes {{ <urn:example:a> <urn:example:b> <urn:example:c> }}",
                        file_iri(target)
                    ),
                    "true"
                )
            )
        };
        let base = file_iri(&suite.join("t.n3"));
        assert_eq!(
            run_source(&doc(&suite.join("data.n3")), &base, &suite, false),
            Verdict::Pass
        );
        assert_eq!(
            run_source(&doc(&dir.join("outside.n3")), &base, &suite, false),
            Verdict::Unavailable(file_iri(&dir.join("outside.n3")))
        );
        // Refused, so a negative case cannot pass on it either; a missing file
        // is an ordinary (measurable) miss.
        let neg = |target: &Path| {
            format!(
                "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}",
                case(
                    "fail-semantics-1",
                    &format!("<{}> log:semantics ?f", file_iri(target)),
                    "false"
                )
            )
        };
        assert_eq!(
            run_source(&neg(&dir.join("outside.n3")), &base, &suite, false),
            Verdict::Unavailable(file_iri(&dir.join("outside.n3")))
        );
        assert_eq!(
            run_source(&neg(&suite.join("missing.n3")), &base, &suite, false),
            Verdict::Pass
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The suite's cases read fixtures by their published codeberg URL; those
    /// map into the checkout (any branch or commit), still confined to it.
    #[test]
    fn suite_fixture_urls_resolve_into_checkout() {
        let dir = tmp("fixture");
        std::fs::create_dir_all(dir.join("suite/sub")).unwrap();
        let data = "@prefix : <urn:example:>.\n:a :b :c.\n";
        std::fs::write(dir.join("suite/HELLO.n3"), data).unwrap();
        std::fs::write(dir.join("suite/sub/a b.n3"), data).unwrap();
        std::fs::write(dir.join("outside.n3"), data).unwrap();
        let suite = dir.join("suite");
        let doc = |iri: &str| {
            format!(
                "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}",
                case(
                    "success-semantics-1",
                    &format!(
                        "<{iri}> log:semantics ?f. ?f log:includes {{ <urn:example:a> <urn:example:b> <urn:example:c> }}"
                    ),
                    "true"
                )
            )
        };
        let base = file_iri(&suite.join("t.n3"));
        let raw = "https://codeberg.org/phochste/notation3tests/raw";
        for iri in [
            format!("{raw}/branch/main/HELLO.n3"),
            format!("{raw}/branch/dev/HELLO.n3"),
            format!("{raw}/commit/0123abc/HELLO.n3"),
            format!("{raw}/branch/main/sub/a%20b.n3"),
        ] {
            assert_eq!(
                run_source(&doc(&iri), &base, &suite, false),
                Verdict::Pass,
                "{iri}"
            );
        }
        // `..` (encoded) cannot climb out of the checkout: the escape is refused
        // (unavailable). A missing fixture is an ordinary, engine-visible miss.
        for iri in [
            format!("{raw}/branch/main/%2E%2E/outside.n3"),
            format!("{raw}/branch/main/sub/%2E%2E/%2E%2E/outside.n3"),
        ] {
            assert_eq!(
                run_source(&doc(&iri), &base, &suite, false),
                Verdict::Unavailable(iri.clone()),
                "{iri}"
            );
        }
        let missing = format!("{raw}/branch/main/NoSuchFile.n3");
        assert_eq!(
            run_source(&doc(&missing), &base, &suite, false),
            Verdict::Incomplete
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A case that needs an external resource the offline runner cannot serve
    /// is reported as unavailable, apart from engine failures. A negative case
    /// "passes" by deriving nothing, which the missing resource alone explains,
    /// so it is unavailable too; only a derived pass signal stands.
    #[test]
    fn external_resources_are_reported_unavailable() {
        let ext = "https://example.net/data.n3";
        let needs = format!(
            "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}",
            case(
                "success-ext-1",
                &format!("<{ext}> log:semantics ?f. ?f log:includes {{ <urn:example:a> <urn:example:b> <urn:example:c> }}"),
                "true"
            )
        );
        let v = run(&needs);
        assert_eq!(v, Verdict::Unavailable(ext.into()));
        assert_eq!(Verdict::from_line(&v.to_line()), v);
        assert!(!v.is_pass());
        let tolerant = format!(
            "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}",
            case("fail-ext-1", &format!("<{ext}> log:semantics ?f"), "false")
        );
        assert_eq!(run(&tolerant), Verdict::Unavailable(ext.into()));
        let rejecting =
            format!("{PFX}{{ <{ext}> <http://www.w3.org/2000/10/swap/log#semantics> ?f }} => {{ :a :b :c }}.\n");
        assert_eq!(run_rejecting(&rejecting), Verdict::Unavailable(ext.into()));
        let positive = format!(
            "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}{{ <{ext}> log:semantics ?f }} => {{ :other :x :y }}.\n",
            case("success-ext-2", ":a :b :c", "true")
        ) + ":a :b :c.\n";
        assert_eq!(run(&positive), Verdict::Pass);
        let rows = [Row {
            test: "t".into(),
            verdict: v,
        }];
        let js: serde_json::Value = serde_json::from_str(&report_json(&rows, "r")).unwrap();
        assert_eq!(js["buckets"]["unavailable"], 1);
        assert!(report_markdown(&rows, "r").contains("| unavailable | 1 |"));
    }

    /// An unresolvable root (here the empty path, whose `starts_with` would
    /// match everything) refuses every local lookup instead of allowing it.
    #[test]
    fn unresolvable_root_fails_closed() {
        let dir = tmp("noroot");
        std::fs::write(
            dir.join("data.n3"),
            "@prefix : <urn:example:>.\n:a :b :c.\n",
        )
        .unwrap();
        let target = file_iri(&dir.join("data.n3"));
        let src = format!(
            "@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n{}",
            case(
                "success-semantics-1",
                &format!("<{target}> log:semantics ?f. ?f log:includes {{ <urn:example:a> <urn:example:b> <urn:example:c> }}"),
                "true"
            )
        );
        for root in [Path::new(""), Path::new("/nonexistent-n3t-root")] {
            assert_eq!(
                run_source(&src, &target, root, false),
                Verdict::Unavailable(target.clone()),
                "{}",
                root.display()
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn numeric_boolean_spellings_are_read_by_value() {
        let p = format!("{PFX}{{}} => {{ :test :contains :success-b }}.\n");
        let one = format!("{p}:test :is \"1\"^^xsd:boolean.\n");
        assert_eq!(run(&one), Verdict::Pass);
        let zero = format!("{p}:test :is \"0\"^^xsd:boolean.\n");
        assert_eq!(run(&zero), Verdict::Nonconform);
        let true_and_zero = format!("{p}:test :is true, \"0\"^^xsd:boolean.\n");
        assert_eq!(run(&true_and_zero), Verdict::Nonconform);
        let one_and_false = format!("{p}:test :is \"1\"^^xsd:boolean, false.\n");
        assert_eq!(run(&one_and_false), Verdict::Nonconform);
        let invalid = format!("{p}:test :is \"yes\"^^xsd:boolean.\n");
        assert_eq!(run(&invalid), Verdict::Incomplete);
        // A negative case: "1"^^xsd:boolean is a verdict, hence nonconform; an
        // invalid spelling is not a verdict, hence still a pass.
        let n = format!("{PFX}{{}} => {{ :test :contains :fail-b }}.\n");
        assert_eq!(
            run(&format!("{n}:test :is \"1\"^^xsd:boolean.\n")),
            Verdict::Nonconform
        );
        assert_eq!(
            run(&format!("{n}:test :is \"yes\"^^xsd:boolean.\n")),
            Verdict::Pass
        );
    }

    /// A symlink planted in a fetched suite must not make the runner read a
    /// file outside the suite root.
    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_suite() {
        use std::os::unix::fs::symlink;
        let dir = tmp("symlink");
        std::fs::create_dir_all(dir.join("suite/tests")).unwrap();
        std::fs::create_dir_all(dir.join("private")).unwrap();
        let secret = format!("{PFX}{{}} => {{ :test :contains :success-x }}.\n:test :is true.\n");
        std::fs::write(dir.join("private/secret.n3"), &secret).unwrap();
        std::fs::write(dir.join("suite/tests/ok.n3"), &secret).unwrap();
        symlink(
            dir.join("private/secret.n3"),
            dir.join("suite/tests/leak.n3"),
        )
        .unwrap();
        symlink(dir.join("private"), dir.join("suite/tests/linkdir")).unwrap();
        let suite = dir.join("suite");
        let found: Vec<String> = discover(&suite)
            .unwrap()
            .iter()
            .map(|p| p.strip_prefix(&suite).unwrap().display().to_string())
            .collect();
        assert_eq!(found, ["tests/ok.n3"]);
        // Independently of discovery, run_one refuses an input that resolves
        // outside the suite root, and does not echo its content.
        let v = run_one(&suite.join("tests/leak.n3"), &suite);
        assert!(
            matches!(&v, Verdict::Crashed(e) if e.contains("outside")),
            "{v:?}"
        );
        let v = run_one(&dir.join("private/secret.n3"), &suite);
        assert!(
            matches!(&v, Verdict::Crashed(e) if e.contains("outside")),
            "{v:?}"
        );
        assert_eq!(run_one(&suite.join("tests/ok.n3"), &suite), Verdict::Pass);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn report_counts_buckets() {
        let rows = vec![
            Row {
                test: "a.n3".into(),
                verdict: Verdict::Pass,
            },
            Row {
                test: "b.n3".into(),
                verdict: Verdict::Incomplete,
            },
        ];
        let md = report_markdown(&rows, "abc");
        assert!(md.contains("**1 / 2 pass (50.0%)**"), "{md}");
        assert!(md.contains("| `b.n3` | FAIL incomplete |"), "{md}");
        let js: serde_json::Value = serde_json::from_str(&report_json(&rows, "abc")).unwrap();
        assert_eq!(js["pass"], 1);
        assert_eq!(js["buckets"]["incomplete"], 1);
        assert_eq!(js["buckets"]["nonconform"], 0);
    }
}
