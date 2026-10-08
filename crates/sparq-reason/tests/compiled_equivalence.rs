//! [FABLE-5] sq-zgbso.3 — the compiled-rules RESULT-EQUIVALENCE oracle (bead invariant):
//! on the sparq-solid WAC/ACP rule corpus + the sq-zgbso.1 ODRL spike rules, the id-level
//! compiled evaluator's closure equals the `reason_n3` closure AS A SET over the same
//! rules + facts — failing loudly with the full symmetric difference on any divergence.
//!
//! The rules files are read READ-ONLY from `../sparq-solid/rules/` at runtime (no
//! sparq-solid code dependency — that would be a dev-dep cycle); the facts are
//! loader-shaped fixtures from `common::{wac_facts, acp_facts}`. Spot asserts on
//! expected auth triples make a both-empty (vacuously equal) pass impossible.

mod common;

use common::{
    acp_facts, assert_set_equal, closure_to_n3, solid_rules, triples_as_strings, wac_facts, Scale,
    ALICE, APP, BOB, CAROL, IDP, POD,
};
use sparq_core::dict::Dict;
use sparq_reason::n3::compiled::{compile, compile_with_cycles, eval, intern_facts};
use sparq_reason::n3::encode_for_uri;
use sparq_reason::{reason_n3, reason_n3_with_cycles, NegationCycles};
use std::collections::BTreeSet;

const AUTH: &str = "https://sparq.dev/ns/auth#";

/// Small tree: 2 subtrees of each policy family would need tops=5; keep every family
/// covered with tops = 5 and a 2×2×2 fan-out (fast enough for the string engine in a
/// debug test run).
fn small() -> Scale {
    Scale {
        tops: 5,
        mids: 2,
        leaves: 2,
        docs: 2,
    }
}

fn text_closure(src: &str) -> (Dict, Vec<[sparq_core::dict::Id; 3]>) {
    let mut dict = Dict::new();
    let ids = reason_n3(&mut dict, src).expect("reason_n3");
    (dict, ids)
}

/// Rule files that conclude a VARIABLE predicate next to a store-scoped negation
/// (acp-c.n3, odrl-spike.n3) are a negation cycle to the automatic stratification, which
/// must assume such a conclusion can derive the negated predicate; both engines refuse
/// them by default. Their caller runs each as its own explicit stratum, so the parity
/// check below opts both engines in to the legacy single-pass evaluation for them.
const LEGACY: NegationCycles = NegationCycles::SinglePass;

/// The sparq-solid rule files the automatic stratification refuses by default.
const REFUSED_CORPUS_FILES: &[&str] =
    &["acp-c.n3", "odrl-b.n3", "odrl-c.n3", "odrl-d.n3", "odrl-spike.n3"];

fn text_closure_legacy(src: &str) -> (Dict, Vec<[sparq_core::dict::Id; 3]>) {
    let mut dict = Dict::new();
    let ids = reason_n3_with_cycles(&mut dict, src, LEGACY).expect("reason_n3_with_cycles");
    (dict, ids)
}

fn compiled_closure_legacy(
    facts_src: &str,
    rules_src: &str,
) -> (Dict, Vec<[sparq_core::dict::Id; 3]>) {
    let rules = compile_with_cycles(rules_src, LEGACY).expect("compile_with_cycles");
    let mut dict = Dict::new();
    let facts = intern_facts(&mut dict, facts_src).expect("intern_facts");
    let ids = eval(&mut dict, &facts, &rules);
    (dict, ids)
}

fn compiled_closure(facts_src: &str, rules_src: &str) -> (Dict, Vec<[sparq_core::dict::Id; 3]>) {
    let rules = compile(rules_src).expect("compile");
    let mut dict = Dict::new();
    let facts = intern_facts(&mut dict, facts_src).expect("intern_facts");
    let ids = eval(&mut dict, &facts, &rules);
    (dict, ids)
}

fn has(set: &BTreeSet<String>, s: &str, p: &str, o: &str) -> bool {
    set.contains(&format!("<{s}> <{p}> <{o}>"))
}

