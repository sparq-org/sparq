//! The INPUT and whole-decision budget of an authorization materialization
//! ([`AuthBudget`]) — the two things the per-closure reasoning allowance cannot bound.
//!
//! * **Input.** The control documents a pod holds become owned triples and are interned
//!   into a closure's dictionary before any rule runs, and `ClosureLimits::max_term_bytes`
//!   bounds only the terms the rules DERIVE. So the input gets its own budget: per-document
//!   and aggregate bytes, the length of one term, and the number of facts. Each document's
//!   terms are charged WHILE it is read, so a document set past the budget is never
//!   materialized whole.
//! * **Whole decision.** `ClosureLimits` applies per CLOSURE, and one materialization runs
//!   several — three for an ACP view. The aggregate across them has its own cap.
//!
//! Every refusal is fail-closed and typed: no view is installed, nothing is granted, and
//! the refusal carries the same [`AuthAllowance`] kind family a reasoning-allowance refusal
//! does.

use sparq_core::Graph;
use sparq_solid::{
    auth_allowance_refusal, default_auth_budget, default_auth_limits, is_auth_allowance_refusal,
    AuthAllowance, AuthBudget, ClosureLimits, Mode, PodStore, Session, AUTH_ALLOWANCE_REFUSAL,
};

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ALICE: &str = "https://alice.ex/card#me";

fn alice() -> Session<'static> {
    Session {
        agent: Some(ALICE),
        ..Session::default()
    }
}

/// Every quad of every stored graph, the reserved authorization view included.
fn all_quads(store: &PodStore) -> Vec<String> {
    let mut out = Vec::new();
    for (name, g) in std::iter::once((String::from("<default>"), store.graph())).chain(
        store
            .graph()
            .named()
            .iter()
            .map(|(n, g)| (format!("{}", n), g)),
    ) {
        let scan = g.store().scan(&[None, None, None]);
        for r in scan.rows.iter() {
            let [s, p, o] = scan.to_spo(r);
            out.push(format!(
                "{} {} {} {}",
                name,
                g.dict().term(s),
                g.dict().term(p),
                g.dict().term(o)
            ));
        }
    }
    out.sort();
    out
}

/// Whether the store grants anything to anyone across every mode.
fn grants_anything(store: &PodStore) -> bool {
    for agent in [Some(ALICE), Some("https://bob.ex/card#me"), None] {
        let s = Session {
            agent,
            ..Session::default()
        };
        for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
            if !store.accessible(&s, mode).is_empty() {
                return true;
            }
        }
    }
    false
}

/// A WAC pod: `n_docs` ACLs each with `pad_len`-byte padding literals, all granting alice
/// Read + Control on the pod root.
fn wac_pod(n_docs: usize, pad_len: usize, pad_count: usize) -> String {
    let mut nq =
        String::from("<https://pod.ex/d#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/d> .\n");
    for d in 0..n_docs {
        let (resource, acl) = if d == 0 {
            (
                "https://pod.ex/".to_owned(),
                "https://pod.ex/.acl".to_owned(),
            )
        } else {
            (
                format!("https://pod.ex/r{}", d),
                format!("https://pod.ex/r{}.acl", d),
            )
        };
        nq.push_str(&format!(
            "<{acl}#a> <{RDF_TYPE}> <{ACL}Authorization> <{acl}> .\n\
             <{acl}#a> <{ACL}accessTo> <{resource}> <{acl}> .\n\
             <{acl}#a> <{ACL}default> <{resource}> <{acl}> .\n\
             <{acl}#a> <{ACL}agent> <{ALICE}> <{acl}> .\n\
             <{acl}#a> <{ACL}mode> <{ACL}Read> <{acl}> .\n\
             <{acl}#a> <{ACL}mode> <{ACL}Control> <{acl}> .\n"
        ));
        for i in 0..pad_count {
            nq.push_str(&format!(
                "<{acl}#a> <https://ex.dev/ns#pad{i}> \"{}\" <{acl}> .\n",
                "x".repeat(pad_len)
            ));
        }
    }
    nq
}

/// A FRESH store over `nquads` under `budget` (and `limits`, when given): never
/// materialized, so what it grants after a refusal is what THIS materialization installed
/// and not what an earlier successful one left in force.
fn fresh(nquads: &str, budget: AuthBudget, limits: Option<ClosureLimits>) -> PodStore {
    let mut store = PodStore::new(Graph::load_dataset(nquads, "nquads").expect("parses"));
    store.set_auth_budget(budget);
    if let Some(l) = limits {
        store.set_auth_limits(l);
    }
    store
}

