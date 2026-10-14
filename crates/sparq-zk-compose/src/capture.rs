// sq-1s2.3 (FL1 follow-up): package NATIVE per-circuit proofs into the
// browser-shippable captured ProofManifest for /showcase/zk-car-hire.
//! Browser-shippable **captured** ProofManifest packaging (sq-1s2.3 / FL1 follow-up).
//!
//! The `/showcase/zk-car-hire` flagship proves the age-gate FILTER live in the
//! browser, and offers a *fallback* that fetches a pre-captured manifest and runs a
//! real `bb.js verifyProof` over its bundled sub-proof in-tab. Today that fallback
//! bundles ONLY the age-gate (`site/scripts/capture-zk-manifest.mjs`), because the
//! other family members (scan / join_eq / hidden_issuer / revoke_unset / holder_pok)
//! need NATIVE witness generation — Poseidon commitments, Schnorr signatures, Merkle
//! roots — that is not feasible in JS.
//!
//! This module is the crate-side seam that lets the fuller manifest be assembled: it
//! takes the [`crate::driver::ProofArtifacts`] a [`crate::driver::CircuitProver`]
//! produces for each member and packages them into the exact JSON shape the site's
//! fallback consumes — the raw UltraHonk `proof` bytes as a JSON-portable `number[]`,
//! and bb's flat `public_inputs` blob split into the `0x`-prefixed 32-byte
//! big-endian field-hex words bb.js reports as `publicInputs: string[]`.
//!
//! # Browser re-verify caveat (target flavour)
//! bb.js re-verifies under a chosen `verifierTarget` (the car-hire page uses `evm`,
//! the keccak-oracle flavour). A captured sub-proof re-verifies in-tab ONLY if it was
//! proved under the matching bb flavour. The default [`crate::driver::CircuitProver`]
//! proves with the `noir-recursive` target; capturing for the browser fallback
//! requires proving with the `evm`/keccak flavour. Selecting that native proving
//! flavour is a separate driver concern (tracked as a follow-up); this module only
//! packages whatever artifacts it is handed, and asserts nothing about which flavour
//! produced them.
//!
//! # HONESTY (privacy-claims gate)
//! Research-grade, NOT externally audited (bead `sq-qhy4`). A bundled sub-proof
//! re-verified in-tab demonstrates only that it is a valid UltraHonk proof of THAT
//! circuit against THOSE public inputs — it does NOT establish that the
//! cross-credential COMPOSITION is sound (the composition verifier is NOT-yet-sound,
//! `sq-qhy4`). No soundness or privacy property is asserted as achieved. See
//! [`CAR_HIRE_CAPTURE_NOTE`], which is baked into every assembled manifest.

use crate::driver::ProofArtifacts;
use serde::Serialize;

/// The honest, load-bearing note baked into every captured car-hire manifest — the
/// privacy-claims-gate caveat the site renders / ships. It must state that the estate
/// is research-grade + NOT externally audited (`sq-qhy4`) and that an in-tab verify of
/// a bundled sub-proof does NOT establish composition soundness.
pub const CAR_HIRE_CAPTURE_NOTE: &str = "Research-grade, NOT externally audited (bead sq-qhy4). \
Native per-circuit UltraHonk proofs, captured via sparq-zk-compose CircuitProver; each bundled \
sub-proof is a real transcript. Re-verifying a sub-proof in-tab proves only that it is a valid \
proof of THAT circuit against THOSE public inputs — NOT that the cross-credential composition is \
sound (the composition verifier is NOT-yet-sound, sq-qhy4). No soundness or privacy property is \
asserted as achieved.";

/// Error packaging a native proof into the captured shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// bb's `public_inputs` blob is not a whole number of 32-byte field words, so it
    /// cannot be split into `publicInputs` hex elements (fail-closed — a malformed
    /// blob is never silently truncated / zero-padded).
    MalformedPublicInputs { len: usize },
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::MalformedPublicInputs { len } => write!(
                f,
                "public_inputs blob length {len} is not a multiple of 32 (one field word)"
            ),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Split a bb `public_inputs` blob into the `0x`-prefixed, lowercase, 64-nibble
