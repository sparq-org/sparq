//! Behavioral oracle assertions — `compile_*` ∘ `oracle_*` fail-closed (bead `sq-3bv7v`).
//!
//! `tests/determinism.rs` exercises the `GenParams`/`IntentRow` round-trip and one WAC
//! fail-closed fixture; this lane adds the BEHAVIORAL kill-assertions the mutation-honesty
//! epic (`sq-3dyje`) targets: per-model assertions over hand-built [`IntentRow`]s that a
//! mutant making any oracle ignore **effect** or **condition** — or making any compiler
//! flip allow/deny polarity — goes red.
//!
//! Three invariants, each asserted PER MODEL:
//! 1. **Fail-closed on no-match**: a request matching NO intent in a NON-EMPTY table
//!    returns [`Decision::Deny`] (wrong agent / wrong resource / insufficient mode).
//! 2. **Effect fidelity (anti-vacuity)**: a `Deny`-effect intent compiled by `compile_*`
//!    carries deny polarity (or is `Unsupported` for WAC) and the matching `oracle_*`
//!    returns `Deny`, never a spurious `Allow` — mirroring the determinism.rs WAC
//!    negative fixture for ACP + ODRL too.
//! 3. **Condition gating is load-bearing**: an `Allow` intent with a satisfied condition
//!    returns `Allow` and with an unsatisfied condition returns `Deny` (ODRL); WAC/ACP
//!    skip conditioned rows entirely (deny, `Unsupported` compile) rather than silently
//!    ignoring the condition.
//!
//! Temporal fixtures are pinned against the crate's documented clock-free benchmark
//! evaluation instant `2026-07-06T00:00:00Z` (`BENCH_EVAL_INSTANT` in `src/oracle.rs`);
//! the count-condition flip is instant-independent.
//!

use sparq_acbench::{
    compile_acp, compile_odrl, compile_wac, oracle_acp, oracle_odrl, oracle_wac, AcModel,
    AccessMode, Audience, Condition, Decision, Effect, Expressibility, IntentRow, Request, Scope,
};

// ── fixtures ─────────────────────────────────────────────────────────────────────────

const ALICE: &str = "https://example.org/alice";
const BOB: &str = "https://example.org/bob";
const RES: &str = "https://example.org/data/res1";
const OTHER_RES: &str = "https://example.org/data/res2";

/// An `Allow` intent granting `agent` read-only access to `resource`.
fn allow_read(agent: &str, resource: &str) -> IntentRow {
    IntentRow {
        audience: Audience::Agent(agent.to_string()),
        scope: Scope::Resource,
        mode: AccessMode::read_only(),
        condition: Condition::None,
        effect: Effect::Allow,
        resource_uri: resource.to_string(),
    }
}

/// A read-only request from `agent` for `resource`.
fn read_request(agent: &str, resource: &str) -> Request {
    Request {
        agent: agent.to_string(),
        client: None,
        resource: resource.to_string(),
        mode: AccessMode::read_only(),
    }
}

/// Evaluate `request` against `intents` under every model, returning `(model, decision)`
/// so a per-model regression names the offending oracle in the assertion message.
fn all_models(request: &Request, intents: &[IntentRow]) -> Vec<(AcModel, Decision)> {
    vec![
        (AcModel::Wac, oracle_wac(request, intents)),
        (AcModel::Acp, oracle_acp(request, intents)),
        (AcModel::Odrl, oracle_odrl(request, intents)),
    ]
}

// ── 1. fail-closed on no-match (cross-model, NON-empty table) ────────────────────────