#[test]
fn wac_corpus_closure_is_identical() {
    let facts = wac_facts(&small());
    let rules = format!("{}\n{}", solid_rules("common.n3"), solid_rules("wac.n3"));

    let (td, tc) = text_closure(&format!("{facts}\n{rules}"));
    let text = triples_as_strings(&td, &tc);
    let (cd, cc) = compiled_closure(&facts, &rules);
    let compiled = triples_as_strings(&cd, &cc);
    assert_set_equal(&text, &compiled, "WAC (common.n3 + wac.n3)");

    // Spot asserts — a vacuous (both-empty / rules-never-fired) pass is impossible.
    // The subtree-0 owner reaches an uncontrolled document via the inheritance walk
    // (d0.ttl carries the resource-specific bob-only ACL, so d1.ttl is the clean case):
    assert!(has(
        &compiled,
        ALICE,
        &format!("{AUTH}read"),
        &format!("{POD}s0/c0/g0/d1.ttl")
    ));
    // The resource-specific ACL shadows the inherited owner grant on d0.ttl:
    assert!(has(
        &compiled,
        BOB,
        &format!("{AUTH}read"),
        &format!("{POD}s0/c0/g0/d0.ttl")
    ));
    assert!(!has(
        &compiled,
        ALICE,
        &format!("{AUTH}read"),
        &format!("{POD}s0/c0/g0/d0.ttl")
    ));
    // The group subtree (s1) SHADOWS the root owner (nearest-ACL, notIncludes):
    assert!(!has(
        &compiled,
        ALICE,
        &format!("{AUTH}read"),
        &format!("{POD}s1/c0/g0/d0.ttl")
    ));
    assert!(has(
        &compiled,
        BOB,
        &format!("{AUTH}write"),
        &format!("{POD}s1/c0/g0/d0.ttl")
    ));
    // The origin-restricted subtree (s4) mints the injective pair principal:
    let pair = format!(
        "urn:sparq:pair?agent={}&client={}",
        encode_for_uri(BOB),
        encode_for_uri(APP)
    );
    assert!(has(
        &compiled,
        &pair,
        &format!("{AUTH}read"),
        &format!("{POD}s4/c0/g0/d0.ttl")
    ));
    // Control gates the ACL resource itself:
    assert!(has(
        &compiled,
        ALICE,
        &format!("{AUTH}write"),
        &format!("{POD}.acl")
    ));
}

#[test]
fn acp_corpus_closure_is_identical_across_all_three_strata() {
    let facts = acp_facts(&small());
    let common_rules = solid_rules("common.n3");
    let (a, b, c) = (
        solid_rules("acp-a.n3"),
        solid_rules("acp-b.n3"),
        solid_rules("acp-c.n3"),
    );

    // TEXT path — exactly materialize_acp's stratification: three reason_n3 calls with
    // the closure re-serialized to N3 text between strata.
    let (d1, c1) = text_closure(&format!("{facts}\n{common_rules}\n{a}"));
    let f1 = closure_to_n3(&d1, &c1);
    let (d2, c2) = text_closure(&format!("{f1}\n{b}"));
    let f2 = closure_to_n3(&d2, &c2);
    let (d3, c3) = text_closure_legacy(&format!("{f2}\n{c}"));
    let text = triples_as_strings(&d3, &c3);

    // COMPILED path — the same three strata chained ENTIRELY at the id level, one
    // dictionary, no text between strata.
    let ra = compile(&format!("{common_rules}\n{a}")).expect("compile stratum A");
    let rb = compile(&b).expect("compile stratum B");
    let rc = compile_with_cycles(&c, LEGACY).expect("compile stratum C");
    let mut dict = Dict::new();
    let f0 = intern_facts(&mut dict, &facts).expect("intern facts");
    let s1 = eval(&mut dict, &f0, &ra);
    let s2 = eval(&mut dict, &s1, &rb);
    let s3 = eval(&mut dict, &s2, &rc);
    let compiled = triples_as_strings(&dict, &s3);

    assert_set_equal(&text, &compiled, "ACP (3 strata: acp-a -> acp-b -> acp-c)");

    // Spot asserts across the policy families.
    // Cumulative root owner (memberAccessControl reaches everything):
    assert!(has(
        &compiled,
        ALICE,
        &format!("{AUTH}read"),
        &format!("{POD}s0/c0/g0/d0.ttl")
    ));
    // anyOf public read on subtree 0:
    assert!(has(
        &compiled,
        &format!("{AUTH}Public"),
        &format!("{AUTH}read"),
        &format!("{POD}s0/c0/g0/d0.ttl")
    ));
    // deny-overrides materialisation (dave denied on subtree 3):
    assert!(has(
        &compiled,
        "https://dave.ex/card#me",
        &format!("{AUTH}denyRead"),
        &format!("{POD}s3/c0/g0/d0.ttl")
    ));
    // allOf (agent, client) pair principal on subtree 2:
    let pair = format!(
        "urn:sparq:pair?agent={}&client={}",
        encode_for_uri(BOB),
        encode_for_uri(APP)
    );
    assert!(has(
        &compiled,
        &pair,
        &format!("{AUTH}read"),
        &format!("{POD}s2/c0/g0/d0.ttl")
    ));
    // Issuer-CONSTRAINED matcher mints the (agent, client, issuer) triple principal:
    let triple = format!(
        "urn:sparq:triple?agent={}&client={}&issuer={}",
        encode_for_uri(BOB),
        encode_for_uri("https://sparq.dev/ns/auth#AnyClient"),
        encode_for_uri(IDP)
    );
    assert!(has(
        &compiled,
        &triple,
        &format!("{AUTH}read"),
        &format!("{POD}s1/c0/g0/d0.ttl")
    ));
    // noneOf (subtree 4) emits a ConditionalGrant node with carol as exception matcher:
    assert!(
        compiled.iter().any(|t| t.contains("urn:sparq:grant?cand=")
            && t.contains(&format!("<{AUTH}ConditionalGrant>"))),
        "expected a minted ConditionalGrant node"
    );
    // CreatorAgent provenance: carol (creator) gains Write ONLY on her created docs:
    assert!(has(
        &compiled,
        CAROL,
        &format!("{AUTH}write"),
        &format!("{POD}s2/c0/g0/d0.ttl")
    ));
    assert!(!has(
        &compiled,
        CAROL,
        &format!("{AUTH}write"),
        &format!("{POD}s2/c1/g0/d0.ttl")
    ));
}

