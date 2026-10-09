//! [GPT-6] Separately versioned successful-result proofs over canonical signed integers.
//!
//! Version three uses one fixed signed-i64 capacity. It preserves exact RDF
//! lexical identity and keeps private sign and lexical length out of the public
//! transcript. The query, released terms, issuer slots and status capacity remain
//! public. This remains research-stage and is not externally audited.

use super::{
    prepare_result_numeric, public_statement_for, reject, verify_statement, CircuitProver,
    CredentialCapacity, DisclosedTerm, FieldHex, NumericContract, NumericProfile, PreparedResult,
    PresentationView, ResultCredential, ResultError, ResultOptions, ResultPolicy, ResultWork,
    SeenNonces, VerifiedResult, VerifierNonce, WitnessSelection,
};
use oxrdf::Term;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

// The public version and independently derived key separate this relation from
// both unsigned contracts, including the predicate-free members.
pub(super) const VERSION: u32 = 3;

/// Public statement for the fixed-capacity canonical signed-i64 result contract.
///
/// The separate wire type rejects unsigned capacity selectors. Sign and exact
/// lexical length are not selector fields. This asserts support for released
/// distinct answers, not complete answers or absence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedResultPresentation {
    /// Must equal the signed contract's version three.
    pub version: u32,
    /// Query additionally matched against the relying party's exact request.
    pub query: String,
    /// Distinct released RDF mappings, with variable names excluding `?`.
    pub rows: Vec<BTreeMap<String, DisclosedTerm>>,
    /// Fresh relying-party challenge.
    pub challenge: FieldHex,
    /// One or two trusted issuer slots, sorted canonically.
    pub issuer_slots: Vec<String>,
    /// Proof bytes checked with the independently selected version-three key.
    pub proof: Vec<u8>,
}

impl SignedResultPresentation {
    fn view(&self) -> PresentationView<'_> {
        PresentationView {
            version: self.version,
            query: &self.query,
            rows: &self.rows,
            challenge: &self.challenge,
            issuer_slots: &self.issuer_slots,
        }
    }
}

/// Prover choices for signed results, without a sign or numeric-length selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedResultOptions {
    /// Selects the public one- or two-credential capacity.
    pub credential_capacity: CredentialCapacity,
    /// Selects bounded joint optimization or first-success witness search.
    pub witness_selection: WitnessSelection,
    /// Maximum candidate triple attempts, including rejected candidates.
    pub max_search_steps: usize,
}

impl Default for SignedResultOptions {
    fn default() -> Self {
        let options = ResultOptions::default();
        Self {
            credential_capacity: options.credential_capacity,
            witness_selection: options.witness_selection,
            max_search_steps: options.max_search_steps,
        }
    }
}

/// Private signed-result witness preparation; deliberately neither serializable nor debug-printable.
pub struct PreparedSignedResult {
    // Shared private proof storage cannot expose an unsigned presentation through
    // this wrapper. The only public transition returns SignedResultPresentation.
    inner: PreparedResult,
}

impl PreparedSignedResult {
    /// Returns prover-local work counts without exposing hidden values.
    pub fn work(&self) -> &ResultWork {
        self.inner.work()
    }

    /// Produces a signed result under its fixed version-three relation.
    ///
    /// # Errors
    /// Rejects invalid labels, toolchain mismatches, unsatisfied witnesses,
    /// backend errors, and reconstructed-public-ABI mismatches.
    pub fn prove(
        &self,
        prover: &CircuitProver,
        out_dir: &Path,
        tag: &str,
    ) -> Result<SignedResultPresentation, ResultError> {
        let p = self.inner.prove(prover, out_dir, tag)?;
        if p.version != VERSION {
            return Err(reject(
                "prepared signed result has the wrong contract version",
            ));
        }
        Ok(SignedResultPresentation {
            version: p.version,
            query: p.query,
            rows: p.rows,
            challenge: p.challenge,
            issuer_slots: p.issuer_slots,
            proof: p.proof,
        })
    }
}

/// Prepares supported distinct answers using the canonical signed-i64 profile.
///
/// # Errors
/// Rejects unsupported syntax, noncanonical or out-of-range signed values,
/// unsupported capacities, false results, failed authentication/status checks,
/// and witness-search exhaustion without a feasible incumbent.
pub fn prepare_signed_result(
    query: &str,
    credentials: &[ResultCredential],
    rows: &[BTreeMap<String, Term>],
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
) -> Result<PreparedSignedResult, ResultError> {
    prepare_signed_result_with_options(
        query,
        credentials,
        rows,
        policy,
        nonce,
        SignedResultOptions::default(),
    )
}