/// A request matching NO intent returns `Deny` under every model — with a non-empty
/// intent table, so a mutant that returns `Allow` whenever ANY intent exists (rather
/// than a MATCHING one) is killed. Three miss shapes: wrong agent, wrong resource,
/// insufficient mode.
#[test]
fn all_oracles_fail_closed_on_no_match() {
    // The table grants: Alice may read RES. Nothing else.
    let intents = vec![allow_read(ALICE, RES)];

    // Miss 1: wrong agent (Bob asks for Alice's grant).
    let wrong_agent = read_request(BOB, RES);
    // Miss 2: wrong resource (Alice asks for a resource with no intent).
    let wrong_resource = read_request(ALICE, OTHER_RES);
    // Miss 3: insufficient mode (Alice asks for write; only read is granted).
    let wrong_mode = Request {
        mode: AccessMode {
            read: false,
            write: true,
            control: false,
        },
        ..read_request(ALICE, RES)
    };

    for (label, request) in [
        ("wrong agent", &wrong_agent),
        ("wrong resource", &wrong_resource),
        ("insufficient mode", &wrong_mode),
    ] {
        for (model, decision) in all_models(request, &intents) {
            assert_eq!(
                decision,
                Decision::Deny,
                "{model:?} oracle must fail closed on {label} (no matching intent)"
            );
        }
    }

    // Sanity anchor (kills an oracle stubbed to constant Deny): the exactly-matching
    // request IS allowed under every model, so the misses above are real misses.
    for (model, decision) in all_models(&read_request(ALICE, RES), &intents) {
        assert_eq!(
            decision,
            Decision::Allow,
            "{model:?} oracle must allow the exactly-matching request (anchor)"
        );
    }
}

// ── 2. effect fidelity: Deny-effect intent never yields Allow ────────────────────────

/// A `Deny`-effect intent, compiled by each `compile_*` and evaluated by the matching
/// `oracle_*`, returns `Deny` — never a spurious `Allow`. Kills an effect-ignoring
/// oracle mutant (e.g. `oracle_acp` always returning `Allow` on a matching row) and a
/// compiler mutant that flips deny polarity to allow.
#[test]
fn deny_effect_never_allows_acp_odrl() {
    let deny_row = IntentRow {
        effect: Effect::Deny,
        ..allow_read(ALICE, RES)
    };
    // The request matches the deny row exactly on agent + resource + mode, so ONLY the
    // effect stands between the oracle and a spurious Allow.
    let request = read_request(ALICE, RES);

    // ACP: compile carries acp:deny (not acp:allow) …
    let acp = compile_acp(&deny_row, &[]);
    assert!(
        acp.nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/solid/acp#deny>")),
        "compile_acp(Deny) must emit acp:deny triples"
    );
    assert!(
        !acp.nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/solid/acp#allow>")),
        "compile_acp(Deny) must not emit acp:allow (polarity flip)"
    );
    // … and the oracle denies.
    assert_eq!(
        oracle_acp(&request, std::slice::from_ref(&deny_row)),
        Decision::Deny,
        "ACP oracle: a matching Deny intent must deny, never spuriously allow"
    );

    // ODRL: compile carries odrl:prohibition (not odrl:permission) …
    let odrl = compile_odrl(&deny_row);
    assert!(
        odrl.nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/odrl/2/prohibition>")),
        "compile_odrl(Deny) must emit an odrl:prohibition rule"
    );
    assert!(
        !odrl
            .nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/odrl/2/permission>")),
        "compile_odrl(Deny) must not emit odrl:permission (polarity flip)"
    );
    // … and the oracle denies (Prohibition without Permission → fail-closed).
    assert_eq!(
        oracle_odrl(&request, std::slice::from_ref(&deny_row)),
        Decision::Deny,
        "ODRL oracle: a matching Prohibition without Permission must deny"
    );

    // WAC (cross-model completeness): deny is inexpressible — Unsupported, no triples,
    // and the oracle skips the row (fail-closed Deny), matching determinism.rs.
    let wac = compile_wac(&deny_row);
    assert_eq!(
        wac.expressibility,
        Expressibility::Unsupported,
        "compile_wac(Deny) must classify as Unsupported (WAC has no deny)"
    );
    assert!(
        wac.nquads.is_empty(),
        "compile_wac(Deny) must emit no triples"
    );
    assert_eq!(
        oracle_wac(&request, std::slice::from_ref(&deny_row)),
        Decision::Deny,
        "WAC oracle: a Deny intent must not produce Allow (anti-vacuity)"
    );
}