#[test]
fn odrl_spike_rules_closure_is_identical() {
    let rules = solid_rules("odrl-spike.n3");
    // The sq-zgbso.1 spike's scenario shapes: ALLOW (unconstrained + within-window),
    // fail-closed denies (after-window, action-mismatch, unmodelled constraint), and
    // DENY-OVERRIDES (prohibition wins).
    let facts = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix spike: <https://sparq.dev/ns/odrl-spike#> .
@prefix ex: <https://ex.dev/odrl#> .

# B1: unconstrained permission, no prohibition -> bob reads t1.
ex:pol1 odrl:permission ex:perm1 .
ex:perm1 odrl:action odrl:read . ex:perm1 odrl:target ex:t1 . ex:perm1 odrl:assignee ex:bob .
ex:req1 a odrl:Request . ex:req1 odrl:action odrl:read . ex:req1 odrl:target ex:t1 .
ex:req1 odrl:assignee ex:bob .

# A1: within-window dateTime constraint -> alice writes t2.
ex:pol2 odrl:permission ex:perm2 .
ex:perm2 odrl:action odrl:modify . ex:perm2 odrl:target ex:t2 . ex:perm2 odrl:assignee ex:alice .
ex:perm2 odrl:constraint ex:c2 .
ex:c2 odrl:leftOperand odrl:dateTime . ex:c2 odrl:operator odrl:lteq .
ex:c2 odrl:rightOperand "2027-01-01T00:00:00Z" .
ex:req2 a odrl:Request . ex:req2 odrl:action odrl:modify . ex:req2 odrl:target ex:t2 .
ex:req2 odrl:assignee ex:alice . ex:req2 spike:atTime "2026-06-01T00:00:00Z" .

# A2: after-window -> NO grant on t2b.
ex:pol2b odrl:permission ex:perm2b .
ex:perm2b odrl:action odrl:modify . ex:perm2b odrl:target ex:t2b . ex:perm2b odrl:assignee ex:alice .
ex:perm2b odrl:constraint ex:c2b .
ex:c2b odrl:leftOperand odrl:dateTime . ex:c2b odrl:operator odrl:lteq .
ex:c2b odrl:rightOperand "2027-01-01T00:00:00Z" .
ex:req2b a odrl:Request . ex:req2b odrl:action odrl:modify . ex:req2b odrl:target ex:t2b .
ex:req2b odrl:assignee ex:alice . ex:req2b spike:atTime "2028-06-01T00:00:00Z" .

# C1: prohibition carves the same-policy permission out -> denyRead, NO read.
ex:pol3 odrl:permission ex:perm3 . ex:pol3 odrl:prohibition ex:px3 .
ex:perm3 odrl:action odrl:read . ex:perm3 odrl:target ex:t3 . ex:perm3 odrl:assignee ex:carol .
ex:px3 odrl:action odrl:read . ex:px3 odrl:target ex:t3 . ex:px3 odrl:assignee ex:carol .
ex:req3 a odrl:Request . ex:req3 odrl:action odrl:read . ex:req3 odrl:target ex:t3 .
ex:req3 odrl:assignee ex:carol .

# A5: action mismatch -> nothing about t4.
ex:pol4 odrl:permission ex:perm4 .
ex:perm4 odrl:action odrl:read . ex:perm4 odrl:target ex:t4 . ex:perm4 odrl:assignee ex:bob .
ex:req4 a odrl:Request . ex:req4 odrl:action odrl:write . ex:req4 odrl:target ex:t4 .
ex:req4 odrl:assignee ex:bob .

# Unmodelled constraint (odrl:purpose) -> fail-closed, nothing about t5.
ex:pol5 odrl:permission ex:perm5 .
ex:perm5 odrl:action odrl:read . ex:perm5 odrl:target ex:t5 . ex:perm5 odrl:assignee ex:bob .
ex:perm5 odrl:constraint ex:c5 .
ex:c5 odrl:leftOperand odrl:purpose . ex:c5 odrl:operator odrl:eq . ex:c5 odrl:rightOperand ex:research .
ex:req5 a odrl:Request . ex:req5 odrl:action odrl:read . ex:req5 odrl:target ex:t5 .
ex:req5 odrl:assignee ex:bob .
"#;

    let (td, tc) = text_closure_legacy(&format!("{facts}\n{rules}"));
    let text = triples_as_strings(&td, &tc);
    let (cd, cc) = compiled_closure_legacy(facts, &rules);
    let compiled = triples_as_strings(&cd, &cc);
    assert_set_equal(&text, &compiled, "ODRL spike (odrl-spike.n3)");

    let ex = "https://ex.dev/odrl#";
    assert!(has(
        &compiled,
        &format!("{ex}bob"),
        &format!("{AUTH}read"),
        &format!("{ex}t1")
    ));
    assert!(has(
        &compiled,
        &format!("{ex}alice"),
        &format!("{AUTH}write"),
        &format!("{ex}t2")
    ));
    assert!(!has(
        &compiled,
        &format!("{ex}alice"),
        &format!("{AUTH}write"),
        &format!("{ex}t2b")
    ));
    assert!(has(
        &compiled,
        &format!("{ex}carol"),
        &format!("{AUTH}denyRead"),
        &format!("{ex}t3")
    ));
    assert!(!has(
        &compiled,
        &format!("{ex}carol"),
        &format!("{AUTH}read"),
        &format!("{ex}t3")
    ));
    assert!(!compiled
        .iter()
        .any(|t| t.contains("t4>") && t.contains("auth#")));
    assert!(!compiled
        .iter()
        .any(|t| t.contains("t5>") && t.contains("auth#")));
}

