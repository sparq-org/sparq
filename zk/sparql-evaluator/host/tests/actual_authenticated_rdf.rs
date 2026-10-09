// zkp-14.5: direct V5 guest execution; these are not receipts or proofs.
// Rust guideline compliant 2026-02-21
//! Every test is ignored and needs `RISC0_SERVER_PATH` naming the real local
//! `r0vm`. Inputs go straight to the executor, bypassing every host precheck, so
//! each rejection here is enforced by the guest image itself. A rejection counts
//! only when one SDK error-chain entry equals the guest's own abort exactly, after
//! a positive execution on the same executor. Any other executor failure,
//! including the session ceiling, fails the test. Not yet run at the current
//! checkpoint; the V5 guest is unbuilt.
#![cfg(feature = "authenticated-rdf")]

#[path = "support/authenticated_rdf.rs"]
mod fixture;

use fixture::Expect;
use risc0_zkvm::{Executor, ExecutorEnv, ExitCode, ExternalProver};
use serde::Serialize;
use sparq_proved_evaluator::{embedded_artifact, embedded_authrdf_artifact};
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, SignedCredential};
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v3};
use std::path::PathBuf;

/// The V5 guest's `reject()` text (`methods/guest-authrdf`) as the pinned SDK
/// renders a guest abort; see `GUEST_ABORT` in `actual_engine_replay.rs`.
const AUTH_ABORT: &str = "Guest panicked: bounded authenticated-RDF relation rejected";
/// The exact V1–V3 guest's abort (`methods/guest`), rendered the same way.
const EXACT_ABORT: &str = "Guest panicked: bounded exact-dataset relation rejected";
/// The production prover's limits: `1 << 25` session cycles, `2^20` segments.
///
/// Whether valid V5 witnesses fit this ceiling has not been measured.
const SESSION_LIMIT: u64 = 1 << 25;
const SEGMENT_LIMIT_PO2: u32 = 20;
const NONCE: [u8; 32] = [0x4b; 32];

fn executor() -> ExternalProver {
    assert!(
        std::env::var_os("RISC0_DEV_MODE").is_none(),
        "RISC0_DEV_MODE must be unset"
    );
    let r0vm = PathBuf::from(
        std::env::var_os("RISC0_SERVER_PATH")
            .expect("RISC0_SERVER_PATH must name the installed real r0vm 3.0.6"),
    );
    assert!(r0vm.is_absolute() && r0vm.is_file(), "r0vm must be an absolute file");
    ExternalProver::new("actual-authrdf-guest", r0vm)
}

fn words(value: &impl Serialize) -> Vec<u32> {
    risc0_zkvm::serde::to_vec(value).expect("typed input encoding")
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn environment(input: &[u8]) -> ExecutorEnv<'_> {
    ExecutorEnv::builder()
        .session_limit(Some(SESSION_LIMIT))
        .segment_limit_po2(SEGMENT_LIMIT_PO2)
        .write_slice(input)
        .build()
        .expect("executor environment")
}

/// Executes raw input; requires `Halted(0)` and returns the committed journal bytes.
fn accepted(input: &[u8], artifact: &[u8], label: &str) -> risc0_zkvm::Journal {
    let session = executor()
        .execute(environment(input), artifact)
        .unwrap_or_else(|error| panic!("{label}: positive execution failed: {error:#}"));
    assert_eq!(session.exit_code, ExitCode::Halted(0), "{label}");
    session.journal
}

/// Requires one error-chain entry to equal `abort`; any other failure fails.
fn rejected(input: &[u8], artifact: &[u8], abort: &str, label: &str) {
    let Some(error) = executor().execute(environment(input), artifact).err() else {
        panic!("{label}: produced a journal");
    };
    // Not a substring: an IPC error, timeout, session limit, OOM or another
    // panic that merely mentions the text is a different failure.
    assert!(
        error.chain().any(|entry| entry.to_string() == abort),
        "{label}: not the guest relation rejection: {error:#}"
    );
}

fn agreed(salt: [u8; 32]) -> DatasetAuthority {
    DatasetAuthority::VerifierAgreed {
        commitment: fixture::anchor(salt),
    }
}

/// Runs the published vector once so later rejections cannot hide a broken executor.
fn positive_control() {
    let witness = fixture::witness(fixture::ASK_ISSUER, agreed(fixture::SALT), NONCE);
    let journal: auth::Journal = accepted(
        &bytes(&words(&witness)),
        embedded_authrdf_artifact(),
        "positive control",
    )
    .decode()
    .expect("V5 journal");
    auth::bind_journal(&journal, &witness.request).expect("positive control binds");
    assert_eq!(journal.result, Expect::Ask(true).result());
}

