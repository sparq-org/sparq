//! Trust-graph admission-program provenance tier — the sourced-honesty ingest gate
//! (bead `sq-7utko`).
//!
//! 🤖 SPARQ agent. Source record:
//! `research/trust-graph-authorisation-2026-07.md` (epic `sq-pfae`).
//!
//! The PKG ingests AGENTS.md / skills / beads but did not surface the trust-graph
//! admission program's provenance. This suite pins the bead's INVARIANT — a
//! pkg-query over the PKG returns the trust-graph honest-constraint facts
//! (clear-path mechanism, unlinkability NOT a v1 guarantee, gated on the open
//! `sq-qhy4` external-audit gate, `sparq-mpc` semi-honest only) WITH their source
//! record, and the ingest introduces no unqualified ZK/privacy claim:
//!
//!   1. **Default feature state** — the compiled tier (a plain data const) carries
//!      the caveat text + the source identifier verbatim (non-vacuous without the
//!      engine).
//!   2. **`validate`** — the tier conforms to `pkg.shapes.ttl` with 0 violations
//!      (every Finding sourced + confident + assured + non-filler).
//!   3. **`query`** — the acceptance criterion: a SPARQL query for the trust-graph
//!      unlinkability status returns a non-empty, SOURCED result whose text carries
//!      the sq-qhy4 / not-a-v1-guarantee caveat; a CONTROL query for an unrelated
//!      term returns the expected different/empty result (non-vacuity witness).
//!
//! Run: `cargo test -p sparq-kb --test trust_graph_pkg` (default state), and
//! `cargo test -p sparq-kb --features query --test trust_graph_pkg` (full suite).

use sparq_kb::PKG_TRUST_GRAPH_FINDINGS;

/// (1) Default state: the ingested tier carries the caveat + the source, verbatim.
/// The bead's honesty invariant is that the STATEMENT ITSELF carries the caveat —
/// so the caveat must live in the data, not only in a test assertion.
#[test]
fn ingested_tier_carries_caveat_and_source() {
    let ttl = PKG_TRUST_GRAPH_FINDINGS;
    // The unlinkability finding + its caveat.
    assert!(
        ttl.contains("Trust-graph unlinkability is NOT a v1 guarantee"),
        "the not-a-v1-guarantee caveat must be ingested verbatim"
    );
    assert!(
        ttl.contains("sq-qhy4"),
        "the open external-audit gate (sq-qhy4) must be named in the ingested tier"
    );
    assert!(
        ttl.contains("semi-honest"),
        "the sparq-mpc semi-honest-only constraint must be ingested"
    );
    assert!(
        ttl.contains("clear-path"),
        "the clear-path characterisation must be ingested"
    );
    // The source record is minted as a pkg:Source and every Finding cites it.
    assert!(
        ttl.contains("research/trust-graph-authorisation-2026-07.md"),
        "the deciding design record must be the minted pkg:Source"
    );
    assert!(
        ttl.contains("prov:wasDerivedFrom kb:doc-research-trust-graph-authorisation-2026-07"),
        "every Finding must cite the record via prov:wasDerivedFrom"
    );
    // No unqualified claim: any STATEMENT (quoted literal) mentioning unlinkability
    // is negated/hedged. (IRI local names like `...unlinkability-not-v1` are skipped
    // — they are identifiers, not claims.)
    for line in ttl
        .lines()
        .filter(|l| l.contains("unlinkability") && l.contains('"'))
    {
        let hedged = [
            "NOT a v1 guarantee",
            "no anonymity, unlinkability, or ZK-soundness claim",
            "no settled cryptographic soundness, privacy, or unlinkability",
            "nothing in the record asserts a settled cryptographic soundness",
        ];
        assert!(
            hedged.iter().any(|h| line.contains(h)),
            "unhedged unlinkability mention in the ingested tier: {line}"
        );
    }
}

/// (2) `validate`: the tier passes the PKG SHACL write-time guardrails — 0
/// violations (FindingShape: sourced + one confidence in [0,1] + assurance enum +
/// non-filler justification; SourceShape: explored-status + real title).
#[cfg(feature = "validate")]
#[test]
fn trust_graph_tier_conforms_zero_violations() {
    let report = sparq_kb::validate::validate_instances(&[PKG_TRUST_GRAPH_FINDINGS])
        .expect("trust-graph tier loads + validates");
    assert!(
        report.conforms && report.results.is_empty(),
        "the trust-graph provenance tier MUST conform with 0 SHACL violations, got \
         {} result(s):\n{}",
        report.results.len(),
        report.to_text()
    );
}

#[cfg(feature = "query")]
mod query_acceptance {
    use oxrdf::Term;
    use sparq_kb::query::{ask_pkg, load_pkg};