/// Prepares a signed result with explicit credential and witness-search choices.
///
/// # Errors
/// Returns the same fail-closed errors as [`prepare_signed_result`].
pub fn prepare_signed_result_with_options(
    query: &str,
    credentials: &[ResultCredential],
    rows: &[BTreeMap<String, Term>],
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
    options: SignedResultOptions,
) -> Result<PreparedSignedResult, ResultError> {
    let inner = prepare_result_numeric(
        query,
        credentials,
        rows,
        policy,
        nonce,
        ResultOptions {
            credential_capacity: options.credential_capacity,
            witness_selection: options.witness_selection,
            max_search_steps: options.max_search_steps,
            ..ResultOptions::default()
        },
        NumericProfile::Signed,
    )?;
    Ok(PreparedSignedResult { inner })
}

/// Verifies signed result support against the relying party's independent request and policy.
///
/// Re-parses signed bounds and reconstructs every public input and canonical key.
/// Unknown versions and unsigned capacity fields do not select this relation.
/// Use a durable nonce store. No external cryptographic audit is implied.
///
/// # Errors
/// Rejects malformed, unsupported, inconsistent, replayed or cryptographically
/// invalid presentations, and backend or toolchain failures.
pub fn verify_signed_result(
    expected_query: &str,
    presentation: &SignedResultPresentation,
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
    seen: &dyn SeenNonces,
    prover: &CircuitProver,
    work_dir: &Path,
) -> Result<VerifiedResult, ResultError> {
    if presentation.query != expected_query {
        return Err(reject("query differs from relying-party request"));
    }
    let statement =
        public_statement_for(presentation.view(), NumericContract::Signed, policy, nonce)?;
    verify_statement(
        expected_query,
        &presentation.proof,
        statement,
        nonce,
        seen,
        prover,
        work_dir,
    )
}