/// Allow-polarity mirror of the deny checks: an `Allow`-effect intent compiles with
/// allow polarity in every model. Together with `deny_effect_never_allows_acp_odrl`
/// this kills a compiler mutant that hardcodes either effect predicate.
#[test]
fn allow_effect_compiles_with_allow_polarity() {
    let allow_row = allow_read(ALICE, RES);

    let acp = compile_acp(&allow_row, &[]);
    assert!(
        acp.nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/solid/acp#allow>")),
        "compile_acp(Allow) must emit acp:allow triples"
    );
    assert!(
        !acp.nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/solid/acp#deny>")),
        "compile_acp(Allow) must not emit acp:deny (polarity flip)"
    );

    let odrl = compile_odrl(&allow_row);
    assert!(
        odrl.nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/odrl/2/permission>")),
        "compile_odrl(Allow) must emit an odrl:permission rule"
    );
    assert!(
        !odrl
            .nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/odrl/2/prohibition>")),
        "compile_odrl(Allow) must not emit odrl:prohibition (polarity flip)"
    );

    let wac = compile_wac(&allow_row);
    assert_eq!(wac.expressibility, Expressibility::Native);
    assert!(
        wac.nquads
            .iter()
            .any(|t| t.contains("<http://www.w3.org/ns/auth/acl#Read>")),
        "compile_wac(Allow, read) must emit an acl:Read mode triple"
    );
}

/// ODRL documented precedence: when BOTH a matching Permission and a matching
/// Prohibition exist, Permission wins. Pins the direction of the override so a mutant
/// inverting the precedence (prohibition-wins) goes red — and, paired with the
/// deny-only case above, distinguishes "Permission overrides" from "ignore effect".
#[test]
fn odrl_permission_overrides_matching_prohibition() {
    let allow_row = allow_read(ALICE, RES);
    let deny_row = IntentRow {
        effect: Effect::Deny,
        ..allow_read(ALICE, RES)
    };
    let request = read_request(ALICE, RES);
    assert_eq!(
        oracle_odrl(&request, &[deny_row, allow_row]),
        Decision::Allow,
        "ODRL oracle: Permission must override a matching Prohibition (documented precedence)"
    );
}

/// `AllExcept` matches agents outside its exclusion list, for both effects and both
/// policy models that support deny. The excluded-agent deny cases deliberately remain
/// fail-closed: exclusion means that row does not match, not that access is granted.
///
/// The `Allow` assertions are the mutation witness: inverting the exclusion membership
/// check swaps the two decisions and makes this test fail for both ACP and ODRL.
#[test]
fn all_except_membership_controls_acp_and_odrl_effects() {
    // sq-n489k — behavioral oracle coverage for AllExcept membership polarity.
    const EXCLUDED_AGENT: &str = "excluded-agent";

    let non_excluded_request = read_request(ALICE, RES);
    let excluded_request = read_request(EXCLUDED_AGENT, RES);

    for (effect, expected_non_excluded) in [
        (Effect::Deny, Decision::Deny),
        (Effect::Allow, Decision::Allow),
    ] {
        let row = IntentRow {
            audience: Audience::AllExcept(vec![EXCLUDED_AGENT.to_string()]),
            effect: effect.clone(),
            ..allow_read(ALICE, RES)
        };

        for (model, non_excluded_decision, excluded_decision) in [
            (
                AcModel::Acp,
                oracle_acp(&non_excluded_request, std::slice::from_ref(&row)),
                oracle_acp(&excluded_request, std::slice::from_ref(&row)),
            ),
            (
                AcModel::Odrl,
                oracle_odrl(&non_excluded_request, std::slice::from_ref(&row)),
                oracle_odrl(&excluded_request, std::slice::from_ref(&row)),
            ),
        ] {
            assert_eq!(
                non_excluded_decision, expected_non_excluded,
                "{model:?} oracle: a non-excluded agent must match an AllExcept {effect:?} row"
            );
            assert_eq!(
                excluded_decision,
                Decision::Deny,
                "{model:?} oracle: an excluded agent must not match an AllExcept {effect:?} row"
            );
        }
    }
}

// ── 3. condition gating flips the decision ────────────────────────────────────────────

