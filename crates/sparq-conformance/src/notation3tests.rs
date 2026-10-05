//! Runner logic for the community **notation3tests** suite
//! (<https://codeberg.org/phochste/notation3tests>, issue #6467) — a corpus of
//! self-checking N3 documents collected across N3 reasoners (EYE, eyeling, cwm,
//! jen3, ...), complementary to the w3c/N3 community-group manifests that
//! `sparq-inference-conformance` already runs.
//!
//! Convention this runner assumes (the suite's own runner contract): each test
//! is ONE `.n3` document; running a reasoner over it must derive the triple
//! `:test :is true` (any IRI whose local name is `test`, predicate local name
//! `is`, object the boolean `true` in any valid `xsd:boolean` spelling, i.e.
//! `true` or `"1"^^xsd:boolean`). A closure that derives `:test :is false`
//! (`false` or `"0"^^xsd:boolean`), derives neither, errors, or exceeds the
//! timeout is a FAIL. The suite is FETCHED at run time (never vendored);
//! `.github/workflows/notation3tests.yml` fetches it and runs
//! `sparq-notation3tests`.
//!
//! This module is pure logic (discovery, verdict, report) so it can be pinned
//! by hand-written cases in the suite's format without the suite present; the
//! binary adds per-test process isolation for the timeout.

use sparq_reason::n3::Term;
use std::path::{Path, PathBuf};

const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

/// Outcome of one test document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The closure derives `:test :is true` and not `:test :is false`.
    Pass,
    /// The closure derives `:test :is false` (with or without `true`).
    False,
    /// The closure derives no `:test :is <boolean>` verdict at all.
    NoVerdict,
    /// Parse or reasoning error.
    Error(String),
    /// Exceeded the per-test timeout.
    Timeout,
}

impl Verdict {
    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Pass)
    }

    /// A single line, used as the child-process protocol and in the report.
    pub fn to_line(&self) -> String {
        match self {
            Verdict::Pass => "PASS".into(),
            Verdict::False => "FAIL false".into(),
            Verdict::NoVerdict => "FAIL no-verdict".into(),
            Verdict::Timeout => "FAIL timeout".into(),
            Verdict::Error(e) => format!("FAIL error {}", e.replace(['\n', '\r'], " ")),
        }
    }

    /// Inverse of [`Verdict::to_line`] (unknown lines become an `Error`).
    pub fn from_line(line: &str) -> Verdict {
        let line = line.trim();
        match line {
            "PASS" => Verdict::Pass,
            "FAIL false" => Verdict::False,
            "FAIL no-verdict" => Verdict::NoVerdict,
            "FAIL timeout" => Verdict::Timeout,
            _ => match line.strip_prefix("FAIL error ") {
                Some(e) => Verdict::Error(e.to_string()),
                None => Verdict::Error(format!("unrecognised child output: {line:?}")),
            },
        }
    }

    fn bucket(&self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::False => "false",
            Verdict::NoVerdict => "no-verdict",
            Verdict::Error(_) => "error",
            Verdict::Timeout => "timeout",
        }
    }
}

