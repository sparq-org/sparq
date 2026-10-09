// [OPUS-4.8] sq-qcnn.24 (epic sq-qcnn, test-quality program): DIRECT, deterministic,
// value-asserting unit tests for the PROVER-FREE public surface of sparq-zk-compose
// that the bb-gated e2e suite reaches only through a live nargo/bb toolchain. These
// raise the per-crate line-coverage floor and kill assertion-light survivors on the
// deterministic manifest-assembly + Prover.toml rendering + verification-driver
// error-classification code — one DIRECT test per public fn (never reached only
// indirectly; cf the #1250 indirect-coverage trap).
//
// SCOPE / HONESTY: these are COVERAGE-ONLY tests. NOTHING here asserts, implies, or
// depends on any ZK soundness, zero-knowledge, or privacy property — the verifier's
// soundness is the subject of the OPEN external audit (sq-qhy4), which these tests do
// NOT touch or advance. They exercise deterministic host-side rendering + fail-closed
// typed-error classification, no cryptographic claim. `format!` uses positional args
// (CodeQL `rust/unused-variable` false-positive avoidance).

use oxrdf::{Literal, NamedNode, Term};
use sparq_zk::encode::encode_term;
use sparq_zk::field::{field_to_hex, Fr};
use sparq_zk::sig::{holder_key_digest, public_key_to_hex, SecretKey};
use sparq_zk_compose::build::{
    encode_decimal_literal, encode_int_literal, encode_signed_int_literal,
};
use sparq_zk_compose::driver::{CircuitProver, DriverError};
use sparq_zk_compose::manifest::{BindingMode, CircuitId, FieldHex, StatusListSnapshot};
use sparq_zk_compose::revocation::{merkle_root, merkle_witness, revoke_prover_toml};
use sparq_zk_compose::toml::join_prover_toml;
use sparq_zk_compose::verifier::{KeySet, VerifierNonce};

const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";

fn fh(s: &str) -> FieldHex {
    FieldHex(s.to_string())
}

// ============================================================================
// toml::join_prover_toml — hidden cross-credential join Prover.toml assembly.
// ============================================================================