/// Condition gating is load-bearing, not ignored: the SAME `Allow` intent flips
/// `Allow` ⇄ `Deny` purely on whether its condition is satisfied.
///
/// - ODRL (native conditions): `Count(1)` admits, `Count(0)` denies
///   (instant-independent); an in-window temporal admits, an expired one denies
///   (against the pinned `2026-07-06T00:00:00Z` instant); a compound `And` denies
///   when either arm is unsatisfied.
/// - WAC/ACP (no conditions in the model): a conditioned `Allow` row is SKIPPED
///   (fail-closed `Deny`) and compiles as `Unsupported` with no triples — a mutant
///   that "helpfully" ignores the condition and allows goes red.
#[test]
fn condition_gating_flips_decision() {
    let request = read_request(ALICE, RES);

    // ODRL: count condition — instant-independent flip.
    let count_ok = IntentRow {
        condition: Condition::Count(1),
        ..allow_read(ALICE, RES)
    };
    let count_zero = IntentRow {
        condition: Condition::Count(0),
        ..allow_read(ALICE, RES)
    };
    assert_eq!(
        oracle_odrl(&request, std::slice::from_ref(&count_ok)),
        Decision::Allow,
        "ODRL oracle: satisfied Count(1) condition must admit"
    );
    assert_eq!(
        oracle_odrl(&request, std::slice::from_ref(&count_zero)),
        Decision::Deny,
        "ODRL oracle: unsatisfied Count(0) condition must deny (gating is load-bearing)"
    );

    // ODRL: temporal window against the pinned benchmark instant (2026-07-06T00:00:00Z).
    let in_window = IntentRow {
        condition: Condition::Temporal {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2027-01-01T00:00:00Z".to_string(),
        },
        ..allow_read(ALICE, RES)
    };
    let expired = IntentRow {
        condition: Condition::Temporal {
            start: "2020-01-01T00:00:00Z".to_string(),
            end: "2021-01-01T00:00:00Z".to_string(),
        },
        ..allow_read(ALICE, RES)
    };
    assert_eq!(
        oracle_odrl(&request, std::slice::from_ref(&in_window)),
        Decision::Allow,
        "ODRL oracle: in-window temporal condition must admit at the pinned instant"
    );
    assert_eq!(
        oracle_odrl(&request, std::slice::from_ref(&expired)),
        Decision::Deny,
        "ODRL oracle: expired temporal window must deny (retention/embargo ground truth)"
    );

    // ODRL: compound And — one unsatisfied arm denies the conjunction.
    let and_one_bad = IntentRow {
        condition: Condition::And(
            Box::new(Condition::Count(1)),
            Box::new(Condition::Temporal {
                start: "2020-01-01T00:00:00Z".to_string(),
                end: "2021-01-01T00:00:00Z".to_string(),
            }),
        ),
        ..allow_read(ALICE, RES)
    };
    assert_eq!(
        oracle_odrl(&request, std::slice::from_ref(&and_one_bad)),
        Decision::Deny,
        "ODRL oracle: And with one unsatisfied arm must deny"
    );

    // WAC/ACP: conditions are inexpressible — a conditioned Allow row must be skipped
    // (Deny), never evaluated as if unconditioned. `count_ok` would admit if the
    // condition were silently dropped, so a condition-ignoring mutant flips these.
    assert_eq!(
        oracle_wac(&request, std::slice::from_ref(&count_ok)),
        Decision::Deny,
        "WAC oracle: a conditioned Allow row must be skipped, not silently un-conditioned"
    );
    assert_eq!(
        oracle_acp(&request, std::slice::from_ref(&count_ok)),
        Decision::Deny,
        "ACP oracle: a conditioned Allow row must be skipped, not silently un-conditioned"
    );
    for (model, compiled) in [
        (AcModel::Wac, compile_wac(&count_ok)),
        (AcModel::Acp, compile_acp(&count_ok, &[])),
    ] {
        assert_eq!(
            compiled.expressibility,
            Expressibility::Unsupported,
            "{model:?} compiler must classify a conditioned intent as Unsupported"
        );
        assert!(
            compiled.nquads.is_empty(),
            "{model:?} compiler must emit no triples for a conditioned intent"
        );
    }
}
