//! [OPUS-4.8] sq-55c1 — DIFFERENTIAL SHACL fuzzing against reference engines.
//!
//! Generates random but VALID SHACL shapes graphs + data graphs, validates each
//! through `sparq-shacl` AND through one-or-more reference SHACL engines, and
//! asserts the reports agree. A disagreement is a candidate sparq-shacl bug; the
//! harness prints the seed and the (small) reproducing Turtle so it replays
//! deterministically.
//!
//! ## Why a seeded RNG loop (not proptest / cargo-fuzz)
//! This mirrors the established differential idiom in `sparq-bench/src/fuzz.rs`
//! (the engine-vs-Oxigraph SPARQL differential): a deterministic SplitMix64 over
//! a `seed_start..seed_start+count` range, one independent case per seed, so any
//! failure reproduces from its printed seed alone. `cargo-fuzz` in `fuzz/` is for
//! a different invariant (hostile bytes must not panic the parser); semantic
//! cross-engine differentials want reproducible *valid* inputs, which the seeded
//! loop gives directly.
//!
//! ## Reference engines (pluggable — sq-eifd report-cli adapters)
//! The reference side is a "report-cli" adapter (bead sq-eifd): a subprocess that
//! reads `{data, shapes}` Turtle and emits a normalised JSON report
//! (`conforms` + a `{focus, component, path}` violation list). Two references are
//! wired, each a drop-in `RefEngine` that produces the *same* JSON shape:
//!   - **pySHACL** (`tests/diff_fuzz/pyshacl_adapter.py`, bead sq-55c1) — the
//!     canonical Python SHACL reference implementation, driven via a Python
//!     interpreter that has `pyshacl` + `rdflib` importable.
//!   - **Apache Jena SHACL** (`tests/diff_fuzz/JenaShaclAdapter.java`, bead
//!     sq-evws) — Jena's `org.apache.jena.shacl` validator, driven via the Java
//!     single-file source launcher (`java -cp <jena-libs> JenaShaclAdapter.java`)
//!     so no compile step / committed jar is needed. A second independent
//!     reference catches bugs where sparq and pySHACL happen to agree but are both
//!     wrong.
//!
//!   - **Node / RDF-JS** (`tests/diff_fuzz/node_shacl_adapter.mjs`, bead sq-vz2v) —
//!     a third, independent family driving the Zazuko `shacl-engine` (default) or
//!     `rdf-validate-shacl` validator (selected by `SHACL_DIFF_NODE_ENGINE`) over an
//!     RDF-JS dataset. Being JS over RDF-JS it is also the on-ramp for diffing
//!     sparq's OWN `@sparq-org/sparq` WASM SHACL (a future JS-vs-JS lane).
//!
//! Each reference is another `RefEngine` over the same report-cli contract; adding
//! one is just a `resolve_*` returning that struct.
//!
//! ## How to run
//! Off by default (`#[ignore]`) so the per-PR `cargo nextest run` fast path is
//! untouched — this is a NIGHTLY/heavy-tier check. The differential runs against
//! EVERY reference engine that resolves; run it explicitly:
//! ```text
//! # pySHACL: point at a Python with pyshacl + rdflib installed:
//! SHACL_DIFF_PYTHON=/path/to/venv/bin/python \
//!   cargo test -p sparq-shacl --test diff_fuzz -- --ignored --nocapture
//! # Jena: point at a `*`-globbable classpath of its jars (an unpacked apache-jena
//! # tarball's `lib`), or set JENA_HOME (its `lib` is derived):
//! SHACL_DIFF_JENA_CP="/opt/apache-jena/lib/*" \
//!   cargo test -p sparq-shacl --test diff_fuzz -- --ignored --nocapture
//! # Node: point at a node_modules with the RDF-JS validator + its deps installed
//! # (SHACL_DIFF_NODE_MODULES, else NODE_PATH); SHACL_DIFF_NODE_ENGINE picks the
//! # validator (shacl-engine | rdf-validate-shacl; default shacl-engine):
//! SHACL_DIFF_NODE_MODULES="$PWD/crates/sparq-shacl/tests/diff_fuzz/node_modules" \
//!   cargo test -p sparq-shacl --test diff_fuzz -- --ignored --nocapture
//! # bound the run: SHACL_DIFF_SEED_START / SHACL_DIFF_COUNT (defaults 0 / 200).
//! ```
//! When NO reference engine resolves, the test SKIPS (prints why and returns) so
//! it stays green on a box without any reference toolchain. Each engine is gated
//! independently — Jena is silently skipped when neither a Jena classpath nor a
//! working `java` is available, and the Node engine when neither a usable `node`
//! nor a resolvable node_modules is present, so the existing pySHACL-only lane is
//! unaffected.
//!
//! ## Comparison policy (per bead sq-55c1)
//! 1. `conforms` bit — exact (cheap, highest signal).
//! 2. The violation SET, compared as a deduplicated set of
//!    `(focusNode, sourceConstraintComponent, path-presence)` tuples, with
//!    blank-node and complex-path tolerance. We deliberately compare the SET, not
//!    the multiset count: sparq's report intentionally does not deduplicate the
//!    way pySHACL does (the bead's "no-dedup caveat"), and `sh:resultMessage`s are
//!    impl-specific. A focus node + violated component that one engine reports and
//!    the other does not is the real signal.

use std::io::Write;
use std::process::{Command, Stdio};

use sparq_core::Graph;
use sparq_shacl::validate;