/// The join Prover.toml renderer emits EXACTLY the field order + formatting the
/// `join_eq_na{n_a}_nb{n_b}` circuit's `Prover.toml` expects: public
/// `challenge, commit_a, commit_b, join_commitment, slot_a, slot_b` then the
/// private witness `enc_a, counts_a, enc_b, counts_b, row_a, row_b, blinding`.
/// The full-string equality pins order + quoting + the `[[...]]` row nesting, so
/// any field swap, dropped field, or format edit is caught (not just no-panic).
#[test]
fn join_prover_toml_renders_exact_field_ordered_body() {
    let enc_a = [[fh("0xa1"), fh("0xa2"), fh("0xa3")]];
    let enc_b = [[fh("0xb1"), fh("0xb2"), fh("0xb3")]];
    let row_a = [fh("0xa1"), fh("0xa2"), fh("0xa3")];
    let row_b = [fh("0xb1"), fh("0xb2"), fh("0xb3")];
    let got = join_prover_toml(
        &fh("0x1"),
        &fh("0x2"),
        &fh("0x3"),
        &fh("0x4"),
        0,
        1,
        &enc_a,
        1,
        &enc_b,
        1,
        &row_a,
        &row_b,
        &fh("0x5"),
    );
    let expected = "challenge = \"0x1\"
commit_a = \"0x2\"
commit_b = \"0x3\"
join_commitment = \"0x4\"
slot_a = \"0\"
slot_b = \"1\"
enc_a = [[\"0xa1\", \"0xa2\", \"0xa3\"]]
counts_a = \"1\"
enc_b = [[\"0xb1\", \"0xb2\", \"0xb3\"]]
counts_b = \"1\"
row_a = [\"0xa1\", \"0xa2\", \"0xa3\"]
row_b = [\"0xb1\", \"0xb2\", \"0xb3\"]
blinding = \"0x5\"
";
    assert_eq!(got, expected);
}

/// A multi-triple witness renders every active row (the `rows_array` loop runs
/// more than once) — asserts the join array is a comma-separated list, catching a
/// mutant that keeps only the first/last row or mis-joins the separator.
#[test]
fn join_prover_toml_renders_multiple_rows_comma_separated() {
    let enc_a = [
        [fh("0x11"), fh("0x12"), fh("0x13")],
        [fh("0x21"), fh("0x22"), fh("0x23")],
    ];
    let enc_b = [[fh("0x31"), fh("0x32"), fh("0x33")]];
    let row_a = [fh("0x11"), fh("0x12"), fh("0x13")];
    let row_b = [fh("0x31"), fh("0x32"), fh("0x33")];
    let got = join_prover_toml(
        &fh("0x0"),
        &fh("0x0"),
        &fh("0x0"),
        &fh("0x0"),
        2,
        0,
        &enc_a,
        2,
        &enc_b,
        1,
        &row_a,
        &row_b,
        &fh("0x0"),
    );
    assert!(
        got.contains("enc_a = [[\"0x11\", \"0x12\", \"0x13\"], [\"0x21\", \"0x22\", \"0x23\"]]\n"),
        "two active rows must render as a comma-separated array:\n{}",
        got
    );
    assert!(got.contains("counts_a = \"2\"\n"), "counts_a follows enc_a: {}", got);
    assert!(got.contains("slot_a = \"2\"\n") && got.contains("slot_b = \"0\"\n"));
}

// ============================================================================
// revocation::revoke_prover_toml — hidden-index revoke Prover.toml assembly.
// ============================================================================

/// The `revoke_unset_d{depth}` Prover.toml renderer emits `challenge, root,
/// index_commitment, index, bit, blinding, siblings` in exactly that order, each
/// field hex-encoded via `field_to_hex` and `siblings` a `depth`-long array. Built
/// against a real (sparse) Merkle witness for an ACTIVE index (bit unset), then
/// asserted byte-for-byte against an independently-assembled expected body — pins
/// field order, the literal `index`, and the sibling count.
#[test]
fn revoke_prover_toml_renders_exact_body_for_active_index() {
    // index 1 revoked; indices 0/2/3 active (bit unset).
    let snapshot = StatusListSnapshot {
        status_list: "http://ex/status".to_string(),
        version: 1,
        bits: vec![0b0000_0010],
    };
    // [GPT-6] Cover the entire byte-sized status snapshot; never truncate it.
    let depth = 3u32;
    let root = merkle_root(&snapshot, depth).expect("root");
    let index = 0u64;
    let witness = merkle_witness(&snapshot, depth, index).expect("witness");
    assert_eq!(witness.bit, Fr::from(0u64), "index 0 is active (bit unset)");
    assert_eq!(witness.siblings.len(), depth as usize, "one sibling per level");

    let challenge = Fr::from(11u64);
    let index_commitment = Fr::from(22u64);
    let blinding = Fr::from(33u64);
    let got = revoke_prover_toml(&challenge, &root, &index_commitment, index, &blinding, &witness);

    let sibs: Vec<String> = witness
        .siblings
        .iter()
        .map(|s| format!("\"{}\"", field_to_hex(s)))
        .collect();
    let expected = format!(
        "challenge = \"{}\"\nroot = \"{}\"\nindex_commitment = \"{}\"\nindex = \"{}\"\nbit = \"{}\"\nblinding = \"{}\"\nsiblings = [{}]\n",
        field_to_hex(&challenge),
        field_to_hex(&root),
        field_to_hex(&index_commitment),
        index,
        field_to_hex(&witness.bit),
        field_to_hex(&blinding),
        sibs.join(", "),
    );
    assert_eq!(got, expected);
}

// ============================================================================
// build::encode_{signed_int,decimal}_literal — deterministic term encoding.
// ============================================================================

/// A NON-negative signed-int literal encodes byte-identically to the unsigned
/// `encode_int_literal` (both build `"<n>"^^xsd:integer` under the salt-0 literal
/// convention); a NEGATIVE one differs (the `-` is bound into the token). Pins the
/// encoding contract without hard-coding a crypto constant, and catches a mutant
/// that drops the sign into the lexical form.
#[test]
fn encode_signed_int_literal_matches_unsigned_for_nonnegative() {
    assert_eq!(
        encode_signed_int_literal(7),
        encode_int_literal(7),
        "+7 signed == 7 unsigned (same `\"7\"^^xsd:integer` token)"
    );
    assert_ne!(
        encode_signed_int_literal(-7),
        encode_int_literal(7),
        "-7 differs from 7 (the sign is part of the committed token)"
    );
    // Deterministic + collision-free across distinct magnitudes.
    assert_eq!(encode_signed_int_literal(-7), encode_signed_int_literal(-7));
    assert_ne!(encode_signed_int_literal(-7), encode_signed_int_literal(-8));
}

/// A decimal literal encodes to the SAME field element as an independently
/// `encode_term`-encoded `"[-]<int>.<frac>"^^xsd:decimal` — pinning the exact
/// lexical form the host builds (sign, integer part, fraction) against the shared
/// encoder, and confirming the three components are each load-bearing.
#[test]
fn encode_decimal_literal_matches_independent_lexical_encoding() {
    let expected = {
        let lit = Literal::new_typed_literal("123.45", NamedNode::new(XSD_DECIMAL).unwrap());
        FieldHex(field_to_hex(
            &encode_term(&Term::Literal(lit), &Fr::from(0u64)).unwrap(),
        ))
    };
    assert_eq!(encode_decimal_literal(false, 123, "45"), expected);

    let neg = {
        let lit = Literal::new_typed_literal("-123.45", NamedNode::new(XSD_DECIMAL).unwrap());
        FieldHex(field_to_hex(
            &encode_term(&Term::Literal(lit), &Fr::from(0u64)).unwrap(),
        ))
    };
    assert_eq!(encode_decimal_literal(true, 123, "45"), neg);

    // Sign / integer / fraction are each load-bearing (no component is dropped).
    assert_ne!(
        encode_decimal_literal(false, 123, "45"),
        encode_decimal_literal(true, 123, "45")
    );
    assert_ne!(
        encode_decimal_literal(false, 123, "45"),
        encode_decimal_literal(false, 124, "45")
    );
    assert_ne!(
        encode_decimal_literal(false, 123, "45"),
        encode_decimal_literal(false, 123, "46")
    );
}

/// Bridge-check: an integer literal round-trips through `encode_int_literal` to a
/// non-degenerate encoding and is representation-stable (kills a
/// `Default::default()`/constant-return mutant on the encoder helper).
#[test]
fn encode_int_literal_is_stable_and_nondegenerate() {
    let a = encode_int_literal(42);
    assert_eq!(a, encode_int_literal(42));
    // Non-degeneracy: the encoding must not collapse to the ZERO field element. A
    // `FieldHex` is a CANONICAL `0x`-prefixed 64-nibble string (via `field_to_hex`), so
    // the zero element renders as `0x000…000`, NOT the bare `"0x0"` — comparing against
    // `fh("0x0")` would be vacuous (a real encoding never equals the non-canonical
    // `"0x0"`). Compare against the true canonical zero so the check actually bites.
    let canonical_zero = FieldHex(field_to_hex(&Fr::from(0u64)));
    assert_ne!(canonical_zero, fh("0x0"), "sanity: canonical zero is not the bare \"0x0\"");
    assert_ne!(a, canonical_zero, "a real encoding is not the zero field element");
    assert_ne!(a, encode_int_literal(43));
}

// ============================================================================
// driver::CircuitProver — verification-driver FAIL-CLOSED error classification.
// The nargo/bb subprocess wrappers must surface a TYPED DriverError, never panic.
// Each path is driven deterministically WITHOUT the toolchain (a guaranteed-absent
// working directory for the spawn arm; a missing parent dir for the io arm).
// ============================================================================

fn nonexistent_dir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "sq-qcnn24-absent-{}-{}-{}",
        tag,
        std::process::id(),
        // a monotonic-ish salt so parallel tests never collide on the name
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&p);
    p
}

/// `compile` spawns `nargo` in the workspace dir; against a NON-EXISTENT
/// `compose_dir` the spawn cannot even chdir, so it fails closed with a typed
/// `DriverError::Spawn` naming `nargo` — never a panic, and distinct from the io
/// arm. (Toolchain-independent: absent-cwd fails the spawn with or without nargo.)
#[test]
fn compile_without_workspace_is_spawn_error_naming_nargo() {
    let prover = CircuitProver::new(nonexistent_dir("compile"));
    let err = prover
        .compile(&CircuitId::FilterInt { d: 1 })
        .expect_err("compile must surface an error, never spawn a proof");
    match err {
        DriverError::Spawn { tool, .. } => assert_eq!(tool, "nargo", "Spawn names the tool"),
        other => panic!("expected Spawn classification, got {:?}", other),
    }
}

/// `gen_witness` writes the per-member `Prover.toml` BEFORE spawning nargo; when
/// `compose_dir` (hence the package dir) does not exist, that write fails and is
/// classified `DriverError::Io` via the `From<io::Error>` conversion — the fail-
/// closed io arm, distinct from a spawn failure.
#[test]
fn gen_witness_without_package_dir_is_io_error() {
    let prover = CircuitProver::new(nonexistent_dir("witness"));
    let err = prover
        .gen_witness(&CircuitId::FilterInt { d: 1 }, "challenge = \"0x1\"\n")
        .expect_err("no package dir => the Prover.toml write fails");
    assert!(
        matches!(err, DriverError::Io(_)),
        "a failed toml write is the Io arm, got {:?}",
        err
    );
}

/// `prove` (via `prove_in`) compiles FIRST, so with no workspace it fails closed at
/// the compile spawn — a typed `DriverError::Spawn`, never a panic, and never a
/// partial artifact read.
#[test]
fn prove_without_workspace_is_spawn_error() {
    let prover = CircuitProver::new(nonexistent_dir("prove"));
    let out = nonexistent_dir("prove-out");
    let err = prover
        .prove(&CircuitId::FilterInt { d: 1 }, "challenge = \"0x1\"\n", &out)
        .expect_err("prove must fail closed without a toolchain");
    assert!(matches!(err, DriverError::Spawn { .. }), "got {:?}", err);
}

/// `canonical_vk` (the verifier-side authentic-vk path, audit #2) also compiles
/// first, so it fails closed at the compile spawn without the toolchain — a typed
/// error, never a prover-supplied vk fallback.
#[test]
fn canonical_vk_without_workspace_is_spawn_error() {
    let prover = CircuitProver::new(nonexistent_dir("vk"));
    let work = nonexistent_dir("vk-work");
    let err = prover
        .canonical_vk(&CircuitId::FilterInt { d: 1 }, &work)
        .expect_err("canonical_vk must fail closed without a toolchain");
    assert!(matches!(err, DriverError::Spawn { .. }), "got {:?}", err);
}

/// `verify_with` (and its `verify` wrapper) create the scratch dir first; when the
/// work path's parent is a regular FILE the `create_dir_all` fails and is
/// classified `DriverError::Io` — the fail-closed io arm reached before any bb call.
#[test]
fn verify_with_uncreatable_workdir_is_io_error() {
    let base = nonexistent_dir("verify");
    std::fs::create_dir_all(&base).expect("scratch base");
    let a_file = base.join("a_file");
    std::fs::write(&a_file, b"not a directory").expect("write blocker file");
    // work_dir's PARENT is a file => create_dir_all cannot succeed.
    let work = a_file.join("cannot_exist");

    let err = CircuitProver::new(&base)
        .verify_with(b"proof", b"public", b"vk", &work)
        .expect_err("create_dir_all under a file must fail");
    assert!(matches!(err, DriverError::Io(_)), "got {:?}", err);

    // The `verify` wrapper delegates to `verify_with` — same fail-closed io arm.
    let art = sparq_zk_compose::driver::ProofArtifacts {
        proof: b"p".to_vec(),
        public_inputs: b"i".to_vec(),
        vk: b"k".to_vec(),
    };
    let err2 = CircuitProver::new(&base)
        .verify(&art, &work)
        .expect_err("verify wrapper propagates the io arm");
    assert!(matches!(err2, DriverError::Io(_)), "got {:?}", err2);

    let _ = std::fs::remove_dir_all(&base);
}

// ============================================================================
// manifest::BindingMode — challenge accessor + holder-key derivation (T2 wiring).
// ============================================================================

/// A `Challenge` binding exposes its challenge and has NO holder key/digest; a
/// `HolderPop` binding exposes the SAME challenge accessor AND derives the presented
/// holder key + its T1 digest, cross-checked against `sparq_zk::sig` directly.
#[test]
fn binding_mode_challenge_and_holder_key_derivation() {
    let challenge = fh("0x9");
    let bare = BindingMode::Challenge { challenge: challenge.clone() };
    assert_eq!(bare.challenge(), &challenge);
    assert!(bare.holder_key().is_none(), "a bare challenge has no holder key");
    assert!(bare.holder_key_digest().is_none());

    let holder_sk = SecretKey::from_seed(42);
    let hpk = holder_sk.public_key();
    let hpk_hex = public_key_to_hex(&hpk);
    let pop = BindingMode::HolderPop {
        challenge: challenge.clone(),
        holder: hpk_hex.clone(),
        pop: "00".to_string(),
        cryptosuite: "https://sparq.dev/ns/zk#poseidon2-schnorr-v1".to_string(),
    };
    assert_eq!(pop.challenge(), &challenge, "HolderPop exposes the same challenge");
    let derived = pop.holder_key().expect("valid holder key parses");
    assert_eq!(
        public_key_to_hex(&derived),
        hpk_hex,
        "the derived key round-trips to the presented hex"
    );
    let want_digest = holder_key_digest(&hpk).expect("digest");
    assert_eq!(
        pop.holder_key_digest(),
        Some(want_digest),
        "the wired T1 digest equals sparq_zk::sig::holder_key_digest"
    );
}

/// A `HolderPop` binding with a MALFORMED holder hex derives NO key/digest (fail-
/// closed: an unparseable key is `None`, not a panic) while still exposing the
/// challenge — the `holder_key` / `holder_key_digest` None arms.
#[test]
fn binding_mode_holder_pop_malformed_key_is_none() {
    let challenge = fh("0xabc");
    let pop = BindingMode::HolderPop {
        challenge: challenge.clone(),
        holder: "zznothex".to_string(),
        pop: "00".to_string(),
        cryptosuite: "https://sparq.dev/ns/zk#poseidon2-schnorr-v1".to_string(),
    };
    assert_eq!(pop.challenge(), &challenge);
    assert!(pop.holder_key().is_none(), "malformed hex => no key");
    assert!(pop.holder_key_digest().is_none(), "no key => no digest");
}

// ============================================================================
// verifier::{VerifierNonce, KeySet} — small deterministic trust-anchor helpers.
// ============================================================================

/// `VerifierNonce::from_field` adopts a field element directly and is
/// representation-equivalent to `from_hex` over that element's canonical hex.
#[test]
fn verifier_nonce_from_field_matches_from_hex() {
    let f = Fr::from(0x1234u64);
    let by_field = VerifierNonce::from_field(f);
    let by_hex = VerifierNonce::from_hex(&field_to_hex(&f)).expect("canonical hex parses");
    assert_eq!(by_field, by_hex, "from_field == from_hex(canonical(field))");
    assert_eq!(
        by_field.as_field_hex(),
        FieldHex(field_to_hex(&f)),
        "as_field_hex renders the adopted element"
    );
}

/// `KeySet::member_index` returns the (stable, distinct) position of a trusted key
/// and `None` for a non-member, and normalizes the queried hex (an `0x`-prefixed /
/// upper-case form resolves to the same member) — the audit-#3 trust-anchor lookup.
#[test]
fn keyset_member_index_is_normalized_and_fail_closed() {
    let k0 = public_key_to_hex(&SecretKey::from_seed(1).public_key());
    let k1 = public_key_to_hex(&SecretKey::from_seed(2).public_key());
    let absent = public_key_to_hex(&SecretKey::from_seed(999).public_key());
    let ks = KeySet::from_hex_keys([k0.clone(), k1.clone()]);

    let i0 = ks.member_index(&k0).expect("k0 is a member");
    let i1 = ks.member_index(&k1).expect("k1 is a member");
    assert_ne!(i0, i1, "distinct members get distinct indices");
    assert!(i0 < 2 && i1 < 2, "indices are within the 2-key set");
    assert!(ks.member_index(&absent).is_none(), "a non-member is None");

    // Representation-insensitive: 0x-prefixed / upper-cased query hits the same slot.
    let k0_decorated = format!("0x{}", k0.trim_start_matches("0x").to_uppercase());
    assert_eq!(
        ks.member_index(&k0_decorated),
        Some(i0),
        "member_index normalizes the queried hex"
    );
}
