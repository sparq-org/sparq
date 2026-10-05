#![cfg(feature = "vc-bridge")]
//! Canonical W3C Data-Integrity test vectors for the off-circuit VC ingest
//! bridge (sq-wklnb; Opus review follow-up on #1155).
//!
//! # Why this file exists
//! The in-module `vc_bridge` tests mint their own key, sign the `hashData`
//! **this crate just computed**, and check that it verifies. That witnesses the
//! fail-closed behaviour (tamper / wrong key / malformed bytes all reject) but
//! it is self-referential: if sparq's canonicalization, its SHA-256 ordering, or
//! its `proofConfigHash || documentHash` concatenation disagreed with the W3C
//! algorithm, those tests would still pass — they would just be internally
//! consistent about the wrong bytes.
//!
//! This suite closes that gap with the **published** vectors, so every step is
//! pinned against bytes sparq did not produce:
//!
//! 1. sparq's RDFC-1.0 canonical N-Quads equal the spec's canonical document
//!    **byte for byte** (including the trailing newline the hash covers);
//! 2. [`hash_data_from_triples`] reproduces the spec's published `hashData`;
//! 3. a proof issued by a **third party** (the W3C editors' key, signature
//!    transcribed from the spec's `proofValue`) verifies through the bridge's
//!    public entry points and ingests end to end.
//!
//! Each positive assertion is paired with a mutation that must go red — a
//! single-bit flip in the signature, a dropped credential triple, the other
//! suite's proof config, the other suite's key — so the suite cannot pass
//! vacuously.
//!
//! Vectors are vendored under `tests/fixtures/w3c-di-vectors/` (see its
//! `PROVENANCE.md` for sources, licence, and the published hashes).
//!
//! # Scope (honest)
//! This is the bridge's **off-circuit, ingest-time** host verification. Interop
//! here says the DI hashing + signature check agree with the W3C algorithm for
//! the two whole-credential `rdfc` suites — including **both** published
//! `ecdsa-rdfc-2019` curve profiles (P-256/SHA-256 and P-384/SHA-384, sq-txg1y);
//! it asserts **no** in-circuit or query-soundness property, and the ZK estate is
//! **not externally audited** (sq-qhy4). The selective-disclosure suites
//! (`bbs-2023`, `ecdsa-sd-2023`) are out of the bridge's scope and have no
//! vectors here.
//!
//! These vectors are **RDF-native**: they start from the specs' *canonical
//! N-Quads*, so they pin the DI hashing + signature check but say nothing about
//! JSON-LD expansion. The JSON envelope layer (`vc_bridge_json`) is exercised by
//! its own in-module tests over self-contained inline-`@context` documents.

use oxrdf::{NamedNode, Triple};
use sha2::{Digest, Sha256, Sha384};
use sparq_core::Graph;
use sparq_zk::canon::{canonicalize_triples, graph_triples};
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::vc_bridge::{
    hash_data_from_triples, hash_data_from_triples_sha384, ingest_verified_vc, verify_source_proof,
    verify_source_proof_by_token, VcBridgeError, VcCryptosuite,
};

// ---------------------------------------------------------------------------
// The vendored vectors.
// ---------------------------------------------------------------------------

/// vc-di-eddsa Example 9 == vc-di-ecdsa Example 7 == vc-di-ecdsa Example 29 — the
/// canonical form of the unsigned Alumni Credential, shared by every vector here
/// (both suites and both ECDSA curve profiles publish the same document).
const CREDENTIAL_NQ: &str = include_str!("fixtures/w3c-di-vectors/credential.nq");
/// vc-di-eddsa Example 10 / vc-di-ecdsa Example 8 — SHA-256 of the above.
const CREDENTIAL_HASH_HEX: &str =
    "517744132ae165a5349155bef0bb0cf2258fff99dfe1dbd914b938d775a36017";
