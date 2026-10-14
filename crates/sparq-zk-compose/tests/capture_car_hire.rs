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
// The real prove -> capture round-trip is `#[ignore]`-free but TOOLCHAIN-GATED
// (skips cleanly without nargo + bb), mirroring `e2e.rs::full_prove_verify_filter_int_d1`.
// A runtime early return is NOT coverage of the real seam: set `SPARQ_ZK_REQUIRE_TOOLCHAIN=1`
// in a toolchain-equipped CI lane to turn a missing toolchain into a HARD FAILURE, so that
// lane provably exercises the native path (see `require_toolchain` below).

use sparq_zk_compose::build::{build_filter_int, encode_int_literal};
use sparq_zk_compose::capture::{CapturedCarHireManifest, CapturedSubProof};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::manifest::{CircuitId, FieldHex, FilterOp};
use sparq_zk_compose::toml::prover_toml_for;

fn toolchain_available() -> bool {
    fn on_path(tool: &str) -> bool {
        std::process::Command::new(tool)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    on_path("nargo") && on_path("bb")
}

/// When `SPARQ_ZK_REQUIRE_TOOLCHAIN` is set (to a non-empty value), a MISSING nargo/bb
/// toolchain is a HARD FAILURE rather than a clean skip — so a toolchain-equipped CI lane
/// provably EXERCISES the native `CircuitProver -> capture` seam instead of silently
/// early-returning and being reported as "passing". Default CI leaves it unset: the
/// lightweight synthetic packaging tests in `capture.rs` provide the toolchain-free cover,
/// and this native round-trip skips cleanly. See the module header + PR #3588 review r1.
const REQUIRE_TOOLCHAIN_ENV: &str = "SPARQ_ZK_REQUIRE_TOOLCHAIN";

fn require_toolchain() -> bool {
    std::env::var_os(REQUIRE_TOOLCHAIN_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq_zk_capture_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A REAL native filter_int_d1 proof packages into a captured sub-proof whose proof
/// bytes are non-empty and whose `publicInputs` split cleanly into `0x`-field-hex
/// words — the CircuitProver -> `capture` seam the fuller manifest is assembled
/// through. Toolchain-gated (skips without nargo/bb); the pure packaging correctness
/// is covered toolchain-free in `capture.rs`'s unit tests.
#[test]
fn native_filter_proof_captures_into_manifest() {
    if !toolchain_available() {
        assert!(
            !require_toolchain(),
            "{REQUIRE_TOOLCHAIN_ENV} is set, but nargo+bb are not both on PATH — the native \
             CircuitProver -> capture seam was NOT exercised. A toolchain-equipped CI lane must \
             fail here rather than pass on an early return; install the toolchain or unset the var."
        );
        eprintln!(
            "nargo/bb absent; skipping native capture round-trip \
             (set {REQUIRE_TOOLCHAIN_ENV}=1 to require it in a toolchain-equipped lane)"
        );
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

    let sp = CapturedSubProof::from_artifacts(id.package(), "5 < 10 over a hidden integer", &art)
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
}
