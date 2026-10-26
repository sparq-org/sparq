// sq-1s2.3 (FL1 follow-up): the CircuitProver -> capture seam for the
// fuller /showcase/zk-car-hire captured ProofManifest.
//
// ## NOT a soundness claim (load-bearing honesty)
// This exercises the PACKAGING of a native per-circuit proof into the
// browser-shippable captured shape; it makes NO claim the composition verifier is
// sound (NOT-yet-sound, sq-qhy4). The full-family fixture regeneration (all of
// scan / join_eq / hidden_issuer / revoke_unset / holder_pok, in the browser
// `evm`/keccak flavour) + the site-side multi-sub-proof verify + honest rendering
// are follow-ups that need the nargo/bb toolchain and the `site/` scope.
//
// The real prove -> capture round-trip is `#[ignore]`-free but LIVE, mirroring
// `e2e.rs::full_prove_verify_filter_int_d1`. A runtime early return is NOT coverage of
// the real seam, so a missing nargo/bb is a HARD FAILURE: the shared gate in
// `tests/support/mod.rs` panics unless the lane declares itself toolchain-free with
// `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN=1`, in which case the skip is printed and named.

use sparq_zk_compose::build::{build_filter_int, encode_int_literal};
use sparq_zk_compose::capture::{CapturedCarHireManifest, CapturedSubProof};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::manifest::{CircuitId, FieldHex, FilterOp};
use sparq_zk_compose::toml::prover_toml_for;

mod support;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq_zk_capture_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A REAL native filter_int_d1 proof packages into a captured sub-proof whose proof
/// bytes are non-empty and whose `publicInputs` split cleanly into `0x`-field-hex
/// words — the CircuitProver -> `capture` seam the fuller manifest is assembled
/// through. LIVE: it needs nargo+bb and FAILS unless the lane declares itself
/// toolchain-free (`support::ALLOW_MISSING_ENV`); the pure packaging correctness is
/// covered toolchain-free in `capture.rs`'s unit tests.
#[test]
fn native_filter_proof_captures_into_manifest() {
    if !support::require_toolchain("native_filter_proof_captures_into_manifest", &["nargo", "bb"]) {
        return;
    }
    // 5 < 10 is true — the same minimal, known-satisfiable member the e2e suite proves.
    let operand_enc = encode_int_literal(5);
    let (filter, digits) = build_filter_int(operand_enc, 5, FilterOp::Lt, 10, true).unwrap();
    let (id, toml) =
        prover_toml_for(&filter, &FieldHex("0x2a".into()), &[], &[], &digits, None, None).unwrap();
    assert_eq!(id, CircuitId::FilterInt { d: 1 });

    let prover = CircuitProver::from_crate_root();
    let out = scratch("filter_d1");
    let art = prover.prove_in(&id, &toml, &out, "capture_d1").expect("prove succeeds");

    let sp = CapturedSubProof::from_artifacts(id.package().unwrap(), "5 < 10 over a hidden integer", &art)
        .expect("packages the native artifacts");
    assert!(!sp.proof.is_empty(), "the captured proof carries the real transcript bytes");
    assert!(
        !sp.public_inputs.is_empty()
            && sp
                .public_inputs
                .iter()
                .all(|h| h.starts_with("0x") && h.len() == 66),
        "public inputs split into 0x + 64-nibble field words"
    );

    let manifest = CapturedCarHireManifest::new("2026-07-19", vec![sp]);
    let json = manifest.to_pretty_json();
    assert!(json.contains("\"subProofs\""));
    assert!(json.contains("filter_int_d1"));
    // The honest caveat travels with the fixture.
    assert!(manifest.note.contains("sq-qhy4"));
    support::complete_required();
}
