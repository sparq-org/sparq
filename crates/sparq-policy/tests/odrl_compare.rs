//! ODRL static policy analysis: conflict + containment detection. [OPUS-4.8] sq-zabv.
//!
//! Candidate #7 (research/feature-research-odrl-policy.md): detect
//! permission/prohibition **conflicts** (a permission a prohibition carves out)
//! and **containment** between two policies (does one permit everything the other
//! does — query-containment refinement). Both are *static* (policy-vs-policy, no
//! Request), and both are **sound / fail-closed**: a `Certain` conflict or a
//! `Contains` verdict is only reported when it can be *proven*; everything not
//! provable degrades to `Possible` / `Unknown` (never over-claimed).

use sparq_policy::{contains, detect_conflicts, parse_policy_str, Containment, Overlap};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn left(local: &str) -> String {
    format!("{ODRL}{local}")
}

// ---------------------------------------------------------------------------
// CONFLICT DETECTION
// ---------------------------------------------------------------------------

/// A permission and a prohibition on the SAME action/target/assignee certainly
/// conflict — the prohibition carves the permission out for every matching request.
#[test]
fn permission_and_prohibition_same_target_conflict() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                    odrl:assignee <https://alice.ex/me> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                     odrl:assignee <https://alice.ex/me> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let conflicts = detect_conflicts(&p);
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].overlap, Overlap::Certain);
}

/// A permission and a prohibition on DIFFERENT targets may still meet: a request's
/// asset membership evidence can place x inside y. A possible conflict, never certain.
#[test]
fn different_targets_possibly_conflict() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/y> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let conflicts = detect_conflicts(&p);
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].overlap, Overlap::Possible);
}

/// A permission and a prohibition on different ACTIONS (read vs write) never
/// overlap — no conflict.
#[test]
fn different_actions_no_conflict() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read  ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:write ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert!(detect_conflicts(&p).is_empty());
}

/// The `odrl:use` umbrella prohibition subsumes a `read` permission (use ⊇ read),
/// so they conflict.
#[test]
fn use_umbrella_prohibition_conflicts_with_read_permission() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:use  ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let conflicts = detect_conflicts(&p);
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].overlap, Overlap::Certain);
}

/// An unrefined prohibition (no target) carves into a targeted permission — a
/// certain overlap (the prohibition applies to *every* target including this one).
#[test]
fn unrefined_prohibition_conflicts_with_targeted_permission() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let conflicts = detect_conflicts(&p);
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].overlap, Overlap::Certain);
    // The conflict names the two rules.
    assert_eq!(conflicts[0].permission_id, p.permissions[0].id);
    assert_eq!(conflicts[0].prohibition_id, p.prohibitions[0].id);
}

/// When the prohibition adds a CONSTRAINT the permission does not, the two only
/// *possibly* overlap (the constraint may or may not hold for a given request) —
/// we cannot prove a certain carve-out, so the verdict is `Possible`, not `Certain`.
#[test]
fn constrained_prohibition_only_possibly_conflicts() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                     odrl:constraint [ odrl:leftOperand odrl:dateTime ;
                       odrl:operator odrl:lt ;
                       odrl:rightOperand "2026-01-01T00:00:00Z"^^xsd:dateTime ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let conflicts = detect_conflicts(&p);
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].overlap, Overlap::Possible);
}

/// Two permissions, or two prohibitions, never conflict with each other — conflict
/// is strictly across the permission/prohibition divide.
#[test]
fn same_kind_rules_never_conflict() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert!(detect_conflicts(&p).is_empty());
}

// ---------------------------------------------------------------------------
// CONTAINMENT  (does `outer` permit everything `inner` permits?)
// ---------------------------------------------------------------------------

/// A policy contains itself (reflexivity) — a clean `Contains`.
#[test]
fn policy_contains_itself() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                    odrl:assignee <https://alice.ex/me> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    assert_eq!(contains(&p, &p), Containment::Contains);
}

