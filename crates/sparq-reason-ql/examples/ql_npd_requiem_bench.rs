//! sq-hmd7l.9 (epic sq-hmd7l): the sparq leg of the OWL 2 QL rewriting comparison
//! vs Ontop on the NPD + Requiem suites (`scripts/bench/reason-ql-same-box.sh`). Run:
//!
//!     cargo run -p sparq-reason-ql --example ql_npd_requiem_bench --release --features experimental -- --smoke
//!     cargo run -p sparq-reason-ql --example ql_npd_requiem_bench --release --features experimental -- \
//!         <suite> <tbox.nt|.ttl> <queries-dir> [--abox <data.nt|.ttl>]
//!
//! TWO metrics, both required by the bead: rewrite WALL TIME and output UCQ SIZE (disjunct
//! count). REGIME LABELS (printed in the `#`-prefixed banner, echoed into the harness
//! envelope): the `*_rewrite_ms` columns are REWRITER-PHASE ONLY (in-process `rewrite` /
//! `rewrite_production`, no execution); `exec_ms`/`e2e_ms` (only with `--abox`) are the
//! execution of the minimised UCQ over the loaded ABox and rewrite+execute respectively —
//! the column comparable to Ontop's end-to-end CLI regime.
//!
//! THE UCQ-EQUIVALENCE SANITY CHECK IS THE GATE (the sq-hmd7l.9 INVARIANT). Before ANY
//! timing row is printed for a query, the RAW PerfectRef UCQ (`rewrite`) and the MINIMISED
//! production UCQ (`rewrite_production`) are BOTH executed over the same data and their
//! result SETS must agree — containment minimisation must never change answers. In gather
//! mode without `--abox` the data is a set of deterministic PER-DISJUNCT WITNESS databases:
//! the frozen canonical instance of every CQ disjunct of the original query, the raw UCQ,
//! and the minimised UCQ (variables/blank nodes frozen to fresh per-disjunct IRIs; IRI and
//! literal constants kept as-is), each disjunct's instance held in its OWN isolated database.
//! Raw and minimised are evaluated over EACH database separately and must agree on each. Every
//! disjunct matches at least its own frozen instance via the identity homomorphism — including
//! query-only predicates absent from the TBox and multi-atom shared-variable joins — so on a
//! given disjunct D's isolated instance the retained UCQ reproduces D's frozen head ONLY via a
//! homomorphism into D's own instance, i.e. only when D was genuinely subsumed: a minimisation
//! that drops a NON-subsumed disjunct fails agreement on that disjunct's database (the classical
//! per-CQ canonical-database containment argument for UCQs). ISOLATION is load-bearing: unioning
//! the instances into one graph would SHARE the retained IRI/literal constants and let a retained
//! disjunct bridge a fact from a dropped disjunct's instance with a fact from a third instance
//! through a shared constant, reproducing the dropped head and masking the unsound drop. Witness
//! mode is FAIL-CLOSED on modifiers: a query whose original/raw/minimised tree carries a
//! re-applied FILTER or VALUES (the B3/B4 pass-through) gets NO witness — the modifier
//! could reject every frozen binding, leaving BOTH UCQs empty and the gate vacuously
//! agreed — and is reported per-row as `needs-abox` with NO timing row; such queries are
//! only timed under `--abox` real data. A disagreement aborts the run (exit 1) — it is
//! an unsoundness signal, never papered over with a plausible timing.
//!
//! `--smoke` is the hermetic acceptance path (no network, no files): NPD/Requiem-shaped
//! fixtures with HAND-VERIFIED closed-form minimised-UCQ sizes and pinned certain-answer
//! counts, each asserted before its timing row — same discipline as `ql_rewrite_bench`'s
//! closed forms (which this example does NOT touch).
//!
//! Out-of-scope queries (OPTIONAL/FILTER/aggregation/… — the fail-closed CQ gate) are
//! REPORTED per-row as `out-of-scope`, not errors: on the real NPD query mix that count is
//! itself an honest datum about rewriter coverage.

use oxrdf::Triple;
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};
use sparq_core::Graph;
use sparq_reason_ql::{rewrite, rewrite_production};
use std::time::Instant;

/// Witness-ABox namespace — disjoint from any real suite vocabulary.
const WITNESS: &str = "http://sparq.dev/bench/ql-witness#";

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1_000.0
}

/// Execute a query and return its result SET as (vars, sorted row strings). All fixture and
/// suite queries are `SELECT DISTINCT`, so the sorted rows are a canonical set encoding.
fn eval_rows(graph: &Graph, query: &Query) -> (Vec<String>, Vec<String>) {
    let res = sparq_engine::query(graph, &query.to_string())
        .unwrap_or_else(|e| panic!("rewritten query must execute: {}", e));
    let vars = res.vars.iter().map(|v| v.to_string()).collect();
    let mut rows: Vec<String> = res
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.as_ref().map(|t| t.to_string()).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect();
    rows.sort();
    rows.dedup(); // DISTINCT already dedupes; belt-and-braces so the comparison is set-vs-set
    (vars, rows)
}

/// The UCQ-equivalence sanity check: raw PerfectRef UCQ vs minimised production UCQ must
/// return the SAME result set over the same data. Returns the agreed row count.
fn assert_ucq_equivalence(graph: &Graph, raw: &Query, minimised: &Query, id: &str) -> usize {
    let (raw_vars, raw_rows) = eval_rows(graph, raw);
    let (min_vars, min_rows) = eval_rows(graph, minimised);
    assert_eq!(
        raw_vars, min_vars,
        "{}: raw and minimised UCQ project different variables",
        id
    );
    assert_eq!(
        raw_rows, min_rows,
        "{}: UCQ-EQUIVALENCE FAILURE — minimisation changed the result set (unsoundness signal; timing suppressed)",
        id
    );
    min_rows.len()
}

