//! CONFORMANCE RUNNER for the trust-expression verifier-to-holder contract
//! (bead `sq-6syab.6`, epic `sq-6syab`, issue
//! [#1592](https://github.com/sparq-org/sparq/issues/1592); design record
//! `research/trust-expression-spec.md` §6).
//!
//! Drives every entry of `tests/trust-expression/manifest.ttl` through the public
//! holder-side API — [`sparq_trust::parse_request`], [`sparq_trust::evaluate_contract`],
//! [`sparq_trust::verify_response`] — and asserts the outcome the specification
//! requires. This is the suite sparq must pass IN FULL (the maintainer-namespace
//! standing rule for specs authored here): there are no `KNOWN_FAILING` entries, and
//! `trust_expression_fixtures.rs` asserts that separately over the data.
//!
//! Three obligations are checked on EVERY case, not just the ones designed to
//! showcase them:
//!
//! 1. **the answer** — `tec:Admitted` binds (`true` / at least one row);
//!    `tec:NoBinding` fails closed (`false` / zero rows). Fail-closed is always the
//!    ABSENCE of an admissible derivation, never a derived denial;
//! 2. **the disclosure** — the response carries exactly the manifest's
//!    `tec:contributingStatements`, so a fail-closed case discloses NOTHING;
//! 3. **the independent verifier re-check** — re-running `Q'` over the response's
//!    named-graph form alone reproduces the same answer. That is the property the
//!    whole clear-path contract rests on: the verifier need not trust the holder's
//!    evaluation, only re-run the rewrite over the disclosed provenance.
//!
//! …plus the **encoding correspondence** of design §4 on every case: the normative
//! RDF 1.2 reifier form (a) and the runnable-today named-graph + PROV-O form (b)
//! must carry the same provenance and the same statements under the fixed
//! `reifier node <-> graph IRI` mapping.
//!
//! HONEST SCOPE: these are SPECIFICATION-conformance assertions. Passing this suite
//! says the implementation matches the contract and fails closed where the contract
//! says it must — it is NOT a cryptographic soundness or privacy claim. The clear
//! path trusts the underlying attestations' signatures and the completeness of what
//! the holder disclosed (design §7.3); framework trust is ANCHORED, not proven
//! (§7.2); the sparq ZK estate has no external accredited-cryptographer sign-off
//! (`sq-qhy4` open) and `sparq-mpc` is honest-majority semi-honest only.
//!
//! Behind the default-OFF `expression` feature. Run:
//! `cargo test -p sparq-trust --features expression --test trust_expression_conformance`.
//!
//! [SONNET-4.6] sq-6syab.6 (epic sq-6syab; issue #1592). 🤖 SPARQ agent —
//! trust-expression conformance runner.
#![cfg(feature = "expression")]

use std::collections::BTreeSet;

use sparq_core::Graph;
use sparq_trust::expression::RDF_REIFIES;
use sparq_trust::{
    evaluate_contract, parse_request, verify_response, ChallengeNonce, ContractAnswer,
    ContractOutcome, ContractRequest, ProvenanceResponse,
};

#[path = "trust_expression_manifest/mod.rs"]
mod manifest;

use manifest::{load_manifest, Case, Expect};

/// Does this answer bind? `false` / zero rows is the fail-closed outcome.
fn binds(answer: &ContractAnswer) -> bool {
    match answer {
        ContractAnswer::Boolean(b) => *b,
        ContractAnswer::Solutions { rows, .. } => !rows.is_empty(),
    }
}

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e))
}