/// A broad `use` permission with no target/assignee contains a narrow `read`
/// permission on a specific target/assignee (the broad rule subsumes the narrow).
#[test]
fn broad_use_contains_narrow_read() {
    let broad = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/broad> a odrl:Set ;
  odrl:permission [ odrl:action odrl:use ] .
"#,
        "turtle",
    )
    .unwrap();
    let narrow = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/narrow> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
                    odrl:assignee <https://alice.ex/me> ] .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(contains(&broad, &narrow), Containment::Contains);
    // Not symmetric: the narrow policy does NOT contain the broad one.
    assert_eq!(contains(&narrow, &broad), Containment::NotContained);
}

/// A narrower outer constraint does NOT contain a broader inner permission: the
/// outer only permits `purpose = research`, the inner permits any purpose, so the
/// inner has requests (any non-research purpose) the outer denies → `NotContained`.
#[test]
fn constrained_outer_does_not_contain_unconstrained_inner() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
      odrl:rightOperand <urn:purpose/research> ] ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(contains(&outer, &inner), Containment::NotContained);
}

/// An unconstrained outer permission DOES contain a more-constrained inner one of
/// the same action/target: every request the (extra-)constrained inner permits is
/// also permitted by the looser outer.
#[test]
fn unconstrained_outer_contains_constrained_inner() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
      odrl:rightOperand <urn:purpose/research> ] ] .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(contains(&outer, &inner), Containment::Contains);
}

/// Containment is fail-safe on the prohibitions of `outer`: if `outer` carries a
/// prohibition that could carve into a permission it would otherwise contain, we
/// cannot prove containment → `Unknown` (never claim a containment a deny-overrides
/// prohibition might break).
#[test]
fn outer_prohibition_blocks_certain_containment() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:use ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    // The outer `use` permission subsumes the inner `read`, BUT outer's prohibition
    // carves exactly that request out — so containment is NOT certain.
    assert_eq!(contains(&outer, &inner), Containment::Unknown);
}

/// An empty inner policy (permits nothing) is contained by anything — vacuous truth.
#[test]
fn empty_inner_is_contained() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(contains(&outer, &inner), Containment::Contains);
}

/// A `neq` constraint on the inner is NOT something we can prove is refined by a
/// bare outer of the same dimension — but an *unconstrained* outer (no constraint
/// at all on that dimension) still contains it. Pin that the `left` IRI is what we
/// key on, not the operator.
#[test]
fn unconstrained_outer_contains_neq_constrained_inner() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:recipient ; odrl:operator odrl:neq ;
      odrl:rightOperand <https://bob.ex/me> ] ] .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(contains(&outer, &inner), Containment::Contains);
}

/// Same dimension, IDENTICAL constraint on both → still contained (the inner is no
/// broader than the outer on that dimension).
#[test]
fn identical_constraint_is_contained() {
    let same = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/X> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
      odrl:rightOperand <urn:purpose/research> ] ] .
"#;
    let outer = parse_policy_str(same, "turtle").unwrap();
    let inner = parse_policy_str(same, "turtle").unwrap();
    assert_eq!(contains(&outer, &inner), Containment::Contains);
}

/// When the inner permits a target the outer's permissions cannot reach (a
/// different concrete target), it is provably NOT contained.
#[test]
fn inner_with_unreachable_target_not_contained() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/y> ] .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(contains(&outer, &inner), Containment::NotContained);
}

/// Helper: the conflict carries the action/target it overlaps on (left() smoke).
#[test]
fn conflict_reports_overlapping_action() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let c = &detect_conflicts(&p)[0];
    assert_eq!(c.action.as_deref(), Some(left("read").as_str()));
    assert_eq!(c.target.as_deref(), Some("urn:asset/x"));
}

/// A CONSTRAINED permission vs an UNCONSTRAINED prohibition on the same footprint
/// is a *certain* conflict: the prohibition (no constraints) fires on every request,
/// including the constrained subset the permission grants. (Asymmetric to the
/// constrained-prohibition case, which is only `Possible`.)
#[test]
fn constrained_permission_vs_unconstrained_prohibition_is_certain() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:purpose ; odrl:operator odrl:eq ;
      odrl:rightOperand <urn:purpose/research> ] ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    let conflicts = detect_conflicts(&p);
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].overlap, Overlap::Certain);
}