#[test]
fn every_corpus_rules_file_compiles() {
    // The compile front end must admit the WHOLE access-control corpus (the sq-zgbso.4
    // build-time flip depends on it) — and report how many rules it lowered.
    for (name, min_rules) in [
        ("common.n3", 4),
        ("wac.n3", 12),
        ("acp-a.n3", 20),
        ("acp-b.n3", 5),
        ("acp-c.n3", 10),
        ("odrl-spike.n3", 3),
    ] {
        let compiled = compile_with_cycles(&solid_rules(name), LEGACY)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            compiled.n_rules() >= min_rules,
            "{name}: expected at least {min_rules} rules, compiled {}",
            compiled.n_rules()
        );
    }
}

/// GH #6201 / #5756: store-scoped `log:notIncludes` over a predicate the SAME rule set
/// derives. Both engines stratify automatically, so the negating rule waits for the
/// deriving rule's fixpoint and the closures stay set-equal; both refuse a negation cycle.
#[test]
fn auto_stratified_negation_is_identical() {
    const PRE: &str = "@prefix : <http://ex/> .\n\
        @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n";
    let cases: [(&str, &str, &str, usize); 2] = [
        (
            "notIncludes probe (#6201)",
            ":r :target :g . :p :prohibits :g . :s :target :h .",
            "{ ?r :target ?g . ?p :prohibits ?g } => { ?r :prohibitedIn ?g } .\n\
             { ?r :target ?g . ?scope log:notIncludes { ?r :prohibitedIn ?g } }\n\
               => { ?r :allowedBy ?g } .",
            2,
        ),
        (
            "three-stratum chain",
            ":x :item :t . :y :item :t . :x :flag :t .",
            "{ ?i :flag :t } => { ?i :flagged :t } .\n\
             { ?i :item :t . ?s log:notIncludes { ?i :flagged :t } } => { ?i :clean :t } .\n\
             { ?i :item :t . ?s log:notIncludes { ?i :clean :t } } => { ?i :suspect :t } .\n\
             { ?a :suspect :t . ?b :clean :t } => { ?a :contrast ?b } .",
            3,
        ),
    ];
    for (what, facts, rules, n_strata) in cases {
        let (facts, rules) = (format!("{PRE}{facts}"), format!("{PRE}{rules}"));
        let (td, tc) = text_closure(&format!("{facts}\n{rules}"));
        let text = triples_as_strings(&td, &tc);
        let (cd, cc) = compiled_closure(&facts, &rules);
        let compiled = triples_as_strings(&cd, &cc);
        assert_set_equal(&text, &compiled, what);
        let set = compile(&rules).expect("compile");
        assert_eq!(set.n_strata(), n_strata, "{what}");
    }
    // A negation cycle: both engines refuse it.
    let cyclic = format!(
        "{PRE}:a :p :b .\n\
         {{ :a :p :b . ?s log:notIncludes {{ :a :q :b }} }} => {{ :a :r :b }} .\n\
         {{ :a :r :b }} => {{ :a :q :b }} ."
    );
    assert!(reason_n3(&mut Dict::new(), &cyclic).is_err());
    let e = compile(&cyclic).expect_err("compile refuses a negation cycle");
    assert!(e.contains("cycle"), "{e}");
    // Spot asserts: the prohibited request is NOT allowed, the free one is.
    let facts = format!("{PRE}:r :target :g . :p :prohibits :g . :s :target :h .");
    let rules = format!("{PRE}{}", cases[0].2);
    let (cd, cc) = compiled_closure(&facts, &rules);
    let compiled = triples_as_strings(&cd, &cc);
    assert!(!has(&compiled, "http://ex/r", "http://ex/allowedBy", "http://ex/g"));
    assert!(has(&compiled, "http://ex/s", "http://ex/allowedBy", "http://ex/h"));
}