/// vc-di-ecdsa Example 30 — **SHA-384** of the same document (the P-384 profile).
const CREDENTIAL_HASH_384_HEX: &str = "8bf6e01df72c5b62f91b685231915ac4b8c58ea95f002c6b8f6bfafa\
                                       1b251df476b56b8e01518e317dab099d3ecbff96";

/// vc-di-eddsa Example 12 — canonical proof options for `eddsa-rdfc-2022`.
const EDDSA_PROOF_CONFIG_NQ: &str =
    include_str!("fixtures/w3c-di-vectors/eddsa-rdfc-2022.proof-config.nq");
/// vc-di-ecdsa Example 10 — canonical proof options for `ecdsa-rdfc-2019` (P-256).
const ECDSA_PROOF_CONFIG_NQ: &str =
    include_str!("fixtures/w3c-di-vectors/ecdsa-rdfc-2019.proof-config.nq");
/// vc-di-ecdsa Example 32 — canonical proof options for `ecdsa-rdfc-2019` (P-384).
/// Same shape as the P-256 one; it names the P-384 `verificationMethod`.
const ECDSA_P384_PROOF_CONFIG_NQ: &str =
    include_str!("fixtures/w3c-di-vectors/ecdsa-rdfc-2019-p384.proof-config.nq");

/// Which DI hash a vector's profile uses. `ecdsa-rdfc-2019` publishes BOTH — the
/// cryptosuite token is identical and the curve of the issuer key selects the
/// hash, which is exactly the discrimination [`Vector::hash_data`] pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiHash {
    /// `eddsa-rdfc-2022` and `ecdsa-rdfc-2019` with a P-256 key.
    Sha256,
    /// `ecdsa-rdfc-2019` with a P-384 key (vc-di-ecdsa §A.3).
    Sha384,
}

/// One published `rdfc` vector, quoted field by field from its spec section.
struct Vector {
    /// A human label, since `suite` no longer identifies a vector uniquely (the
    /// two ECDSA profiles share the `ecdsa-rdfc-2019` token).
    label: &'static str,
    /// The suite under test.
    suite: VcCryptosuite,
    /// The DI hash this vector's curve profile selects.
    hash: DiHash,
    /// The vendored canonical proof-options document.
    proof_config_nq: &'static str,
    /// The vector's own hash of `proof_config_nq` as published (eddsa Ex. 13 /
    /// ecdsa Ex. 11 / ecdsa Ex. 33) — SHA-256 or SHA-384 per `hash`.
    proof_config_hash_hex: &'static str,
    /// The vector's own hash of `CREDENTIAL_NQ` as published (eddsa Ex. 10 /
    /// ecdsa Ex. 8 / ecdsa Ex. 30).
    credential_hash_hex: &'static str,
    /// `proofConfigHash || documentHash` as published (eddsa Ex. 14 / ecdsa
    /// Ex. 12 / ecdsa Ex. 34).
    hash_data_hex: &'static str,
    /// The raw signature over `hash_data` (eddsa Ex. 15 / ecdsa Ex. 13 / Ex. 35).
    signature_hex: &'static str,
    /// The byte width of that signature (`r‖s`): 64 for Ed25519 and P-256, 96 for
    /// P-384. Pinned so a truncated transcription cannot pass unnoticed.
    signature_len: usize,
    /// The issuer verification key (eddsa Ex. 7 / ecdsa Ex. 5 / ecdsa Ex. 27),
    /// multibase base58-btc over a multicodec-prefixed public key.
    public_key_multibase: &'static str,
    /// The multicodec prefix `public_key_multibase` must carry once decoded:
    /// `ed25519-pub` = `0xed 0x01`, `p256-pub` = `0x80 0x24`,
    /// `p384-pub` = `0x81 0x24`.
    key_multicodec: &'static [u8],
    /// The `proof.proofValue` of the signed credential (eddsa Ex. 16 / ecdsa
    /// Ex. 14 / ecdsa Ex. 36) — multibase base58-btc over `signature_hex`.
    proof_value: &'static str,
}

