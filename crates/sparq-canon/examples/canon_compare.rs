//! [FABLE-5] (sq-p3ssl) Native in-process rdf-canon column for the RDFC-1.0
//! canonicalization panel (`bench/canon/`, sq-hmd7l.16).
//!
//! `bench/canon/run.sh` drives the rdf-canon Rust crate through a gather-time
//! *scratch CLI subprocess* (`scripts/bench-adapters/canon_adapter.sh`), so its
//! rdf-canon column carries process-spawn noise. This example canonicalizes the
//! SAME committed conformance graphs (the vendored W3C rdf-canon suite snapshot,
//! `tests/rdf-canon-testdata/`) through BOTH implementations **in one process**:
//!
//! - `sparq` — the public `sparq-canon` API (`canonicalize_quads` /
//!   `canonicalize_quads_with::<Sha384>`), oxrdf-0.3↔0.2 bridge included; and
//! - `rdf-canon` — the `rdf_canon` crate driven natively over oxrdf-0.2 quads
//!   parsed directly from the fixture bytes (no bridge).
//!
//! HONESTY: `sparq-canon` runs this same `rdf-canon` algorithm, vendored as its
//! private `src/rdfc/` module plus sparq's label-independence tie-breaks
//! (`src/rdfc/SPARQ-PATCHES.md`), so the delta between the two columns is a
//! **bridge + patch overhead measure** (serialize-0.3→text→parse-0.2, guard
//! configuration, and the tie-break keys), NOT an independent-implementation
//! comparison (that is the JS `rdf-canonize` column of `bench/canon/run.sh`).
//! The `rdf-canon` column is upstream, unpatched, pulled as a dev-dependency.
//!
//! INVARIANT (the sq-p3ssl gate): **no timing row is emitted unless BOTH
//! implementations produced canonical N-Quads byte-identical to the vendored
//! W3C expected file (the exact-image RDFC-1.0 oracle) on EVERY sane
//! conformance fixture** — equality with the oracle implies pairwise equality.
//! Poison / must-fail fixtures are a separate outcome panel: a cap-hit or a
//! fail-closed guard refusal is a RECORDED RESULT (stderr + envelope), never a
//! timing win — capped/guarded runs never appear in the timing rows.
//!
//! ```sh
//! cargo run -p sparq-canon --release --example canon_compare -- --smoke
//! cargo run -p sparq-canon --release --example canon_compare -- [--iters N] [--cap-s N] [--json-out out.json]
//! ```
//!
//! stdout: `<workload>\t<count>\t<us>` rows (per-fixture rows have
//! `count` = quads canonicalized; the `sane-total/<impl>` rows have
//! `count` = fixtures), then a single-line `CANON_COMPARE_JSON {…}` envelope
//! (the `MEMBPT_JSON` precedent). Diagnostics + poison outcomes on stderr.
//! Work-box timings are NON-canonical (`bench/CATALOG.md` QUIET-BOX); nothing
//! here is a committed number.
//!
//! Exit codes: `0` green | `2` red (parity/soundness failure) | `1` usage/IO/
//! manifest drift.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const RDFC: &str = "https://w3c.github.io/rdf-canon/tests/vocab#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const BASE: &str = "https://w3c.github.io/rdf-canon/tests/";

fn usage() -> ! {
    eprintln!(
        "usage: canon_compare [--smoke] [--iters N] [--cap-s N] [--json-out FILE]\n\
         \x20 --smoke     single-iteration acceptance run (the sq-p3ssl gate)\n\
         \x20 --iters N   min-of-N timing iterations (default 3; smoke forces 1)\n\
         \x20 --cap-s N   per-canonicalization soft wall-clock cap in seconds\n\
         \x20             (default $CANON_CAP_S or 10)\n\
         exit: 0 green | 2 parity/soundness red | 1 usage/IO"
    );
    std::process::exit(1);
}

fn die(msg: &str) -> ! {
    eprintln!("[canon-compare] ERROR: {msg}");
    std::process::exit(1);
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rdf-canon-testdata")
}