mod gen;
use gen::{Rng, Scenario};

/// [GPT-5.6] (sq-qvqk7) The explicit element type must keep string-literal picks
/// sized on rustc 1.88 while preserving the selected value.
#[test]
fn rng_string_pick_has_sized_elements() {
    let mut rng = Rng::new(0);
    let picked: &str = rng.pick::<&str>(&["typed"]);
    assert_eq!(picked, "typed");
}

// ---------------------------------------------------------------------------
// Reference engine: a report-cli adapter (sq-eifd "report-cli" kind).
// ---------------------------------------------------------------------------

/// A normalised reference-engine report, comparable to sparq's `ValidationReport`.
#[derive(Debug, serde::Deserialize)]
struct RefReport {
    conforms: bool,
    violations: Vec<RefViolation>,
}

#[derive(Debug, serde::Deserialize, Clone)]
struct RefViolation {
    focus: Option<String>,
    component: Option<String>,
    path: Option<String>,
}

/// A resolved reference engine: a human name plus the program + leading args to
/// spawn its report-cli adapter, and any extra environment to set on the child
/// (e.g. the Node adapter's `NODE_PATH`/engine switch). The (data, shapes) request
/// is always written to the child's stdin and the normalised JSON report read from
/// stdout, regardless of engine — so adding an engine is just another `resolve_*`
/// returning this.
struct RefEngine {
    name: String,
    program: String,
    args: Vec<String>,
    /// Extra `(key, value)` environment overrides for the spawned child; empty for
    /// engines that need none (pySHACL, Jena).
    envs: Vec<(String, String)>,
}

fn adapter_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/diff_fuzz")
}