impl Vector {
    /// Derive this vector's `hashData` through the bridge's public transform,
    /// choosing the entry point its curve profile selects.
    fn hash_data(&self, credential: &[Triple]) -> Vec<u8> {
        let cfg = triples(self.proof_config_nq);
        match self.hash {
            DiHash::Sha256 => hash_data_from_triples(credential, &cfg)
                .expect("hashData must be derivable from the vector")
                .to_vec(),
            DiHash::Sha384 => hash_data_from_triples_sha384(credential, &cfg)
                .expect("hashData must be derivable from the vector")
                .to_vec(),
        }
    }

    /// Hash a byte string with the digest this vector's profile uses.
    fn digest(&self, bytes: &[u8]) -> Vec<u8> {
        match self.hash {
            DiHash::Sha256 => Sha256::digest(bytes).to_vec(),
            DiHash::Sha384 => Sha384::digest(bytes).to_vec(),
        }
    }
}

const EDDSA_RDFC_2022: Vector = Vector {
    label: "eddsa-rdfc-2022",
    suite: VcCryptosuite::EddsaRdfc2022,
    hash: DiHash::Sha256,
    proof_config_nq: EDDSA_PROOF_CONFIG_NQ,
    proof_config_hash_hex: "bea7b7acfbad0126b135104024a5f1733e705108f42d59668b05c0c50004c6b0",
    credential_hash_hex: CREDENTIAL_HASH_HEX,
    hash_data_hex: "bea7b7acfbad0126b135104024a5f1733e705108f42d59668b05c0c50004c6b0\
                    517744132ae165a5349155bef0bb0cf2258fff99dfe1dbd914b938d775a36017",
    signature_hex: "4d8e53c2d5b3f2a7891753eb16ca993325bdb0d3cfc5be1093d0a18426f5ef85\
                    78cadc0fd4b5f4dd0d1ce0aefd15ab120b7a894d0eb094ffda4e6553cd1ed50d",
    signature_len: 64,
    public_key_multibase: "z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2",
    key_multicodec: &[0xed, 0x01],
    proof_value:
        "z2YwC8z3ap7yx1nZYCg4L3j3ApHsF8kgPdSb5xoS1VR7vPG3F561B52hYnQF9iseabecm3ijx4K1FBTQsCZahKZme",
};

const ECDSA_RDFC_2019_P256: Vector = Vector {
    label: "ecdsa-rdfc-2019 (P-256)",
    suite: VcCryptosuite::EcdsaRdfc2019,
    hash: DiHash::Sha256,
    proof_config_nq: ECDSA_PROOF_CONFIG_NQ,
    proof_config_hash_hex: "3a8a522f689025727fb9d1f0fa99a618da023e8494ac74f51015d009d35abc2e",
    credential_hash_hex: CREDENTIAL_HASH_HEX,
    hash_data_hex: "3a8a522f689025727fb9d1f0fa99a618da023e8494ac74f51015d009d35abc2e\
                    517744132ae165a5349155bef0bb0cf2258fff99dfe1dbd914b938d775a36017",
    signature_hex: "1cb4290918ffb04a55ff7ae1e55e316a9990fda8eec67325eac7fcbf2ddf9dd2\
                    b06716a657e72b284c9604df3a172ecbf06a1a475b49ac807b1d9162df855636",
    signature_len: 64,
    public_key_multibase: "zDnaepBuvsQ8cpsWrVKw8fbpGpvPeNSjVPTWoq6cRqaYzBKVP",
    key_multicodec: &[0x80, 0x24],
    proof_value:
        "zaHXrr7AQdydBk3ahpCDpWbxfLokDqmCToYm2dyWvpcFVyWooC2he63w1f7UNQoAMKdhaRtcnaE2KTo5o5vTCcfw",
};

