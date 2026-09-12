//! [FABLE-5] sq-xg6uu — the OWL 2 RL differential-vs-external-oracle test (the
//! bead's ACCEPTANCE test):
//!
//! ```text
//! cargo test -p sparq-reason-diff --features rl-oracle \
//!     rl_differential_vs_owlrl_golden -- --nocapture
//! ```
//!
//! Walks every `bench/reason-diff/rl/<case>.nt` + its pinned `.expected` owlrl
//! golden vector (captured OFFLINE by capture.py — owlrl never runs here), diffs
//! the normalized canonical N-Triples multisets, and fails loud on any
//! UNdispositioned divergence (or any stale/drifted disposition). Prints the
//! per-case ledger under `--nocapture`.

use sparq_reason_diff::rl;
use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/reason-diff/rl")
}

/// The corpus cases: every `.nt` file, each REQUIRED to have a `.expected` golden
/// vector; `.disposition` is optional. Orphan `.expected`/`.disposition` files
/// (no matching `.nt`) fail — the fixture set can never rot silently.
fn cases() -> Vec<(String, PathBuf)> {
    let dir = corpus_dir();
    let mut nt = Vec::new();
    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("bench/reason-diff/rl exists") {
        let path = entry.expect("readable dir entry").path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("nt") => nt.push((
                path.file_stem().unwrap().to_str().unwrap().to_string(),
                path.clone(),
            )),
            Some("expected") | Some("disposition") if !path.with_extension("nt").exists() => {
                orphans.push(path.display().to_string());
            }
            _ => {}
        }
    }
    assert!(
        orphans.is_empty(),
        "orphan fixture files (no matching .nt): {orphans:?}"
    );
    assert!(!nt.is_empty(), "empty corpus in {}", dir.display());
    nt.sort();
    nt
}

/// [OPUS-5] sq-ovl6f — the corpus's W3C-vocabulary-subject invariant as its own
/// gate, so a fixture edit that would mask a divergence fails with a message naming
/// the offending DOCUMENT rather than only surfacing inside a case run.
/// `rl::sparq_rl_closure_lines` enforces the same constraint on the closure of every
/// case the differential test runs (the entailed channel this lint cannot see).
#[test]
fn corpus_documents_have_no_w3c_vocab_subjects() {
    let mut failures = Vec::new();
    for (name, nt_path) in cases() {
        let nt_text = std::fs::read_to_string(&nt_path).expect("readable .nt case");
        if let Err(report) = rl::lint_corpus_document(&nt_text) {
            failures.push(format!("{name}: {report}"));
        }
    }
    assert!(
        failures.is_empty(),
        "corpus documents violating the no-user-data-at-a-W3C-vocab-subject \
         invariant:\n{}",
        failures.join("\n")
    );
}

#[test]
fn rl_differential_vs_owlrl_golden() {
    let mut failures = Vec::new();
    println!("== sparq-reason OWL 2 RL vs pinned owlrl golden vectors ==");
    for (name, nt_path) in cases() {
        let nt_text = std::fs::read_to_string(&nt_path).expect("readable .nt case");
        let golden = rl::load_golden(&nt_path.with_extension("expected"))
            .expect("every case has a valid pinned .expected golden vector");
        let disp_path = nt_path.with_extension("disposition");
        let disposition = disp_path
            .exists()
            .then(|| rl::load_disposition(&disp_path).expect("valid disposition"));
        let outcome = rl::run_case(&name, &nt_text, &golden, disposition).expect("case runs");
        match rl::judge(&outcome) {
            Ok(verdict) => println!("  {name}: {verdict}"),
            Err(report) => {
                println!("  {name}: FAIL\n{report}");
                failures.push(name);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "undispositioned / drifted divergences in: {failures:?} (ledger above)"
    );
}

/// NON-VACUITY witness (the bead's ACCEPTANCE second clause): deliberately mutate a
/// real golden vector — inject one bogus oracle triple and drop one real one — and
/// prove the harness goes RED on both sides of the multiset diff. If the differ,
/// the normalizer, or the judge were vacuous (e.g. comparing a side to itself),
/// this test fails.
#[test]
fn mutated_golden_vector_goes_red() {
    // Baseline: the first UNdispositioned case (a dispositioned one has a non-empty
    // pinned diff, so its pristine run would not be a clean MATCH to mutate from).
    let (name, nt_path) = cases()
        .into_iter()
        .find(|(_, p)| !p.with_extension("disposition").exists())
        .expect("at least one exact-match case in the corpus");
    let nt_text = std::fs::read_to_string(&nt_path).expect("readable .nt case");
    let mut golden =
        rl::load_golden(&nt_path.with_extension("expected")).expect("valid golden vector");

    // The pristine vector must pass (otherwise this witness proves nothing).
    let pristine = rl::run_case(&name, &nt_text, &golden, None).expect("case runs");
    rl::judge(&pristine).expect("pristine case matches — mutate from a green baseline");

    // Mutation 1: the oracle "entails" a triple sparq does not derive.
    let bogus = "<http://ex.org/mutant> <http://ex.org/p> <http://ex.org/mutant2> .".to_string();
    golden.lines.push(bogus.clone());
    golden.lines.sort_unstable();
    // Mutation 2: remove one USER-SPACE triple the oracle really entails (a
    // normalized-away system triple would be invisible — pick from the normalized
    // view to guarantee visibility).
    let dropped = rl::normalize(&golden.lines)
        .into_iter()
        .find(|l| *l != bogus)
        .expect("golden vector has a normalized user-space triple");
    let pos = golden.lines.iter().position(|l| *l == dropped).unwrap();
    golden.lines.remove(pos);

    let outcome = rl::run_case(&name, &nt_text, &golden, None).expect("case runs");
    let report = rl::judge(&outcome).expect_err("mutated golden vector MUST go red");
    assert!(
        report.contains(&bogus),
        "missing-side mutation not detected:\n{report}"
    );
    assert!(
        report.contains(&dropped),
        "extra-side mutation not detected:\n{report}"
    );
}