/// Containment short-circuits to `NotContained` as soon as ANY inner permission is
/// not subsumed, even if others are — `outer` must cover *every* inner ask.
#[test]
fn one_unsubsumed_inner_permission_breaks_containment() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/y> ] .
"#,
        "turtle",
    )
    .unwrap();
    // x is covered but y is not → not contained.
    assert_eq!(contains(&outer, &inner), Containment::NotContained);
}

/// A NUMERIC range refinement is decided soundly: inner `count lteq 3` implies outer
/// `count lteq 5` (3 ≤ 5), so the inner is contained.
#[test]
fn tighter_numeric_bound_is_contained() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ;
      odrl:rightOperand 5 ] ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ;
      odrl:rightOperand 3 ] ] .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(contains(&outer, &inner), Containment::Contains);
    // The reverse (outer `lteq 3`, inner `lteq 5`) is *genuinely* not contained
    // (inner permits count=4,5 the outer denies), but proving it needs reasoning
    // that the looser inner bound reaches above the tighter outer one — which the
    // conservative analysis does not attempt. It honestly degrades to `Unknown`
    // (sound: it never *claims* containment it cannot prove) rather than
    // over-claiming `NotContained`. (See the module soundness contract.)
    assert_eq!(contains(&inner, &outer), Containment::Unknown);
}

// ---------------------------------------------------------------------------
// COMPOUND-CONSTRAINT DEGRADATION (the sq-a0zef fail-OPEN guards). [OPUS-4.8] sq-bif.
//
// The static analyser does NOT model compound `odrl:LogicalConstraint`s. The existing
// `Possible`/`Unknown` cases above degrade on an *atomic* constraint the analyser
// cannot prove total; these pin the distinct **compound-constraint** guards:
//
//  * a prohibition carrying ANY compound constraint can never be proven to carve out
//    the WHOLE permission → the conflict is `Possible`, never `Certain` (even though
//    its atomic+structural footprint matches the permission exactly); and
//  * an OUTER permission carrying a compound constraint may be strictly narrower than
//    its atomic+structural footprint suggests → containment degrades to `Unknown`,
//    never `Contains` (claiming `Contains` would be the fail-OPEN error).
//
// A regression that dropped either guard would over-claim `Certain`/`Contains`.
// ---------------------------------------------------------------------------

/// A prohibition whose ONLY refinement is a compound `LogicalConstraint` — its
/// action/target/assignee match the permission exactly and it carries no *atomic*
/// constraint, so a naive analyser would call the conflict `Certain`. Because the
/// compound is not modelled, the verdict honestly degrades to `Possible`.
#[test]
fn compound_constrained_prohibition_is_only_possible() {
    let ttl = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
          odrl:rightOperand "2024-01-01T00:00:00Z"^^xsd:dateTime ] ,
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
          odrl:rightOperand "2030-01-01T00:00:00Z"^^xsd:dateTime ] ] ] .
"#;
    let p = parse_policy_str(ttl, "turtle").unwrap();
    // The compound is parsed onto the prohibition (so it really exercises the guard,
    // not an empty-constraints shortcut).
    assert_eq!(
        p.prohibitions[0].logical_constraints.len(),
        1,
        "the prohibition carries a compound constraint"
    );
    assert!(
        p.prohibitions[0].constraints.is_empty(),
        "and NO atomic constraint — only the compound could make it Possible"
    );
    let conflicts = detect_conflicts(&p);
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(
        conflicts[0].overlap,
        Overlap::Possible,
        "a compound on the prohibition forbids a Certain verdict"
    );
}

/// An OUTER permission whose only refinement is a compound `LogicalConstraint`,
/// against an unconstrained INNER permission on the same action/target. Structurally
/// `outer ⊇ inner`, but the unmodelled compound could make `outer` strictly narrower,
/// so containment is `Unknown` (never `Contains`). The atomic-footprint twin
/// (`unconstrained_outer_contains_constrained_inner`) returns `Contains`; this is the
/// compound-guard counterpart.
#[test]
fn compound_constrained_outer_makes_containment_unknown() {
    let outer = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/outer> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ a odrl:LogicalConstraint ; odrl:or
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lt ;
          odrl:rightOperand "2024-01-01T00:00:00Z"^^xsd:dateTime ] ,
        [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;
          odrl:rightOperand "2030-01-01T00:00:00Z"^^xsd:dateTime ] ] ] .