// =========================================================================================
// --smoke: hermetic NPD/Requiem-shaped fixtures with closed-form UCQ sizes + pinned answers
// =========================================================================================

const TURTLE_PREFIXES: &str = "@prefix : <http://ex/> . \
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> . \
    @prefix owl: <http://www.w3.org/2002/07/owl#> . ";
const SPARQL_PREFIXES: &str = "PREFIX : <http://ex/> \
    PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
    PREFIX owl: <http://www.w3.org/2002/07/owl#> ";

struct SmokeCase {
    id: &'static str,
    tbox_ttl: &'static str,
    abox_ttl: &'static str,
    query: &'static str,
    /// Hand-verified closed-form MINIMISED UCQ size (the size axis of the comparison).
    expected_min_disjuncts: usize,
    /// Pinned certain-answer count over the fixture ABox.
    expected_answers: usize,
}

/// Isolated per-case fixtures (no shared TBox — every closed form is hand-verifiable in
/// isolation). Cases 1–2 mirror NPD shapes (facility/platform/wellbore hierarchy, an
/// operatedBy domain axiom); cases 3–6 mirror Requiem shapes (inverse role, role chain,
/// existential generator, redundant conjunction that minimises).
const SMOKE_CASES: &[SmokeCase] = &[
    SmokeCase {
        id: "npd-class-hierarchy",
        tbox_ttl: ":Platform rdfs:subClassOf :Facility . :Facility rdfs:subClassOf :Asset . \
                   :Wellbore rdfs:subClassOf :Asset .",
        abox_ttl: ":a1 a :Asset . :f1 a :Facility . :p1 a :Platform . :w1 a :Wellbore . :o1 a :Other .",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Asset }",
        // Asset + Facility + Platform + Wellbore — 2-deep chain plus a sibling.
        expected_min_disjuncts: 4,
        expected_answers: 4,
    },
    SmokeCase {
        id: "npd-domain-role",
        tbox_ttl: ":operatedBy rdfs:domain :Facility . :Platform rdfs:subClassOf :Facility .",
        abox_ttl: ":f1 a :Facility . :p1 a :Platform . :x1 :operatedBy :op1 .",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Facility }",
        // Facility + Platform + the ∃operatedBy disjunct the domain axiom introduces.
        expected_min_disjuncts: 3,
        expected_answers: 3,
    },
    SmokeCase {
        id: "requiem-inverse-role",
        tbox_ttl: ":employs owl:inverseOf :worksFor .",
        abox_ttl: ":acme :employs :alice . :bob :worksFor :initech .",
        query: "SELECT DISTINCT ?c ?w WHERE { ?c :employs ?w }",
        // employs(c,w) + worksFor(w,c).
        expected_min_disjuncts: 2,
        expected_answers: 2,
    },
    SmokeCase {
        id: "requiem-role-chain",
        tbox_ttl: ":manages rdfs:subPropertyOf :supervises . :supervises rdfs:subPropertyOf :worksFor .",
        abox_ttl: ":a :worksFor :b . :c :supervises :d . :e :manages :f .",
        query: "SELECT DISTINCT ?x ?y WHERE { ?x :worksFor ?y }",
        // worksFor + supervises + manages.
        expected_min_disjuncts: 3,
        expected_answers: 3,
    },
    SmokeCase {
        id: "requiem-exists-generator",
        tbox_ttl: ":Employee rdfs:subClassOf [ owl:onProperty :worksFor ; owl:someValuesFrom owl:Thing ] .",
        abox_ttl: ":bob :worksFor :initech . :erin a :Employee .",
        query: "SELECT DISTINCT ?x WHERE { ?x :worksFor ?y }",
        // worksFor(x,y) with ?y non-distinguished + the Employee generator.
        expected_min_disjuncts: 2,
        expected_answers: 2,
    },
    SmokeCase {
        id: "requiem-join-minimises",
        tbox_ttl: ":Manager rdfs:subClassOf :Employee .",
        abox_ttl: ":dave a :Manager . :erin a :Employee .",
        query: "SELECT DISTINCT ?x WHERE { ?x a :Manager . ?x a :Employee }",
        // Manager(x) ∧ Employee(x) collapses to Manager(x): minimised UCQ is a single CQ.
        expected_min_disjuncts: 1,
        expected_answers: 1,
    },
];