/// The WAC pod `nquads` materializes and grants under the DEFAULT budget — so a refusal of
/// the same pod under a tightened one is the budget's doing and not the pod's.
fn assert_materializes_by_default(nquads: &str) {
    let mut store = fresh(nquads, default_auth_budget(), None);
    store
        .materialize_wac()
        .expect("materializes under the default budget");
    assert!(
        grants_anything(&store),
        "the pod does grant under the defaults"
    );
}

/// A refusal is fail-closed and typed: the materialization fails with the `kind` of
/// allowance refusal, no quad changed, and nothing is granted.
fn assert_refused(store: &mut PodStore, kind: AuthAllowance, what: &str) {
    let before = all_quads(store);
    let err = store.materialize_wac().expect_err(what);
    assert!(is_auth_allowance_refusal(&err), "{}: {}", what, err);
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(kind),
        "{}: {}",
        what,
        err
    );
    assert!(err.contains(AUTH_ALLOWANCE_REFUSAL), "{}: {}", what, err);
    assert_eq!(
        all_quads(store),
        before,
        "{}: a refusal installed something",
        what
    );
    assert!(
        !grants_anything(store),
        "{}: a refused pod granted something",
        what
    );
}

/// ONE control document past `max_document_bytes` refuses the whole materialization, even
/// though the documents TOGETHER are well inside `max_input_bytes`.
#[test]
fn one_oversized_control_document_is_refused() {
    let nquads = wac_pod(1, 1_000, 16);
    assert_materializes_by_default(&nquads);
    let mut budget = default_auth_budget();
    budget.max_document_bytes = 8_000;
    assert!(budget.max_document_bytes < budget.max_input_bytes);
    let mut store = fresh(&nquads, budget, None);
    assert_refused(
        &mut store,
        AuthAllowance::InputBytes,
        "a ~16 KiB document under an 8,000-byte per-document budget",
    );
}

/// Many documents each INSIDE the per-document budget are still refused once they pass the
/// aggregate one — the dimension a per-document cap alone cannot bound.
#[test]
fn documents_inside_the_per_document_budget_are_refused_in_aggregate() {
    let nquads = wac_pod(12, 400, 2);
    assert_materializes_by_default(&nquads);
    let mut budget = default_auth_budget();
    budget.max_document_bytes = 4_000;
    budget.max_input_bytes = 6_000;
    let mut store = fresh(&nquads, budget, None);
    assert_refused(
        &mut store,
        AuthAllowance::InputBytes,
        "12 documents of ~1 KiB each under a 6,000-byte aggregate budget",
    );

    // Each document on its own fits the per-document budget, so raising ONLY the aggregate
    // lets the same pod through: it was the aggregate that refused it.
    let mut wide = budget;
    wide.max_input_bytes = default_auth_budget().max_input_bytes;
    let mut store = fresh(&nquads, wide, None);
    store
        .materialize_wac()
        .expect("every document is inside the per-document budget");
    assert!(grants_anything(&store));
}

/// One long term in a control document is refused by `max_input_term_bytes` — the
/// dimension `ClosureLimits::max_term_bytes` does not cover, since it bounds only the terms
/// the rules DERIVE.
#[test]
fn a_long_input_term_is_refused() {
    let nquads = wac_pod(1, 3_000, 1);
    assert_materializes_by_default(&nquads);

    // The reasoning allowance's own term bound is raised far above the term's size, so it
    // is plainly not what refuses this: `max_term_bytes` covers only DERIVED terms.
    let mut limits = default_auth_limits();
    limits.max_term_bytes = 1 << 20;
    let mut budget = default_auth_budget();
    budget.max_input_term_bytes = 512;
    let mut store = fresh(&nquads, budget, Some(limits));
    assert_refused(
        &mut store,
        AuthAllowance::InputTermBytes,
        "a 3,000-byte literal under a 512-byte input-term budget",
    );
}

/// The assembled input's fact count is refused by `max_input_facts` before it is interned.
#[test]
fn too_many_input_facts_are_refused() {
    let nquads = wac_pod(40, 8, 1);
    assert_materializes_by_default(&nquads);
    let mut budget = default_auth_budget();
    budget.max_input_facts = 64;
    let mut store = fresh(&nquads, budget, None);
    assert_refused(
        &mut store,
        AuthAllowance::InputFacts,
        "40 control documents under a 64-fact input budget",
    );
}