/// Build the request + holder dataset a manifest case describes.
fn load_case(case: &Case) -> (ContractRequest, Graph) {
    let query = read(&case.query);
    let requirements_ttl = read(&case.requirements);
    let requirements: Vec<oxrdf::Triple> = oxttl::TurtleParser::new()
        .for_reader(requirements_ttl.as_bytes())
        .map(|r| r.unwrap_or_else(|e| panic!("{}: TR must be valid Turtle: {}", case.label(), e)))
        .collect();
    // The nonce legitimately arrives off the wire here (the manifest IS the wire),
    // which is exactly the case `from_wire` names: it promises nothing about
    // freshness, and a conformance fixture is deliberately not fresh.
    let nonce = ChallengeNonce::from_wire(&case.nonce)
        .unwrap_or_else(|e| panic!("{}: nonce must be non-empty: {}", case.label(), e));
    let request = parse_request(&query, &requirements, &nonce)
        .unwrap_or_else(|e| panic!("{}: the request must parse: {}", case.label(), e));
    let holder = Graph::load_dataset(&read(&case.data), "trig")
        .unwrap_or_else(|e| panic!("{}: the holder dataset must load: {}", case.label(), e));
    (request, holder)
}

fn evaluate(case: &Case) -> (ContractRequest, ContractOutcome) {
    let (request, holder) = load_case(case);
    let outcome = evaluate_contract(&request, &holder, None)
        .unwrap_or_else(|e| panic!("{}: evaluation must not error: {}", case.label(), e));
    (request, outcome)
}

fn strip_trailing_dot(line: &str) -> String {
    let line = line.trim();
    line.strip_suffix('.').unwrap_or(line).trim().to_string()
}

/// Split the (b) named-graph response form into its default-graph provenance
/// statements and its `(graph IRI, statement)` pairs.
fn parse_dataset_form(trig: &str) -> (BTreeSet<String>, BTreeSet<(String, String)>) {
    let (mut provenance, mut statements) = (BTreeSet::new(), BTreeSet::new());
    let mut graph: Option<String> = None;
    for line in trig.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("GRAPH ") {
            graph = Some(rest.trim_end_matches('{').trim().to_string());
        } else if line == "}" {
            graph = None;
        } else if let Some(g) = &graph {
            statements.insert((g.clone(), strip_trailing_dot(line)));
        } else {
            provenance.insert(strip_trailing_dot(line));
        }
    }
    (provenance, statements)
}

/// Split the (a) RDF 1.2 reifier response form into its provenance statements and
/// its `(reifier node, reified statement)` pairs.
fn parse_reifier_form(text: &str) -> (BTreeSet<String>, BTreeSet<(String, String)>) {
    let (mut provenance, mut reified) = (BTreeSet::new(), BTreeSet::new());
    let marker = format!("<{}> <<( ", RDF_REIFIES);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line.find(&marker) {
            Some(at) => {
                let reifier = line[..at].trim().to_string();
                let inner = strip_trailing_dot(&line[at + marker.len()..]);
                let inner = inner.strip_suffix(")>>").unwrap_or(&inner).trim().to_string();
                reified.insert((reifier, inner));
            }
            None => {
                provenance.insert(strip_trailing_dot(line));
            }
        }
    }
    (provenance, reified)
}

/// Design §4: the two encodings of the response are mechanically interconvertible
/// under the fixed `reifier node <-> graph IRI` mapping. Checked on EVERY case (the
/// §6 case-8 entry is the one built to make it bite, with two bundles).
fn assert_encodings_correspond(case: &Case, response: &ProvenanceResponse) {
    let (dataset_prov, dataset_statements) = parse_dataset_form(&response.dataset_form);
    let (reifier_prov, reified) = parse_reifier_form(&response.reifier_form);
    assert_eq!(
        dataset_prov,
        reifier_prov,
        "{}: the (a) reifier form and the (b) named-graph form must carry the SAME \
         provenance — the mapping is lossless in both directions",
        case.label()
    );
    assert_eq!(
        dataset_statements,
        reified,
        "{}: every named graph's statement must appear reified by that graph's IRI \
         (and nothing else must)",
        case.label()
    );
    assert_eq!(
        dataset_statements.len(),
        response.contributing_statements,
        "{}: contributing_statements must count exactly the disclosed statements",
        case.label()
    );
}