fn parse_tbox_ttl(ttl: &str) -> Vec<Triple> {
    let full = format!("{}{}", TURTLE_PREFIXES, ttl);
    oxttl::TurtleParser::new()
        .for_reader(full.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .expect("fixture TBox turtle must parse")
}

fn parse_query(body: &str) -> Query {
    SparqlParser::new()
        .parse_query(&format!("{}{}", SPARQL_PREFIXES, body))
        .expect("fixture query must parse")
}

fn run_smoke() {
    println!("ql_npd_requiem_bench --smoke: hermetic NPD/Requiem-shaped fixtures (no network)");
    println!("# regime: *_rewrite_ms = rewriter-phase only (in-process); answers = certain answers over the fixture ABox");
    println!("case\tstatus\traw_disjuncts\tmin_disjuncts\traw_rewrite_ms\tmin_rewrite_ms\tanswers\tequivalence");
    for case in SMOKE_CASES {
        let tbox = parse_tbox_ttl(case.tbox_ttl);
        let abox = Graph::load_str(&format!("{}{}", TURTLE_PREFIXES, case.abox_ttl), "turtle")
            .expect("fixture ABox must load");
        let query = parse_query(case.query);

        let t_raw = Instant::now();
        let raw = rewrite(&query, &tbox)
            .unwrap_or_else(|e| panic!("{} must be in QL scope: {}", case.id, e));
        let raw_ms = ms(t_raw);
        let t_min = Instant::now();
        let minimised = rewrite_production(&query, &tbox)
            .unwrap_or_else(|e| panic!("{} must be in QL scope: {}", case.id, e));
        let min_ms = ms(t_min);

        // --- ALL correctness gates BEFORE the timing row ---
        assert!(
            minimised.report.disjuncts <= minimised.report.disjuncts_before_minimisation,
            "{}: minimisation must only REMOVE disjuncts",
            case.id
        );
        assert!(
            minimised.report.disjuncts <= raw.report.disjuncts,
            "{}: minimised UCQ ({}) larger than raw UCQ ({})",
            case.id,
            minimised.report.disjuncts,
            raw.report.disjuncts
        );
        assert_eq!(
            minimised.report.disjuncts, case.expected_min_disjuncts,
            "{}: minimised UCQ size {} != hand-verified closed form {}",
            case.id, minimised.report.disjuncts, case.expected_min_disjuncts
        );
        let answers = assert_ucq_equivalence(&abox, &raw.query, &minimised.query, case.id);
        assert_eq!(
            answers, case.expected_answers,
            "{}: certain-answer count {} != pinned {}",
            case.id, answers, case.expected_answers
        );

        println!(
            "{}\tok\t{}\t{}\t{:.6}\t{:.6}\t{}\tagree(fixture-abox)",
            case.id, raw.report.disjuncts, minimised.report.disjuncts, raw_ms, min_ms, answers
        );
    }
    println!(
        "SMOKE OK: {} cases — closed-form minimised-UCQ sizes held, raw-vs-minimised result sets agree, answer counts pinned (timings trend-only).",
        SMOKE_CASES.len()
    );
}

// =========================================================================================
// gather mode: <suite> <tbox.nt|.ttl> <queries-dir> [--abox <data.nt|.ttl>]
// =========================================================================================

fn load_triples(path: &str) -> Vec<Triple> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read TBox {}: {}", path, e));
    if path.ends_with(".nt") || path.ends_with(".ntriples") {
        oxttl::NTriplesParser::new()
            .for_reader(text.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|e| panic!("TBox {} N-Triples parse: {}", path, e))
    } else {
        oxttl::TurtleParser::new()
            .for_reader(text.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|e| panic!("TBox {} Turtle parse: {}", path, e))
    }
}

/// Collect the CQ disjuncts (conjunctive bodies) of a query's pattern tree in DISJUNCTIVE
/// NORMAL FORM. Each returned entry is ONE disjunct's BGP; the witness freezes each with a
/// distinct namespace. `Union` yields ALTERNATIVES (its children's disjuncts concatenated).
/// `Join` yields the CONJUNCTION — the cross product of its children's disjuncts, each pair
/// merged into a single BGP (join distributes over union: `(A ∪ B) ⋈ C = (A ⋈ C) ∪ (B ⋈ C)`).
/// This is load-bearing: a variable SHARED ACROSS A JOIN (e.g. `{ { {?x :p ?d} UNION {…} }
/// {?d :q ?u} }`, which survives as a `Join` node because a `Union` branch cannot fold into
/// the adjacent BGP) must land in ONE disjunct so it freezes to a SINGLE constant — treating
/// `Join` like `Union` would freeze the shared variable to two different IRIs, empty the
/// joined CQ over the witness, and make the equivalence gate vacuous on that path. A re-applied `Filter` or `Values`
/// modifier (the B3/B4 pass-through) is REFUSED (`Err` naming the modifier): freezing ignores
/// modifier semantics, so the modifier could reject every frozen binding, leaving BOTH the
/// raw and the minimised UCQ empty and the equivalence gate vacuously agreed — such queries
/// need `--abox` real data. Any other shape means the gate/emitter produced something
/// unexpected: PANIC rather than synthesise a witness that silently under-populates the data.
fn collect_bgps(pattern: &GraphPattern) -> Result<Vec<Vec<TriplePattern>>, &'static str> {
    match pattern {
        GraphPattern::Bgp { patterns } => Ok(vec![patterns.clone()]),
        GraphPattern::Union { left, right } => {
            let mut out = collect_bgps(left)?;
            out.extend(collect_bgps(right)?);
            Ok(out)
        }
        GraphPattern::Join { left, right } => {
            let l = collect_bgps(left)?;
            let r = collect_bgps(right)?;
            // Conjunction of two UCQs: every (left-disjunct, right-disjunct) pair merged
            // into one BGP, so variables shared across the join freeze consistently.
            let mut out = Vec::with_capacity(l.len().saturating_mul(r.len()));
            for dl in &l {
                for dr in &r {
                    let mut merged = dl.clone();
                    merged.extend(dr.iter().cloned());
                    out.push(merged);
                }
            }
            Ok(out)
        }
        GraphPattern::Filter { .. } => Err("FILTER"),
        GraphPattern::Values { .. } => Err("VALUES"),
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => collect_bgps(inner),
        other => panic!(
            "witness ABox synthesis: unsupported pattern shape {:?} — run with --abox",
            other
        ),
    }
}

fn query_pattern(q: &Query) -> &GraphPattern {
    match q {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
        _ => panic!("witness ABox synthesis: expected a SELECT/ASK query"),
    }
}