/// [OPUS-5] sq-txg1y — vc-di-ecdsa § A.3 "Representation: ecdsa-rdfc-2019, with
/// curve P-384". Same cryptosuite token as the vector above; SHA-384 throughout,
/// a 96-byte `hashData` and a 96-byte signature.
const ECDSA_RDFC_2019_P384: Vector = Vector {
    label: "ecdsa-rdfc-2019 (P-384)",
    suite: VcCryptosuite::EcdsaRdfc2019,
    hash: DiHash::Sha384,
    proof_config_nq: ECDSA_P384_PROOF_CONFIG_NQ,
    proof_config_hash_hex: "e32805a26492eac777aa7a138f6d8da3c74e0c7be7b296dcaccf97420c3b92ea\
                            ad7be6449ca565e165031567f5c7cbc1",
    credential_hash_hex: CREDENTIAL_HASH_384_HEX,
    hash_data_hex: "e32805a26492eac777aa7a138f6d8da3c74e0c7be7b296dcaccf97420c3b92ea\
                    ad7be6449ca565e165031567f5c7cbc1\
                    8bf6e01df72c5b62f91b685231915ac4b8c58ea95f002c6b8f6bfafa1b251df4\
                    76b56b8e01518e317dab099d3ecbff96",
    signature_hex: "177ac088806c2506d49f0bfec16056a6a80ace62cd029888ad561aba22a59d19\
                    2d77d9b1fc28df80dea5ee6c8bceb16f1b8bff6bd6ff2d8f8778bdde48bafa7b\
                    6cc1f914c0168b5c04499882f632deea9cb7d977e888bb0e1ee9fb20ff03b025",
    signature_len: 96,
    public_key_multibase: "z82LkuBieyGShVBhvtE2zoiD6Kma4tJGFtkAhxR5pfkp5QPw4LutoYWhvQCnGjdVn14kujQ",
    key_multicodec: &[0x81, 0x24],
    proof_value: "z967Mvv5bxtmLNqTzPZ8KmJjFmFXaAKeQNzq7GWnQkMcLtaGSSmuozE5WtJ8PipMe178B1tE28K1vs\
                  Jur9bGVJhz6jgSJsRHFSQeqgH8hhjcg8gZDFJC1b9FsR5ggNmDBqHv",
};

const VECTORS: [&Vector; 3] = [
    &EDDSA_RDFC_2022,
    &ECDSA_RDFC_2019_P256,
    &ECDSA_RDFC_2019_P384,
];