/// THE conformance test: every manifest entry produces the outcome the
/// specification requires, discloses exactly what it should, and survives an
/// independent verifier re-check over its own response.
#[test]
fn every_manifest_case_matches_the_specification() {
    let cases = load_manifest();
    assert!(!cases.is_empty(), "the manifest lists no cases");
    for case in &cases {
        let (request, outcome) = evaluate(case);

        match case.expect {
            Expect::Admitted => assert!(
                binds(&outcome.answer),
                "{}: the contract must BIND — got {:?}",
                case.label(),
                outcome.answer
            ),
            Expect::NoBinding => assert!(
                !binds(&outcome.answer),
                "{}: the contract must FAIL CLOSED (no admissible derivation ⇒ no \
                 binding) — got {:?}",
                case.label(),
                outcome.answer
            ),
        }

        assert_eq!(
            outcome.response.contributing_statements,
            case.contributing,
            "{}: the response must disclose exactly {} contributing statement(s)",
            case.label(),
            case.contributing
        );

        // The independent verifier re-check: `Q'` over the response ALONE.
        let replayed = verify_response(&request, &outcome.response).unwrap_or_else(|e| {
            panic!("{}: the verifier re-check must not error: {}", case.label(), e)
        });
        assert_eq!(
            binds(&replayed),
            binds(&outcome.answer),
            "{}: re-running Q' over the response's own provenance must reproduce the \
             holder's answer — otherwise the response is not self-sufficient evidence",
            case.label()
        );

        assert_encodings_correspond(case, &outcome.response);
    }
}

/// Non-vacuity guard for the re-check: an admitted case's response stops binding
/// the moment its default-graph provenance is stripped. Without this, the re-check
/// above could be passing because `Q'` binds on the bare statements.
#[test]
fn stripping_provenance_from_an_admitted_response_fails_closed() {
    let admitted: Vec<Case> = load_manifest()
        .into_iter()
        .filter(|c| c.expect == Expect::Admitted)
        .collect();
    assert!(
        !admitted.is_empty(),
        "no admitted case remains — the suite would only ever test rejection"
    );
    for case in &admitted {
        let (request, outcome) = evaluate(case);
        // Keep the disclosed statements, drop every default-graph provenance line
        // (attribution, status attestations, certifications).
        let mut stripped = String::new();
        let mut inside = false;
        for line in outcome.response.dataset_form.lines() {
            if line.trim_start().starts_with("GRAPH ") {
                inside = true;
            }
            if inside {
                stripped.push_str(line);
                stripped.push('\n');
            }
            if line.trim() == "}" {
                inside = false;
            }
        }
        assert!(
            !stripped.trim().is_empty(),
            "{}: an admitted response must carry at least one named graph",
            case.label()
        );
        let tampered = ProvenanceResponse {
            nonce: outcome.response.nonce.clone(),
            reifier_form: outcome.response.reifier_form.clone(),
            dataset_form: stripped,
            contributing_statements: outcome.response.contributing_statements,
        };
        let replayed = verify_response(&request, &tampered)
            .unwrap_or_else(|e| panic!("{}: the re-check must not error: {}", case.label(), e));
        assert!(
            !binds(&replayed),
            "{}: a response stripped of its provenance MUST stop binding — the \
             re-check is not actually checking admissibility",
            case.label()
        );
    }
}

/// The freshness binding survives the suite: a response replayed against a request
/// carrying a different nonce is refused outright, never re-evaluated.
#[test]
fn a_response_replayed_under_another_cases_nonce_is_refused() {
    let cases = load_manifest();
    let (first, second) = (&cases[0], &cases[1]);
    assert_ne!(
        first.nonce, second.nonce,
        "the fixtures must carry distinct nonces for this to test anything"
    );
    let (_, outcome) = evaluate(first);
    let (other_request, _) = load_case(second);
    assert!(
        verify_response(&other_request, &outcome.response).is_err(),
        "a response minted under one challenge must be refused under another"
    );
}