"#,
        "turtle",
    )
    .unwrap();
    let inner = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/inner> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
"#,
        "turtle",
    )
    .unwrap();
    assert_eq!(
        outer.permissions[0].logical_constraints.len(),
        1,
        "the outer permission carries the compound that drives the Unknown"
    );
    assert_eq!(
        contains(&outer, &inner),
        Containment::Unknown,
        "a compound on the outer permission forbids a Contains verdict"
    );
}

/// Containment is definite only for shapes the rule comparison decides statically.
/// Every shape below depends on something it cannot see (membership evidence, a duty
/// that is never discharged), on either side, and must come back `Unknown`.
#[test]
fn undecidable_shapes_are_never_definite() {
    let pol = |body: &str| {
        parse_policy_str(
            &format!(
                "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
                 <urn:pol/p> a odrl:Set ;\n{body} ."
            ),
            "turtle",
        )
        .unwrap()
    };
    let read = |extra: &str| {
        format!(
            "odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; \
             odrl:assignee <https://alice.ex/me> {extra} ]"
        )
    };
    let plain = pol(&read(""));
    let constrained_duty = "; odrl:duty [ odrl:action odrl:compensate ; odrl:constraint [ \
        odrl:leftOperand odrl:payAmount ; odrl:operator odrl:eq ; odrl:rightOperand 5 ] ]";
    let shapes = [
        // A constrained duty on the permission.
        ("constrained duty", pol(&read(constrained_duty))),
        // An unconstrained duty the other side does not require.
        ("unshared duty", pol(&read("; odrl:duty [ odrl:action odrl:attribute ]"))),
        // A declared party collection.
        (
            "party collection",
            pol(&format!(
                "{} .\n<https://lab.ex/team> a odrl:PartyCollection",
                read("")
            )),
        ),
        // A prohibition on another party: membership can make alice part of it.
        (
            "prohibition on another party",
            pol(&format!(
                "{} ;\n odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ; \
                 odrl:assignee <https://lab.ex/team> ]",
                read("")
            )),
        ),
        // A prohibition on another asset: membership can make x part of it.
        (
            "prohibition on another asset",
            pol(&format!(
                "{} ;\n odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/all> ]",
                read("")
            )),
        ),
    ];
    for (name, shape) in &shapes {
        assert_eq!(contains(shape, &plain), Containment::Unknown, "{name} as outer");
        if !name.starts_with("prohibition") && *name != "unshared duty" {
            // A prohibition or an unconstrained duty only narrows the inner side.
            assert_eq!(contains(&plain, shape), Containment::Unknown, "{name} as inner");
        }
    }
    // An empty outer policy against an inner permission with a constrained duty: the
    // inner grants nothing, so `NotContained` would have no witness.
    let empty = pol("odrl:prohibition [ odrl:action odrl:delete ; odrl:target <urn:asset/y> ]");
    assert_eq!(contains(&empty, &shapes[0].1), Containment::Unknown);
    // Controls: the same unconstrained duty on both sides, and the plain shape, decide.
    assert_eq!(contains(&shapes[1].1, &shapes[1].1), Containment::Contains);
    assert_eq!(contains(&plain, &plain), Containment::Contains);
}

/// `use` does not cover the `transfer` subtree, so a `use` permission does not contain
/// a `sell` one.
#[test]
fn use_does_not_contain_sell() {
    let pol = |action: &str| {
        parse_policy_str(
            &format!(
                "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
                 <urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:{action} ] ."
            ),
            "turtle",
        )
        .unwrap()
    };
    assert_eq!(contains(&pol("use"), &pol("sell")), Containment::NotContained);
    assert_eq!(contains(&pol("use"), &pol("read")), Containment::Contains);
}
