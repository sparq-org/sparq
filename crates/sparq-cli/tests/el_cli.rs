//! [OPUS-5] (sq-2ch27, Phase E6 of research/owl2-el-ql-reasoning-spike.md §5) The OWL 2 EL
//! classifier CLI surface end-to-end: `classify <data-file> <format> [out.nt]` and
//! `--reason el` / `reason <f> <fmt> el`.
//!
//! Spawns the *built* `sparq-cli` binary (via `CARGO_BIN_EXE_sparq-cli`, the same mechanism as
//! `dump_cli.rs` / `terse_cli.rs`). The surface lives behind the opt-in `el` feature, so this
//! whole file compiles and runs only under it: `cargo test -p sparq-cli --features el`.
//! The feature-OFF half of the contract (a `--reason el` that refuses to fall back to `owl`)
//! is pinned separately in `error_paths_cli.rs`, which runs in the DEFAULT build.
#![cfg(feature = "el")]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh scratch dir under the cargo per-test-target tmp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("el-cli-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write fixture");
    p
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// (exit code, stdout, stderr) from the built binary.
fn run3(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(args)
        .output()
        .expect("spawning sparq-cli");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `A ⊑ ∃r.B`, `B ⊑ C`, `∃r.C ⊑ D` ⊨ `A ⊑ D` — the CR3/CR4 existential-successor derivation
/// (the spike's §1.3 shape). Exercises the EL completion rules through the CLI.
const EXISTENTIAL: &str = r#"
@prefix : <http://ex/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
:B rdfs:subClassOf :C .
[ owl:onProperty :r ; owl:someValuesFrom :C ] rdfs:subClassOf :D .
"#;

/// `B ⊑ C`, `B ⊑ E`, `C ⊓ E ⊑ D` ⊨ `B ⊑ D` — completion rule **CR2** (conjunction on the LEFT
/// of an axiom, composed from two separately-derived memberships).
///
/// This is the fixture the RL-differential test below uses, because OWL 2 RL genuinely cannot
/// derive it: `scm-int` only DECOMPOSES an intersection (`C ⊓ E ⊑ C`, `C ⊓ E ⊑ E`), and the
/// COMPOSITION direction (`B ⊑ C`, `B ⊑ E` ⊢ `B ⊑ C ⊓ E`) exists in the RL/RDF rule set only
/// as the assertional `cls-int1`, over individuals — never over classes. Verified empirically
/// against this repo's own `--reason owl`, not assumed (see `el_derives_what_rl_cannot`).
const CONJUNCTION: &str = r#"
@prefix : <http://ex/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:B rdfs:subClassOf :C , :E .
[ owl:intersectionOf ( :C :E ) ] rdfs:subClassOf :D .
"#;

/// `Bad ⊑ ∃age.(xsd:integer, [18, 10])` — a CONCRETE-DOMAIN axiom that is squarely INSIDE the
/// OWL 2 EL profile. With `sparq-reason-el/cdomain` the empty facet range is ⊑ ⊥ (CR7) and the
/// clash propagates to `Bad` (CR5); the CLI's `el` feature does NOT enable `cdomain`, so the
/// axiom is deferred instead. Fixture mirrors `sparq-reason-el/tests/cdomain.rs`.
const CONCRETE_DOMAIN: &str = r#"
@prefix : <http://ex/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
:Bad rdfs:subClassOf
  [ owl:onProperty :age ; owl:someValuesFrom
    [ owl:onDatatype xsd:integer ;
      owl:withRestrictions ( [ xsd:minInclusive 18 ] [ xsd:maxInclusive 10 ] ) ] ] .
"#;

const SUB_CLASS_OF: &str = "<http://www.w3.org/2000/01/rdf-schema#subClassOf>";
const SUB_PROPERTY_OF: &str = "<http://www.w3.org/2000/01/rdf-schema#subPropertyOf>";

/// `classify` materializes the lattice (complete for the E1+E2 fragment the CLI's `el` feature
/// enables — `rbox` on, `cdomain` off), reports it as `name<TAB>value` lines on stdout, and
/// (with `out.nt`) writes the augmented graph as N-Triples.
#[test]
fn classify_emits_the_existential_subsumption() {
    let dir = scratch("classify");
    let ttl = write(&dir, "ex.ttl", EXISTENTIAL);
    let out = dir.join("closure.nt");

    let (code, stdout, stderr) = run3(&["classify", s(&ttl), "turtle", s(&out)]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("emitted_subclassof\t1"), "stdout: {stdout}");
    assert!(stdout.contains("skipped_axioms\t0"), "stdout: {stdout}");
    assert!(stdout.contains("unsatisfiable_classes\t0"), "stdout: {stdout}");
    assert!(stdout.contains("thing_unsatisfiable\tfalse"), "stdout: {stdout}");

    let closure = std::fs::read_to_string(&out).expect("read closure");
    assert!(
        closure.contains(&format!("<http://ex/A> {SUB_CLASS_OF} <http://ex/D> .")),
        "the CR4 subsumption A ⊑ D must be materialized: {closure}"
    );
}

/// The HONESTY witness for the narrowed capability claim: the CLI's `el` feature is E1+E2
/// (`rbox` on, `cdomain` OFF), so a *valid EL* concrete-domain axiom is counted in
/// `skipped_axioms` and NOT applied — `Bad ⊑ ∃age.(xsd:integer, [18, 10])` stays satisfiable
/// here even though CR7+CR5 would refute it with `cdomain` on. This is why the README / skills
/// scope the CLI surface to E1+E2 rather than claiming complete OWL 2 EL classification. If a
/// later change forwards `cdomain`, this test goes red and the docs must be re-scoped with it.
#[test]
fn concrete_domain_axioms_are_deferred_not_applied() {
    let dir = scratch("cdomain-deferred");
    let ttl = write(&dir, "ex.ttl", CONCRETE_DOMAIN);

    let (code, stdout, stderr) = run3(&["classify", s(&ttl), "turtle"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        !stdout.contains("skipped_axioms\t0"),
        "the concrete-domain axiom must be COUNTED as skipped without `cdomain`: {stdout}"
    );
    assert!(
        stdout.contains("unsatisfiable_classes\t0"),
        "without `cdomain` the empty [18, 10] range is never decided, so Bad is not refuted \
         — the documented incompleteness: {stdout}"
    );
    assert!(
        stderr.contains("cdomain"),
        "the stderr NOTE must name the `cdomain` deferral, not blame constructs outside EL: {stderr}"
    );
}

/// The differential that makes the `el` surface worth having: `--reason el` derives a class
/// subsumption `--reason owl` does not. Both profiles run over the SAME file in the SAME
/// binary, so the difference is the calculus, not the setup.
#[test]
fn el_derives_what_rl_cannot() {
    let dir = scratch("differential");
    let ttl = write(&dir, "ex.ttl", CONJUNCTION);
    let q = format!(
        "SELECT ?sup WHERE {{ <http://ex/B> {SUB_CLASS_OF} ?sup FILTER(?sup = <http://ex/D>) }}"
    );
    let ask = |profile: &str| run3(&["query", s(&ttl), "turtle", &q, "--reason", profile, "--format", "tsv"]);

    let (code, el_out, err) = ask("el");
    assert_eq!(code, 0, "stderr: {err}");
    assert!(el_out.contains("<http://ex/D>"), "EL must derive B ⊑ D (CR2): {el_out}");

    let (code, rl_out, err) = ask("owl");
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        !rl_out.contains("<http://ex/D>"),
        "OWL 2 RL is sound but INCOMPLETE here — if this ever starts passing, the \
         'use el, not --reason owl' note in the docs needs re-deriving: {rl_out}"
    );
}

/// The standalone `reason <f> <fmt> el [out.nt]` positional form routes to the same classifier
/// as the `--reason el` flag.
#[test]
fn reason_subcommand_accepts_el() {
    let dir = scratch("reason");
    let ttl = write(&dir, "ex.ttl", EXISTENTIAL);
    let out = dir.join("closure.nt");

    let (code, stdout, stderr) = run3(&["reason", s(&ttl), "turtle", "el", s(&out)]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("triples after el reasoning"), "stdout: {stdout}");
    assert!(stderr.contains("classified [OWL 2 EL]"), "stderr: {stderr}");

    let closure = std::fs::read_to_string(&out).expect("read closure");
    assert!(
        closure.contains(&format!("<http://ex/A> {SUB_CLASS_OF} <http://ex/D> .")),
        "{closure}"
    );
}

/// The CLI's `el` feature pulls `sparq-reason-el/rbox` (Phase E2 — the bead depends on E1+E2),
/// so the role-lattice readoff must be live: a told `r ⊑ s ⊑ t` chain yields the derived
/// non-reflexive pair `r ⊑ t` as an `rdfs:subPropertyOf` triple.
#[test]
fn rbox_role_lattice_is_enabled() {
    let dir = scratch("rbox");
    let ttl = write(
        &dir,
        "roles.ttl",
        "@prefix : <http://ex/> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         :r rdfs:subPropertyOf :s .\n\
         :s rdfs:subPropertyOf :t .\n",
    );
    let out = dir.join("closure.nt");

    let (code, stdout, stderr) = run3(&["classify", s(&ttl), "turtle", s(&out)]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("emitted_subpropertyof\t1"), "stdout: {stdout}");
    assert!(stdout.contains("rbox_non_regular\tfalse"), "stdout: {stdout}");

    let closure = std::fs::read_to_string(&out).expect("read closure");
    assert!(
        closure.contains(&format!("<http://ex/r> {SUB_PROPERTY_OF} <http://ex/t> .")),
        "the CR10 role-inclusion closure must be materialized: {closure}"
    );
}

/// `classify` with too few positionals is a usage error (exit 2), matching every other
/// subcommand's hand-rolled contract.
#[test]
fn classify_usage_error_exits_2() {
    let (code, _, err) = run3(&["classify"]);
    assert_eq!(code, 2);
    assert!(err.contains("usage: sparq-cli classify"), "stderr: {err}");
}