/// The local name of an IRI: the part after the last `#`, `/` or `:`.
fn local_name(iri: &str) -> &str {
    iri.rsplit(['#', '/', ':']).next().unwrap_or(iri)
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

/// Decide the verdict from a closure's facts.
pub fn verdict_from_facts(facts: &[[Term; 3]]) -> Verdict {
    let (mut t, mut f) = (false, false);
    for [s, p, o] in facts {
        let (Term::Iri(s), Term::Iri(p)) = (s, p) else {
            continue;
        };
        if local_name(s) != "test" || local_name(p) != "is" {
            continue;
        }
        match boolean_value(o) {
            Some(true) => t = true,
            Some(false) => f = true,
            None => {}
        }
    }
    match (t, f) {
        (_, true) => Verdict::False,
        (true, false) => Verdict::Pass,
        (false, false) => Verdict::NoVerdict,
    }
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

/// Run sparq's N3 reasoner over one test document. `log:semantics` /
/// `log:content` may read `file://` documents, but only inside `suite_root`
/// (strictly offline; nothing outside the fetched suite is readable). The
/// test document itself is held to the same rule: it is canonicalized (which
/// resolves symlinks) and refused, without being read, unless it lies inside
/// the canonical `suite_root`.
pub fn run_one(path: &Path, suite_root: &Path) -> Verdict {
    let (Ok(canon), Ok(root)) = (
        std::fs::canonicalize(path),
        std::fs::canonicalize(suite_root),
    ) else {
        return Verdict::Error(format!("cannot resolve {}", path.display()));
    };
    if !canon.starts_with(&root) {
        return Verdict::Error(format!(
            "refusing {}: resolves outside the suite root",
            path.display()
        ));
    }
    let src = match std::fs::read_to_string(&canon) {
        Ok(s) => s,
        Err(e) => return Verdict::Error(format!("read {}: {e}", path.display())),
    };
    run_source(&src, &file_iri(path), suite_root)
}

/// As [`run_one`], over in-memory source with an explicit base IRI.
pub fn run_source(src: &str, base: &str, suite_root: &Path) -> Verdict {
    let root = std::fs::canonicalize(suite_root).unwrap_or_else(|_| suite_root.to_path_buf());
    let resolver = move |iri: &str| -> Option<String> {
        let p = std::fs::canonicalize(file_iri_path(iri)?).ok()?;
        if !p.starts_with(&root) {
            return None;
        }
        std::fs::read_to_string(p).ok()
    };
    match sparq_reason::n3::reason_n3_terms_with_resolver(src, Some(base), Some(&resolver)) {
        Ok(c) => verdict_from_facts(&c.facts),
        Err(e) => Verdict::Error(e),
    }
}

/// Every `.n3` test under the suite: `<root>/test-cases` when it exists,
/// otherwise `<root>` itself; recursive, sorted, hidden dirs skipped. Symlinks
/// (to files or directories) are never followed or returned, so a fetched
/// suite cannot point the runner at files outside it.
pub fn discover(root: &Path) -> Vec<PathBuf> {
    let base = root.join("test-cases");
    let base = if base.is_dir() {
        base
    } else {
        root.to_path_buf()
    };
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
    out
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
    s.push_str("- Pass criterion: closure derives `:test :is true` and not `:test :is false`\n\n");
    s.push_str(&format!("**{pass} / {total} pass ({pct:.1}%)**\n\n"));
    s.push_str("| bucket | count |\n|---|---|\n");
    for b in ["pass", "false", "no-verdict", "error", "timeout"] {
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
    for b in ["pass", "false", "no-verdict", "error", "timeout"] {
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

    fn run(src: &str) -> Verdict {
        run_source(src, "file:///nonexistent/t.n3", Path::new("/nonexistent"))
    }

    #[test]
    fn forward_rule_deriving_true_passes() {
        let src = "@prefix : <urn:example:>.\n:a :b :c.\n{ :a :b :c } => { :test :is true }.\n";
        assert_eq!(run(src), Verdict::Pass);
    }

    #[test]
    fn hash_namespace_and_builtin_pass() {
        let src = "@prefix : <http://example.org/ns#>.\n\
                   @prefix math: <http://www.w3.org/2000/10/swap/math#>.\n\
                   { (1 2) math:sum 3 } => { :test :is true }.\n";
        assert_eq!(run(src), Verdict::Pass);
    }

    #[test]
    fn false_verdict_fails_even_alongside_true() {
        let src = "@prefix : <urn:example:>.\n:test :is true.\n{ :test :is true } => { :test :is false }.\n";
        assert_eq!(run(src), Verdict::False);
    }

    #[test]
    fn unmet_premise_is_no_verdict() {
        let src = "@prefix : <urn:example:>.\n{ :a :b :c } => { :test :is true }.\n";
        assert_eq!(run(src), Verdict::NoVerdict);
    }

    #[test]
    fn string_true_is_not_a_boolean_verdict() {
        let src = "@prefix : <urn:example:>.\n:test :is \"true\".\n";
        assert_eq!(run(src), Verdict::NoVerdict);
    }

    #[test]
    fn parse_error_is_error() {
        assert!(matches!(
            run("@prefix : <urn:example:> .\n:a :b"),
            Verdict::Error(_)
        ));
    }

    #[test]
    fn verdict_line_round_trips() {
        for v in [
            Verdict::Pass,
            Verdict::False,
            Verdict::NoVerdict,
            Verdict::Timeout,
            Verdict::Error("bad\nthing".into()),
        ] {
            let back = Verdict::from_line(&v.to_line());
            match (&v, &back) {
                (Verdict::Error(_), Verdict::Error(b)) => assert_eq!(b, "bad thing"),
                _ => assert_eq!(v, back),
            }
        }
    }

    #[test]
    fn discover_prefers_test_cases_dir_and_skips_hidden() {
        let dir = std::env::temp_dir().join(format!("n3t-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("test-cases/sub")).unwrap();
        std::fs::create_dir_all(dir.join("test-cases/.git")).unwrap();
        std::fs::write(dir.join("outside.n3"), "").unwrap();
        std::fs::write(dir.join("test-cases/a.n3"), "").unwrap();
        std::fs::write(dir.join("test-cases/sub/b.n3"), "").unwrap();
        std::fs::write(dir.join("test-cases/sub/c.ttl"), "").unwrap();
        std::fs::write(dir.join("test-cases/.git/d.n3"), "").unwrap();
        let found: Vec<String> = discover(&dir)
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().display().to_string())
            .collect();
        assert_eq!(found, ["test-cases/a.n3", "test-cases/sub/b.n3"]);
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
        let src = "@prefix : <urn:example:>.\n@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
                   { <data.n3> log:semantics ?f. ?f log:includes { :a :b :c } } => { :test :is true }.\n";
        let base = file_iri(&suite.join("t.n3"));
        assert!(!base.contains('#') && !base.contains(' '), "{base}");
        assert_eq!(
            file_iri_path(&base).unwrap(),
            std::fs::canonicalize(&suite).unwrap().join("t.n3")
        );
        assert_eq!(run_source(src, &base, &suite), Verdict::Pass);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolver_reads_inside_suite_only() {
        let dir = std::env::temp_dir().join(format!("n3t-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("suite")).unwrap();
        let data = "@prefix : <urn:example:>.\n:a :b :c.\n";
        std::fs::write(dir.join("suite/data.n3"), data).unwrap();
        std::fs::write(dir.join("outside.n3"), data).unwrap();
        let suite = dir.join("suite");
        let doc = |target: &Path| {
            format!(
                "@prefix : <urn:example:>.\n@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
                 {{ <{}> log:semantics ?f. ?f log:includes {{ :a :b :c }} }} => {{ :test :is true }}.\n",
                file_iri(target)
            )
        };
        let base = file_iri(&suite.join("t.n3"));
        assert_eq!(
            run_source(&doc(&suite.join("data.n3")), &base, &suite),
            Verdict::Pass
        );
        assert_eq!(
            run_source(&doc(&dir.join("outside.n3")), &base, &suite),
            Verdict::NoVerdict
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn numeric_boolean_spellings_are_read_by_value() {
        let p = "@prefix : <urn:example:>.\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#>.\n";
        let one = format!("{p}:test :is \"1\"^^xsd:boolean.\n");
        assert_eq!(run(&one), Verdict::Pass);
        let zero = format!("{p}:test :is \"0\"^^xsd:boolean.\n");
        assert_eq!(run(&zero), Verdict::False);
        let true_and_zero = format!("{p}:test :is true, \"0\"^^xsd:boolean.\n");
        assert_eq!(run(&true_and_zero), Verdict::False);
        let one_and_false = format!("{p}:test :is \"1\"^^xsd:boolean, false.\n");
        assert_eq!(run(&one_and_false), Verdict::False);
        let invalid = format!("{p}:test :is \"yes\"^^xsd:boolean.\n");
        assert_eq!(run(&invalid), Verdict::NoVerdict);
    }

    /// A symlink planted in a fetched suite must not make the runner read a
    /// file outside the suite root.
    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_suite() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("n3t-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("suite/test-cases")).unwrap();
        std::fs::create_dir_all(dir.join("private")).unwrap();
        let secret = "@prefix : <urn:example:>.\n:test :is true.\n";
        std::fs::write(dir.join("private/secret.n3"), secret).unwrap();
        std::fs::write(dir.join("suite/test-cases/ok.n3"), secret).unwrap();
        symlink(
            dir.join("private/secret.n3"),
            dir.join("suite/test-cases/leak.n3"),
        )
        .unwrap();
        symlink(dir.join("private"), dir.join("suite/test-cases/linkdir")).unwrap();
        let suite = dir.join("suite");
        let found: Vec<String> = discover(&suite)
            .iter()
            .map(|p| p.strip_prefix(&suite).unwrap().display().to_string())
            .collect();
        assert_eq!(found, ["test-cases/ok.n3"]);
        // Independently of discovery, run_one refuses an input that resolves
        // outside the suite root, and does not echo its content.
        let v = run_one(&suite.join("test-cases/leak.n3"), &suite);
        assert!(
            matches!(&v, Verdict::Error(e) if e.contains("outside")),
            "{v:?}"
        );
        let v = run_one(&dir.join("private/secret.n3"), &suite);
        assert!(
            matches!(&v, Verdict::Error(e) if e.contains("outside")),
            "{v:?}"
        );
        assert_eq!(
            run_one(&suite.join("test-cases/ok.n3"), &suite),
            Verdict::Pass
        );
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
                verdict: Verdict::NoVerdict,
            },
        ];
        let md = report_markdown(&rows, "abc");
        assert!(md.contains("**1 / 2 pass (50.0%)**"), "{md}");
        assert!(md.contains("| `b.n3` | FAIL no-verdict |"), "{md}");
        let js: serde_json::Value = serde_json::from_str(&report_json(&rows, "abc")).unwrap();
        assert_eq!(js["pass"], 1);
        assert_eq!(js["buckets"]["no-verdict"], 1);
    }
}
