//! [OPUS-4.8] sq-e5atd (epic sq-pbz04) — the D-entailment (datatype / value-space)
//! conformance lane, wired as a RATCHETED gate that mirrors the SPARQL / SHACL /
//! GeoSPARQL / Solid / JSON-LD ratchets in this crate (crate-local `cargo test` +
//! a pinned pass-count FLOOR that may only RISE, registered in the central
//! `scoreboard::SUITES` and guarded by `tests/scoreboard_floors.rs`).
//!
//! ## What is gated
//!
//! The genuinely D-ONLY tests of the W3C `sparql11/entailment` suite — those whose
//! `sd:entailmentRegime` is `ent:D` WITHOUT any stronger RDFS/RDF/OWL-RDF-Based
//! regime (those are exercised under their own regime by the inference binary). Each
//! is driven through the REAL D path: the premise data is materialized through
//! `sparq_reason::Profile::D` (the rdfD1 datatype-typing closure, with the typed
//! value-space comparator — `"1"^^xsd:integer` ≡ `"1.0"^^xsd:decimal`, NOT an f64
//! fast path), then the query is run over the closure through the SAME
//! evaluation/comparison machinery as the gating SPARQL harness, with the SPARQL
//! 1.1 Entailment-Regimes answer restriction applied (literal-subject + surrogate
//! blank-node bindings are never answers).
//!
//! These tests previously sat in the inference scoreboard's OutOfScope bucket
//! ("entailment regime(s) D not supported"); this lane GRADUATES them to Pass.
//!
//! ## Honest floor + the tracked-not-asserted remainder
//!
//! `D_ENTAIL_FLOOR` is the MEASURED D-only pass count at the pinned rdf-tests
//! revision, NOT an aspirational target. At the current pin the suite contains a
//! single D-only test (`d-ent-01`), which passes — so the floor is 1. The broader
//! D-entailment surface (D-inconsistency / ill-typed-literal clashes, value-space
//! subset reasoning across the integer/decimal/temporal spaces, the rdf-mt
//! recognized-datatype tests that ride RDFS rather than D-only) is exercised
//! elsewhere — the inference binary's rdf-mt section, and `sparq-reason`'s own
//! `dtype` unit tests for the value-space invariant — and the rest is honestly
//! tracked-not-asserted (a child of the D-entailment epic), never skip-laundered to
//! inflate this count. The floor may only RISE as the D-only corpus grows.
//!
//! ## Feature gating (both states)
//!
//! The whole lane is behind this crate's opt-in `d-entail` feature (forwards to
//! `sparq-reason/d-entail`). With the feature OFF this file compiles to a single
//! self-SKIP `#[test]` (no `Profile::D` code links, and the inference BINARY keeps
//! these tests OutOfScope, so its ratchet floor is byte-for-byte unchanged — the
//! lean opt-in posture). With it ON the runner executes the D-only tests and
//! asserts the pinned floor. The rdf-tests fixtures are fetched by
//! `scripts/fetch-inference-suites.sh` into the gitignored `tests/w3c/rdf-tests/`;
//! when absent the runner SKIPS so a fresh offline checkout stays green.

// [OPUS-4.8] When the lane feature is OFF the runner is a single self-SKIP test so
// the default + `--workspace` builds neither link the D materializer nor go red on
// a fresh checkout. (cfg gate, not a runtime branch, so zero D-entailment code
// compiles in the default state — the lean-core invariant.)
#[cfg(not(feature = "d-entail"))]
#[test]
fn d_entail_suite_skipped_without_feature() {
    eprintln!(
        "SKIP: D-entailment conformance lane is OFF — build with \
         `--features d-entail` (and run scripts/fetch-inference-suites.sh) to run it."
    );
}

#[cfg(feature = "d-entail")]
mod gated {
    use sparq_conformance::inference::report::{Outcome, TestResult};
    use sparq_conformance::inference::sparql_entail;
    use std::path::PathBuf;

    /// The D-entailment pass floor (the RATCHET). MEASURED D-only pass count at the
    /// pinned rdf-tests revision; MIRRORED in the central scoreboard
    /// (`scoreboard::SUITES`) and read textually by the guard test
    /// `tests/scoreboard_floors.rs`. It may only RISE — never lower it (raise as the
    /// D-only corpus / the materializer's value-space coverage grow). This is the
    /// ACTUAL current pass count, not an aspirational target.
    pub const D_ENTAIL_FLOOR: usize = 1;