    const UNLINKABILITY_QUERY: &str = r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX kb:      <https://sparq.dev/ns/pkg/kb#>
PREFIX rdfs:    <http://www.w3.org/2000/01/rdf-schema#>
PREFIX prov:    <http://www.w3.org/ns/prov#>
PREFIX sigimpl: <https://w3id.org/zkp-sparql/sig-impl#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT ?label ?just ?src ?srcId WHERE {
  ?f a pkg:Finding ;
     pkg:about kb:topic-trust-graph-authz ;
     rdfs:label ?label ;
     sigimpl:justification ?just ;
     prov:wasDerivedFrom ?src .
  ?src dcterms:identifier ?srcId .
}
"#;

    fn lit(cell: &Option<Term>) -> String {
        match cell {
            Some(Term::Literal(l)) => l.value().to_string(),
            Some(Term::NamedNode(n)) => n.as_str().to_string(),
            other => panic!("expected a bound literal/IRI, got {other:?}"),
        }
    }

    /// (3a) ACCEPTANCE: "what is the status of trust-graph unlinkability?" — the
    /// pkg-query returns a non-empty result whose text carries the sq-qhy4 /
    /// not-a-v1-guarantee caveat, and whose source binding resolves to the deciding
    /// design record (sourced, not fabricated).
    #[test]
    fn unlinkability_status_is_sourced_and_caveated() {
        let g = load_pkg().expect("PKG loads");
        let r = ask_pkg(&g, UNLINKABILITY_QUERY).expect("query runs");
        assert!(
            !r.rows.is_empty(),
            "the trust-graph provenance query must return >=1 sourced Finding"
        );
        // The unlinkability finding is among them, caveat + gate + source intact.
        let hit = r
            .rows
            .iter()
            .find(|row| lit(&row[0]).contains("Trust-graph unlinkability"))
            .expect("an unlinkability-status Finding must be retrievable");
        let (label, just, src, src_id) = (lit(&hit[0]), lit(&hit[1]), lit(&hit[2]), lit(&hit[3]));
        assert!(
            label.contains("NOT a v1 guarantee"),
            "the answer itself must carry the not-a-v1-guarantee caveat: {label}"
        );
        assert!(
            label.contains("sq-qhy4") || just.contains("sq-qhy4"),
            "the answer must name the open sq-qhy4 gate: {label} / {just}"
        );
        assert!(
            just.contains("semi-honest"),
            "the justification must carry the MPC semi-honest constraint: {just}"
        );
        assert_eq!(
            src, "https://sparq.dev/ns/pkg/kb#doc-research-trust-graph-authorisation-2026-07",
            "the Finding must be derived from the minted source node"
        );
        assert_eq!(
            src_id, "research/trust-graph-authorisation-2026-07.md",
            "the source must identify the deciding design record on disk"
        );
    }

    /// (3b) The full honest-constraint set is retrievable: clear-path, the
    /// closure-model deciding record, and the MPC posture — all from the same
    /// sourced topic.
    #[test]
    fn honest_constraint_set_is_retrievable() {
        let g = load_pkg().expect("PKG loads");
        let r = ask_pkg(&g, UNLINKABILITY_QUERY).expect("query runs");
        let labels: Vec<String> = r.rows.iter().map(|row| lit(&row[0])).collect();
        for needle in [
            "clear-path authorisation mechanism",
            "certification-edge closure model",
            "honest-majority semi-honest only",
        ] {
            assert!(
                labels.iter().any(|l| l.contains(needle)),
                "expected a sourced Finding containing {needle:?}, got {labels:?}"
            );
        }
    }

    /// (3c) CONTROL (non-vacuity): the same query shape over an unrelated term —
    /// a topic that exists in the PKG (merge discipline, from the AGENTS.md tier)
    /// — returns a DIFFERENT result set with no trust-graph statement in it, and a
    /// nonexistent topic returns the empty set. So (3a) is satisfied by the
    /// ingested data, not by a query that matches anything.
    #[test]
    fn control_queries_differ_or_are_empty() {
        let g = load_pkg().expect("PKG loads");

        // Unrelated EXISTING topic: rows exist, none about trust-graph unlinkability.
        let unrelated =
            UNLINKABILITY_QUERY.replace("kb:topic-trust-graph-authz", "kb:topic-merge-discipline");
        let r = ask_pkg(&g, &unrelated).expect("control query runs");
        assert!(
            !r.rows.is_empty(),
            "control topic (merge discipline) should have its own findings"
        );
        for row in &r.rows {
            let label = lit(&row[0]);
            assert!(
                !label.contains("unlinkability"),
                "an unrelated topic must not surface the trust-graph statement: {label}"
            );
        }

        // Nonexistent topic: the empty set.
        let absent =
            UNLINKABILITY_QUERY.replace("kb:topic-trust-graph-authz", "kb:topic-does-not-exist");
        let r = ask_pkg(&g, &absent).expect("absent-topic query runs");
        assert!(
            r.rows.is_empty(),
            "a nonexistent topic must return the empty set, got {} row(s)",
            r.rows.len()
        );
    }
}