/// The access-control corpus under the automatic stratification. A rule file that
/// concludes a variable predicate next to a store-scoped negation is refused by default
/// (see [`LEGACY`]); every other rules file compiles.
#[test]
fn corpus_rules_files_stratification() {
    let rules_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../sparq-solid/rules");
    let mut refused: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(rules_dir).expect("rules dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "n3") {
            let src = std::fs::read_to_string(&path).expect("read");
            if let Err(e) = compile(&src) {
                if e.contains("cycle") {
                    assert!(e.contains("variable predicate"), "{}: {e}", path.display());
                    refused.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
            }
        }
    }
    refused.sort();
    assert_eq!(refused, REFUSED_CORPUS_FILES, "refused rule files changed");
}

/// A variable conclusion predicate may derive anything, including the predicate its own
/// premise negates: both engines refuse the rule set.
#[test]
fn variable_predicate_negation_cycle_is_refused_by_both_engines() {
    const PRE: &str = "@prefix : <http://ex/> .\n\
        @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n";
    let rules = format!(
        "{PRE}{{ ?m :pred ?p . ?x :item :t . ?s log:notIncludes {{ ?x :blocked :t }} }}\n\
           => {{ ?x ?p :t }} ."
    );
    let e = compile(&rules).expect_err("compile refuses the cycle");
    assert!(e.contains("cycle"), "{e}");
    assert!(reason_n3(&mut Dict::new(), &format!("{PRE}:m :pred :ok .\n{rules}")).is_err());
}