/// Resolves the pySHACL engine, or `None` if pySHACL is not importable (so it can
/// be SKIPPED cleanly). Honours `SHACL_DIFF_PYTHON` (a venv's `python`); otherwise
/// tries `python3` then `python`.
fn resolve_pyshacl() -> Option<RefEngine> {
    let candidates = std::env::var("SHACL_DIFF_PYTHON")
        .ok()
        .into_iter()
        .chain(["python3".to_string(), "python".to_string()]);
    for py in candidates {
        let ok = Command::new(&py)
            .args(["-c", "import pyshacl, rdflib"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            let adapter = adapter_dir().join("pyshacl_adapter.py");
            return Some(RefEngine {
                name: format!("pySHACL via {py}"),
                program: py,
                args: vec![adapter.to_string_lossy().into_owned()],
                envs: vec![],
            });
        }
    }
    None
}

/// Pure classpath selection (env values passed in, so it is unit-testable without
/// mutating process-global env in parallel tests): an explicit `SHACL_DIFF_JENA_CP`
/// wins; otherwise `${JENA_HOME}/lib/*` if `JENA_HOME` is set; otherwise `None`
/// (Jena skipped). An empty string is treated as unset so a blank env var does not
/// produce a degenerate empty classpath.
fn jena_classpath(cp_env: Option<String>, jena_home_env: Option<String>) -> Option<String> {
    if let Some(cp) = cp_env.filter(|s| !s.is_empty()) {
        return Some(cp);
    }
    jena_home_env.filter(|s| !s.is_empty()).map(|home| {
        std::path::Path::new(&home)
            .join("lib")
            .join("*")
            .to_string_lossy()
            .into_owned()
    })
}

/// Resolves the Apache Jena SHACL engine, or `None` if Jena cannot be driven (so
/// it is SKIPPED cleanly — the pySHACL-only lane is unaffected). Needs:
///   - a `java` launcher: `SHACL_DIFF_JAVA` else `java` on PATH (must run), and
///   - a Jena classpath: `SHACL_DIFF_JENA_CP` (a `*`-glob over its jars) else
///     `${JENA_HOME}/lib/*` if `JENA_HOME` is set.
///
/// The adapter is the single-file `JenaShaclAdapter.java` run via the Java
/// single-file source launcher (`java -cp <cp> <file>.java`), so no jar/.class is
/// committed and there is no separate compile step.
fn resolve_jena() -> Option<RefEngine> {
    let classpath = jena_classpath(
        std::env::var("SHACL_DIFF_JENA_CP").ok(),
        std::env::var("JENA_HOME").ok(),
    )?;
    let java = std::env::var("SHACL_DIFF_JAVA").unwrap_or_else(|_| "java".to_string());
    // `java -version` must actually run (writes to stderr, exit 0) — otherwise no
    // usable launcher and we skip rather than spawn-error every case.
    let java_ok = Command::new(&java)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !java_ok {
        return None;
    }
    let adapter = adapter_dir().join("JenaShaclAdapter.java");
    Some(RefEngine {
        name: format!("Apache Jena SHACL via {java} (cp {classpath})"),
        program: java,
        args: vec![
            "-cp".to_string(),
            classpath,
            adapter.to_string_lossy().into_owned(),
        ],
        envs: vec![],
    })
}

/// Pure node_modules selection (env values passed in, so it is unit-testable
/// without mutating process-global env in parallel tests): an explicit
/// `SHACL_DIFF_NODE_MODULES` wins; otherwise the adapter-local `node_modules`
/// (`tests/diff_fuzz/node_modules`, what the CI lane installs) if it exists;
/// otherwise `NODE_PATH` if set; otherwise `None` (Node engine skipped). An empty
/// string is treated as unset so a blank env var does not produce a degenerate
/// empty path. `local_exists` lets the test decide whether to consider the local
/// dir without touching the filesystem.
fn node_modules_path(
    explicit_env: Option<String>,
    node_path_env: Option<String>,
    local_modules: &std::path::Path,
    local_exists: bool,
) -> Option<String> {
    if let Some(p) = explicit_env.filter(|s| !s.is_empty()) {
        return Some(p);
    }
    if local_exists {
        return Some(local_modules.to_string_lossy().into_owned());
    }
    node_path_env.filter(|s| !s.is_empty())
}

/// Resolves the Node / RDF-JS SHACL engine, or `None` if it cannot be driven (so
/// it is SKIPPED cleanly — the pySHACL/Jena lanes are unaffected). Needs:
///   - a `node` launcher: `SHACL_DIFF_NODE` else `node` on PATH (must run), and
///   - a resolvable `node_modules` with the RDF-JS validator installed:
///     `SHACL_DIFF_NODE_MODULES` else the adapter-local `node_modules` else
///     `NODE_PATH`.
///
/// The adapter is the single-file `node_shacl_adapter.mjs` run directly by Node
/// (no build step); `SHACL_DIFF_NODE_ENGINE` (shacl-engine | rdf-validate-shacl)
/// selects the validator inside the adapter. `NODE_PATH` is set on the child so the
/// adapter's bare `import`s resolve from the chosen `node_modules`.
fn resolve_node() -> Option<RefEngine> {
    let local = adapter_dir().join("node_modules");
    let modules = node_modules_path(
        std::env::var("SHACL_DIFF_NODE_MODULES").ok(),
        std::env::var("NODE_PATH").ok(),
        &local,
        local.is_dir(),
    )?;
    let node = std::env::var("SHACL_DIFF_NODE").unwrap_or_else(|_| "node".to_string());
    // `node --version` must actually run — otherwise no usable launcher and we skip
    // rather than spawn-error every case.
    let node_ok = Command::new(&node)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !node_ok {
        return None;
    }
    let engine_sel =
        std::env::var("SHACL_DIFF_NODE_ENGINE").unwrap_or_else(|_| "shacl-engine".to_string());
    let adapter = adapter_dir().join("node_shacl_adapter.mjs");
    Some(RefEngine {
        name: format!("Node RDF-JS SHACL ({engine_sel}) via {node}"),
        program: node,
        args: vec![adapter.to_string_lossy().into_owned()],
        // Bare `import`s in the .mjs adapter resolve from NODE_PATH; pass the engine
        // selection through so it is honoured even if the parent env differs.
        envs: vec![
            ("NODE_PATH".to_string(), modules),
            ("SHACL_DIFF_NODE_ENGINE".to_string(), engine_sel),
        ],
    })
}

/// All reference engines available on this box, in a stable order. The
/// differential runs against each; an empty list means SKIP the whole test.
fn resolve_engines() -> Vec<RefEngine> {
    [resolve_pyshacl(), resolve_jena(), resolve_node()]
        .into_iter()
        .flatten()
        .collect()
}

/// Runs one reference engine over one (data, shapes) pair. `Err` means the adapter
/// itself failed (engine error / bad invocation) — distinct from a produced
/// non-conforming report.
fn run_reference(engine: &RefEngine, data: &str, shapes: &str) -> Result<RefReport, String> {
    let req = serde_json::json!({ "data": data, "shapes": shapes }).to_string();
    let mut child = Command::new(&engine.program)
        .args(&engine.args)
        .envs(engine.envs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn adapter: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(req.as_bytes())
        .map_err(|e| format!("write request: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("wait adapter: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "adapter exit {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| {
        format!(
            "parse adapter JSON: {e}; stdout={}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

// ---------------------------------------------------------------------------
// Normalisation: map both engines' reports to a comparable violation set.
// ---------------------------------------------------------------------------

/// A graph-independent violation key: (focus, component, has-simple-path).
///
/// - focus: full IRI, or `_:bnode` for any blank node (no cross-graph identity).
/// - component: the `sh:sourceConstraintComponent` IRI.
/// - path: `None` (node-level result), `Some(<predicate IRI>)` for a simple
///   predicate path (where both engines agree on the bare IRI), or
///   `Some("_:path")` for any complex / blank-rooted path (we don't byte-match
///   the impl-specific Turtle serialisation of complex paths — presence + the
///   (focus, component) pair carry the signal).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    focus: String,
    component: String,
    path: Option<String>,
}

fn norm_term(t: &oxrdf::Term) -> String {
    match t {
        oxrdf::Term::BlankNode(_) => "_:bnode".to_string(),
        oxrdf::Term::NamedNode(n) => n.as_str().to_string(),
        other => other.to_string(),
    }
}

/// sparq's path → the same comparable string the adapter emits: a bare predicate
/// IRI for a simple path, else the `_:path` complex-path sentinel.
fn norm_path(p: &sparq_shacl::Path) -> Option<String> {
    match p {
        sparq_shacl::Path::Predicate(iri) => Some(iri.clone()),
        _ => Some("_:path".to_string()),
    }
}

fn sparq_keys(report: &sparq_shacl::ValidationReport) -> std::collections::BTreeSet<Key> {
    report
        .results
        .iter()
        .map(|r| Key {
            focus: norm_term(&r.focus_node),
            component: r.source_component.clone(),
            path: r.path.as_ref().and_then(norm_path),
        })
        .collect()
}

fn ref_keys(report: &RefReport) -> std::collections::BTreeSet<Key> {
    report
        .violations
        .iter()
        .map(|v| Key {
            // [OPUS-4.8] A genuinely-absent `focus` in the reference JSON is a
            // distinct fact from a blank-node focus: collapsing both to "_:bnode"
            // would let a report-shape bug (adapter dropping the focus field)
            // silently compare equal to sparq emitting a blank-node focus, masking
            // the disagreement. sparq's `norm_term` never yields this sentinel
            // (a real blank node maps to "_:bnode", a named node to its IRI), so a
            // missing-focus reference result can only ever match another
            // missing-focus result — and will mismatch any sparq result, surfacing
            // the adapter/engine shape bug instead of hiding it.
            focus: v
                .focus
                .clone()
                .unwrap_or_else(|| "<missing-focus>".to_string()),
            component: v
                .component
                .clone()
                .unwrap_or_else(|| "<no-component>".to_string()),
            path: v.path.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// One differential case.
// ---------------------------------------------------------------------------

enum CaseOutcome {
    Agree,
    /// Engine could not run this case (e.g. the reference choked on a construct);
    /// not a sparq bug — counted separately.
    Skipped(String),
    /// A genuine disagreement: the formatted diagnostic (seed + reproducer).
    Mismatch(String),
}

fn run_case(engine: &RefEngine, seed: u64) -> CaseOutcome {
    let mut rng = Rng::new(seed);
    let scenario = Scenario::generate(&mut rng);
    let data_ttl = scenario.data_turtle();
    let shapes_ttl = scenario.shapes_turtle();

    // sparq must at least parse its own generated graphs — a parse failure here is
    // a generator bug, surfaced as a mismatch so it cannot pass silently.
    let data = match Graph::load_str(&data_ttl, "turtle") {
        Ok(g) => g,
        Err(e) => {
            return mismatch(
                engine,
                seed,
                &data_ttl,
                &shapes_ttl,
                &format!("sparq data parse: {e}"),
            )
        }
    };
    let shapes = match Graph::load_str(&shapes_ttl, "turtle") {
        Ok(g) => g,
        Err(e) => {
            return mismatch(
                engine,
                seed,
                &data_ttl,
                &shapes_ttl,
                &format!("sparq shapes parse: {e}"),
            )
        }
    };
    let sparq_report = validate(&data, &shapes);

    let reference = match run_reference(engine, &data_ttl, &shapes_ttl) {
        Ok(r) => r,
        Err(e) => return CaseOutcome::Skipped(format!("reference engine error: {e}")),
    };

    // 1. conforms bit — exact.
    if sparq_report.conforms != reference.conforms {
        return mismatch(
            engine,
            seed,
            &data_ttl,
            &shapes_ttl,
            &format!(
                "CONFORMS differs: sparq={} reference={} (sparq {} results, reference {} violations)",
                sparq_report.conforms,
                reference.conforms,
                sparq_report.results.len(),
                reference.violations.len(),
            ),
        );
    }

    // 2. violation set (deduplicated; blank-node + complex-path tolerant).
    let s = sparq_keys(&sparq_report);
    let r = ref_keys(&reference);
    if s != r {
        let only_sparq: Vec<_> = s.difference(&r).collect();
        let only_ref: Vec<_> = r.difference(&s).collect();
        return mismatch(
            engine,
            seed,
            &data_ttl,
            &shapes_ttl,
            &format!(
                "VIOLATION SET differs (conforms agree = {}):\n  only in sparq ({}): {:?}\n  only in reference ({}): {:?}",
                sparq_report.conforms,
                only_sparq.len(),
                only_sparq,
                only_ref.len(),
                only_ref,
            ),
        );
    }

    CaseOutcome::Agree
}

fn mismatch(engine: &RefEngine, seed: u64, data: &str, shapes: &str, msg: &str) -> CaseOutcome {
    CaseOutcome::Mismatch(format!(
        "MISMATCH seed={seed} reference={}\n{msg}\n--- shapes.ttl ---\n{shapes}\n--- data.ttl ---\n{data}",
        engine.name,
    ))
}

// ---------------------------------------------------------------------------
// The driver.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "differential SHACL fuzz — nightly/heavy tier; needs a reference engine (pyshacl / Jena / Node RDF-JS)"]
fn differential_shacl_fuzz() {
    let engines = resolve_engines();
    if engines.is_empty() {
        eprintln!(
            "SKIP: no reference SHACL engine resolved. Install pyshacl+rdflib and set \
             SHACL_DIFF_PYTHON to its interpreter (e.g. a venv's bin/python), and/or point \
             SHACL_DIFF_JENA_CP (or JENA_HOME) at an apache-jena lib classpath for the Jena \
             reference, and/or install the RDF-JS validator into a node_modules pointed at by \
             SHACL_DIFF_NODE_MODULES (or NODE_PATH) for the Node reference."
        );
        return;
    }
    eprintln!(
        "differential SHACL fuzz: {} reference engine(s):",
        engines.len()
    );
    for e in &engines {
        eprintln!("  - {}", e.name);
    }

    let seed_start: u64 = env_u64("SHACL_DIFF_SEED_START", 0);
    let count: u64 = env_u64("SHACL_DIFF_COUNT", 200);

    // The differential is run independently against EVERY resolved engine over the
    // same seed range, so a bug surfaces against whichever reference catches it
    // (and a sparq-vs-pySHACL agreement that is jointly wrong is still caught if
    // Jena disagrees). Any per-engine disagreement, or any resolved engine that
    // compared zero cases, fails the whole test.
    let mut all_mismatches: Vec<String> = Vec::new();
    let mut all_skip_examples: Vec<String> = Vec::new();
    let mut vacuous_engines: Vec<String> = Vec::new();
    let mut total_mismatch_count = 0u64;

    for engine in &engines {
        let mut agree = 0u64;
        let mut skipped = 0u64;
        let mut mismatches: Vec<String> = Vec::new();
        let mut skip_examples: Vec<String> = Vec::new();

        for seed in seed_start..seed_start + count {
            match run_case(engine, seed) {
                CaseOutcome::Agree => agree += 1,
                CaseOutcome::Skipped(why) => {
                    skipped += 1;
                    if skip_examples.len() < 5 {
                        skip_examples.push(format!("seed={seed}: {why}"));
                    }
                }
                CaseOutcome::Mismatch(detail) => {
                    eprintln!("MISMATCH seed={seed} reference={}", engine.name);
                    // Cap stored reproducers per engine so a systematically-wrong
                    // run does not produce megabytes of output; counts stay exact.
                    if mismatches.len() < 10 {
                        mismatches.push(detail);
                    }
                }
            }
        }

        let engine_mismatch = count - agree - skipped;
        total_mismatch_count += engine_mismatch;
        eprintln!(
            "\nSHACL diff fuzz [{}]: seeds {seed_start}..{} — agree={agree} skipped={skipped} mismatch={engine_mismatch}",
            engine.name,
            seed_start + count,
        );
        for ex in &skip_examples {
            eprintln!("SKIP example: {ex}");
        }
        all_mismatches.append(&mut mismatches);
        all_skip_examples.append(&mut skip_examples);

        // [OPUS-4.8] Guard against a VACUOUS pass for THIS engine. We only resolve
        // an engine when its toolchain is present, so it is expected to actually
        // compare. If it compared zero cases (resolved yet errored on every seed —
        // wrong venv / missing jar / adapter regression) that is a broken setup,
        // not agreement; record it as a hard failure.
        if let Err(why) = assess_run(agree, skipped) {
            vacuous_engines.push(format!("{}: {why}", engine.name));
        }
    }

    if total_mismatch_count > 0 {
        for d in &all_mismatches {
            eprintln!("\n{d}");
        }
        panic!(
            "{total_mismatch_count} disagreement(s) between sparq-shacl and the reference \
             engine(s) (showing up to {} reproducers above) — file a bead per distinct bug \
             with the seed.",
            all_mismatches.len()
        );
    }

    if !vacuous_engines.is_empty() {
        for ex in &all_skip_examples {
            eprintln!("SKIP example: {ex}");
        }
        panic!(
            "reference engine(s) compared ZERO cases (resolved yet errored on every seed): {}",
            vacuous_engines.join("; ")
        );
    }
}

/// Decide whether a *resolved-adapter* run produced a meaningful comparison.
///
/// [OPUS-4.8] `agree` is the number of cases that ran the full sparq-vs-reference
/// comparison and matched; by the time the driver calls this, any mismatch has
/// already panicked, so `agree == compared` (the count of cases that actually
/// compared). If every case was instead `Skipped` — the adapter resolved yet
/// errored on every seed (wrong venv, missing rdflib, adapter regression) —
/// `compared` is 0 and the lane would otherwise "pass" while comparing nothing.
/// That is a broken reference setup, not agreement, so we return `Err`.
///
/// This is intentionally separate from the "adapter intentionally absent" path,
/// which the driver handles earlier by returning before any case runs.
fn assess_run(agree: u64, skipped: u64) -> Result<(), String> {
    let compared = agree;
    if compared == 0 {
        return Err(format!(
            "SHACL diff fuzz compared ZERO cases (skipped={skipped}): the reference adapter \
             was resolved yet errored on every seed — a broken reference setup/adapter, not \
             agreement. Check pySHACL/rdflib in the configured interpreter."
        ));
    }
    Ok(())
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ---------------------------------------------------------------------------
// Fast self-tests of the GENERATOR (no reference engine; run in the per-PR
// fast path). These guard the differential from going vacuous: a generator that
// silently degenerated to always-conforming, or that produced shapes sparq
// can't parse, would make the ignored differential pass for the wrong reason.
// ---------------------------------------------------------------------------

/// Every generated scenario must parse (shapes + data) and validate without
/// panicking, and across a batch the generator must produce BOTH conforming and
/// violating reports (so the differential exercises real disagreement surface).
#[test]
fn generator_is_well_formed_and_mixes_outcomes() {
    let mut conforming = 0u32;
    let mut violating = 0u32;
    let mut total_violations = 0usize;
    for seed in 0..400u64 {
        let mut rng = Rng::new(seed);
        let scenario = Scenario::generate(&mut rng);
        let data_ttl = scenario.data_turtle();
        let shapes_ttl = scenario.shapes_turtle();
        let data = Graph::load_str(&data_ttl, "turtle")
            .unwrap_or_else(|e| panic!("seed {seed}: data parse: {e}\n{data_ttl}"));
        let shapes = Graph::load_str(&shapes_ttl, "turtle")
            .unwrap_or_else(|e| panic!("seed {seed}: shapes parse: {e}\n{shapes_ttl}"));
        let report = validate(&data, &shapes);
        if report.conforms {
            conforming += 1;
        } else {
            violating += 1;
            total_violations += report.results.len();
        }
    }
    // The generator is healthy iff it produces a real mix — neither all-conforming
    // (the differential would never test violation reporting) nor all-violating
    // (it would never test that BOTH engines agree a graph conforms). Each instance
    // has several properties each violated ~half the time, so violations dominate by
    // construction (a scenario conforms only if EVERY value satisfies EVERY shape);
    // the floor is set to "both outcomes meaningfully represented", not 50/50.
    assert!(
        conforming >= 10 && violating >= 10,
        "generator is lopsided: {conforming} conforming, {violating} violating over 400 seeds \
         — the differential needs both outcomes represented"
    );
    assert!(total_violations > 0, "no violations generated at all");
}

/// The two normalisation directions must agree on a hand-built case so a refactor
/// of either `sparq_keys` or `ref_keys` can't silently drift the comparison.
#[test]
fn key_normalisation_is_consistent() {
    use sparq_shacl::Path;
    let report = sparq_shacl::ValidationReport {
        conforms: false,
        results: vec![sparq_shacl::ValidationResult {
            focus_node: oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://example.org/x",
            )),
            path: Some(Path::Predicate("http://example.org/p".into())),
            value: None,
            source_shape: oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://example.org/S",
            )),
            source_constraint: None,
            source_component: "http://www.w3.org/ns/shacl#DatatypeConstraintComponent".into(),
            severity: "http://www.w3.org/ns/shacl#Violation".into(),
            messages: vec![],
            default_message: String::new(),
            details: vec![],
        }],
        // [OPUS-4.8] (sq-lz99x) report now carries skipped-constraint diagnostics;
        // none here (this fixture is a hand-built result, not a real validation).
        diagnostics: vec![],
    };
    let sk = sparq_keys(&report);
    let rr = RefReport {
        conforms: false,
        violations: vec![RefViolation {
            focus: Some("http://example.org/x".into()),
            component: Some("http://www.w3.org/ns/shacl#DatatypeConstraintComponent".into()),
            path: Some("http://example.org/p".into()),
        }],
    };
    assert_eq!(
        sk,
        ref_keys(&rr),
        "normalisation must agree on the same logical result"
    );
}

/// [OPUS-4.8] Fast guard for the line-354 fix: a resolved adapter that errored on
/// every seed (all `Skipped`, zero compared) must be a FAILURE, not a pass; a run
/// that compared at least one case is OK. Runs in the per-PR fast lane (no
/// reference engine needed), so the floor logic can't silently regress.
#[test]
fn all_skipped_run_is_a_failure_not_a_vacuous_pass() {
    // Adapter resolved but errored on every one of 200 seeds.
    assert!(
        assess_run(0, 200).is_err(),
        "an all-skipped run (compared==0) must fail — otherwise a broken reference \
         adapter masquerades as agreement"
    );
    // At least one real comparison ran → not vacuous.
    assert!(assess_run(1, 199).is_ok());
    assert!(assess_run(200, 0).is_ok());
}

/// [OPUS-4.8] Fast guard for the line-199 fix: a reference violation with a
/// genuinely-absent `focus` must NOT normalise to the same key as a blank-node
/// focus, so a dropped-focus report-shape bug can't silently compare equal.
#[test]
fn missing_focus_is_distinct_from_blank_node_focus() {
    let comp = "http://www.w3.org/ns/shacl#MinCountConstraintComponent".to_string();
    let missing = ref_keys(&RefReport {
        conforms: false,
        violations: vec![RefViolation {
            focus: None,
            component: Some(comp.clone()),
            path: None,
        }],
    });
    // sparq never yields a missing focus; the closest is a blank-node focus,
    // which `norm_term` maps to "_:bnode". The two keys must differ.
    let blank = missing
        .iter()
        .next()
        .map(|k| Key {
            focus: "_:bnode".to_string(),
            component: k.component.clone(),
            path: k.path.clone(),
        })
        .unwrap();
    assert!(
        !missing.contains(&blank),
        "missing focus collapsed into the blank-node sentinel — masks report-shape bugs"
    );
}

/// [OPUS-4.8] (sq-evws) Jena classpath gating: an explicit `SHACL_DIFF_JENA_CP`
/// wins; else `${JENA_HOME}/lib/*`; else `None` (Jena SKIPPED, pySHACL-only lane
/// unaffected). A blank value is treated as unset so an empty env var can't yield
/// a degenerate empty classpath. Pure (env passed in) so it runs in the fast lane
/// without mutating process-global env across parallel tests.
#[test]
fn jena_classpath_gating() {
    // Explicit classpath wins, even when JENA_HOME is also set.
    assert_eq!(
        jena_classpath(Some("/opt/jena/lib/*".into()), Some("/ignored".into())),
        Some("/opt/jena/lib/*".to_string())
    );
    // Derive from JENA_HOME when no explicit classpath.
    let derived = jena_classpath(None, Some("/opt/apache-jena".into())).unwrap();
    assert!(
        derived.starts_with("/opt/apache-jena")
            && (derived.ends_with("lib/*") || derived.ends_with("lib\\*")),
        "JENA_HOME-derived classpath should be <home>/lib/*, got {derived}"
    );
    // Neither set → Jena is skipped (None), which is the pySHACL-only lane.
    assert_eq!(jena_classpath(None, None), None);
    // Blank values are treated as unset (no degenerate empty classpath).
    assert_eq!(jena_classpath(Some(String::new()), None), None);
    assert_eq!(jena_classpath(None, Some(String::new())), None);
    assert_eq!(
        jena_classpath(Some(String::new()), Some("/opt/apache-jena".into()))
            .as_deref()
            .map(|s| s.ends_with("lib/*") || s.ends_with("lib\\*")),
        Some(true),
        "blank explicit cp must fall through to JENA_HOME"
    );
}

/// [OPUS-4.8] (sq-vsqr) The Jena adapter must mirror the pySHACL adapter's sq-0hj7
/// `sh:detail` exclusion: a nested sub-result that is the object of an `sh:detail`
/// must NOT appear as a top-level violation (it lives in `ValidationResult::details`
/// in sparq, and `sh:detail` is non-normative per SHACL §3.6.2 — diffing it would
/// be a latent over-report, not a real disagreement). The adapter ships a
/// `--selftest` mode that asserts this on a synthetic report graph (no validator
/// run, so it is independent of whether THIS Jena build emits `sh:detail`); we drive
/// it here. SKIPPED cleanly when Java + a Jena classpath don't resolve (the
/// pySHACL-only / no-Jena lanes are unaffected) — exactly the diff-fuzz gating.
#[test]
fn jena_adapter_excludes_nested_sh_detail() {
    let Some(engine) = resolve_jena() else {
        eprintln!("SKIP jena_adapter_excludes_nested_sh_detail: Jena did not resolve");
        return;
    };
    // engine.args is `[-cp, <classpath>, <adapter>.java]`; append `--selftest` so the
    // single-file source launcher runs the adapter in self-test mode.
    let mut args = engine.args.clone();
    args.push("--selftest".to_string());
    let out = Command::new(&engine.program)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn Jena adapter --selftest");
    assert!(
        out.status.success(),
        "Jena adapter --selftest failed (exit {:?}) — the sh:detail exclusion \
         regressed (nested sub-result leaked into the top-level violation set):\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

/// [OPUS-4.8] (sq-vz2v) Node node_modules gating, pure (no filesystem / process
/// env touched, so it is parallel-safe): an explicit `SHACL_DIFF_NODE_MODULES`
/// wins; else the adapter-local `node_modules` IF it exists; else `NODE_PATH`;
/// else `None` (Node engine SKIPPED — the pySHACL/Jena lanes unaffected). Blank
/// values are treated as unset so a degenerate empty path can't slip through.
#[test]
fn node_modules_gating() {
    let local = std::path::Path::new("/repo/tests/diff_fuzz/node_modules");
    // Explicit env wins, even when the local dir and NODE_PATH are also present.
    assert_eq!(
        node_modules_path(
            Some("/custom/nm".into()),
            Some("/ignored/nm".into()),
            local,
            true
        ),
        Some("/custom/nm".to_string())
    );
    // No explicit env, local dir exists → the adapter-local node_modules.
    assert_eq!(
        node_modules_path(None, Some("/np".into()), local, true)
            .as_deref()
            .map(|s| s.ends_with("node_modules")),
        Some(true)
    );
    // No explicit, no local → fall through to NODE_PATH.
    assert_eq!(
        node_modules_path(None, Some("/np".into()), local, false),
        Some("/np".to_string())
    );
    // Nothing set → Node skipped.
    assert_eq!(node_modules_path(None, None, local, false), None);
    // Blank values are unset.
    assert_eq!(
        node_modules_path(Some(String::new()), None, local, false),
        None
    );
    assert_eq!(
        node_modules_path(None, Some(String::new()), local, false),
        None
    );
    // Blank explicit falls through to the local dir when it exists.
    assert_eq!(
        node_modules_path(Some(String::new()), None, local, true)
            .as_deref()
            .map(|s| s.ends_with("node_modules")),
        Some(true)
    );
}

/// [OPUS-4.8] (sq-vz2v) The Node adapter must mirror the pySHACL/Jena adapters'
/// `sh:detail` exclusion: a nested sub-result that is the object of an `sh:detail`
/// must NOT appear as a top-level violation (it lives in `ValidationResult::details`
/// in sparq, and `sh:detail` is non-normative per SHACL §3.6.2 — diffing it would
/// be a latent over-report, not a real disagreement). The adapter ships a
/// `--selftest` mode asserting this on a synthetic RDF-JS report dataset (no
/// validator run, so it is independent of which validator/version is installed);
/// we drive it here. SKIPPED cleanly when Node + a node_modules don't resolve (the
/// pySHACL/Jena / no-Node lanes are unaffected) — exactly the diff-fuzz gating.
#[test]
fn node_adapter_excludes_nested_sh_detail() {
    let Some(engine) = resolve_node() else {
        eprintln!("SKIP node_adapter_excludes_nested_sh_detail: Node engine did not resolve");
        return;
    };
    // engine.args is `[<adapter>.mjs]`; append `--selftest` and carry the resolved
    // NODE_PATH/engine env so the adapter's bare imports resolve.
    let mut args = engine.args.clone();
    args.push("--selftest".to_string());
    let out = Command::new(&engine.program)
        .args(&args)
        .envs(engine.envs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn Node adapter --selftest");
    assert!(
        out.status.success(),
        "Node adapter --selftest failed (exit {:?}) — the sh:detail exclusion \
         regressed (nested sub-result leaked into the top-level violation set):\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-0hj7) Guards for the extended generator: logical components,
// sh:node nested-shape refs, and complex SHAPE paths. These run in the per-PR
// fast lane (no reference engine) so the extension can't silently degenerate.
// ---------------------------------------------------------------------------

/// Over a batch of seeds the generator must actually emit each extended family:
/// every logical operator (and/or/not/xone), at least one sh:node ref, and at
/// least one complex path. A regression that stopped generating one of these
/// would shrink the differential's coverage silently — this fails loudly.
#[test]
fn generator_covers_logical_node_and_complex_paths() {
    let mut seen = gen::Coverage::default();
    for seed in 0..4000u64 {
        let mut rng = Rng::new(seed);
        let cov = Scenario::generate(&mut rng).coverage();
        seen.logical_and |= cov.logical_and;
        seen.logical_or |= cov.logical_or;
        seen.logical_not |= cov.logical_not;
        seen.logical_xone |= cov.logical_xone;
        seen.node |= cov.node;
        seen.complex_path |= cov.complex_path;
    }
    assert!(seen.logical_and, "no sh:and generated over 4000 seeds");
    assert!(seen.logical_or, "no sh:or generated over 4000 seeds");
    assert!(seen.logical_not, "no sh:not generated over 4000 seeds");
    assert!(seen.logical_xone, "no sh:xone generated over 4000 seeds");
    assert!(seen.node, "no sh:node ref generated over 4000 seeds");
    assert!(
        seen.complex_path,
        "no complex path generated over 4000 seeds"
    );
}

/// The generator's conform/violate CONSTRUCTION must be sound: a case it intends
/// to conform must validate as conforming in sparq, and a case it intends to
/// violate must validate as non-conforming — for every extended component and
/// every complex-path form. This is the invariant the differential leans on
/// (a reference engine then checks sparq's *answer*; this checks the *input*
/// actually exercises the intended outcome), verified here without one.
#[test]
fn extended_components_conform_violate_construction_is_sound() {
    use gen::PathSpecKind::*;

    // (label, value builder, path form) — both conform & violate are checked.
    // For each we assert sparq's global conformance bit matches the intent. We
    // assert on the conformance BIT (not the violation set) so the check is
    // robust to impl-specific result counts; the differential's set comparison
    // adds the per-component agreement on top.
    let value_cases: Vec<(&str, gen::Constraint)> = vec![
        ("and(2)", gen::logical_and(2)),
        ("and(3)", gen::logical_and(3)),
        ("or(2)", gen::logical_or(2)),
        ("or(3)", gen::logical_or(3)),
        ("not", gen::logical_not()),
        ("xone(2)", gen::logical_xone(2)),
        ("xone(3)", gen::logical_xone(3)),
        ("node(datatype integer)", gen::node_datatype_integer()),
        ("node(nodeKind IRI)", gen::node_nodekind_iri()),
    ];

    for (label, value) in &value_cases {
        for conform in [true, false] {
            let (shapes_ttl, data_ttl) = gen::single_case(value.clone(), Predicate, conform, 1);
            assert_conformance(label, "predicate", &shapes_ttl, &data_ttl, conform);
        }
    }

    // Complex SHAPE paths. The value-node-set-includes-focus forms (zeroOrOne /
    // zeroOrMore) are generated value-component-free; we test them with an
    // integer-datatype value applied to the NON-focus reachable value to confirm
    // the path still wires up + validates (focus is a value node and is a typed
    // IRI, which is NOT an integer literal, so those forms would mis-violate the
    // focus — hence the generator keeps them value-free; here we only exercise
    // the focus-EXCLUDING forms with a value constraint).
    let path_cases = [
        ("sequence", Sequence, gen::datatype_integer()),
        ("oneOrMore", OneOrMore, gen::datatype_integer()),
        ("inverse", Inverse, gen::class_kind()),
    ];
    for (label, kind, value) in path_cases {
        for conform in [true, false] {
            let (shapes_ttl, data_ttl) = gen::single_case(value.clone(), kind, conform, 7);
            assert_conformance("complex-path", label, &shapes_ttl, &data_ttl, conform);
        }
    }

    // And the focus-INCLUDING forms (value-component-free, exactly as the
    // generator emits them): they must parse + validate as CONFORMING (no value
    // constraint to violate, no cardinality). A regression that made them
    // ill-formed or spuriously violating (e.g. checking the focus against
    // something) would fail here.
    for (label, kind) in [("zeroOrOne", ZeroOrOne), ("zeroOrMore", ZeroOrMore)] {
        let (shapes_ttl, data_ttl) = gen::single_case_path_only(kind, 9);
        assert_conformance("complex-path", label, &shapes_ttl, &data_ttl, true);
    }
}

/// Validate `(shapes, data)` through sparq and assert the global conformance bit
/// equals `expect_conforms`, with the reproducer in the panic message.
fn assert_conformance(
    group: &str,
    label: &str,
    shapes_ttl: &str,
    data_ttl: &str,
    expect_conforms: bool,
) {
    let data = Graph::load_str(data_ttl, "turtle")
        .unwrap_or_else(|e| panic!("{group}/{label}: data parse: {e}\n{data_ttl}"));
    let shapes = Graph::load_str(shapes_ttl, "turtle")
        .unwrap_or_else(|e| panic!("{group}/{label}: shapes parse: {e}\n{shapes_ttl}"));
    let report = validate(&data, &shapes);
    assert_eq!(
        report.conforms,
        expect_conforms,
        "{group}/{label}: generator intended conforms={expect_conforms} but sparq reported \
         conforms={} ({} results) — the conform/violate construction is UNSOUND, so the \
         differential would compare a case whose intended outcome doesn't hold.\
         \n--- shapes.ttl ---\n{shapes_ttl}\n--- data.ttl ---\n{data_ttl}\n--- results ---\n{:#?}",
        report.conforms,
        report.results.len(),
        report.results,
    );
}
