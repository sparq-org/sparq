//! Every budgeted or fallible step of N3 evaluation goes through `src/n3/bounded.rs`:
//! its `Bounded<T>` result is `#[must_use]` with private contents, and `settle` is the
//! only way to read it, recording a cut so the negation gate can refuse an incomplete
//! store. That type is the real guarantee. This test is a tripwire on top: it fails when
//! an N3 evaluator source (outside its test modules)
//!
//! * discards an error (`.ok()`, `if let Ok(`, `.err()`, `Err(_)`, `.is_ok()`,
//!   `.is_err()`) without a `// no-match: <reason>` (a spec-defined no-match, such as
//!   an ill-typed lexical form) or `// not-a-cut: <reason>` marker on that line or the
//!   line above;
//! * names a limit (a limit constant, a `max_*` / `*_limit` / `*_budget` / `*_cap` /
//!   `*_threshold` field or config, `Regex::new`, `RegexBuilder`) without a
//!   `// not-a-limit: <reason>` marker, unless the line goes through `bounded::`;
//! * compares, ranges, takes or caps against a numeric literal of 1000 or more without
//!   such a marker.
//!
//! It cannot see every implicit limit (recursion depth, a third-party crate's internal
//! caps, a limit spelled some other way); those still have to surface as a `Bounded`.

use regex::Regex;

const EVALUATORS: &[&str] = &[
    "src/n3/mod.rs",
    "src/n3/compiled.rs",
    "src/n3/strata.rs",
    "src/n3/parser.rs",
    "src/n3/model.rs",
    "src/n3/serialize.rs",
    "src/incremental.rs",
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
fn errors_and_limits_go_through_the_bounded_module() {
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
    let discard = Regex::new(r"\.ok\(\)|if let Ok\(|\.err\(\)|Err\(_\)|\.is_ok\(\)|\.is_err\(\)")
        .expect("discard pattern");
    let limit_name = Regex::new(
        r"(?i)\b(bw_depth|containment_budget|list_walk_cap|parse_depth|max_depth|epoch_year_cap|max_\w+|\w+_limit|\w+_budget|\w+_cap|\w+_threshold|limit|budget)\b|Regex::new|RegexBuilder",
    )
    .expect("limit pattern");
    let limit_literal = Regex::new(
        r"(<=?|>=?|==|!=|\.\.=?|\.take\(|\.min\(|\.max\(|=)\s*\d[\d_]{3,}\b|\b\d[\d_]{3,}\s*(<=?|>=?|==|!=|\.\.)",
    )
    .expect("literal pattern");
    let mut hits = Vec::new();
    for rel in EVALUATORS {
        let Ok(src) = std::fs::read_to_string(format!("{root}/{rel}")) else {
            continue; // an optional module absent from this checkout
        };
        for (n, code, comments) in code_lines(&src) {
            let marked = |m: &str| comments.contains(m);
            if discard.is_match(&code) && !marked("// no-match:") && !marked("// not-a-cut:") {
                hits.push(format!("{rel}:{n}: discarded error: {}", code.trim()));
            }
            let limit = limit_name.is_match(&code) || limit_literal.is_match(&code);
            if limit && !code.contains("bounded::") && !marked("// not-a-limit:") {
                hits.push(format!("{rel}:{n}: limit outside bounded.rs: {}", code.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "unsettled errors or limits outside src/n3/bounded.rs:\n{}",
        hits.join("\n")
    );
}