/// big-endian field-hex words bb.js reports as `publicInputs: string[]`. Each 32-byte
/// word is one public input (the fixed-width layout `field_to_be_bytes_32` mirrors and
/// the verifier's reconstruction relies on). Fails closed if `blob` is not a whole
/// number of 32-byte words.
pub fn public_inputs_to_hex(blob: &[u8]) -> Result<Vec<String>, CaptureError> {
    if !blob.len().is_multiple_of(32) {
        return Err(CaptureError::MalformedPublicInputs { len: blob.len() });
    }
    Ok(blob.chunks_exact(32).map(hex_word).collect())
}

/// `0x` + 64 lowercase nibbles of one 32-byte big-endian field word — the same
/// representation as `sparq_zk::field::field_to_hex`, so a captured public input
/// round-trips against the registry / manifest hex form verbatim.
fn hex_word(word: &[u8]) -> String {
    let mut s = String::with_capacity(2 + 64);
    s.push_str("0x");
    for b in word {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// One captured per-circuit sub-proof, in the exact JSON shape the site fallback
/// (`site/src/lib/zk-prover.ts` `CapturedSubProof`) consumes: the member package id,
/// a plain-language relation, the raw UltraHonk `proof` bytes (JSON-portable
/// `number[]`), and the `publicInputs` as `0x`-field-hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapturedSubProof {
    /// The circuit-family package id (e.g. `filter_int_d2`, `scan_k1_n16_r4`).
    pub member: String,
    /// A plain-language description of what this sub-proof proves.
    pub relation: String,
    /// The UltraHonk proof bytes, verbatim (rehydrated to a `Uint8Array` in-tab).
    pub proof: Vec<u8>,
    /// The public inputs as `0x`-prefixed 32-byte big-endian field hex (bb.js's
    /// `publicInputs` element form).
    #[serde(rename = "publicInputs")]
    pub public_inputs: Vec<String>,
}

impl CapturedSubProof {
    /// Package a natively-proved member into the browser-shippable shape. The proof
    /// bytes are carried verbatim; the bb `public_inputs` blob is split into
    /// `0x`-field-hex words. `art.vk` is deliberately NOT bundled — the site
    /// re-verifies against the circuit it already ships, and a prover-supplied vk is
    /// never a trust anchor (audit #2).
    pub fn from_artifacts(
        member: impl Into<String>,
        relation: impl Into<String>,
        art: &ProofArtifacts,
    ) -> Result<Self, CaptureError> {
        Ok(CapturedSubProof {
            member: member.into(),
            relation: relation.into(),
            proof: art.proof.clone(),
            public_inputs: public_inputs_to_hex(&art.public_inputs)?,
        })
    }
}

/// The fuller captured car-hire manifest: an honest note plus every natively-captured
/// per-circuit sub-proof, serialized to the JSON the site ships at
/// `site/public/zk/car-hire-manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapturedCarHireManifest {
    /// The manifest type urn, mirroring the age-gate fixture.
    #[serde(rename = "type")]
    pub type_: String,
    /// The honest, research-grade caveat ([`CAR_HIRE_CAPTURE_NOTE`]).
    pub note: String,
    /// ISO date the proofs were captured. The caller stamps this — this crate never
    /// reads the clock, so `to_pretty_json` is reproducible for a fixed input.
    #[serde(rename = "capturedAt")]
    pub captured_at: String,
    /// The bundled per-circuit sub-proofs — MORE than just the age-gate.
    #[serde(rename = "subProofs")]
    pub sub_proofs: Vec<CapturedSubProof>,
}

impl CapturedCarHireManifest {
    /// Assemble the manifest from the natively-captured sub-proofs, baking in the
    /// honest [`CAR_HIRE_CAPTURE_NOTE`] and the fixed manifest type. `captured_at` is
    /// a caller-supplied ISO date (kept out of this crate so the output is
    /// reproducible / does not depend on the clock).
    pub fn new(captured_at: impl Into<String>, sub_proofs: Vec<CapturedSubProof>) -> Self {
        CapturedCarHireManifest {
            type_: "urn:sparq:zk:ProofManifest".to_string(),
            note: CAR_HIRE_CAPTURE_NOTE.to_string(),
            captured_at: captured_at.into(),
            sub_proofs,
        }
    }

