//! Every budgeted or fallible step of N3 evaluation goes through `src/n3/bounded.rs`:
//! its `Bounded<T>` result is `#[must_use]` with private contents, and `settle` is the
//! only way to read it, recording a cut so the negation gate can refuse an incomplete
//! store. That type is the real guarantee. This test is a tripwire on top: it fails when
//! an N3 evaluator source (outside its test modules)
//!
//! * discards an error (`.ok()`, `if let Ok(`, `.err()`, `Err(_)`, `.is_ok()`,
//!   `.is_err()`) without a `// no-match: <category>` (a spec-defined no-match, such as
//!   an ill-typed lexical form) or `// not-a-cut: <category>` marker on that line or the
//!   line above;
//! * names a limit (a limit constant, a `max_*` / `*_limit` / `*_budget` / `*_cap` /
//!   `*_threshold` field or config, `Regex::new`, `RegexBuilder`) without a
//!   `// not-a-limit: <category>` marker, unless the line goes through `bounded::`;
//! * calls a `checked_*` / `overflowing_*` arithmetic step that neither goes through
//!   `rep(` / `bounded::` on that line nor carries a `// not-a-cut:` marker;
//! * compares, ranges, takes or caps against a numeric literal of 1000 or more without
//!   such a marker;
//! * converts a number lossily: an `as f64` / `as i64` cast, the f64 image `num(…)` or
//!   `.to_f64()`, without going through `bounded::` on that line or carrying a
//!   `// not-a-cut:` marker (the value feeds comparisons, joins and negation, so a lost
//!   digit must be a cut, or the conversion exact or the builtin defined over f64).
//!
//! It also fails when a fresh cut record (`Truncation::top_level`) is made anywhere but
//! an entry point (a `pub fn`, or a crate-internal `reason_n3*` one): a nested
//! evaluation must record into its parent's.
//!
//! Each marker must name a category from [`ALLOWED`] (`// no-match: ill-typed (…)`);
//! an unknown category fails, so a marker cannot be an unreviewed free-text waiver.
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

/// The marker categories, each with the reason it is not a cut:
const ALLOWED: &[(&str, &str)] = &[
    // The lexical form is not of the type: the builtin is false by definition.
    ("no-match", "ill-typed"),
    // Not part of evaluating a premise (e.g. serializer labels).
    ("not-a-cut", "not-evaluation"),
    // The caller records the cut (e.g. `numval_in` over `numval`).
    ("not-a-cut", "settled-by-caller"),
    // The value is computed exactly on the digits, so nothing is lost.
    ("not-a-cut", "exact-on-digits"),
    // A numeric conversion that cannot lose precision (a count, or a value checked to be
    // whole and below 2^53).
    ("not-a-cut", "exact-cast"),
    // The builtin is defined over f64 values (the trig/log family, double arithmetic,
    // the math: comparisons until GH #6745), so the f64 image is its value.
    ("not-a-cut", "defined-float"),
    // A data value that is named like a limit (an ontology cardinality).
    ("not-a-limit", "data-value"),
    // A switch between sequential and parallel evaluation of the same result.
    ("not-a-limit", "parallelism"),
    // A join budget type that never stops the join.
    ("not-a-limit", "unbounded-join"),
];

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
    // A checked or overflowing step whose `None` is not settled as a cut.
    let checked = Regex::new(r"\.(checked|overflowing)_\w+\(").expect("checked pattern");
    // A numeric conversion that can lose digits.
    let lossy = Regex::new(r"\bas\s+(f64|i64)\b|(^|[^\w:])num\(|\.to_f64\(\)|NumVal::to_f64")
        .expect("lossy pattern");
    // Every marker names one allowed category; an unknown one fails.
    let marker = Regex::new(r"//\s*(no-match|not-a-cut|not-a-limit):\s*([a-z-]*)").expect("marker");
    let mut hits = Vec::new();
    for rel in EVALUATORS {
        let Ok(src) = std::fs::read_to_string(format!("{root}/{rel}")) else {
            continue; // an optional module absent from this checkout
        };
        for (n, line) in src.lines().enumerate() {
            for m in marker.captures_iter(line) {
                let (kind, cat) = (&m[1], &m[2]);
                if !ALLOWED.contains(&(kind, cat)) {
                    hits.push(format!(
                        "{rel}:{}: marker `{kind}: {cat}` is not an allowed category",
                        n + 1
                    ));
                }
            }
        }
        // The function each code line sits in: a fresh cut record (`Truncation::top_level`)
        // is made only by an entry point; every nested evaluation reuses its parent's.
        let mut current_fn = String::new();
        for (n, code, comments) in code_lines(&src) {
            if code.starts_with("pub fn ")
                || code.starts_with("fn ")
                || code.starts_with("pub(crate) fn ")
            {
                current_fn = code.trim().to_string();
            }
            let entry = current_fn.starts_with("pub fn ")
                || current_fn.starts_with("pub(crate) fn reason_n3");
            if code.contains("Truncation::top_level(") && !entry {
                hits.push(format!(
                    "{rel}:{n}: a fresh cut record outside an entry point ({current_fn}): {}",
                    code.trim()
                ));
            }
            let marked = |m: &str| comments.contains(m);
            if discard.is_match(&code) && !marked("// no-match:") && !marked("// not-a-cut:") {
                hits.push(format!("{rel}:{n}: discarded error: {}", code.trim()));
            }
            if checked.is_match(&code)
                && !code.contains("rep(")
                && !code.contains("bounded::")
                && !marked("// not-a-cut:")
            {
                hits.push(format!(
                    "{rel}:{n}: unsettled checked arithmetic: {}",
                    code.trim()
                ));
            }
            if lossy.is_match(&code)
                && !code.contains("bounded::")
                && !code.contains("fn num(")
                && !marked("// not-a-cut:")
            {
                hits.push(format!(
                    "{rel}:{n}: unsettled lossy numeric conversion: {}",
                    code.trim()
                ));
            }
            let limit = limit_name.is_match(&code) || limit_literal.is_match(&code);
            if limit && !code.contains("bounded::") && !marked("// not-a-limit:") {
                hits.push(format!(
                    "{rel}:{n}: limit outside bounded.rs: {}",
                    code.trim()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "unsettled errors or limits outside src/n3/bounded.rs:\n{}",
        hits.join("\n")
    );
}