/// The FROZEN CANONICAL N-TRIPLES of every CQ disjunct of every given query (original, raw
/// UCQ, minimised UCQ), as ONE String PER DISJUNCT — the callers load each string into its
/// OWN isolated database. Freezing maps each variable / blank node to a fresh per-disjunct
/// witness IRI and keeps IRI/literal constants as-is, so every disjunct — query-only
/// predicates and multi-atom shared-variable joins included — matches at least its own
/// instance (the identity homomorphism). FAIL-CLOSED: `Err` (naming the modifier) if any
/// query carries a FILTER or VALUES modifier, whose semantics freezing ignores.
///
/// Emitting per disjunct (rather than one unioned document) is the constant-bridge fix: the
/// retained IRI/literal constants are shared across disjuncts, so a single unioned graph
/// lets a retained disjunct bridge a fact from a dropped disjunct D's instance with a fact
/// from a third disjunct's instance through a shared constant and reproduce D's frozen head
/// even when D is NOT contained in the retained UCQ — masking the unsound drop. Evaluating
/// each disjunct's instance in ISOLATION removes the bridge (see [`witness_aboxes`]).
fn witness_nt_per_disjunct(queries: &[&Query]) -> Result<Vec<String>, &'static str> {
    let mut per_disjunct = Vec::new();
    for (qi, q) in queries.iter().enumerate() {
        let bgps = collect_bgps(query_pattern(q))?;
        for (di, bgp) in bgps.iter().enumerate() {
            // One fresh constant per (query, disjunct, kind, name) — shared variables
            // within a disjunct freeze to the SAME constant, so joins are satisfied.
            let frozen = |kind: &str, name: &str| {
                format!("<{}q{}d{}{}{}>", WITNESS, qi, di, kind, name)
            };
            let mut nt = String::new();
            for tp in bgp.iter() {
                let s = match &tp.subject {
                    TermPattern::NamedNode(n) => format!("<{}>", n.as_str()),
                    TermPattern::BlankNode(b) => frozen("b", b.as_str()),
                    TermPattern::Variable(v) => frozen("v", v.as_str()),
                    other => panic!("witness ABox: unsupported subject pattern {:?}", other),
                };
                let p = match &tp.predicate {
                    NamedNodePattern::NamedNode(n) => format!("<{}>", n.as_str()),
                    NamedNodePattern::Variable(v) => frozen("p", v.as_str()),
                };
                let o = match &tp.object {
                    TermPattern::NamedNode(n) => format!("<{}>", n.as_str()),
                    TermPattern::Literal(l) => l.to_string(), // canonical N-Triples form
                    TermPattern::BlankNode(b) => frozen("b", b.as_str()),
                    TermPattern::Variable(v) => frozen("v", v.as_str()),
                    other => panic!("witness ABox: unsupported object pattern {:?}", other),
                };
                nt.push_str(&format!("{} {} {} .\n", s, p, o));
            }
            per_disjunct.push(nt);
        }
    }
    Ok(per_disjunct)
}

/// Synthesise the deterministic PER-DISJUNCT WITNESS databases for one equivalence check:
/// the FROZEN CANONICAL INSTANCE of every CQ disjunct of every given query (original, raw
/// UCQ, minimised UCQ), each held in its OWN ISOLATED [`Graph`]. Evaluating raw and minimised
/// over EACH database and requiring agreement on each is the classical per-CQ canonical-
/// database containment argument: a kept disjunct produces a dropped disjunct D's frozen head
/// tuple over D's own database only via a homomorphism into D's instance, i.e. only when D was
/// genuinely subsumed, so a minimisation that drops a non-subsumed disjunct fails agreement on
/// that disjunct's database.
///
/// ISOLATION is load-bearing. A SINGLE unioned graph would share the retained IRI/literal
/// constants across disjuncts, letting a retained disjunct bridge D's instance to a third
/// disjunct's instance through a shared constant and reproduce D's head even when D is NOT
/// contained in the retained UCQ — the vacuous agreement that masks the unsound drop (pinned
/// by `isolated_witness_detects_constant_bridge_masking`). One database per disjunct removes
/// the bridge.
///
/// FAIL-CLOSED: `Err` (naming the modifier) if any query carries a FILTER or VALUES modifier,
/// whose semantics freezing ignores — the caller must fall back to `--abox` real data rather
/// than accept a possibly-vacuous agreement. Regression-tested below.
fn witness_aboxes(queries: &[&Query]) -> Result<Vec<Graph>, &'static str> {
    Ok(witness_nt_per_disjunct(queries)?
        .iter()
        .map(|nt| Graph::load_str(nt, "ntriples").expect("witness ABox must load"))
        .collect())
}