fn v3_witness() -> v3::Witness {
    v3::Witness {
        request: v3::Request {
            version: v3::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v3::Dialect::SparqSparql11GraphResultsV3,
            query: "ASK {}".into(),
            authority: DatasetAuthority::HolderDeclared,
            policy: v3::Policy::default(),
            nonce: NONCE,
        },
        dataset: v3::PrivateDataset {
            nquads: String::new(),
            named_graphs: vec![],
            salt: fixture::SALT,
        },
    }
}

#[test]
#[ignore = "actual guest execution: needs a real RISC0_SERVER_PATH; creates no receipt"]
fn actual_authrdf_guest_evaluates_the_published_vector_under_both_authorities() {
    fixture::check_published_vector();
    let cases = [
        (fixture::SELECT_BAG, Expect::Bag),
        (fixture::ASK_ISSUER, Expect::Ask(true)),
        (fixture::ASK_OTHER_ISSUER, Expect::Ask(false)),
        (fixture::CONSTRUCT, Expect::Graph),
    ];
    for authority in [agreed(fixture::SALT), DatasetAuthority::HolderDeclared] {
        for (query, expect) in cases {
            let witness = fixture::witness(query, authority.clone(), NONCE);
            let native = auth::evaluate(&witness).expect("native oracle");
            let label = format!("{query} / {authority:?}");
            let journal: auth::Journal =
                accepted(&bytes(&words(&witness)), embedded_authrdf_artifact(), &label)
                    .decode()
                    .expect("V5 journal");
            auth::bind_journal(&journal, &witness.request).unwrap();
            assert_eq!(journal, native, "{label}: guest vs native model");
            assert_eq!(journal.result, expect.result(), "{label}: hand-defined result");
            assert_eq!(journal.dataset_commitment, fixture::anchor(fixture::SALT));
        }
    }
}

#[test]
#[ignore = "actual guest execution: needs a real RISC0_SERVER_PATH; creates no receipt"]
fn actual_authrdf_guest_rejects_forged_spliced_and_unauthorized_credentials() {
    positive_control();
    let holder = DatasetAuthority::HolderDeclared;
    let base = || fixture::witness(fixture::ASK_ISSUER, holder.clone(), NONCE);
    let with_credential = |edit: &dyn Fn(&mut SignedCredential)| {
        let mut witness = base();
        edit(&mut witness.dataset.credentials[0]);
        witness
    };
    let proof_line = |line: &str| {
        with_credential(&|credential| credential.proof_config.push_str(line))
    };
    let mut cases = vec![
        (
            "forged_signature",
            with_credential(&|credential| credential.signature[0] ^= 1),
        ),
        (
            "short_signature",
            with_credential(&|credential| {
                credential.signature.pop();
            }),
        ),
        (
            "spliced_document_literal",
            with_credential(&|credential| {
                credential.document = credential
                    .document
                    .replace("The School of Examples", "The School of Exampled");
            }),
        ),
        (
            "spliced_document_statement",
            with_credential(&|credential| {
                credential
                    .document
                    .push_str("<did:example:abcdefgh> <https://schema.org/name> \"Mallory\" .\n");
            }),
        ),
        (
            "spliced_proof_created",
            with_credential(&|credential| {
                credential.proof_config = credential.proof_config.replace("23:36:38Z", "23:36:39Z");
            }),
        ),
        (
            "unsupported_option_challenge",
            proof_line("_:c14n0 <https://w3id.org/security#challenge> \"abc\" .\n"),
        ),
        (
            "unsupported_option_previous_proof",
            proof_line("_:c14n0 <https://w3id.org/security#previousProof> <urn:proof:1> .\n"),
        ),
        (
            "embedded_proof_value",
            proof_line("_:c14n0 <https://w3id.org/security#proofValue> \"z1\" .\n"),
        ),
    ];
    // Verifier-side table changes: the genuine credential no longer matches it.
    let mut other_issuer = base();
    other_issuer.request.policy.authorization[0].issuer = fixture::OTHER_ISSUER.into();
    cases.push(("issuer_mismatch", other_issuer));
    let mut other_method = base();
    other_method.request.policy.authorization[0].verification_method = fixture::OTHER_VM.into();
    cases.push(("verification_method_mismatch", other_method));
    let mut rekeyed = base();
    rekeyed.request.policy.authorization[0].public_key =
        fixture::hex(fixture::RFC8032_TEST1_PUBLIC_KEY);
    cases.push(("other_authorized_key", rekeyed));
    // Anchors and blinding.
    let mut wrong_anchor = base();
    wrong_anchor.request.authority = DatasetAuthority::VerifierAgreed {
        commitment: [42; 32],
    };
    cases.push(("wrong_agreed_anchor", wrong_anchor));
    let mut other_salt = base();
    other_salt.request.authority = agreed(fixture::OTHER_SALT);
    cases.push(("anchor_under_other_salt", other_salt));
    let mut unblinded = base();
    unblinded.dataset.salt = [0; 32];
    cases.push(("zero_salt", unblinded));
    let mut empty = base();
    empty.dataset.credentials.clear();
    cases.push(("no_credentials", empty));
    let mut duplicated = base();
    duplicated.dataset.credentials.push(fixture::credential());
    cases.push(("duplicate_credential", duplicated));
    // Typed V5 layout carrying the V3 version word.
    let mut earlier = base();
    earlier.request.version = v3::VERSION;
    cases.push(("v3_version_in_v5_layout", earlier));

    for (label, witness) in cases {
        assert!(auth::evaluate(&witness).is_err(), "{label}: native model agrees");
        rejected(
            &bytes(&words(&witness)),
            embedded_authrdf_artifact(),
            AUTH_ABORT,
            label,
        );
    }
}