// ---------------------------------------------------------------------------
// Small transcoders, so the vectors above stay verbatim-quotable from the spec.
// (Deliberately no new dependency for two hex/base58 helpers.)
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// base58-btc (the `z` multibase). Schoolbook big-integer decode over a byte
/// vector — the inputs here are under 100 characters.
fn base58btc_decode(s: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut out: Vec<u8> = Vec::new();
    for c in s.bytes() {
        let mut carry = ALPHABET
            .iter()
            .position(|&a| a == c)
            .expect("base58-btc alphabet") as u32;
        for byte in out.iter_mut().rev() {
            let v = (*byte as u32) * 58 + carry;
            *byte = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            out.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Leading '1's are leading zero bytes.
    let zeros = s.bytes().take_while(|&c| c == b'1').count();
    let mut prefixed = vec![0u8; zeros];
    prefixed.extend_from_slice(&out);
    prefixed
}

/// Decode a `publicKeyMultibase`, asserting the multibase prefix and multicodec
/// the vector declares, and return the raw key bytes the bridge consumes
/// (Ed25519: 32B; P-256: 33B SEC1 compressed; P-384: 49B SEC1 compressed).
fn multibase_public_key(v: &Vector) -> Vec<u8> {
    let (base, rest) = v.public_key_multibase.split_at(1);
    assert_eq!(base, "z", "publicKeyMultibase must be base58-btc");
    let decoded = base58btc_decode(rest);
    assert_eq!(
        &decoded[..v.key_multicodec.len()],
        v.key_multicodec,
        "unexpected multicodec prefix for {}",
        v.label
    );
    decoded[v.key_multicodec.len()..].to_vec()
}

/// Decode a `proofValue` (multibase base58-btc, no multicodec prefix) into the
/// raw signature bytes the bridge consumes.
fn proof_value_signature(v: &Vector) -> Vec<u8> {
    let (base, rest) = v.proof_value.split_at(1);
    assert_eq!(base, "z", "proofValue must be base58-btc");
    base58btc_decode(rest)
}

/// Parse a vendored canonical N-Quads document back into triples. The vectors
/// are single default-graph documents, which is exactly the shape the bridge's
/// `&[Triple]` API takes.
fn triples(nquads: &str) -> Vec<Triple> {
    let g = Graph::load_str(nquads, "ntriples").expect("vendored vector must parse");
    graph_triples(&g).expect("vendored vector must materialize as triples")
}

// ---------------------------------------------------------------------------
// 1. The vendored bytes are intact.
// ---------------------------------------------------------------------------

/// Drift guard: each fixture file, **as stored**, must hash to the value the
/// spec publishes next to it. A stray edit (or a lost trailing newline — the
/// hash covers it) fails here rather than silently weakening everything below.
#[test]
fn vendored_vectors_hash_to_published_values() {
    assert_eq!(
        hex_encode(&Sha256::digest(CREDENTIAL_NQ.as_bytes())),
        CREDENTIAL_HASH_HEX,
        "credential.nq no longer matches its published SHA-256"
    );
    assert_eq!(
        hex_encode(&Sha384::digest(CREDENTIAL_NQ.as_bytes())),
        CREDENTIAL_HASH_384_HEX,
        "credential.nq no longer matches its published SHA-384"
    );
    for v in VECTORS {
        // Each vector is checked under the digest ITS profile publishes.
        assert_eq!(
            hex_encode(&v.digest(v.proof_config_nq.as_bytes())),
            v.proof_config_hash_hex,
            "{} proof config no longer matches its published hash",
            v.label
        );
        assert_eq!(
            hex_encode(&v.digest(CREDENTIAL_NQ.as_bytes())),
            v.credential_hash_hex,
            "{} credential hash no longer matches its published value",
            v.label
        );
    }
}

/// The published `hashData` is `proofConfigHash || documentHash` — pin the
/// concatenation ORDER against the vector itself, so a transposed
/// implementation cannot hide behind a self-consistent test.
#[test]
fn published_hash_data_is_proof_config_then_document() {
    for v in VECTORS {
        assert_eq!(
            v.hash_data_hex,
            format!("{}{}", v.proof_config_hash_hex, v.credential_hash_hex),
            "{} vector transcription is internally inconsistent",
            v.label
        );
    }
}

/// The spec's `proofValue` and its hex signature are two encodings of the same
/// bytes — checking both pins the base58-btc transcoder used below.
#[test]
fn proof_value_decodes_to_published_signature() {
    for v in VECTORS {
        assert_eq!(
            hex_encode(&proof_value_signature(v)),
            v.signature_hex,
            "{} proofValue must decode to the published signature",
            v.label
        );
        assert_eq!(
            proof_value_signature(v).len(),
            v.signature_len,
            "{} signature width",
            v.label
        );
    }
}

// ---------------------------------------------------------------------------
// 2. sparq's canonicalization reproduces the W3C canonical documents.
// ---------------------------------------------------------------------------

/// **Byte-level interop, step one.** Round-trip each vendored canonical
/// document through sparq's RDFC-1.0 layer (parse → canonicalize → serialize)
/// and require the result to be byte-identical to the spec's document — same
/// term serialization, same code-point ordering, same `c14n0` blank-node
/// labelling, same trailing newline.
#[test]
fn sparq_canonicalization_reproduces_w3c_canonical_documents() {
    let cases: [(&str, &str); 4] = [
        ("credential", CREDENTIAL_NQ),
        ("eddsa-rdfc-2022 proof config", EDDSA_PROOF_CONFIG_NQ),
        ("ecdsa-rdfc-2019 P-256 proof config", ECDSA_PROOF_CONFIG_NQ),
        (
            "ecdsa-rdfc-2019 P-384 proof config",
            ECDSA_P384_PROOF_CONFIG_NQ,
        ),
    ];
    for (label, nq) in cases {
        let canonical = canonicalize_triples(&triples(nq))
            .expect("vendored vector must canonicalize")
            .to_nquads();
        assert_eq!(canonical, nq, "{} canonical N-Quads diverge from W3C", label);
    }
}

/// **Byte-level interop, step two.** The bridge's public transform must derive
/// the spec's published `hashData` from the vectors' triples — this is the
/// whole DI hashing step (canonicalize both documents, SHA-256 each, and
/// concatenate proof-config-first) checked against bytes sparq did not produce.
#[test]
fn hash_data_matches_published_vector() {
    let cred = triples(CREDENTIAL_NQ);
    for v in VECTORS {
        assert_eq!(
            hex_encode(&v.hash_data(&cred)),
            v.hash_data_hex,
            "{} hashData diverges from the published vector",
            v.label
        );
    }
}

/// **The profile discrimination, pinned against published bytes.** The two
/// `ecdsa-rdfc-2019` vectors carry the SAME cryptosuite token, so nothing in the
/// token tells the bridge which digest to use — only the issuer key's curve does.
/// Deriving the P-384 vector's `hashData` with the SHA-256 entry point (what a
/// dispatch that ignored the curve would do) must NOT reproduce the published
/// value, and vice versa. [OPUS-5] sq-txg1y.
#[test]
fn ecdsa_curve_profiles_select_different_hash_data() {
    let cred = triples(CREDENTIAL_NQ);
    let p384_cfg = triples(ECDSA_P384_PROOF_CONFIG_NQ);

    let wrong_hash = hash_data_from_triples(&cred, &p384_cfg).unwrap();
    assert_eq!(wrong_hash.len(), 64);
    assert_ne!(
        hex_encode(&wrong_hash),
        ECDSA_RDFC_2019_P384.hash_data_hex,
        "SHA-256 must not reproduce the P-384 vector's published hashData"
    );

    let wrong_hash_384 =
        hash_data_from_triples_sha384(&cred, &triples(ECDSA_PROOF_CONFIG_NQ)).unwrap();
    assert_eq!(wrong_hash_384.len(), 96);
    assert_ne!(
        hex_encode(&wrong_hash_384),
        ECDSA_RDFC_2019_P256.hash_data_hex,
        "SHA-384 must not reproduce the P-256 vector's published hashData"
    );
}

/// Mutation guard for the test above: a `hashData` derived from the *other*
/// suite's proof config must NOT match, so the assertion is discriminating
/// rather than trivially true.
#[test]
fn hash_data_is_bound_to_its_own_proof_config() {
    let cred = triples(CREDENTIAL_NQ);
    let crossed = hash_data_from_triples(&cred, &triples(ECDSA_PROOF_CONFIG_NQ)).unwrap();
    assert_ne!(
        hex_encode(&crossed),
        EDDSA_RDFC_2022.hash_data_hex,
        "hashData must be bound to the proof config it was derived from"
    );
}

// ---------------------------------------------------------------------------
// 3. A third-party-issued W3C proof verifies through the bridge.
// ---------------------------------------------------------------------------

/// **The interop assertion.** The signature was produced by the W3C editors'
/// key over the W3C `hashData`; nothing in this repo signed it. It must verify
/// through the bridge for both in-scope suites, which is the property the
/// self-signed in-module tests structurally cannot witness.
#[test]
fn w3c_issued_proofs_verify_through_the_bridge() {
    let cred = triples(CREDENTIAL_NQ);
    for v in VECTORS {
        verify_source_proof(
            &cred,
            &triples(v.proof_config_nq),
            v.suite,
            &multibase_public_key(v),
            &proof_value_signature(v),
        )
        .unwrap_or_else(|e| panic!("published {} proof must verify: {}", v.label, e));
    }
}

/// Same vectors through the token entry point a caller reaches from a VC's
/// verbatim `proof.cryptosuite` value.
#[test]
fn w3c_issued_proofs_verify_by_cryptosuite_token() {
    let cred = triples(CREDENTIAL_NQ);
    for v in VECTORS {
        verify_source_proof_by_token(
            &cred,
            &triples(v.proof_config_nq),
            v.suite.token(),
            &multibase_public_key(v),
            &proof_value_signature(v),
        )
        .unwrap_or_else(|e| {
            panic!("published {} proof must verify by token: {}", v.label, e)
        });
    }
}

/// End to end: a published W3C credential goes through verify → re-commit →
/// provenance, and the registry entry records the source suite verbatim.
#[test]
fn w3c_vectors_ingest_and_record_source_cryptosuite() {
    let cred = triples(CREDENTIAL_NQ);
    let document = NamedNode::new("urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33").unwrap();
    let salt = salt_from_bytes(&[3u8; 32]);
    for v in VECTORS {
        let ingested = ingest_verified_vc(
            document.clone(),
            &cred,
            &triples(v.proof_config_nq),
            v.suite,
            &multibase_public_key(v),
            &proof_value_signature(v),
            salt,
        )
        .unwrap_or_else(|e| panic!("published {} VC must ingest: {}", v.label, e));

        assert_eq!(ingested.source_cryptosuite, v.suite);
        assert_eq!(ingested.commitment.salt, salt);
        let entry = ingested.registry_entry();
        assert_eq!(entry.document, document);
        assert_eq!(entry.commitment, ingested.commitment.commitment);
        assert_eq!(entry.source_cryptosuite.as_deref(), Some(v.suite.token()));
    }
    // Both suites commit the SAME credential graph under the SAME salt, so the
    // re-commitment is a function of the content alone — the source suite is
    // provenance, not an input to `C(G)`.
    let commit_of = |v: &Vector| {
        ingest_verified_vc(
            document.clone(),
            &cred,
            &triples(v.proof_config_nq),
            v.suite,
            &multibase_public_key(v),
            &proof_value_signature(v),
            salt,
        )
        .unwrap()
        .commitment
        .commitment
    };
    assert_eq!(
        commit_of(&EDDSA_RDFC_2022),
        commit_of(&ECDSA_RDFC_2019_P256),
        "C(G) must depend on the credential graph, not the source suite"
    );
    assert_eq!(
        commit_of(&ECDSA_RDFC_2019_P256),
        commit_of(&ECDSA_RDFC_2019_P384),
        "C(G) must depend on the credential graph, not the source curve profile"
    );
}

// ---------------------------------------------------------------------------
// 4. Mutations of the published vectors must go red.
// ---------------------------------------------------------------------------

/// Flipping a single bit of the published signature must reject. Without this
/// the positive assertions above could pass against a verifier that accepted
/// anything.
#[test]
fn single_bit_flip_in_published_signature_fails_closed() {
    let cred = triples(CREDENTIAL_NQ);
    for v in VECTORS {
        let mut sig = proof_value_signature(v);
        sig[0] ^= 0x01;
        assert!(
            matches!(
                verify_source_proof(
                    &cred,
                    &triples(v.proof_config_nq),
                    v.suite,
                    &multibase_public_key(v),
                    &sig,
                ),
                Err(VcBridgeError::VerificationFailed)
            ),
            "{}: a one-bit signature mutation must not verify",
            v.label
        );
    }
}

/// Dropping one credential triple changes the canonical document, hence the
/// `hashData` — the published proof must no longer verify, and the bridge must
/// refuse to commit the mutated graph.
#[test]
fn mutated_credential_fails_closed_and_never_commits() {
    for v in VECTORS {
        let mut mutated = triples(CREDENTIAL_NQ);
        mutated.pop().expect("vector has triples");
        let cfg = triples(v.proof_config_nq);
        let pk = multibase_public_key(v);
        let sig = proof_value_signature(v);
        assert!(
            matches!(
                verify_source_proof(&mutated, &cfg, v.suite, &pk, &sig),
                Err(VcBridgeError::VerificationFailed)
            ),
            "{}: a mutated credential must not verify",
            v.label
        );
        assert!(
            ingest_verified_vc(
                NamedNode::new("urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33").unwrap(),
                &mutated,
                &cfg,
                v.suite,
                &pk,
                &sig,
                salt_from_bytes(&[3u8; 32]),
            )
            .is_err(),
            "{}: a mutated credential must never reach the commitment pipeline",
            v.label
        );
    }
}

/// Substituting the other suite's proof config (same credential, same key,
/// same signature) changes the `hashData` and must reject — the proof is bound
/// to the proof options it was created over.
#[test]
fn swapped_proof_config_fails_closed() {
    let cred = triples(CREDENTIAL_NQ);
    for (v, other) in [
        (&EDDSA_RDFC_2022, ECDSA_PROOF_CONFIG_NQ),
        (&ECDSA_RDFC_2019_P256, EDDSA_PROOF_CONFIG_NQ),
        // Same cryptosuite token, different curve profile: the P-384 proof is
        // bound to the proof options naming ITS verification method.
        (&ECDSA_RDFC_2019_P384, ECDSA_PROOF_CONFIG_NQ),
    ] {
        assert!(
            matches!(
                verify_source_proof(
                    &cred,
                    &triples(other),
                    v.suite,
                    &multibase_public_key(v),
                    &proof_value_signature(v),
                ),
                Err(VcBridgeError::VerificationFailed)
            ),
            "{}: a proof must not verify under a foreign proof config",
            v.label
        );
    }
}

/// Suite confusion: each vector's key and signature, presented under the OTHER
/// suite, must be rejected — never silently accepted, and never a panic.
#[test]
fn cross_suite_key_and_signature_fail_closed() {
    let cred = triples(CREDENTIAL_NQ);
    for (v, wrong_suite) in [
        (&EDDSA_RDFC_2022, VcCryptosuite::EcdsaRdfc2019),
        (&ECDSA_RDFC_2019_P256, VcCryptosuite::EddsaRdfc2022),
        (&ECDSA_RDFC_2019_P384, VcCryptosuite::EddsaRdfc2022),
    ] {
        let err = verify_source_proof(
            &cred,
            &triples(v.proof_config_nq),
            wrong_suite,
            &multibase_public_key(v),
            &proof_value_signature(v),
        )
        .expect_err("cross-suite verification must fail closed");
        assert!(
            matches!(
                err,
                VcBridgeError::MalformedPublicKey
                    | VcBridgeError::MalformedSignature
                    | VcBridgeError::VerificationFailed
            ),
            "{} under {}: unexpected error {:?}",
            v.label,
            wrong_suite.token(),
            err
        );
    }
}

/// **Curve-profile confusion** (sq-txg1y): the two `ecdsa-rdfc-2019` vectors are
/// indistinguishable by cryptosuite token, so this is the pairing a
/// profile-blind implementation would wave through. Each published key with the
/// OTHER profile's published signature must fail closed — and it must fail on the
/// signature width, before any curve arithmetic, since 64 ≠ 96 bytes.
#[test]
fn ecdsa_cross_profile_key_and_signature_fail_closed() {
    let cred = triples(CREDENTIAL_NQ);
    for (v, other) in [
        (&ECDSA_RDFC_2019_P256, &ECDSA_RDFC_2019_P384),
        (&ECDSA_RDFC_2019_P384, &ECDSA_RDFC_2019_P256),
    ] {
        let err = verify_source_proof(
            &cred,
            &triples(v.proof_config_nq),
            VcCryptosuite::EcdsaRdfc2019,
            &multibase_public_key(v),
            &proof_value_signature(other),
        )
        .expect_err("cross-profile verification must fail closed");
        assert!(
            matches!(err, VcBridgeError::MalformedSignature),
            "{} key with {} signature: expected MalformedSignature, got {:?}",
            v.label,
            other.label,
            err
        );
    }
}