/// One manifest entry we exercise (map tests are skipped: the byte-parity
/// artifact here is canonical N-Quads, not the issued-identifier map).
struct Entry {
    /// Short id (`test001c`).
    id: String,
    /// `RDFC10EvalTest` | `RDFC10NegativeEvalTest`.
    kind: String,
    /// Local path of the input `.nq`.
    input: PathBuf,
    /// Local path of the expected canonical `.nq` (eval tests).
    expected: Option<PathBuf>,
    /// test075 is parameterized over SHA-384.
    sha384: bool,
    /// `mf:name` contains "poison" — the DoS panel, same rule as run.sh.
    poison: bool,
}

fn local_path(iri: &str) -> PathBuf {
    let tail = iri.rsplit_once("rdfc10/").map_or(iri, |(_, t)| t);
    testdata().join("rdfc10").join(tail)
}

/// Parses the vendored manifest with oxttl (a regular dep — no engine needed)
/// and splits it into (sane evals, poison evals, negatives).
fn manifest_entries() -> (Vec<Entry>, Vec<Entry>, Vec<Entry>) {
    let ttl = std::fs::read(testdata().join("manifest.ttl"))
        .unwrap_or_else(|e| die(&format!("read manifest.ttl: {e}")));
    #[derive(Default)]
    struct Raw {
        kind: Option<String>,
        action: Option<String>,
        result: Option<String>,
        name: Option<String>,
        hash: Option<String>,
    }
    let mut raw: BTreeMap<String, Raw> = BTreeMap::new();
    let (mf_action, mf_result, mf_name) = (
        format!("{MF}action"),
        format!("{MF}result"),
        format!("{MF}name"),
    );
    let rdfc_hash = format!("{RDFC}hashAlgorithm");
    let parser = oxttl::TurtleParser::new()
        .with_base_iri(BASE)
        .unwrap_or_else(|e| die(&format!("bad base IRI: {e}")));
    for t in parser.for_slice(&ttl) {
        let t = t.unwrap_or_else(|e| die(&format!("manifest.ttl parse: {e}")));
        let oxrdf::NamedOrBlankNode::NamedNode(s) = &t.subject else {
            continue;
        };
        let e = raw.entry(s.as_str().to_owned()).or_default();
        let p = t.predicate.as_str();
        match (p, &t.object) {
            (RDF_TYPE, oxrdf::Term::NamedNode(o)) if o.as_str().starts_with(RDFC) => {
                e.kind = Some(o.as_str()[RDFC.len()..].to_owned());
            }
            (_, oxrdf::Term::NamedNode(o)) if p == mf_action => {
                e.action = Some(o.as_str().to_owned());
            }
            (_, oxrdf::Term::NamedNode(o)) if p == mf_result => {
                e.result = Some(o.as_str().to_owned());
            }
            (_, oxrdf::Term::Literal(o)) if p == mf_name => {
                e.name = Some(o.value().to_owned());
            }
            (_, oxrdf::Term::Literal(o)) if p == rdfc_hash => {
                e.hash = Some(o.value().to_owned());
            }
            _ => {}
        }
    }

    let (mut sane, mut poison, mut negative) = (Vec::new(), Vec::new(), Vec::new());
    let mut total = 0usize;
    for (iri, r) in raw {
        let Some(kind) = r.kind else { continue };
        let Some(action) = r.action else { continue };
        total += 1;
        if kind == "RDFC10MapTest" {
            continue;
        }
        let entry = Entry {
            id: iri
                .rsplit_once('#')
                .map_or(iri.as_str(), |(_, t)| t)
                .to_owned(),
            kind: kind.clone(),
            input: local_path(&action),
            expected: r.result.as_deref().map(local_path),
            sha384: r.hash.as_deref() == Some("SHA384"),
            poison: r.name.as_deref().is_some_and(|n| n.contains("poison")),
        };
        match kind.as_str() {
            "RDFC10EvalTest" if entry.poison => poison.push(entry),
            "RDFC10EvalTest" => sane.push(entry),
            "RDFC10NegativeEvalTest" => negative.push(entry),
            _ => die(&format!("unknown test type {kind} on {iri}")),
        }
    }
    // Drift guards mirroring bench/canon/run.sh: the vendored snapshot is the
    // 86-entry W3C manifest; the sane byte-equality set must not silently shrink.
    if total != 86 {
        die(&format!(
            "expected the 86-entry vendored manifest, got {total} — snapshot drift"
        ));
    }
    if sane.len() < 60 {
        die(&format!(
            "sane eval set too small ({}) — manifest parse drift",
            sane.len()
        ));
    }
    if poison.is_empty() || negative.is_empty() {
        die("no poison/negative fixtures found — manifest parse drift");
    }
    (sane, poison, negative)
}