/// An ACP pod whose three strata are each INSIDE the per-closure reasoning allowance is
/// still refused once their aggregate is past the whole-decision cap.
///
/// This is the property a per-closure `ClosureLimits` cannot express: the smallest
/// whole-decision allowance this pod materializes under is strictly LARGER than the
/// smallest per-closure one, so the three strata together spend more than any one of them
/// may — and only the aggregate ledger sees that.
#[test]
fn the_three_acp_strata_are_bounded_in_aggregate_not_only_per_closure() {
    let acr = "https://pod.ex/.acr";
    let nquads = format!(
        "<https://pod.ex/d#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/d> .\n\
         <{acr}> <{ACP}memberAccessControl> <{acr}#c> <{acr}> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#pol> <{acr}> .\n\
         <{acr}#pol> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
         <{acr}#pol> <{ACP}allow> <{ACL}Write> <{acr}> .\n\
         <{acr}#pol> <{ACP}allOf> <{acr}#m> <{acr}> .\n\
         <{acr}#m> <{ACP}agent> <{ALICE}> <{acr}> .\n"
    );
    let graph = Graph::load_dataset(&nquads, "nquads").expect("parses");

    /// The smallest value of a budget/limit field at which this pod still materializes.
    fn minimum(nquads: &str, set: impl Fn(&mut PodStore, usize)) -> usize {
        let runs = |v: usize| {
            let mut store = PodStore::new(Graph::load_dataset(nquads, "nquads").expect("parses"));
            set(&mut store, v);
            store.materialize_acp().is_ok()
        };
        let (mut lo, mut hi) = (0usize, 1usize);
        while !runs(hi) {
            lo = hi;
            hi *= 2;
            assert!(hi < 1 << 30, "no upper bound found");
        }
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if runs(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    let per_closure = minimum(&nquads, |store, v| {
        let mut l = default_auth_limits();
        l.max_total_join_rows = v;
        store.set_auth_limits(l);
    });
    let whole_decision = minimum(&nquads, |store, v| {
        let mut b = default_auth_budget();
        b.max_decision_join_rows = v;
        store.set_auth_budget(b);
    });
    assert!(
        whole_decision > per_closure,
        "the strata together ({}) must cost more than the dearest one ({}), or this pod \
         cannot show the difference",
        whole_decision,
        per_closure
    );

    // At the per-closure minimum — which every stratum satisfies — the aggregate cap still
    // refuses, naming itself.
    let mut store = PodStore::new(graph);
    let mut budget = default_auth_budget();
    budget.max_decision_join_rows = whole_decision - 1;
    store.set_auth_budget(budget);
    let before = all_quads(&store);
    let err = store
        .materialize_acp()
        .expect_err("the strata's aggregate is past the whole-decision cap");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::DecisionWork),
        "{}",
        err
    );
    assert!(err.contains("max_decision_join_rows"), "{}", err);
    assert_eq!(all_quads(&store), before, "a refusal installed something");
    assert!(!grants_anything(&store));

    // And one notch up it materializes, so the cap is what refused it.
    budget.max_decision_join_rows = whole_decision;
    store.set_auth_budget(budget);
    store
        .materialize_acp()
        .expect("materializes at the aggregate minimum");
    assert!(!store.accessible(&alice(), Mode::Read).is_empty());
}

/// The whole-decision FACT cap is the same story for the facts the strata hold.
#[test]
fn the_acp_strata_facts_are_bounded_in_aggregate() {
    let nquads = sparq_solid::acp_fixture();
    let mut store = PodStore::new(Graph::load_dataset(&nquads, "nquads").expect("parses"));
    // Each stratum stays well inside its own `max_facts`, which is left at the default.
    let mut budget = default_auth_budget();
    budget.max_decision_facts = 1_000;
    store.set_auth_budget(budget);
    let before = all_quads(&store);
    let err = store
        .materialize_acp()
        .expect_err("past the aggregate fact cap");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::DecisionWork),
        "{}",
        err
    );
    assert!(err.contains("max_decision_facts"), "{}", err);
    assert!(
        default_auth_limits().max_facts > budget.max_decision_facts,
        "the per-closure fact allowance is the looser of the two here, so only the \
         aggregate can refuse this"
    );
    assert_eq!(all_quads(&store), before);
    assert!(!grants_anything(&store));

    store.set_auth_budget(default_auth_budget());
    store
        .materialize_acp()
        .expect("materializes under the default budget");
    assert!(grants_anything(&store));
}