    fn rdf_tests_root() -> PathBuf {
        // CARGO_MANIFEST_DIR is crates/sparq-conformance; the data lives at the
        // workspace-root `tests/w3c/rdf-tests` (the same clone the SPARQL +
        // inference harnesses use, fetched by scripts/fetch-inference-suites.sh).
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdf-tests")
    }

    #[test]
    fn d_entailment_ratchet() {
        let root = rdf_tests_root();
        if !root
            .join("sparql/sparql11/entailment/manifest.ttl")
            .exists()
        {
            eprintln!(
                "SKIP: rdf-tests `sparql11/entailment` not present under {} — run \
                 scripts/fetch-inference-suites.sh",
                root.display()
            );
            return;
        }

        let mut results: Vec<TestResult> = Vec::new();
        sparql_entail::run_d_regime_suite(&root, &mut results)
            .unwrap_or_else(|e| panic!("D-regime suite error: {e}"));

        let mut pass = 0usize;
        let mut fail = 0usize;
        let mut oos = 0usize;
        let mut fails: Vec<String> = Vec::new();
        for r in &results {
            match &r.outcome {
                Outcome::Pass | Outcome::Divergence(_, _) => pass += 1,
                Outcome::Fail(why) => {
                    fail += 1;
                    fails.push(format!("{}: {}", r.name, why));
                }
                Outcome::OutOfScope(_) => oos += 1,
            }
        }

        // [OPUS-4.8] The CI ratchet greps this exact `TOTAL d-entail` line (pass
        // count in field $3), mirroring the JSON-LD lane's `TOTAL <cat>` shape.
        // Positional `format!`/`println!` args (not inline `{pass}`) per the CodeQL
        // `rust/unused-variable` false-positive guard in the shared agent contract.
        println!("\nW3C SPARQL 1.1 D-entailment conformance (pinned rdf-tests)");
        println!(
            "TOTAL d-entail {} {} {} (floor {})",
            pass, fail, oos, D_ENTAIL_FLOOR
        );
        if !fails.is_empty() {
            println!("D-entailment FAILS:");
            for f in &fails {
                println!("  - {}", f);
            }
        }

        assert_eq!(fail, 0, "D-entailment lane has failing tests: {:?}", fails);
        assert!(
            pass >= D_ENTAIL_FLOOR,
            "D-entailment pass count {} regressed below the ratchet floor {}",
            pass,
            D_ENTAIL_FLOOR
        );
    }

    // ── [FABLE-5] sq-pbz04.6.4 — the crate-local D VALUE-SPACE MATRIX arm ────────────
    //
    // A sparq-EXTENSION ratchet, tallied SEPARATELY from the W3C `D_ENTAIL_FLOOR` above
    // (mirroring the OWL 2 QL certain-answer-oracle precedent, program honesty rule 4:
    // the standards-conformance count is NOT padded with sparq's own extension cases).
    // The W3C `sparql11/entailment` corpus is a SINGLE D-only test; the real
    // value-space coverage lives here, driven through the REAL `Profile::D` materializer
    // + the same value-space comparator (`d_value_eq`/`d_value_key`, now on the shared
    // `sparq-substrate` seam — sq-pbz04.6.3) + the same end-to-end
    // materialize→answer-restriction→engine-query path as the W3C lane.
    //
    // FLOOR = the MEASURED assertion count (a sparq EXTENSION unit, never aspirational),
    // may only RISE. Mirrored in the central scoreboard (`scoreboard::SUITES`) and pinned
    // by `tests/scoreboard_floors.rs` (read textually).
    pub const D_VALUE_MATRIX_FLOOR: usize = 24;

    use sparq_core::dict::{Dict, Id};
    use sparq_reason::{d_value_eq, d_value_key, materialize_d, Profile, Recognized};

    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

    /// A value-space matrix case: two typed literals + whether they denote the SAME
    /// D-value. Drives the REAL `d_value_eq` (the substrate-delegated value-space key).
    struct EqCase {
        a_lex: &'static str,
        a_dt: &'static str,
        b_lex: &'static str,
        b_dt: &'static str,
        equal: bool,
        why: &'static str,
    }