    /// Serialize to the pretty JSON the site ships. The `proof` byte arrays are the
    /// bulky part; callers that want the age-gate fixture's one-line-proof formatting
    /// can post-process, but the semantics are identical.
    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("captured manifest is serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_zk::field::{field_to_be_bytes_32, field_to_hex, Fr};

    /// A `public_inputs` blob is split word-by-word into the SAME hex form as
    /// `sparq_zk::field::field_to_hex` — so a captured public input matches the
    /// manifest / registry hex representation of the field element verbatim. This is
    /// the load-bearing correctness property: the browser compares these against the
    /// proof's committed public inputs.
    #[test]
    fn public_inputs_split_matches_field_to_hex() {
        let fields: Vec<Fr> = [1u64, 25, 0, 0x132f_a587].into_iter().map(Fr::from).collect();
        let mut blob = Vec::new();
        for f in &fields {
            blob.extend_from_slice(&field_to_be_bytes_32(f));
        }
        let hex = public_inputs_to_hex(&blob).expect("well-formed blob");
        assert_eq!(hex.len(), fields.len());
        for (h, f) in hex.iter().zip(&fields) {
            assert_eq!(h, &field_to_hex(f), "word hex must equal field_to_hex");
            assert!(h.starts_with("0x") && h.len() == 66, "0x + 64 nibbles");
        }
    }

    /// A blob whose length is not a whole number of 32-byte words is REJECTED (fail
    /// closed) — never silently truncated or zero-padded into a wrong public input.
    #[test]
    fn public_inputs_reject_ragged_blob() {
        assert_eq!(
            public_inputs_to_hex(&[0u8; 33]),
            Err(CaptureError::MalformedPublicInputs { len: 33 }),
        );
        // Empty blob is a whole (zero) number of words — a proof with no public inputs.
        assert_eq!(public_inputs_to_hex(&[]), Ok(vec![]));
    }

    /// `from_artifacts` carries the proof bytes verbatim and splits the public inputs;
    /// it does NOT bundle the vk (audit #2 — the verifier never trusts a prover vk).
    #[test]
    fn from_artifacts_packages_proof_and_public_inputs() {
        let art = ProofArtifacts {
            proof: vec![9, 8, 7, 6],
            public_inputs: field_to_be_bytes_32(&Fr::from(42u64)).to_vec(),
            vk: vec![1, 2, 3], // must not leak into the captured shape
        };
        let sp = CapturedSubProof::from_artifacts("filter_int_d2", "age >= 25", &art)
            .expect("packages");
        assert_eq!(sp.member, "filter_int_d2");
        assert_eq!(sp.proof, vec![9, 8, 7, 6], "proof bytes verbatim");
        assert_eq!(sp.public_inputs, vec![field_to_hex(&Fr::from(42u64))]);
        let json = serde_json::to_string(&sp).unwrap();
        assert!(!json.contains("\"vk\""), "the prover vk is never bundled");
    }

    /// A malformed-PI member surfaces the fail-closed error through `from_artifacts`.
    #[test]
    fn from_artifacts_rejects_malformed_public_inputs() {
        let art = ProofArtifacts { proof: vec![1], public_inputs: vec![0u8; 5], vk: vec![] };
        assert_eq!(
            CapturedSubProof::from_artifacts("scan_k1_n16_r4", "scan", &art),
            Err(CaptureError::MalformedPublicInputs { len: 5 }),
        );
    }

    /// The assembled manifest carries MORE than the age-gate, the honest sq-qhy4
    /// caveat, and the exact camelCase JSON keys the site fallback consumes.
    #[test]
    fn manifest_assembles_multiple_members_with_honest_note() {
        let age = CapturedSubProof {
            member: "filter_int_d2".into(),
            relation: "age >= 25 over a hidden integer age".into(),
            proof: vec![1, 2, 3],
            public_inputs: vec![field_to_hex(&Fr::from(1u64))],
        };
        let scan = CapturedSubProof {
            member: "scan_k1_n16_r4".into(),
            relation: "BGP scan over a committed credential graph".into(),
            proof: vec![4, 5, 6],
            public_inputs: vec![field_to_hex(&Fr::from(2u64))],
        };
        let m = CapturedCarHireManifest::new("2026-07-19", vec![age, scan]);
        assert_eq!(m.sub_proofs.len(), 2, "more than just the age-gate");

        // Honest note (privacy-claims gate): research-grade + sq-qhy4 + no soundness.
        assert!(m.note.contains("sq-qhy4"));
        assert!(m.note.contains("NOT externally audited"));
        assert!(m.note.contains("NOT that the cross-credential composition is sound"));

        let json = m.to_pretty_json();
        assert!(json.contains("\"type\": \"urn:sparq:zk:ProofManifest\""));
        assert!(json.contains("\"subProofs\""));
        assert!(json.contains("\"publicInputs\""));
        assert!(json.contains("\"capturedAt\": \"2026-07-19\""));
        assert!(json.contains("\"proof\""));
        // Round-trips as valid JSON.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["subProofs"].as_array().unwrap().len(), 2);
        assert_eq!(v["subProofs"][0]["proof"].as_array().unwrap().len(), 3);
    }

    // ---- Browser-consumer CONTRACT ----------------------------------------------------
    // The mirror below is a faithful, field-for-field copy of the TypeScript interfaces
    // the browser fallback deserializes into (`CapturedManifest` / `CapturedSubProof` in
    // `site/src/lib/zk-prover.ts`). `deny_unknown_fields` + `deny_unknown_fields` on both
    // makes deserialization FAIL if the crate ever emits an extra/renamed key or drops
    // one — i.e. this is a real "deserialize through the consumer schema" contract, not a
    // substring check. If you change either side, change BOTH (and the fixture + the
    // `capture-zk-manifest.mjs` generator) in the same PR.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct BrowserSubProof {
        #[allow(dead_code)]
        member: String,
        #[allow(dead_code)]
        relation: String,
        #[allow(dead_code)]
        proof: Vec<u8>,
        #[serde(rename = "publicInputs")]
        #[allow(dead_code)]
        public_inputs: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct BrowserManifest {
        #[serde(rename = "type")]
        #[allow(dead_code)]
        type_: String,
        #[allow(dead_code)]
        note: String,
        #[serde(rename = "capturedAt")]
        #[allow(dead_code)]
        captured_at: String,
        #[serde(rename = "subProofs")]
        sub_proofs: Vec<BrowserSubProof>,
    }

    /// The serialized manifest deserializes CLEANLY into a `deny_unknown_fields` mirror of
    /// the browser consumer's `CapturedManifest` — the exact schema `zk-prover.ts` reads.
    /// This is the contract the old substring test could not enforce: an extra, missing,
    /// or renamed key here is a hard deserialize error, catching browser-incompatibility
    /// at crate-test time instead of in the field.
    #[test]
    fn serialized_manifest_matches_browser_consumer_schema() {
        let sp = CapturedSubProof {
            member: "filter_int_d2".into(),
            relation: "age >= 25 over a hidden integer age".into(),
            proof: vec![1, 2, 3, 4],
            public_inputs: vec![field_to_hex(&Fr::from(25u64))],
        };
        let m = CapturedCarHireManifest::new("2026-07-19", vec![sp]);
        let json = m.to_pretty_json();

        let parsed: BrowserManifest =
            serde_json::from_str(&json).expect("crate output must deserialize as the browser schema");
        assert_eq!(parsed.sub_proofs.len(), 1);

        // And the old singular key the browser USED to require must NOT be emitted — that
        // was exactly the incompatibility (plural `subProofs` vs singular `subProof`).
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("subProof").is_none(), "must not emit the legacy singular key");
        assert!(v.get("circuit").is_none(), "the browser no longer reads `circuit`");
    }
}