/// The budget a store is given is the one it applies, and it survives on the store across
/// materializations (the configuration seam [`PodStore::set_auth_budget`]).
#[test]
fn the_configured_budget_is_the_one_in_force() {
    let mut store = PodStore::new(Graph::default());
    assert_eq!(store.auth_budget(), default_auth_budget());
    let mut budget = default_auth_budget();
    budget.max_input_facts = 7;
    budget.max_document_bytes = 11;
    store.set_auth_budget(budget);
    assert_eq!(store.auth_budget(), budget);
    store
        .materialize_wac()
        .expect("an empty pod needs no budget");
    assert_eq!(
        store.auth_budget(),
        budget,
        "a materialization kept the budget"
    );
}

/// Every budget refusal joins the ONE refusal family the reasoning allowance uses, so a
/// caller that already tells an allowance refusal from a deny needs no second check. The
/// kinds are distinct, and each names the field it stands for.
#[test]
fn the_budget_refusal_kinds_are_distinct_and_named() {
    let kinds = [
        AuthAllowance::InputBytes,
        AuthAllowance::InputFacts,
        AuthAllowance::InputTermBytes,
        AuthAllowance::DecisionWork,
    ];
    let mut names: Vec<&str> = kinds.iter().map(|k| k.limit_name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), kinds.len(), "each kind names its own field(s)");
    assert_eq!(
        AuthAllowance::InputTermBytes.limit_name(),
        "max_input_term_bytes"
    );
    assert_ne!(
        AuthAllowance::InputTermBytes.limit_name(),
        AuthAllowance::TermBytes.limit_name(),
        "an INPUT term and a DERIVED term are different refusals"
    );
    // A budget refusal is a `ClosureLimits`-shaped one as far as any caller is concerned.
    let budget: AuthBudget = default_auth_budget();
    let limits: ClosureLimits = default_auth_limits();
    assert!(budget.max_decision_facts >= limits.max_facts);
}

/// The constraint-assembly budget of the ODRL parse (`sparq_policy`) reaches this crate's
/// bridge, and a policy past it is FAIL-CLOSED here: `materialize_odrl_n3` returns `Err`,
/// nothing is written to the graph, and no `auth:*` grant appears.
///
/// The bridge re-parses the policy through the Rust reference evaluator before it
/// materialises anything, so a document the parse refuses cannot be materialised by the N3
/// path alone.
#[cfg(feature = "odrl-bridge")]
#[test]
fn a_policy_past_the_constraint_budget_is_fail_closed_in_the_bridge() {
    use sparq_solid::odrl_bridge::materialize_odrl_n3;

    // A shared-subgraph combinator chain: linear in triples, 2^30 operand nodes assembled.
    let mut ttl = String::from(
        "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
         @prefix ex: <https://ex.dev/> .\n\
         ex:p a odrl:Set ; odrl:permission ex:r .\n\
         ex:r odrl:action odrl:read ; odrl:target ex:asset ; odrl:assignee ex:alice ;\n\
              odrl:constraint ex:root .\n",
    );
    for lvl in 0..30 {
        for side in ['a', 'b'] {
            let name = if lvl == 0 && side == 'a' {
                "ex:root".to_owned()
            } else {
                format!("ex:n{}{}", lvl, side)
            };
            ttl.push_str(&format!(
                "{} a odrl:LogicalConstraint ; odrl:and ex:n{}a, ex:n{}b .\n",
                name,
                lvl + 1,
                lvl + 1
            ));
        }
    }
    for side in ['a', 'b'] {
        ttl.push_str(&format!(
            "ex:n30{} odrl:leftOperand odrl:dateTime ; odrl:operator odrl:gt ;\n\
             \x20   odrl:rightOperand \"2000-01-01T00:00:00Z\" .\n",
            side
        ));
    }

    let req = sparq_policy::Request::new("http://www.w3.org/ns/odrl/2/read")
        .on("https://ex.dev/asset")
        .by("https://ex.dev/alice");
    let mut graph = Graph::default();
    let before = graph.named().len();
    let err = materialize_odrl_n3(&mut graph, &ttl, &req)
        .expect_err("a policy the reference parse refuses must not be materialised");
    assert!(
        err.contains("does not parse as an ODRL policy"),
        "the bridge refuses what the reference parse cannot read: {}",
        err
    );
    assert_eq!(
        sparq_policy::constraint_budget_refusal(&err),
        Some(sparq_policy::ConstraintBudget::Expansion),
        "the budget kind survives the bridge's own wrapping: {}",
        err
    );
    assert_eq!(
        graph.named().len(),
        before,
        "a refusal installed a named graph"
    );
    assert!(
        !graph
            .named()
            .iter()
            .any(|(n, _)| format!("{}", n).contains("urn:sparq:auth")),
        "a refusal installed an auth view"
    );
}