    /// A well-formedness (rdfD1-typability) matrix case: a single typed literal + whether
    /// it has a D-VALUE (is well-formed for its recognized datatype). An ill-formed literal
    /// has NO value and rdfD1 must NOT type it.
    struct WellFormedCase {
        lex: &'static str,
        dt: &'static str,
        well_formed: bool,
        why: &'static str,
    }

    fn dt(local: &str) -> String {
        format!("{}{}", XSD, local)
    }

    /// The value-EQUALITY matrix: value-equal-distinct-lexical pairs (integer/decimal incl.
    /// the 2^53+1 guard, boolean true/1, the hex/base64 octet pair) + disjoint-space
    /// negatives (decimal vs double, date vs dateTime at a shared instant).
    fn eq_cases() -> Vec<EqCase> {
        vec![
            // ── value-equal-distinct-lexical POSITIVES ──
            EqCase {
                a_lex: "1",
                a_dt: "integer",
                b_lex: "1.0",
                b_dt: "decimal",
                equal: true,
                why: "integer ⊂ decimal: 1 == 1.0",
            },
            EqCase {
                a_lex: "01",
                a_dt: "integer",
                b_lex: "+1",
                b_dt: "integer",
                equal: true,
                why: "leading zero / sign are the same value",
            },
            EqCase {
                a_lex: "-0",
                a_dt: "integer",
                b_lex: "0",
                b_dt: "decimal",
                equal: true,
                why: "signed zero == zero",
            },
            EqCase {
                a_lex: "1.50",
                a_dt: "decimal",
                b_lex: "1.5",
                b_dt: "decimal",
                equal: true,
                why: "trailing fraction zeros are insignificant",
            },
            EqCase {
                a_lex: "true",
                a_dt: "boolean",
                b_lex: "1",
                b_dt: "boolean",
                equal: true,
                why: "boolean true == 1",
            },
            EqCase {
                a_lex: "false",
                a_dt: "boolean",
                b_lex: "0",
                b_dt: "boolean",
                equal: true,
                why: "boolean false == 0",
            },
            EqCase {
                a_lex: "61",
                a_dt: "hexBinary",
                b_lex: "YQ==",
                b_dt: "base64Binary",
                equal: true,
                why: "octet [0x61] is one value across hex/base64 (D2)",
            },
            // ── DISJOINT-space + distinct-value NEGATIVES ──
            EqCase {
                a_lex: "1.0",
                a_dt: "decimal",
                b_lex: "1.0",
                b_dt: "double",
                equal: false,
                why: "decimal and double are DISJOINT primitive value spaces",
            },
            EqCase {
                a_lex: "1",
                a_dt: "integer",
                b_lex: "1.0",
                b_dt: "double",
                equal: false,
                why: "integer/decimal space is disjoint from IEEE double",
            },
            EqCase {
                a_lex: "1.0",
                a_dt: "decimal",
                b_lex: "1.0",
                b_dt: "float",
                equal: false,
                why: "decimal and float are DISJOINT value spaces",
            },
            EqCase {
                a_lex: "2004-04-12T13:20:00Z",
                a_dt: "dateTime",
                b_lex: "2004-04-12",
                b_dt: "date",
                equal: false,
                why: "date and dateTime are DISJOINT value spaces even at a shared instant",
            },
            EqCase {
                a_lex: "1",
                a_dt: "integer",
                b_lex: "2",
                b_dt: "decimal",
                equal: false,
                why: "distinct numeric values",
            },
            // ── the 2^53+1 NON-ALIASING guard (an f64 would wrongly equate these) ──
            EqCase {
                a_lex: "9007199254740993",
                a_dt: "integer",
                b_lex: "9007199254740992",
                b_dt: "integer",
                equal: false,
                why: "2^53+1 must NOT alias 2^53 (unsound-f64 guard)",
            },
            EqCase {
                a_lex: "9007199254740993",
                a_dt: "integer",
                b_lex: "9007199254740993.0",
                b_dt: "decimal",
                equal: true,
                why: "2^53+1 exactly equals its own decimal spelling",
            },
        ]
    }