/// A fixture parsed ONCE per term model, so timing measures canonicalization
/// (bridge included on the sparq side), not file IO or the N-Quads parse.
struct Loaded {
    entry: Entry,
    quads03: Vec<oxrdf::Quad>,
    quads02: Vec<oxrdf02::Quad>,
    expected: Option<Vec<u8>>,
}

fn load(entry: Entry) -> Loaded {
    let bytes = std::fs::read(&entry.input)
        .unwrap_or_else(|e| die(&format!("read {}: {e}", entry.input.display())));
    let text = String::from_utf8(bytes.clone())
        .unwrap_or_else(|e| die(&format!("{}: not UTF-8: {e}", entry.input.display())));
    let quads03 = sparq_canon::parse_nquads(&text)
        .unwrap_or_else(|e| die(&format!("{}: oxrdf-0.3 parse: {e}", entry.input.display())));
    let quads02 = oxttl01::NQuadsParser::new()
        .for_slice(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| die(&format!("{}: oxrdf-0.2 parse: {e}", entry.input.display())));
    let expected = entry
        .expected
        .as_ref()
        .map(|p| std::fs::read(p).unwrap_or_else(|e| die(&format!("read {}: {e}", p.display()))));
    Loaded {
        entry,
        quads03,
        quads02,
        expected,
    }
}

const IMPLS: [&str; 2] = ["sparq", "rdf-canon"];

/// One canonicalization attempt. `Err` = fail-closed rejection (the HNDQ
/// call-limit guard — the same class `canon_bench` maps to exit 2).
type CanonRun = (Result<String, String>, u128);

fn canon_sparq(quads: &[oxrdf::Quad], sha384: bool) -> CanonRun {
    let t = Instant::now();
    let r = if sha384 {
        sparq_canon::canonicalize_quads_with::<sha2::Sha384>(quads)
    } else {
        sparq_canon::canonicalize_quads(quads)
    };
    (r.map_err(|e| e.to_string()), t.elapsed().as_micros())
}

fn canon_native(quads: &[oxrdf02::Quad], sha384: bool) -> CanonRun {
    let t = Instant::now();
    let r = if sha384 {
        let opts = rdf_canon::CanonicalizationOptions::default();
        rdf_canon::canonicalize_quads_with::<sha2::Sha384>(quads, &opts)
    } else {
        rdf_canon::canonicalize_quads(quads)
    };
    (r.map_err(|e| e.to_string()), t.elapsed().as_micros())
}

/// Runs one canonicalization on a worker thread under a soft wall-clock cap.
/// `None` = cap blown (recorded as `capped`; the detached worker still
/// terminates via the crates' default HNDQ call limit — bounded, not runaway).
fn run_capped(
    cap: Duration,
    imp: &str,
    quads03: &[oxrdf::Quad],
    quads02: &[oxrdf02::Quad],
    sha384: bool,
) -> Option<CanonRun> {
    let (tx, rx) = std::sync::mpsc::channel();
    match imp {
        "sparq" => {
            let q = quads03.to_vec();
            std::thread::spawn(move || {
                let _ = tx.send(canon_sparq(&q, sha384));
            });
        }
        "rdf-canon" => {
            let q = quads02.to_vec();
            std::thread::spawn(move || {
                let _ = tx.send(canon_native(&q, sha384));
            });
        }
        _ => unreachable!("unknown impl {imp}"),
    }
    rx.recv_timeout(cap).ok()
}