// ── The resource ENUMERATION (`max_resource_iri_bytes` / `max_resource_path_depth`) ──
//
// A resource of the view brings every container on its chain, each an owned prefix string,
// so one name of length L under D containers costs up to L x D bytes — a cost set by the
// pod's NAMES, paid while the input is assembled, and bounded by neither the documents'
// bytes (the names are not document content) nor the fact count (the strings exist before
// the facts they become). The two caps bound one name's L and D, and the aggregate
// `max_input_bytes` bounds the sum over all of them.

/// A WAC pod whose only resource graph is named `resource`, with an ACL granting alice
/// Read + Control on it: enough that the pod materializes and grants when the enumeration
/// admits the name.
fn pod_named(resource: &str) -> String {
    let acl = format!("{}.acl", resource);
    format!(
        "<{resource}#it> <https://ex.dev/ns#k> \"v\" <{resource}> .\n\
         <{acl}#a> <{RDF_TYPE}> <{ACL}Authorization> <{acl}> .\n\
         <{acl}#a> <{ACL}accessTo> <{resource}> <{acl}> .\n\
         <{acl}#a> <{ACL}agent> <{ALICE}> <{acl}> .\n\
         <{acl}#a> <{ACL}mode> <{ACL}Read> <{acl}> .\n\
         <{acl}#a> <{ACL}mode> <{ACL}Control> <{acl}> .\n"
    )
}

/// A budget whose resource caps admit any name, so a pod refused under the defaults can be
/// shown to be a pod the rest of the budget is fine with.
fn resource_caps_open() -> AuthBudget {
    let mut budget = default_auth_budget();
    budget.max_resource_iri_bytes = usize::MAX;
    budget.max_resource_path_depth = usize::MAX;
    budget
}

/// `nquads` materializes and grants under `budget`.
fn assert_materializes_under(nquads: &str, budget: AuthBudget, what: &str) {
    let mut store = fresh(nquads, budget, None);
    store.materialize_wac().unwrap_or_else(|e| panic!("{}: {}", what, e));
    assert!(grants_anything(&store), "{}: granted nothing", what);
}

/// ONE long resource IRI is refused — by its LENGTH, before the container chain whose cost
/// grows with it is built. The same pod materializes with the caps open, so the refusal is
/// the cap's and not the pod's.
#[test]
fn one_long_resource_iri_is_refused() {
    let nquads = pod_named(&format!("https://pod.ex/{}", "a".repeat(4_000)));
    assert_materializes_under(&nquads, resource_caps_open(), "with the resource caps open");
    let mut store = fresh(&nquads, default_auth_budget(), None);
    assert_refused(
        &mut store,
        AuthAllowance::ResourceIri,
        "a 4,000-byte resource IRI is past max_resource_iri_bytes",
    );
}

/// ONE DEEP resource IRI is refused on its container count, a dimension its length does not
/// catch: the name here is well inside `max_resource_iri_bytes` and still makes a chain
/// longer than `max_resource_path_depth` allows.
#[test]
fn one_deep_resource_iri_is_refused() {
    let depth = default_auth_budget().max_resource_path_depth + 8;
    let resource = format!("https://pod.ex/{}doc", "a/".repeat(depth));
    assert!(
        resource.len() < default_auth_budget().max_resource_iri_bytes,
        "the name is inside the LENGTH cap, so only the depth cap can refuse it"
    );
    let nquads = pod_named(&resource);
    assert_materializes_under(&nquads, resource_caps_open(), "with the resource caps open");
    let mut store = fresh(&nquads, default_auth_budget(), None);
    assert_refused(
        &mut store,
        AuthAllowance::ResourceIri,
        "a chain of more containers than max_resource_path_depth",
    );
}

