//! Every budgeted or fallible step of N3 evaluation goes through `src/n3/bounded.rs`,
//! whose `settle` records a cut on the run so the negation gate can refuse an incomplete
//! store. This test fails if an N3 evaluator source names a limit, or compiles a regex,
//! anywhere else: such a step could stop early without the gate ever knowing.

const EVALUATORS: &[&str] = &[
    "src/n3/mod.rs",
    "src/n3/compiled.rs",
    "src/n3/strata.rs",
    "src/n3/parser.rs",
    "src/n3/model.rs",
    "src/n3/serialize.rs",
    "src/incremental.rs",
];

/// Names that only `bounded.rs` may use: its limit constants, a raw regex compile, and
/// the literal value the old inline caps used.
const FORBIDDEN: &[&str] = &[
    "BW_DEPTH",
    "CONTAINMENT_BUDGET",
    "LIST_WALK_CAP",
    "Regex::new",
    "RegexBuilder",
    "100_000",
];

#[test]
fn limits_and_regex_compiles_live_only_in_the_bounded_module() {
    let root = env!("CARGO_MANIFEST_DIR");
    let n3_dir = format!("{root}/src/n3");
    // Every N3 source file is listed: a new evaluator file must join the check.
    for entry in std::fs::read_dir(&n3_dir).expect("src/n3") {
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
    let mut hits = Vec::new();
    for rel in EVALUATORS {
        let path = format!("{root}/{rel}");
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue; // an optional module absent from this checkout
        };
        for (i, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for name in FORBIDDEN {
                if code.contains(name) {
                    hits.push(format!("{rel}:{}: {name}: {}", i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "limits / regex compiles outside src/n3/bounded.rs:\n{}",
        hits.join("\n")
    );
}