fn run_gather(suite: &str, tbox_path: &str, queries_dir: &str, abox_path: Option<&str>) {
    let tbox = load_triples(tbox_path);
    let real_abox: Option<Graph> = abox_path.map(|p| {
        let text = std::fs::read_to_string(p)
            .unwrap_or_else(|e| panic!("cannot read ABox {}: {}", p, e));
        let fmt = if p.ends_with(".nt") || p.ends_with(".ntriples") { "ntriples" } else { "turtle" };
        Graph::load_str(&text, fmt).unwrap_or_else(|e| panic!("ABox {} load: {}", p, e))
    });
    let data_label = if real_abox.is_some() { "abox" } else { "witness-abox" };

    let mut query_files: Vec<std::path::PathBuf> = std::fs::read_dir(queries_dir)
        .unwrap_or_else(|e| panic!("cannot read queries dir {}: {}", queries_dir, e))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "rq" || ext == "sparql"))
        .collect();
    query_files.sort();
    assert!(
        !query_files.is_empty(),
        "no .rq/.sparql query files in {}",
        queries_dir
    );

    println!(
        "# suite {}: TBox {} ({} triples), {} queries, equivalence data = {} ({})",
        suite,
        tbox_path,
        tbox.len(),
        query_files.len(),
        data_label,
        abox_path.unwrap_or(
            "per-disjunct ISOLATED frozen canonical instances of the original/raw/minimised UCQ \
             disjuncts (evaluated separately so shared IRI/literal constants cannot bridge \
             across instances); FILTER/VALUES queries fail closed to needs-abox (no timing row)"
        )
    );
    println!("# regime: *_rewrite_ms = rewriter-phase only (in-process rewrite/rewrite_production);");
    println!("#         exec_ms = minimised-UCQ execution over the loaded data; e2e_ms = rewrite+execute (comparable to an end-to-end competitor column)");
    println!("suite\tcase\tstatus\traw_disjuncts\tmin_disjuncts\tskipped_axioms\traw_rewrite_ms\tmin_rewrite_ms\tequivalence\tanswers\texec_ms\te2e_ms");

    let (mut in_scope, mut out_of_scope, mut needs_abox, mut parse_errors) =
        (0usize, 0usize, 0usize, 0usize);
    for path in &query_files {
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("query");
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read query {}: {}", path.display(), e));
        let query = match SparqlParser::new().parse_query(&text) {
            Ok(q) => q,
            Err(e) => {
                parse_errors += 1;
                println!(
                    "{}\t{}\tparse-error: {}\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA",
                    suite,
                    id,
                    e.to_string().replace(['\t', '\n'], " ")
                );
                continue;
            }
        };

        let t_raw = Instant::now();
        let raw = match rewrite(&query, &tbox) {
            Ok(r) => r,
            Err(e) => {
                out_of_scope += 1;
                println!(
                    "{}\t{}\tout-of-scope: {}\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA",
                    suite,
                    id,
                    e.to_string().replace(['\t', '\n'], " ")
                );
                continue;
            }
        };
        let raw_ms = ms(t_raw);
        let t_min = Instant::now();
        let minimised = rewrite_production(&query, &tbox)
            .unwrap_or_else(|e| panic!("{}: raw rewrite succeeded but production failed: {}", id, e));
        let min_ms = ms(t_min);

        // UCQ-equivalence sanity check BEFORE the timing row (the sq-hmd7l.9 invariant).
        // Without --abox the data is the PER-DISJUNCT witness: one ISOLATED frozen canonical
        // instance per disjunct of the original, raw, and minimised queries. Raw and minimised
        // must agree over EACH database separately — isolation stops shared IRI/literal
        // constants from bridging a dropped disjunct's instance to another's (which would mask
        // an unsound drop). FAIL-CLOSED: a FILTER/VALUES modifier anywhere in the
        // original/raw/minimised tree refuses witness mode (the modifier could reject every
        // frozen binding and make the agreement vacuous) — needs-abox row and NO timings.
        let witness_dbs;
        let data: Vec<&Graph> = match &real_abox {
            Some(g) => vec![g],
            None => match witness_aboxes(&[&query, &raw.query, &minimised.query]) {
                Ok(gs) => {
                    witness_dbs = gs;
                    witness_dbs.iter().collect()
                }
                Err(modifier) => {
                    needs_abox += 1;
                    println!(
                        "{}\t{}\tneeds-abox: {} present — witness mode fails closed (frozen bindings could fail the modifier, gate would be vacuous); rerun with --abox\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA",
                        suite, id, modifier
                    );
                    continue;
                }
            },
        };
        // Agreement is required on EVERY database; `answers` totals the agreed rows across them
        // (a single iteration over the one real ABox with --abox, unchanged).
        let answers: usize = data
            .iter()
            .map(|g| assert_ucq_equivalence(g, &raw.query, &minimised.query, id))
            .sum();

        // End-to-end leg (rewrite + execute), labelled; execution-only measured separately.
        // Both legs run over every database, so witness-mode timing covers all disjunct
        // instances (a single iteration over the one real ABox with --abox, unchanged).
        let t_exec = Instant::now();
        for g in &data {
            let _ = sparq_engine::count(g, &minimised.query.to_string())
                .unwrap_or_else(|e| panic!("{}: minimised UCQ execution failed: {}", id, e));
        }
        let exec_ms = ms(t_exec);
        let t_e2e = Instant::now();
        let e2e = rewrite_production(&query, &tbox).expect("second rewrite must succeed");
        for g in &data {
            let _ = sparq_engine::count(g, &e2e.query.to_string())
                .unwrap_or_else(|e| panic!("{}: e2e execution failed: {}", id, e));
        }
        let e2e_ms = ms(t_e2e);

        in_scope += 1;
        println!(
            "{}\t{}\tok\t{}\t{}\t{}\t{:.6}\t{:.6}\tagree({})\t{}\t{:.6}\t{:.6}",
            suite,
            id,
            raw.report.disjuncts,
            minimised.report.disjuncts,
            minimised.report.skipped_axioms,
            raw_ms,
            min_ms,
            data_label,
            answers,
            exec_ms,
            e2e_ms
        );
    }
    println!(
        "# summary suite {}: {} in-scope, {} out-of-scope (fail-closed CQ gate), {} needs-abox (witness mode refused: FILTER/VALUES), {} parse-errors of {} queries",
        suite,
        in_scope,
        out_of_scope,
        needs_abox,
        parse_errors,
        query_files.len()
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--smoke") => run_smoke(),
        Some(suite) if args.len() >= 3 => {
            let abox = args
                .iter()
                .position(|a| a == "--abox")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            run_gather(suite, &args[1], &args[2], abox);
        }
        _ => {
            eprintln!(
                "usage: ql_npd_requiem_bench --smoke | <suite> <tbox.nt|.ttl> <queries-dir> [--abox <data.nt|.ttl>]"
            );
            std::process::exit(2);
        }
    }
}