/// Each name inside both caps, the ENUMERATION of them all past the aggregate: the strings
/// the resources and their containers own are charged against `max_input_bytes` as they are
/// made, so a pod cannot spend the aggregate budget many short names at a time either.
#[test]
fn the_resource_enumeration_is_charged_against_the_aggregate() {
    let mut nquads = String::new();
    for i in 0..200 {
        nquads.push_str(&format!(
            "<https://pod.ex/c{i}/r#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/c{i}/r> .\n"
        ));
    }
    nquads.push_str(&pod_named("https://pod.ex/root"));
    assert_materializes_by_default(&nquads);
    let mut budget = default_auth_budget();
    budget.max_input_bytes = 2_000;
    let mut store = fresh(&nquads, budget, None);
    let before = all_quads(&store);
    let err = store
        .materialize_wac()
        .expect_err("the enumeration alone is past the aggregate");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::InputBytes),
        "{}",
        err
    );
    assert!(
        err.contains("resources and their containers"),
        "the refusal names the enumeration, not a document: {}",
        err
    );
    assert_eq!(all_quads(&store), before, "a refusal installed something");
    assert!(!grants_anything(&store), "a refused pod granted something");
}

/// The caps are the store's CONFIGURATION, honoured as every other budget field is: a pod
/// the defaults admit is refused under a lowered cap, and the view in force is untouched.
#[test]
fn a_lowered_resource_cap_is_honoured() {
    let nquads = pod_named("https://pod.ex/a/b/c/d");
    assert_materializes_by_default(&nquads);
    let mut budget = default_auth_budget();
    budget.max_resource_path_depth = 2;
    let mut store = fresh(&nquads, budget, None);
    assert_refused(
        &mut store,
        AuthAllowance::ResourceIri,
        "four containers under a cap of two",
    );
    assert_eq!(store.auth_budget().max_resource_path_depth, 2);
}

/// The fixtures — the largest pods this crate has — are nowhere near the caps, and the view
/// they materialize is the SAME one they materialize with the caps open: the defaults bound
/// a hostile name without touching a legitimate pod.
#[test]
fn the_fixtures_are_unchanged_by_the_resource_caps() {
    for (label, nquads) in [
        ("wac", sparq_solid::wac_fixture()),
        ("wac50", sparq_solid::fixture::wac_fixture_sized(50)),
    ] {
        let mut capped = fresh(&nquads, default_auth_budget(), None);
        capped.materialize_wac().expect("the fixture is inside the caps");
        let mut open = fresh(&nquads, resource_caps_open(), None);
        open.materialize_wac().expect("and inside the rest of the budget");
        assert_eq!(
            all_quads(&capped),
            all_quads(&open),
            "{}: the resource caps changed the view",
            label
        );
        assert!(grants_anything(&capped), "{}: granted nothing", label);
    }
}

/// The ODRL N3 lane runs FIVE closures, and the ledger that bounds their aggregate is the
/// store's configured budget rather than a hardcoded default: a lowered one is honoured
/// here as it is on the WAC and ACP lanes, and the refusal is fail-closed (nothing is
/// written to the graph).
#[cfg(feature = "odrl-bridge")]
#[test]
fn the_odrl_n3_lane_honours_a_configured_decision_budget() {
    use sparq_solid::odrl_bridge::{materialize_odrl_n3, materialize_odrl_n3_with_budget};

    let ttl = "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
               <urn:pol/p> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; \
               odrl:target <https://pod.ex/n1> ; odrl:assignee <https://alice.ex/card#me> ] .";
    let req = sparq_policy::Request::new("http://www.w3.org/ns/odrl/2/read")
        .on("https://pod.ex/n1")
        .by(ALICE);

    let mut graph = Graph::default();
    materialize_odrl_n3(&mut graph, ttl, &req).expect("inside the default budget");

    let mut budget = default_auth_budget();
    budget.max_decision_facts = 1;
    let mut graph = Graph::default();
    let before = graph.named().len();
    let err = materialize_odrl_n3_with_budget(&mut graph, ttl, &req, budget)
        .expect_err("past the configured whole-decision budget");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::DecisionWork),
        "{}",
        err
    );
    assert_eq!(graph.named().len(), before, "a refusal wrote to the graph");
}

