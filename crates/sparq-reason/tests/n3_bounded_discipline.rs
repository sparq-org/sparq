//! Every limit of N3 evaluation lives in `src/n3/bounded.rs`, private, and is reachable
//! only through that module, whose limited steps return a `#[must_use]` `Bounded<T>` that
//! `settle` alone can read. That type is the real guarantee. This test is a tripwire on
//! top: it fails when an N3 evaluator source (outside its test modules)
//!
//! * names a limit (one of the limit constants, `MAX_DEPTH`, any `max_*` / `*_limit` /
//!   `*_budget` / `*_cap` / `*_threshold` field or config, `limit`, `budget`,
//!   `Regex::new`, `RegexBuilder`) without going through `bounded::` on that line;
//! * compares, ranges, takes or caps against a numeric literal of 1000 or more;
//!
//! unless the line, or the line above, carries a `// not-a-limit: <category>` marker
//! whose category is in [`ALLOWED`]. An unknown category fails, so a marker is never a
//! free-text waiver. It also fails when a new `src/n3/*.rs` file is not on its list, and
//! when a fresh cut record (`Cuts::top_level`) is made anywhere but an entry point (a
//! `pub fn`, or a crate-internal `reason_n3*` one): a nested evaluation must record into
//! its parent's.
//!
//! A text scan cannot see every implicit limit (recursion depth, a third-party crate's
//! internal caps, a limit spelled some other way).

use regex::Regex;

const EVALUATORS: &[&str] = &[
    "src/n3/mod.rs",
    "src/n3/compiled.rs",
    "src/n3/parser.rs",
    "src/n3/model.rs",
    "src/n3/serialize.rs",
    "src/incremental.rs",
];

/// The `not-a-limit` categories, each with why it is not a limit on evaluation:
const ALLOWED: &[&str] = &[
    // A data value that is named like a limit (an ontology cardinality).
    "data-value",
    // A switch between sequential and parallel evaluation of the same result.
    "parallelism",
    // A join budget type that never stops the join.
    "unbounded-join",
];

/// The code lines of `src` before its first `#[cfg(test)] mod …`, with their numbers
/// and the comment text of the line and of the line above (where markers live).
fn code_lines(src: &str) -> Vec<(usize, String, String)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let next_is_mod = lines
            .get(i + 1)
            .is_some_and(|n| n.trim_start().starts_with("mod "));
        if line.trim() == "#[cfg(test)]" && next_is_mod {
            break;
        }
        let (code, comment) = match line.find("//") {
            Some(k) => (&line[..k], &line[k..]),
            None => (*line, ""),
        };
        let above = if i > 0 { lines[i - 1] } else { "" };
        out.push((i + 1, code.to_string(), format!("{comment} {above}")));
    }
    out
}

#[test]
fn limits_live_in_the_bounded_module() {
    let root = env!("CARGO_MANIFEST_DIR");
    // Every N3 source file is listed: a new evaluator file must join the check.
    for entry in std::fs::read_dir(format!("{root}/src/n3")).expect("src/n3") {
        let name = entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        if name.ends_with(".rs") && name != "bounded.rs" {
            let rel = format!("src/n3/{name}");
            assert!(
                EVALUATORS.contains(&rel.as_str()),
                "{rel} is not checked; add it"
            );
        }
    }
    let limit_name = Regex::new(
        r"(?i)\b(bw_depth|containment_budget|list_walk_cap|parse_depth|max_depth|epoch_year_cap|max_\w+|\w+_limit|\w+_budget|\w+_cap|\w+_threshold|limit|budget)\b|Regex::new|RegexBuilder",
    )
    .expect("limit pattern");
    let limit_literal = Regex::new(
        r"(<=?|>=?|==|!=|\.\.=?|\.take\(|\.min\(|\.max\(|=)\s*\d[\d_]{3,}\b|\b\d[\d_]{3,}\s*(<=?|>=?|==|!=|\.\.)",
    )
    .expect("literal pattern");
    let marker = Regex::new(r"//\s*not-a-limit:\s*([a-z-]*)").expect("marker pattern");
    let mut hits = Vec::new();
    for rel in EVALUATORS {
        let Ok(src) = std::fs::read_to_string(format!("{root}/{rel}")) else {
            continue; // an optional module absent from this checkout
        };
        for (n, line) in src.lines().enumerate() {
            for m in marker.captures_iter(line) {
                if !ALLOWED.contains(&&m[1]) {
                    hits.push(format!(
                        "{rel}:{}: `not-a-limit: {}` is not an allowed category",
                        n + 1,
                        &m[1]
                    ));
                }
            }
        }
        // The function each code line sits in: a fresh cut record (`Cuts::top_level`) is
        // made only by a `pub` entry point; every nested evaluation reuses its parent's.
        let mut current_fn = String::new();
        for (n, code, comments) in code_lines(&src) {
            let t = code.trim_start();
            if t.starts_with("pub fn ") || t.starts_with("fn ") || t.starts_with("pub(crate) fn ") {
                current_fn = t.to_string();
            }
            let entry = current_fn.starts_with("pub fn ")
                || current_fn.starts_with("pub(crate) fn reason_n3");
            if code.contains("Cuts::top_level(") && !entry {
                hits.push(format!(
                    "{rel}:{n}: a fresh cut record outside a public entry point ({}): {}",
                    current_fn.trim(),
                    code.trim()
                ));
            }
            let limit = limit_name.is_match(&code) || limit_literal.is_match(&code);
            if limit && !code.contains("bounded::") && !comments.contains("// not-a-limit:") {
                hits.push(format!(
                    "{rel}:{n}: limit outside bounded.rs: {}",
                    code.trim()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "limits outside src/n3/bounded.rs:\n{}",
        hits.join("\n")
    );
}