    /// The WELL-FORMEDNESS (facet) matrix: facet-ill-formed negatives (rdfD1 must NOT type
    /// `200^^xsd:byte`, a leading-space `xsd:token`) + well-formed positives.
    fn well_formed_cases() -> Vec<WellFormedCase> {
        vec![
            // ── facet-ill-formed NEGATIVES (rdfD1 must NOT type) ──
            WellFormedCase {
                lex: "200",
                dt: "byte",
                well_formed: false,
                why: "200 is outside xsd:byte [-128,127] — no D-value",
            },
            WellFormedCase {
                lex: " a",
                dt: "token",
                well_formed: false,
                why: "leading space is illegal for xsd:token",
            },
            WellFormedCase {
                lex: "4294967296",
                dt: "unsignedInt",
                well_formed: false,
                why: "exceeds xsd:unsignedInt max",
            },
            WellFormedCase {
                lex: "abc",
                dt: "integer",
                well_formed: false,
                why: "non-numeric lexical is ill-formed for xsd:integer",
            },
            // ── well-formed POSITIVES ──
            WellFormedCase {
                lex: "127",
                dt: "byte",
                well_formed: true,
                why: "xsd:byte max",
            },
            WellFormedCase {
                lex: "a b",
                dt: "token",
                well_formed: true,
                why: "single internal space is a valid xsd:token",
            },
            WellFormedCase {
                lex: "1",
                dt: "integer",
                well_formed: true,
                why: "valid integer",
            },
            WellFormedCase {
                lex: "2004-04-12T13:20:00Z",
                dt: "dateTime",
                well_formed: true,
                why: "valid dateTime with tz",
            },
            WellFormedCase {
                lex: "en-US",
                dt: "language",
                well_formed: true,
                why: "valid xsd:language",
            },
            WellFormedCase {
                lex: "YQ==",
                dt: "base64Binary",
                well_formed: true,
                why: "valid base64 octet",
            },
        ]
    }

    /// The crate-local D value-space matrix arm: every case classifies through the REAL
    /// value-space comparator; the floor is the MEASURED assertion count. Feature-ON.
    ///
    /// MUTATION WITNESS: flip any `equal:`/`well_formed:` expectation (e.g. set the
    /// `2^53+1 must NOT alias` case to `equal: true`) and this test goes RED — the arm is
    /// non-vacuous over the value-space semantics, not a tautology.
    #[test]
    fn d_value_space_matrix() {
        let eqs = eq_cases();
        let wfs = well_formed_cases();
        let mut asserted = 0usize;

        for c in &eqs {
            let got = d_value_eq(c.a_lex, &dt(c.a_dt), c.b_lex, &dt(c.b_dt));
            assert_eq!(
                got, c.equal,
                "value-equality matrix: ({:?}^^{}, {:?}^^{}) expected equal={} ({}), got {}",
                c.a_lex, c.a_dt, c.b_lex, c.b_dt, c.equal, c.why, got
            );
            asserted += 1;
        }
        for c in &wfs {
            let got = d_value_key(c.lex, &dt(c.dt)).is_some();
            assert_eq!(
                got, c.well_formed,
                "well-formedness (rdfD1-typability) matrix: {:?}^^{} expected well_formed={} \
                 ({}), got {}",
                c.lex, c.dt, c.well_formed, c.why, got
            );
            asserted += 1;
        }

        // The floor is the MEASURED count and may only RISE.
        assert_eq!(
            asserted, D_VALUE_MATRIX_FLOOR,
            "D value-space matrix asserted {} cases but the pinned floor is {} — raise the \
             floor (and the scoreboard mirror) in the same commit when you add cases; a floor \
             may only RISE",
            asserted, D_VALUE_MATRIX_FLOOR
        );
        println!("\nsparq D value-space matrix (EXTENSION, NOT W3C conformance)");
        // Positional args per the CodeQL `rust/unused-variable` false-positive guard.
        println!(
            "TOTAL d-value-matrix {} (floor {})",
            asserted, D_VALUE_MATRIX_FLOOR
        );
    }