#[test]
fn generated_strings_have_a_separate_configurable_allowance() {
    let source = wac_pod(1, 8, 1);
    let mut budget = default_auth_budget();
    budget.max_generated_bytes = 256;
    let mut store = fresh(&source, budget, None);
    assert_refused(
        &mut store,
        AuthAllowance::GeneratedBytes,
        "generated strings need their own allowance",
    );
    assert_eq!(
        AuthAllowance::GeneratedBytes.limit_name(),
        "max_generated_bytes"
    );
    budget.max_generated_bytes = default_auth_budget().max_generated_bytes;
    store.set_auth_budget(budget);
    store.materialize_wac().unwrap();
    assert!(grants_anything(&store));
}

#[test]
fn default_budget_accepts_the_nested_resource_counterexample() {
    use std::fmt::Write;
    let mut source = String::new();
    for i in 0..6_000 {
        let resource = format!("https://pod.ex/a{}/d{i}", i / 16);
        writeln!(
            source,
            "<{resource}#it> <https://ex.dev/p> \"v\" <{resource}> ."
        )
        .unwrap();
    }
    source.push_str(&format!(
        "<https://pod.ex/.acl#a> <{RDF_TYPE}> <{ACL}Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <{ACL}default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <{ACL}agent> <{ALICE}> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <{ACL}mode> <{ACL}Read> <https://pod.ex/.acl> .\n"
    ));
    let mut store = fresh(&source, default_auth_budget(), None);
    store
        .materialize_wac()
        .expect("generated strings do not consume source bytes");
    let allowed = store.accessible(&alice(), Mode::Read);
    for i in 0..6_000 {
        let resource = format!("https://pod.ex/a{}/d{i}", i / 16);
        assert!(
            allowed.iter().any(|iri| iri.as_str() == resource),
            "{resource}"
        );
    }
    assert!(
        !store
            .decide(&Session::default(), "https://pod.ex/a0/d0", Mode::Read)
            .allow
    );
}

#[test]
fn rejected_document_rows_do_not_consume_assembled_facts() {
    let mut source = String::new();
    for i in 0..100 {
        source.push_str(&format!(
            "<https://else.ex/s> <urn:p> \"{i}\" <https://pod.ex/doc.acl> .\n"
        ));
    }
    let mut budget = default_auth_budget();
    // The rejected ACL contributes its governing-document and rejection markers only.
    budget.max_input_facts = 2;
    let mut store = fresh(&source, budget, None);
    store.materialize_wac().unwrap();
    assert!(!grants_anything(&store));
    budget.max_input_facts = 1;
    let mut store = fresh(&source, budget, None);
    assert_refused(
        &mut store,
        AuthAllowance::InputFacts,
        "assembled marker facts still count",
    );
}

#[test]
fn filtered_group_rows_do_not_consume_assembled_facts() {
    let mut source = format!(
        "<https://pod.ex/doc#it> <urn:p> \"v\" <https://pod.ex/doc> .\n\
         <https://pod.ex/doc.acl#a> <{RDF_TYPE}> <{ACL}Authorization> <https://pod.ex/doc.acl> .\n\
         <https://pod.ex/doc.acl#a> <{ACL}accessTo> <https://pod.ex/doc> <https://pod.ex/doc.acl> .\n\
         <https://pod.ex/doc.acl#a> <{ACL}mode> <{ACL}Read> <https://pod.ex/doc.acl> .\n\
         <https://pod.ex/doc.acl#a> <{ACL}agentGroup> <https://pod.ex/group#team> <https://pod.ex/doc.acl> .\n\
         <https://pod.ex/group#team> <http://www.w3.org/2006/vcard/ns#hasMember> <{ALICE}> <https://pod.ex/group> .\n"
    );
    let mut budget = default_auth_budget();
    budget.max_input_facts = 13;
    let mut clean = fresh(&source, budget, None);
    clean.materialize_wac().unwrap();
    assert!(
        clean
            .decide(&alice(), "https://pod.ex/doc", Mode::Read)
            .allow
    );
    for i in 0..100 {
        source.push_str(&format!(
            "<https://else.ex/s> <urn:p> \"{i}\" <https://pod.ex/group> .\n"
        ));
    }
    let mut store = fresh(&source, budget, None);
    store.materialize_wac().unwrap();
    assert_eq!(
        store.accessible(&alice(), Mode::Read),
        clean.accessible(&alice(), Mode::Read)
    );
}