fn main() {
    let mut smoke = false;
    let mut iters: u32 = 3;
    let mut cap_s: u64 = std::env::var("CANON_CAP_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let mut json_out: Option<PathBuf> = None;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--smoke" => smoke = true,
            "--iters" => {
                i += 1;
                iters = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--cap-s" => {
                i += 1;
                cap_s = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--json-out" => {
                i += 1;
                json_out = Some(PathBuf::from(args.get(i).unwrap_or_else(|| usage())));
            }
            _ => usage(),
        }
        i += 1;
    }
    if smoke {
        iters = 1;
    }
    if iters == 0 {
        die("--iters must be >= 1");
    }
    let cap = Duration::from_secs(cap_s.max(1));

    let (sane, poison, negative) = manifest_entries();
    eprintln!(
        "[canon-compare] fixtures: {} sane evals, {} poison evals, {} negatives (cap {}s, iters {iters})",
        sane.len(),
        poison.len(),
        negative.len(),
        cap.as_secs(),
    );
    let sane: Vec<Loaded> = sane.into_iter().map(load).collect();
    let panel: Vec<Loaded> = poison.into_iter().chain(negative).map(load).collect();

    let mut red = false;

    // ---- PHASE A: the equality gate (exact-image oracle) — BEFORE any timing.
    // Both implementations must reproduce the vendored W3C expected bytes on
    // every sane fixture; oracle equality on both sides IS pairwise equality.
    let mut gate_pass: BTreeMap<&str, u32> = BTreeMap::new();
    let mut gate_fail: BTreeMap<&str, u32> = BTreeMap::new();
    for f in &sane {
        let exp = f
            .expected
            .as_deref()
            .unwrap_or_else(|| die(&format!("sane fixture {} has no expected file", f.entry.id)));
        for imp in IMPLS {
            let ok = match run_capped(cap, imp, &f.quads03, &f.quads02, f.entry.sha384) {
                Some((Ok(out), _)) if out.as_bytes() == exp => true,
                Some((Ok(_), _)) => {
                    eprintln!(
                        "[canon-compare] PARITY FAIL: {imp} on {} (wrong bytes)",
                        f.entry.id
                    );
                    false
                }
                Some((Err(e), _)) => {
                    eprintln!(
                        "[canon-compare] PARITY FAIL: {imp} on {} (rejected: {e})",
                        f.entry.id
                    );
                    false
                }
                None => {
                    eprintln!(
                        "[canon-compare] PARITY FAIL: {imp} on {} (capped at {}s)",
                        f.entry.id,
                        cap.as_secs()
                    );
                    false
                }
            };
            let bucket = if ok { &mut gate_pass } else { &mut gate_fail };
            *bucket.entry(imp).or_insert(0) += 1;
            red |= !ok;
        }
    }
    let gate_green = IMPLS
        .iter()
        .all(|imp| gate_fail.get(imp).copied().unwrap_or(0) == 0);
    for imp in IMPLS {
        let (p, fl) = (
            gate_pass.get(imp).copied().unwrap_or(0),
            gate_fail.get(imp).copied().unwrap_or(0),
        );
        if fl == 0 {
            eprintln!("[canon-compare] PHASE A: {imp} byte-exact vs the W3C oracle on all {p} sane fixtures");
        } else {
            eprintln!(
                "[canon-compare] PHASE A: {imp} FAILED byte-equality on {fl}/{} — NO timing rows",
                p + fl
            );
        }
    }

    // ---- PHASE B: timing rows — ONLY behind a green gate (the invariant).
    let mut totals: BTreeMap<&str, u128> = BTreeMap::new();
    if gate_green {
        for f in &sane {
            for imp in IMPLS {
                let mut best: Option<u128> = None;
                for _ in 0..iters {
                    match run_capped(cap, imp, &f.quads03, &f.quads02, f.entry.sha384) {
                        Some((Ok(_), us)) => best = Some(best.map_or(us, |b| b.min(us))),
                        // A rerun failing/capping AFTER the gate passed is a
                        // harness anomaly (nondeterminism), not a result.
                        _ => die(&format!("timing rerun failed: {imp} on {}", f.entry.id)),
                    }
                }
                let us = best.expect("iters >= 1");
                println!("{}/{imp}\t{}\t{us}", f.entry.id, f.quads03.len());
                *totals.entry(imp).or_insert(0) += us;
            }
        }
        for imp in IMPLS {
            println!(
                "sane-total/{imp}\t{}\t{}",
                sane.len(),
                totals.get(imp).copied().unwrap_or(0)
            );
        }
    } else {
        eprintln!("[canon-compare] equality gate RED — timing suppressed entirely");
    }

    // ---- PHASE C: poison / must-fail outcome panel (results, never wins).
    let mut poison_rows: Vec<(String, &str, &'static str, Option<u128>)> = Vec::new();
    for f in &panel {
        let must_fail = f.entry.kind == "RDFC10NegativeEvalTest";
        for imp in IMPLS {
            let (outcome, us): (&'static str, Option<u128>) =
                match run_capped(cap, imp, &f.quads03, &f.quads02, f.entry.sha384) {
                    Some((Ok(out), us)) => {
                        if must_fail {
                            ("accepted", Some(us)) // computed a must-fail case
                        } else if f.expected.as_deref() == Some(out.as_bytes()) {
                            ("ok", Some(us))
                        } else {
                            ("wrong", Some(us))
                        }
                    }
                    Some((Err(_), us)) => ("guard", Some(us)), // fail-closed refusal
                    None => ("capped", None),                  // blew the soft cap
                };
            // Soundness inside the panel: wrong bytes, or accepting a
            // must-fail case, contradicts the W3C suite -> red. `guard` and
            // `capped` are honest recorded outcomes.
            if outcome == "wrong" || outcome == "accepted" {
                red = true;
                eprintln!(
                    "[canon-compare] SOUNDNESS FAIL: {imp} '{outcome}' on {}",
                    f.entry.id
                );
            }
            eprintln!(
                "[canon-compare] poison {}/{imp} -> {outcome}{}",
                f.entry.id,
                us.map_or_else(
                    || format!(" (cap {}s)", cap.as_secs()),
                    |u| format!(" ({u}us)")
                ),
            );
            poison_rows.push((f.entry.id.clone(), imp, outcome, us));
        }
    }

    // ---- Envelope (single-line JSON on stdout, the MEMBPT_JSON precedent).
    // The rdf-canon pin is read from this crate's manifest AT RUNTIME (never
    // hard-coded), falling back to "unknown" outside a source checkout.
    let rdf_canon_pin =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.trim_start().starts_with("rdf-canon = "))
                    .and_then(|l| l.split('"').nth(1).map(str::to_owned))
            })
            .unwrap_or_else(|| "unknown".to_owned());
    let envelope = serde_json::json!({
        "suite": "canon-compare",
        "mode": if smoke { "smoke" } else { "full" },
        "cap_s": cap.as_secs(),
        "iters": iters,
        "sane_fixtures": sane.len(),
        "gate": IMPLS.iter().map(|imp| (imp.to_string(), serde_json::json!({
            "pass": gate_pass.get(imp).copied().unwrap_or(0),
            "fail": gate_fail.get(imp).copied().unwrap_or(0),
        }))).collect::<serde_json::Map<_, _>>(),
        "timing_total_us": totals.iter().map(|(k, v)| ((*k).to_string(), serde_json::json!(v)))
            .collect::<serde_json::Map<_, _>>(),
        "poison": poison_rows.iter().map(|(fixture, imp, outcome, us)| serde_json::json!({
            "fixture": fixture, "impl": imp, "outcome": outcome, "canon_us": us,
        })).collect::<Vec<_>>(),
        "versions": {
            "sparq_canon": env!("CARGO_PKG_VERSION"),
            "rdf_canon": rdf_canon_pin,
        },
        "red": red,
        "note": "bridge + patch overhead measure: the rdf-canon column is upstream at this pin; \
                 the sparq column runs the vendored, label-independence-patched copy of it \
                 plus the oxrdf-0.3<->0.2 bridge. Work-box timings are \
                 NON-canonical (bench/CATALOG.md QUIET-BOX); the independent-implementation \
                 check is the rdf-canonize JS column of bench/canon/run.sh.",
    });
    println!("CANON_COMPARE_JSON {envelope}");
    if let Some(p) = json_out {
        std::fs::write(&p, format!("{envelope:#}\n"))
            .unwrap_or_else(|e| die(&format!("write {}: {e}", p.display())));
        eprintln!("[canon-compare] envelope written to {}", p.display());
    }

    if red {
        eprintln!("[canon-compare] FAILED: see PARITY/SOUNDNESS FAIL lines above");
        std::process::exit(2);
    }
    eprintln!("[canon-compare] OK: both implementations byte-exact on the sane set; poison outcomes recorded");
}