    /// Broadened-map END-TO-END case: a recognized well-formed literal is driven through
    /// the REAL `Profile::D` materializer, then the closure is serialized DROPPING
    /// literal-subject rows (the SPARQL Entailment-Regimes answer restriction — the same
    /// path as the W3C lane) and queried through the REAL engine. rdfD1's typing triple
    /// `"l"^^d rdf:type d` is GENERALIZED (literal subject), so it can NEVER be a SPARQL
    /// answer — the query over the typed subject correctly returns NO rows, exactly why
    /// `d-ent-01` returns no rows. This proves the closure + restriction end-to-end.
    #[test]
    fn d_materialize_answer_restriction_end_to_end() {
        let mut dict = Dict::new();
        // Data: <s> <p> "1"^^xsd:integer .   (a recognized, well-formed literal object)
        let s = dict.intern(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
            "http://e/s",
        )));
        let p = dict.intern(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
            "http://e/p",
        )));
        let one = dict.intern(&oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            "1",
            oxrdf::NamedNode::new_unchecked(dt("integer")),
        )));
        let mut ids: Vec<[Id; 3]> = vec![[s, p, one]];

        // Materialize the D closure through the REAL Profile::D (STANDARD recognized map).
        let added = sparq_reason::materialize(Profile::D, &mut dict, &mut ids);
        assert_eq!(
            added, 1,
            "rdfD1 types the recognized integer literal (generalized triple)"
        );

        // Serialize the closure DROPPING literal-subject rows — the answer restriction.
        let mut nt = String::new();
        for t in &ids {
            let (so, po, oo) = (dict.term(t[0]), dict.term(t[1]), dict.term(t[2]));
            if matches!(so, oxrdf::Term::Literal(_)) {
                continue; // generalized triple: literal subject can never be an answer
            }
            nt.push_str(&format!("{} {} {} .\n", so, po, oo));
        }

        let graph = sparq_core::Graph::load_dataset(&nt, "nquads")
            .unwrap_or_else(|e| panic!("load closure: {e}"));

        // (a) The rdfD1 typing triple is NOT observable (literal subject dropped): a query
        //     for `?x rdf:type xsd:integer` returns NO rows.
        let type_q = format!(
            "SELECT ?x WHERE {{ ?x <{}type> <{}integer> }}",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#", XSD
        );
        let r = sparq_engine::query(&graph, &type_q).unwrap_or_else(|e| panic!("query: {e}"));
        assert!(
            r.rows.is_empty(),
            "the generalized rdfD1 typing triple must NOT surface as a SPARQL answer \
             (answer restriction), but got rows: {:?}",
            r.rows
        );

        // (b) The ORIGINAL asserted triple is still answerable end-to-end.
        let data_q = "SELECT ?o WHERE { <http://e/s> <http://e/p> ?o }";
        let r2 = sparq_engine::query(&graph, data_q).unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(r2.rows.len(), 1, "the asserted data triple is answerable");
    }

    /// Idempotence + unrecognized-datatype fail-closed, driven through the REAL
    /// materializer (a second broadened-map end-to-end guard).
    #[test]
    fn d_materialize_idempotent_and_fail_closed() {
        // Unrecognized-shaped datatype: the STANDARD map does not cover it → no typing.
        let mut dict = Dict::new();
        let s = dict.intern(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
            "http://e/s",
        )));
        let p = dict.intern(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
            "http://e/p",
        )));
        let custom = dict.intern(&oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            "x",
            oxrdf::NamedNode::new_unchecked("http://example.org/myType"),
        )));
        let mut ids: Vec<[Id; 3]> = vec![[s, p, custom]];
        let added = sparq_reason::materialize(Profile::D, &mut dict, &mut ids);
        assert_eq!(
            added, 0,
            "an unrecognized datatype is fail-closed: no rdfD1 typing"
        );

        // Idempotence over a custom recognized map: materialize_d twice adds nothing the
        // second time.
        let mut dict2 = Dict::new();
        let s2 = dict2.intern(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
            "http://e/s",
        )));
        let p2 = dict2.intern(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
            "http://e/p",
        )));
        let two = dict2.intern(&oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            "2",
            oxrdf::NamedNode::new_unchecked(dt("integer")),
        )));
        let mut ids2: Vec<[Id; 3]> = vec![[s2, p2, two]];
        let d = Recognized::new([dt("integer")]);
        let a1 = materialize_d(&d, &mut dict2, &mut ids2);
        let a2 = materialize_d(&d, &mut dict2, &mut ids2);
        assert_eq!(a1, 1, "first materialize types the recognized literal");
        assert_eq!(a2, 0, "second materialize is idempotent");
    }
}
