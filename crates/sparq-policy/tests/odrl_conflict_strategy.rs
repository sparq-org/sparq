//! ODRL `odrl:conflict` conflict-resolution-strategy admissibility. [OPUS-4.8] sq-ihqbl.
//!
//! The bridge implements exactly one ODRL conflict strategy — `odrl:prohibit`
//! (deny-overrides). [`conflict_admissibility`] is the fail-closed guard that decides
//! whether a policy's declared strategy is one the engine can faithfully honour: it
//! **refuses** (`Err`, a loud error) `odrl:perm`, an `odrl:invalid` policy that has a
//! detected conflict, and any unknown strategy IRI — rather than silently coercing them
//! into deny-overrides (an authorization-correctness hazard). These pin that contract,
//! and that the parser records the declared strategy in the model.
//!
//! Non-vacuous: the refusal cases parse to policies that — under the pre-fix lenient
//! behaviour (no `odrl:conflict` consultation at all) — would have been processed as
//! ordinary deny-overrides; here they are rejected outright.

use sparq_policy::{conflict_admissibility, parse_policy_str, ConflictStrategy};

/// A conflicting permission/prohibition pair (same action/target/assignee) — the shape
/// `detect_conflicts` flags as a `Certain` conflict — with the given `odrl:conflict`
/// clause spliced in (empty string = leave it unset).
fn conflicting_policy(conflict_clause: &str) -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  {conflict_clause}
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                     odrl:assignee <https://alice.ex/me> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                     odrl:assignee <https://alice.ex/me> ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

// ---------------------------------------------------------------------------
// Parsing: the declared strategy lands in the model.
// ---------------------------------------------------------------------------

#[test]
fn parses_each_conflict_term() {
    assert_eq!(
        conflicting_policy("odrl:conflict odrl:prohibit ;").conflict,
        Some(ConflictStrategy::Prohibit)
    );
    assert_eq!(
        conflicting_policy("odrl:conflict odrl:perm ;").conflict,
        Some(ConflictStrategy::Perm)
    );
    assert_eq!(
        conflicting_policy("odrl:conflict odrl:invalid ;").conflict,
        Some(ConflictStrategy::Invalid)
    );
    // Unset → None (the model records absence; the bridge defaults it to deny-overrides).
    assert_eq!(conflicting_policy("").conflict, None);
}

#[test]
fn parses_unknown_conflict_iri_verbatim() {
    let p = conflicting_policy("odrl:conflict <urn:custom:mediate> ;");
    assert_eq!(
        p.conflict,
        Some(ConflictStrategy::Unknown("urn:custom:mediate".to_owned())),
        "an unrecognised conflict term is preserved as Unknown(iri), not dropped",
    );
}

// ---------------------------------------------------------------------------
// Admissibility: the fail-closed decision.
// ---------------------------------------------------------------------------

/// `odrl:conflict odrl:prohibit` is exactly what the bridge implements → admissible,
/// even with a real conflict present (deny-overrides resolves it).
#[test]
fn prohibit_strategy_is_admissible() {
    assert!(conflict_admissibility(&conflicting_policy("odrl:conflict odrl:prohibit ;")).is_ok());
}

/// Unset `odrl:conflict` defaults to the implemented deny-overrides → admissible
/// (this is the bridge's core use case; refusing it would gut the feature).
#[test]
fn unset_conflict_defaults_admissible() {
    assert!(conflict_admissibility(&conflicting_policy("")).is_ok());
}

/// `odrl:conflict odrl:perm` (permissions override prohibitions) is unrepresentable by
/// the bridge's allow-minus-deny subtraction → REFUSED with a clear error.
#[test]
fn perm_strategy_is_refused() {
    let err = conflict_admissibility(&conflicting_policy("odrl:conflict odrl:perm ;"))
        .expect_err("odrl:perm must be refused, not silently enforced as deny-overrides");
    assert!(err.contains("perm"), "error names the strategy: {err}");
}

/// `odrl:conflict odrl:invalid` WITH a detected conflict → the policy is void as a whole,
/// which the bridge cannot represent → REFUSED.
#[test]
fn invalid_strategy_with_conflict_is_refused() {
    let err = conflict_admissibility(&conflicting_policy("odrl:conflict odrl:invalid ;"))
        .expect_err("odrl:invalid with a detected conflict must be refused");
    assert!(err.contains("invalid"), "error names the strategy: {err}");
}

/// `odrl:conflict odrl:invalid` with NO detected conflict (disjoint actions) → nothing to
/// void → admissible. Disjoint targets are not enough: asset membership evidence can
/// place one inside the other.
#[test]
fn invalid_strategy_without_conflict_is_admissible() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:conflict odrl:invalid ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:modify ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(p.conflict, Some(ConflictStrategy::Invalid));
    assert!(
        conflict_admissibility(&p).is_ok(),
        "an invalid policy with no detected conflict has nothing to void",
    );
}

/// An unknown `odrl:conflict` strategy IRI → REFUSED (the engine has no semantics for it).
#[test]
fn unknown_strategy_is_refused() {
    let err = conflict_admissibility(&conflicting_policy("odrl:conflict <urn:custom:mediate> ;"))
        .expect_err("an unknown conflict strategy must be refused, not ignored");
    assert!(err.contains("urn:custom:mediate"), "error names the offending IRI: {err}");
}

/// **Fail-closed on ambiguity (sq-ihqbl).** A graph asserting *two* distinct
/// `odrl:conflict` strategies — a benign `odrl:prohibit` that sorts BEFORE an unknown
/// `<urn:zzz:mediate>` — must be refused. Under the old "take the first-sorted `?c`"
/// behaviour the benign `odrl:prohibit` sorted first and the graph parsed as admissible,
/// silently masking the co-asserted unimplementable strategy. The parser now refuses the
/// ambiguous set outright.
#[test]
fn multiple_conflicting_strategies_refused_ambiguous() {
    let ttl = "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
        <urn:pol/p> a odrl:Set ;\n\
          odrl:conflict odrl:prohibit ;\n\
          odrl:conflict <urn:zzz:mediate> ;\n\
          odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .";
    let err = parse_policy_str(ttl, "turtle")
        .expect_err("a graph declaring multiple distinct conflict strategies must be refused");
    assert!(
        err.contains("multiple") && err.contains("conflict"),
        "error explains the ambiguous conflict set: {err}",
    );
}

/// An empty policy (no rules) declaring no conflict is trivially admissible.
#[test]
fn empty_policy_is_admissible() {
    let p = parse_policy_str(
        "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n<urn:pol/e> a odrl:Set .",
        "turtle",
    )
    .unwrap();
    assert!(conflict_admissibility(&p).is_ok());
}