// =========================================================================================
// Witness-gate regression tests (run by `cargo test -p sparq-reason-ql --features
// experimental` — the [[example]] entry sets `test = true`). They pin the vacuity the old
// TBox-vocabulary witness had for query-only predicates + multi-atom joins, prove the
// frozen-instance witness both populates every disjunct and DETECTS a wrongly-dropped one,
// and pin that PER-DISJUNCT ISOLATION detects a constant-bridge masking that a single unioned
// witness graph agrees on vacuously.
// =========================================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// A raw UCQ whose predicates occur ONLY in the query (never in any TBox): a multi-atom
    /// shared-variable join disjunct, plus a literal-object disjunct.
    const RAW_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { ?x :headOf ?d . ?d :partOf ?u } UNION { ?x :label \"w\" } }";
    /// A deliberately WRONG minimisation of [`RAW_UCQ`]: the non-subsumed join disjunct is
    /// dropped, so the two queries are semantically different.
    const WRONG_MIN_UCQ: &str = "SELECT DISTINCT ?x WHERE { ?x :label \"w\" }";
    /// A sound minimisation of [`RAW_UCQ`] (same UCQ, disjuncts reordered).
    const EQUIV_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { ?x :label \"w\" } UNION { ?x :headOf ?d . ?d :partOf ?u } }";
    /// [`RAW_UCQ`] under a distinguished-variable FILTER (the B3 pass-through shape the
    /// emitter produces). No frozen witness IRI can ever equal `:alice`, so the filter
    /// rejects EVERY frozen binding.
    const FILTERED_RAW_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { { ?x :headOf ?d . ?d :partOf ?u } UNION { ?x :label \"w\" } } FILTER(?x = :alice) }";
    /// A deliberately WRONG minimisation of [`FILTERED_RAW_UCQ`]: the non-subsumed join
    /// disjunct is dropped, under the same FILTER.
    const FILTERED_WRONG_MIN_UCQ: &str =
        "SELECT DISTINCT ?x WHERE { { ?x :label \"w\" } FILTER(?x = :alice) }";
    /// [`RAW_UCQ`] joined with a VALUES block (the B4 pass-through shape): the bound
    /// constant can never equal a frozen witness IRI.
    const VALUES_RAW_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { { ?x :headOf ?d . ?d :partOf ?u } UNION { ?x :label \"w\" } } VALUES ?x { :alice } }";
    /// A raw UCQ that survives as a real `Join(Union, Bgp)` — spargebra folds ADJACENT BGPs
    /// into one BGP, so a genuine `Join` node needs a non-mergeable branch (here a `Union`).
    /// `?d` is shared ACROSS the join; in DNF this is two CQ disjuncts, `headOf(x,d) ∧
    /// partOf(d,u)` and `chairOf(x,d) ∧ partOf(d,u)`. Frozen like `Union` (each atom its own
    /// disjunct), `?d` would freeze to different IRIs and neither joined CQ would match.
    const JOIN_RAW_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { { ?x :headOf ?d } UNION { ?x :chairOf ?d } } { ?d :partOf ?u } }";
    /// A deliberately WRONG minimisation of [`JOIN_RAW_UCQ`]: the non-subsumed `headOf` join
    /// disjunct is dropped, keeping only `chairOf(x,d) ∧ partOf(d,u)`.
    const JOIN_WRONG_MIN_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { ?x :chairOf ?d } { ?d :partOf ?u } }";

    /// The CONSTANT-BRIDGE masking fixture (review round 6). A UCQ whose disjuncts share the
    /// IRI constant `:c`: dropped D `{?x :p :c}`, retained E `{?x :p :c . :c :q ?z}`, retained
    /// F `{:c :q ?x}`. D is NOT contained in E ∪ F — an `x` with `x :p :c` but no outgoing
    /// `:c :q _` is a D answer only — so dropping D is UNSOUND.
    const BRIDGE_RAW_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { ?x :p :c } UNION { ?x :p :c . :c :q ?z } UNION { :c :q ?x } }";
    /// A deliberately WRONG minimisation of [`BRIDGE_RAW_UCQ`]: the non-subsumed D `{?x :p :c}`
    /// disjunct is dropped. Over a SINGLE unioned witness graph the shared `:c` lets E borrow a
    /// `:c :q _` fact from another disjunct's instance and reproduce D's frozen `?x` via D's own
    /// `?x :p :c` fact, so raw and this wrongly-minimised UCQ agree VACUOUSLY.
    const BRIDGE_WRONG_MIN_UCQ: &str = "SELECT DISTINCT ?x WHERE { \
        { ?x :p :c . :c :q ?z } UNION { :c :q ?x } }";

    /// True iff the pattern tree contains a `Join` node (the shape the regression exercises).
    fn query_has_join(p: &GraphPattern) -> bool {
        match p {
            GraphPattern::Join { .. } => true,
            GraphPattern::Union { left, right } => {
                query_has_join(left) || query_has_join(right)
            }
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => query_has_join(inner),
            _ => false,
        }
    }

    /// The PRE-FIX witness construction, reproduced literally for the TBox
    /// `:Manager rdfs:subClassOf :Employee .`: one rdf:type assertion per TBox class. The
    /// query-only predicates above receive no data from it, so BOTH the raw and the wrongly
    /// minimised UCQ evaluate to the empty set and the old gate agreed VACUOUSLY — the
    /// defect this regression pins.
    #[test]
    fn tbox_vocabulary_witness_was_vacuous_for_query_only_predicates() {
        let data = Graph::load_str(
            "<http://sparq.dev/bench/ql-witness#ci0> \
             <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Employee> .\n\
             <http://sparq.dev/bench/ql-witness#ci1> \
             <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Manager> .\n",
            "ntriples",
        )
        .expect("old-style witness must load");
        let (_, raw_rows) = eval_rows(&data, &parse_query(RAW_UCQ));
        let (_, wrong_rows) = eval_rows(&data, &parse_query(WRONG_MIN_UCQ));
        assert!(
            raw_rows.is_empty() && wrong_rows.is_empty(),
            "TBox-vocabulary witness leaves query-only predicates empty on both sides — \
             the vacuous agreement the per-query frozen-instance witness replaces"
        );
    }

    /// The frozen-instance witness populates the join disjunct (query-only predicates,
    /// shared-variable structure) AND the literal-object disjunct, and the equivalence gate
    /// DETECTS the dropped non-subsumed disjunct as a result-set mismatch on some disjunct's
    /// own isolated database.
    #[test]
    fn frozen_instance_witness_detects_wrongly_dropped_join_disjunct() {
        let raw = parse_query(RAW_UCQ);
        let wrong = parse_query(WRONG_MIN_UCQ);
        let dbs = witness_aboxes(&[&raw, &wrong]).expect("modifier-free UCQs must synthesise");
        assert!(
            dbs.iter().any(|g| !eval_rows(g, &raw).1.is_empty()),
            "every raw disjunct must match its own frozen instance"
        );
        assert!(
            dbs.iter().any(|g| !eval_rows(g, &wrong).1.is_empty()),
            "the literal-object disjunct must match its own frozen instance"
        );
        assert!(
            dbs.iter().any(|g| eval_rows(g, &raw).1 != eval_rows(g, &wrong).1),
            "dropping a non-subsumed disjunct must surface as a result-set mismatch on some \
             disjunct's isolated instance"
        );
    }

    /// A sound minimisation (equivalent UCQ, reordered disjuncts) still agrees over the
    /// frozen-instance witness, with a non-empty (non-vacuous) agreed row set.
    #[test]
    fn frozen_instance_witness_agrees_for_equivalent_ucqs() {
        let raw = parse_query(RAW_UCQ);
        let equiv = parse_query(EQUIV_UCQ);
        let dbs = witness_aboxes(&[&raw, &equiv]).expect("modifier-free UCQs must synthesise");
        let answers: usize = dbs
            .iter()
            .map(|g| assert_ucq_equivalence(g, &raw, &equiv, "equivalent-ucqs"))
            .sum();
        assert!(answers > 0, "the agreement must be over a non-empty row set");
    }

    /// The vacuity a modifier-blind witness has under FILTER: reproduce the pre-fix path
    /// (freeze the BGPs, ignore the modifier) by synthesising from the modifier-FREE
    /// skeletons — identical BGPs, so identical frozen instances — then evaluate the
    /// FILTERed queries over it. The filter rejects every frozen binding, so the raw UCQ
    /// AND a wrongly-minimised UCQ (non-subsumed join disjunct dropped) BOTH come back
    /// empty: the gate would agree without detecting the drop. This is the vacuous
    /// agreement witness mode now refuses.
    #[test]
    fn modifier_blind_witness_is_vacuous_under_filter() {
        let dbs = witness_aboxes(&[&parse_query(RAW_UCQ), &parse_query(WRONG_MIN_UCQ)])
            .expect("modifier-free skeletons must synthesise");
        for g in &dbs {
            let (_, raw_rows) = eval_rows(g, &parse_query(FILTERED_RAW_UCQ));
            let (_, wrong_rows) = eval_rows(g, &parse_query(FILTERED_WRONG_MIN_UCQ));
            assert!(
                raw_rows.is_empty() && wrong_rows.is_empty(),
                "the FILTER rejects every frozen binding: raw and wrongly-minimised UCQs are \
                 both empty, so an equivalence check over this witness would pass vacuously"
            );
        }
    }

    /// The revised path REFUSES witness mode for FILTER- and VALUES-carrying queries
    /// (fail-closed): `witness_abox` returns `Err` naming the modifier, and `run_gather`
    /// emits a `needs-abox` row with NO timings instead of a vacuously-gated timing row.
    #[test]
    fn witness_mode_refuses_filter_and_values_queries() {
        let err = witness_aboxes(&[&parse_query(FILTERED_RAW_UCQ)])
            .map(|_| ())
            .expect_err("a FILTER anywhere in the tree must refuse witness synthesis");
        assert_eq!(err, "FILTER");
        let err = witness_aboxes(&[&parse_query(VALUES_RAW_UCQ)])
            .map(|_| ())
            .expect_err("a VALUES block anywhere in the tree must refuse witness synthesis");
        assert_eq!(err, "VALUES");
        // The refusal covers the whole query LIST: one modifier-carrying query poisons
        // the witness even when the others are plain UCQs.
        let err = witness_aboxes(&[&parse_query(RAW_UCQ), &parse_query(FILTERED_WRONG_MIN_UCQ)])
            .map(|_| ())
            .expect_err("a single modifier-carrying query in the list must refuse synthesis");
        assert_eq!(err, "FILTER");
    }

    /// A `Join(Bgp, Bgp)` sharing a variable across the join must freeze that variable to a
    /// SINGLE constant: the conjunction is one CQ, not two disjuncts. The join disjunct then
    /// matches its own frozen instance, so the witness is non-empty and DETECTS a wrongly
    /// dropped non-subsumed join disjunct — instead of the vacuous agreement that treating
    /// `Join` like `Union` produced (`?d` frozen to two IRIs → the joined CQ empty on both
    /// sides). Pins the regression for the admitted nested-group Join path.
    #[test]
    fn frozen_instance_witness_handles_join_shared_variable() {
        let raw = parse_query(JOIN_RAW_UCQ);
        assert!(
            query_has_join(query_pattern(&raw)),
            "fixture must exercise the Join path (nested groups should parse as Join)"
        );
        let wrong = parse_query(JOIN_WRONG_MIN_UCQ);
        let dbs = witness_aboxes(&[&raw, &wrong]).expect("modifier-free UCQs must synthesise");
        assert!(
            dbs.iter().any(|g| !eval_rows(g, &raw).1.is_empty()),
            "the shared-variable join disjunct must match its own frozen instance"
        );
        assert!(
            dbs.iter().any(|g| eval_rows(g, &raw).1 != eval_rows(g, &wrong).1),
            "dropping the non-subsumed join disjunct must surface as a result-set mismatch on \
             some disjunct's isolated instance"
        );
    }

    /// The constant-bridge regression: a UCQ whose disjuncts share the IRI constant `:c`
    /// (D/E/F, see [`BRIDGE_RAW_UCQ`]). A SINGLE unioned witness graph MASKS the unsound drop
    /// of D — the shared `:c` bridges D's instance to another disjunct's instance so raw and
    /// the wrongly-minimised UCQ agree vacuously — whereas PER-DISJUNCT ISOLATED databases
    /// remove the bridge and DETECT the drop on D's own instance. Pins that isolation, not the
    /// single unioned graph, is what makes the equivalence gate sound.
    #[test]
    fn isolated_witness_detects_constant_bridge_masking() {
        let raw = parse_query(BRIDGE_RAW_UCQ);
        let wrong = parse_query(BRIDGE_WRONG_MIN_UCQ);
        let per_disjunct = witness_nt_per_disjunct(&[&raw, &wrong])
            .expect("modifier-free UCQs must synthesise");

        // Pre-fix construction: unioning the per-disjunct instances into ONE graph shares the
        // `:c` constant across instances and masks the drop — raw and wrong agree vacuously.
        let union = Graph::load_str(&per_disjunct.concat(), "ntriples")
            .expect("unioned witness graph must load");
        assert_eq!(
            eval_rows(&union, &raw).1,
            eval_rows(&union, &wrong).1,
            "the unioned witness masks the unsound drop: the shared `:c` bridges the instances \
             so raw and the wrongly-minimised UCQ agree — the defect this regression pins"
        );

        // Fixed construction: one ISOLATED database per disjunct removes the bridge, so raw and
        // wrong disagree on D `{?x :p :c}`'s own instance (E/F cannot reproduce D's frozen ?x
        // there — that instance carries no `:c :q _` fact for E/F to join through).
        let dbs: Vec<Graph> = per_disjunct
            .iter()
            .map(|nt| Graph::load_str(nt, "ntriples").expect("isolated database must load"))
            .collect();
        assert!(
            dbs.iter().any(|g| eval_rows(g, &raw).1 != eval_rows(g, &wrong).1),
            "per-disjunct isolation must detect the wrongly dropped disjunct D as a result-set \
             mismatch on its own canonical instance"
        );
    }

    /// Drive the whole `--smoke` harness (`run_smoke` + `parse_tbox_ttl` + `ms` + every
    /// `SMOKE_CASE` closed-form / answer-count assertion) as a hermetic regression, so the
    /// smoke leg's pins are checked on every `cargo test` — not only when the example is
    /// invoked as a binary. A panic in any smoke assertion fails this test.
    #[test]
    fn smoke_harness_runs_all_closed_forms() {
        run_smoke();
    }

    /// Drive the gather harness end to end over temp fixtures in BOTH data regimes —
    /// witness mode (no `--abox`) and real-`--abox` mode — exercising `run_gather`,
    /// `load_triples` (Turtle and N-Triples branches), and the in-scope / out-of-scope /
    /// parse-error row paths. This keeps the gather leg regression-tested rather than
    /// dead-at-test-time; the smoke path above covers the closed-form correctness pins.
    #[test]
    fn gather_harness_runs_witness_and_abox_modes() {
        let dir = std::env::temp_dir().join(format!("ql_npd_requiem_gather_{}", std::process::id()));
        let qdir = dir.join("queries");
        std::fs::create_dir_all(&qdir).expect("temp queries dir must create");

        // In-scope class query (witness mode proceeds; UCQ has no re-applied modifier).
        std::fs::write(
            qdir.join("in_scope.rq"),
            format!("{}SELECT DISTINCT ?x WHERE {{ ?x a :Employee }}", SPARQL_PREFIXES),
        )
        .expect("in-scope query must write");
        // OPTIONAL is out of the fail-closed CQ gate — `rewrite` rejects it (out-of-scope row).
        std::fs::write(
            qdir.join("out_of_scope.rq"),
            format!(
                "{}SELECT DISTINCT ?x WHERE {{ ?x a :Employee OPTIONAL {{ ?x a :Manager }} }}",
                SPARQL_PREFIXES
            ),
        )
        .expect("out-of-scope query must write");
        // Unparseable text drives the parse-error row.
        std::fs::write(qdir.join("bad.rq"), "this is not sparql").expect("bad query must write");

        // Witness mode: Turtle TBox exercises the Turtle branch of `load_triples`.
        let tbox_ttl = dir.join("tbox.ttl");
        std::fs::write(
            &tbox_ttl,
            format!("{}:Manager rdfs:subClassOf :Employee .", TURTLE_PREFIXES),
        )
        .expect("Turtle TBox must write");
        run_gather("temp-witness", tbox_ttl.to_str().unwrap(), qdir.to_str().unwrap(), None);

        // Real-ABox mode: N-Triples TBox + ABox exercise the N-Triples branch of
        // `load_triples` and the `Some(real_abox)` execution path in `run_gather`.
        let tbox_nt = dir.join("tbox.nt");
        std::fs::write(
            &tbox_nt,
            "<http://ex/Manager> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/Employee> .\n",
        )
        .expect("N-Triples TBox must write");
        let abox_nt = dir.join("abox.nt");
        std::fs::write(
            &abox_nt,
            "<http://ex/m1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Manager> .\n\
             <http://ex/e1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Employee> .\n",
        )
        .expect("N-Triples ABox must write");
        run_gather(
            "temp-abox",
            tbox_nt.to_str().unwrap(),
            qdir.to_str().unwrap(),
            Some(abox_nt.to_str().unwrap()),
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