pub(super) fn package(
    credentials: usize,
    filters: usize,
    depth: u32,
) -> Result<(&'static str, u32), ResultError> {
    let f = match filters {
        0 => 0,
        1..=super::F => 2,
        _ => return Err(reject("private filter capacity")),
    };
    let member = match (credentials, f, depth) {
        (1, 0, 10) => "result_v3_k1_n16_p3_r4_f0_s0_d10",
        (1, 0, 17) => "result_v3_k1_n16_p3_r4_f0_s0_d17",
        (1, 0, 20) => "result_v3_k1_n16_p3_r4_f0_s0_d20",
        (2, 0, 10) => "result_v3_k2_n16_p3_r4_f0_s0_d10",
        (2, 0, 17) => "result_v3_k2_n16_p3_r4_f0_s0_d17",
        (2, 0, 20) => "result_v3_k2_n16_p3_r4_f0_s0_d20",
        (1, 2, 10) => "result_v3_k1_n16_p3_r4_f2_s64_d10",
        (1, 2, 17) => "result_v3_k1_n16_p3_r4_f2_s64_d17",
        (1, 2, 20) => "result_v3_k1_n16_p3_r4_f2_s64_d20",
        (2, 2, 10) => "result_v3_k2_n16_p3_r4_f2_s64_d10",
        (2, 2, 17) => "result_v3_k2_n16_p3_r4_f2_s64_d17",
        (2, 2, 20) => "result_v3_k2_n16_p3_r4_f2_s64_d20",
        _ => return Err(reject("unsupported signed result capacity bucket")),
    };
    Ok((member, VERSION))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::signed::biased;
    use crate::result::{tests::numeric_fixture, PrivateIntegerCapacity, ResultPresentation};
    use crate::verifier::InMemorySeenNonces;
    use oxrdf::{Literal, NamedNode};
    use serde_json::{json, Value};

    fn integer(lexical: &str) -> Term {
        Literal::new_typed_literal(
            lexical,
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        )
        .into()
    }

    fn query(predicate: &str) -> String {
        format!("SELECT DISTINCT ?person WHERE {{ ?person <urn:age> ?v FILTER({predicate}) }}")
    }

    fn presentation(p: &PreparedSignedResult) -> SignedResultPresentation {
        let transport = &p.inner.presentation;
        SignedResultPresentation {
            version: transport.version,
            query: transport.query.clone(),
            rows: transport.rows.clone(),
            challenge: transport.challenge.clone(),
            issuer_slots: transport.issuer_slots.clone(),
            proof: transport.proof.clone(),
        }
    }

    #[test]
    fn signed_preparation_preserves_boundary_values_and_private_bias() {
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            let (c, policy, nonce, rows) = numeric_fixture(integer(&value.to_string()), 3, 128);
            let p = prepare_signed_result(
                &query(&format!("?v = {value}")),
                &[c],
                &rows,
                &policy,
                &nonce,
            )
            .unwrap();
            assert_eq!(p.inner.presentation.version, VERSION);
            assert_eq!(p.inner.package, "result_v3_k1_n16_p3_r4_f2_s64_d10");
            let serialized = p
                .inner
                .toml
                .lines()
                .find_map(|line| line.strip_prefix("filter_values = "))
                .unwrap();
            let values: Value = serde_json::from_str(serialized).unwrap();
            let actual = values[0][0]
                .as_u64()
                .unwrap_or_else(|| values[0][0].as_str().unwrap().parse().unwrap());
            assert_eq!(actual, biased(value));
            let wire = serde_json::to_value(presentation(&p)).unwrap();
            assert!(wire.get("integer_capacity").is_none());
            assert!(wire.get("sign").is_none());
            assert!(wire.get("length").is_none());
        }
    }

    #[test]
    fn signed_preparation_uses_signed_order_across_zero() {
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            let (c, policy, nonce, rows) = numeric_fixture(integer(&value.to_string()), 3, 128);
            for bound in [i64::MIN, -1, 0, 1, i64::MAX] {
                let answer = prepare_signed_result(
                    &query(&format!("?v < {bound}")),
                    std::slice::from_ref(&c),
                    &rows,
                    &policy,
                    &nonce,
                );
                assert_eq!(
                    answer.is_ok(),
                    value < bound,
                    "value={value}, bound={bound}"
                );
            }
        }
    }

    #[test]
    fn signed_preparation_rejects_noncanonical_private_and_public_operands() {
        for lexical in [
            "-0",
            "+1",
            "01",
            "-01",
            "-9223372036854775809",
            "9223372036854775808",
            " 1",
            "1 ",
        ] {
            let (c, policy, nonce, rows) = numeric_fixture(integer(lexical), 3, 128);
            assert!(
                prepare_signed_result(
                    &query("?v >= -9223372036854775808"),
                    &[c],
                    &rows,
                    &policy,
                    &nonce
                )
                .is_err(),
                "{lexical}"
            );
        }
        for datatype in ["decimal", "int", "string"] {
            let term = Literal::new_typed_literal(
                "-1",
                NamedNode::new(format!("http://www.w3.org/2001/XMLSchema#{datatype}")).unwrap(),
            )
            .into();
            let (c, policy, nonce, rows) = numeric_fixture(term, 3, 128);
            assert!(
                prepare_signed_result(&query("?v < 0"), &[c], &rows, &policy, &nonce).is_err(),
                "{datatype}"
            );
        }
    }

    #[test]
    fn signed_public_predicates_use_the_predicate_free_version_three_member() {
        let value = integer("-1");
        let (c, policy, nonce, _) = numeric_fixture(value.clone(), 3, 128);
        let rows = vec![BTreeMap::from([("v".into(), value)])];
        let q = "SELECT DISTINCT ?v WHERE { <urn:alice> <urn:age> ?v FILTER(?v < 0) }";
        let p = prepare_signed_result(q, &[c], &rows, &policy, &nonce).unwrap();
        assert_eq!(p.inner.package, "result_v3_k1_n16_p3_r4_f0_s0_d10");
        assert_eq!(p.work().public_predicates, 1);
        assert_eq!(p.work().private_predicates, 0);
        assert!(!p.inner.toml.contains("filter_values ="));
        let mut malformed = presentation(&p);
        malformed.rows[0].insert("v".into(), crate::result::disclosed(&integer("0")).unwrap());
        assert!(
            public_statement_for(malformed.view(), NumericContract::Signed, &policy, &nonce)
                .is_err()
        );
    }

    #[test]
    fn signed_type_and_version_do_not_extend_unsigned_wire_acceptance() {
        let (c, policy, nonce, rows) = numeric_fixture(integer("42"), 3, 128);
        let q = query("?v >= 18");
        let p = prepare_signed_result(&q, &[c], &rows, &policy, &nonce).unwrap();
        let mut wire = serde_json::to_value(presentation(&p)).unwrap();
        let unsigned: ResultPresentation = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(unsigned.integer_capacity, PrivateIntegerCapacity::TwoDigits);
        assert!(crate::result::public_statement(&unsigned, &policy, &nonce).is_err());
        wire["integer_capacity"] = json!("full_u64");
        assert!(serde_json::from_value::<SignedResultPresentation>(wire).is_err());
        let driver = CircuitProver::from_crate_root();
        for version in [0, 1, 2, 4] {
            let mut wrong = presentation(&p);
            wrong.version = version;
            assert!(verify_signed_result(
                &q,
                &wrong,
                &policy,
                &nonce,
                &InMemorySeenNonces::default(),
                &driver,
                Path::new("unused")
            )
            .is_err());
        }
    }

    #[test]
    fn signed_depth_derives_from_the_complete_policy_and_padding_is_explicit() {
        let (c, mut policy, nonce, rows) = numeric_fixture(integer("-1"), 3, 128);
        policy.snapshots.push(crate::manifest::StatusListSnapshot {
            status_list: "urn:status:unselected".into(),
            version: 7,
            bits: vec![0; (1 << 20) / 8],
        });
        let p = prepare_signed_result_with_options(
            &query("?v < 0"),
            &[c],
            &rows,
            &policy,
            &nonce,
            SignedResultOptions {
                credential_capacity: CredentialCapacity::HideInTwo,
                ..SignedResultOptions::default()
            },
        )
        .unwrap();
        assert_eq!(p.inner.package, "result_v3_k2_n16_p3_r4_f2_s64_d20");
        assert_eq!(p.work().signature_checks, 2);
        assert_eq!(p.work().selected_credentials, 1);
    }

    #[test]
    #[ignore = "requires pinned nargo; executes every signed wrapper and boundary witness"]
    fn signed_relation_executes_capacity_matrix_and_rejects_bias_tampering() {
        crate::result::pinned_toolchain().unwrap();
        let driver = CircuitProver::from_crate_root();
        for depth in [10, 17, 20] {
            for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
                for public in [false, true] {
                    let (credential, policy, nonce, private_rows) =
                        numeric_fixture(integer("-1"), 3, (1 << depth) / 8);
                    let (q, rows) = if public {
                        (
                            "SELECT DISTINCT ?v WHERE { <urn:alice> <urn:age> ?v FILTER(?v < 0) }"
                                .to_owned(),
                            vec![BTreeMap::from([("v".into(), integer("-1"))])],
                        )
                    } else {
                        (query("?v < 0"), private_rows)
                    };
                    let p = prepare_signed_result_with_options(
                        &q,
                        &[credential],
                        &rows,
                        &policy,
                        &nonce,
                        SignedResultOptions {
                            credential_capacity: capacity,
                            ..SignedResultOptions::default()
                        },
                    )
                    .unwrap();
                    driver
                        .private_package_witness(p.inner.package, &p.inner.toml, "signed_matrix")
                        .unwrap();
                }
            }
        }
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            let (credential, policy, nonce, rows) =
                numeric_fixture(integer(&value.to_string()), 3, 128);
            let p = prepare_signed_result(
                &query("?v >= -9223372036854775808"),
                &[credential],
                &rows,
                &policy,
                &nonce,
            )
            .unwrap();
            driver
                .private_package_witness(p.inner.package, &p.inner.toml, "signed_boundary")
                .unwrap();
            // The predicate stays true for every bias; only the unchanged issuer
            // leaf's canonical signed token can reject this different witness.
            let wrong_bias = biased(value) ^ (1u64 << 63);
            let malformed =
                crate::result::tests::alter_input(&p.inner.toml, "filter_values", |v| {
                    v[0][0] = json!(wrong_bias.to_string());
                });
            match driver.private_package_witness(p.inner.package, &malformed, "signed_wrong_bias") {
                Err(crate::driver::DriverError::Tool { tool, stderr }) => {
                    assert_eq!(tool, "nargo execute");
                    assert!(
                        stderr.contains("signed operand encoding mismatch"),
                        "{stderr}"
                    );
                }
                _ => panic!("wrong signed bias must fail the literal-binding constraint"),
            }
        }
    }

    #[test]
    #[ignore = "requires pinned nargo and bb; genuine signed boundaries, predicate-free proof, and cross-contract rejection"]
    fn signed_real_proofs_bind_version_policy_and_signed_order() {
        crate::result::pinned_toolchain().unwrap();
        let driver = CircuitProver::from_crate_root();
        let directory =
            std::env::temp_dir().join(format!("sparq_signed_result_{}", std::process::id()));
        // Genuine proof coverage is separate from the all-wrapper executions.
        for (value, public, depth, capacity) in [
            (i64::MIN, false, 10, CredentialCapacity::Smallest),
            (i64::MAX, false, 20, CredentialCapacity::HideInTwo),
            (1, false, 10, CredentialCapacity::Smallest),
            (-1, true, 17, CredentialCapacity::Smallest),
        ] {
            let (credential, policy, nonce, private_rows) =
                numeric_fixture(integer(&value.to_string()), 3, (1 << depth) / 8);
            let (q, rows) = if public {
                (
                    "SELECT DISTINCT ?v WHERE { <urn:alice> <urn:age> ?v FILTER(?v < 0) }"
                        .to_owned(),
                    vec![BTreeMap::from([("v".into(), integer("-1"))])],
                )
            } else if value == 1 {
                (query("?v >= 0"), private_rows)
            } else {
                (query(&format!("?v = {value}")), private_rows)
            };
            let prepared = prepare_signed_result_with_options(
                &q,
                std::slice::from_ref(&credential),
                &rows,
                &policy,
                &nonce,
                SignedResultOptions {
                    credential_capacity: capacity,
                    ..SignedResultOptions::default()
                },
            )
            .unwrap();
            let proof = prepared.prove(&driver, &directory, "signed_real").unwrap();
            let seen = InMemorySeenNonces::default();
            let checked =
                verify_signed_result(&q, &proof, &policy, &nonce, &seen, &driver, &directory)
                    .unwrap();
            assert_eq!(checked.rows, rows);
            assert!(
                verify_signed_result(&q, &proof, &policy, &nonce, &seen, &driver, &directory)
                    .is_err()
            );
            let mut changed_policy = policy.clone();
            changed_policy.snapshots[0].bits[0] |= 8;
            assert!(verify_signed_result(
                &q,
                &proof,
                &changed_policy,
                &nonce,
                &InMemorySeenNonces::default(),
                &driver,
                &directory
            )
            .is_err());
            if value == 1 {
                // Same nonnegative query/credential/rows are admitted by both
                // contracts. Rejections therefore reach real proof verification.
                let mut unsigned_wire = serde_json::to_value(&proof).unwrap();
                unsigned_wire["version"] = json!(2);
                unsigned_wire["integer_capacity"] = json!("full_u64");
                let downgraded: ResultPresentation = serde_json::from_value(unsigned_wire).unwrap();
                assert!(crate::result::public_statement(&downgraded, &policy, &nonce).is_ok());
                assert!(crate::result::verify_result(
                    &q,
                    &downgraded,
                    &policy,
                    &nonce,
                    &InMemorySeenNonces::default(),
                    &driver,
                    &directory
                )
                .is_err());
                let unsigned = crate::result::prepare_result_with_options(
                    &q,
                    &[credential],
                    &rows,
                    &policy,
                    &nonce,
                    ResultOptions {
                        integer_capacity: crate::result::IntegerCapacityPolicy::HideInU64,
                        ..ResultOptions::default()
                    },
                )
                .unwrap()
                .prove(&driver, &directory, "unsigned_cross_contract")
                .unwrap();
                let mut signed_wire = serde_json::to_value(unsigned).unwrap();
                signed_wire["version"] = json!(VERSION);
                signed_wire
                    .as_object_mut()
                    .unwrap()
                    .remove("integer_capacity");
                let upgraded: SignedResultPresentation =
                    serde_json::from_value(signed_wire).unwrap();
                assert!(public_statement_for(
                    upgraded.view(),
                    NumericContract::Signed,
                    &policy,
                    &nonce
                )
                .is_ok());
                assert!(verify_signed_result(
                    &q,
                    &upgraded,
                    &policy,
                    &nonce,
                    &InMemorySeenNonces::default(),
                    &driver,
                    &directory
                )
                .is_err());
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