#[test]
#[ignore = "actual guest execution: needs a real RISC0_SERVER_PATH; creates no receipt"]
fn actual_authrdf_guest_rejects_noncanonical_and_foreign_wire_forms() {
    positive_control();
    let witness = fixture::witness(fixture::ASK_ISSUER, agreed(fixture::SALT), NONCE);
    let original = words(&witness);
    // Pin the SDK layout each corruption relies on, so none is a no-op.
    assert_eq!(original[0], auth::VERSION, "the version is the first word");
    let query = witness.request.query.as_bytes();
    assert_eq!(original[1] as usize, query.len(), "the query length follows");
    let padded_words = query.len().div_ceil(4);
    let mut padded = query.to_vec();
    padded.resize(4 * padded_words, 0);
    assert_eq!(
        bytes(&original[2..2 + padded_words]),
        padded,
        "the query is packed little-endian with zero padding"
    );
    assert_ne!(query.len() % 4, 0, "the padding control needs a padded query");
    // The salt is the final field, one word per byte.
    assert_eq!(
        original[original.len() - 32..],
        fixture::SALT.map(u32::from),
        "the salt is the final 32 words"
    );
    assert!(4 * original.len() < auth::MAX_WITNESS_BYTES);

    let mut invalid: Vec<(String, Vec<u8>)> = Vec::new();
    for version in [1, 2, 3, 4, 6] {
        let mut changed = original.clone();
        changed[0] = version;
        invalid.push((format!("version_word_{version}"), bytes(&changed)));
    }
    let mut trailing = original.clone();
    trailing.push(0);
    invalid.push(("trailing_word".into(), bytes(&trailing)));
    // A salt word above 255 is a narrowing alias of the same byte.
    let mut narrowing = original.clone();
    *narrowing.last_mut().unwrap() |= 0x100;
    invalid.push(("narrowing_alias".into(), bytes(&narrowing)));
    // The top byte of the last query word is padding (asserted above).
    let mut padding = original.clone();
    padding[1 + padded_words] |= 0xff << 24;
    invalid.push(("nonzero_string_padding".into(), bytes(&padding)));
    let mut truncated = bytes(&original);
    truncated.truncate(truncated.len() - 4);
    invalid.push(("truncated".into(), truncated));
    // One word past the raw bound; the guest must refuse it before decoding.
    let mut oversized = bytes(&original);
    oversized.resize(auth::MAX_WITNESS_BYTES + 4, 0);
    invalid.push(("oversized".into(), oversized));
    for short in [Vec::new(), vec![5], vec![5, 0, 0], vec![5, 0, 0, 0, 0]] {
        invalid.push((format!("unframed_{}_bytes", short.len()), short));
    }
    // A complete, valid V3 witness is not a V5 input.
    invalid.push(("v3_witness".into(), bytes(&words(&v3_witness()))));

    for (label, input) in invalid {
        rejected(&input, embedded_authrdf_artifact(), AUTH_ABORT, &label);
    }
}

#[test]
#[ignore = "actual guest execution: needs a real RISC0_SERVER_PATH; creates no receipt"]
fn actual_exact_guest_rejects_v5_input_and_still_accepts_v3() {
    // The exact guest is unchanged: V3 still executes, and V5 is not one of its versions.
    let journal: v3::Journal = accepted(
        &bytes(&words(&v3_witness())),
        embedded_artifact(),
        "exact guest V3 control",
    )
    .decode()
    .expect("V3 journal");
    assert_eq!(journal.result, v3::CanonicalResult::Ask(true));
    for authority in [agreed(fixture::SALT), DatasetAuthority::HolderDeclared] {
        let witness = fixture::witness(fixture::ASK_ISSUER, authority, NONCE);
        rejected(
            &bytes(&words(&witness)),
            embedded_artifact(),
            EXACT_ABORT,
            "v5_witness_in_exact_guest",
        );
    }
}
