//! The reasoning allowance of an authorization materialization.
//!
//! Every authorization closure — the WAC stratum and each of the three ACP strata — runs
//! under explicit [`ClosureLimits`] with FINITE `max_join_rows` (one round) and
//! `max_total_join_rows` (all rounds of that closure together), both of which the reasoner's
//! own default leaves unbounded. A pod whose policies would need more is REFUSED: the
//! materialization fails before anything is installed, the refusal says which limit it
//! reached, and it is told from every other materialization error by
//! [`is_auth_allowance_refusal`] so an operator can read "the allowance is too small for
//! this pod" rather than "these policies deny access". No partial or widened view survives
//! a refusal, and nothing is granted that the fail-closed path did not already guarantee.

use sparq_core::Graph;
use sparq_solid::{
    auth_allowance_refusal, default_auth_limits, is_auth_allowance_refusal, AuthAllowance,
    ClosureLimits, Mode, PodStore, Session, AUTH_ALLOWANCE_REFUSAL,
};

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ALICE: &str = "https://alice.ex/card#me";

fn alice() -> Session<'static> {
    Session {
        agent: Some(ALICE),
        ..Session::default()
    }
}

/// An allowance that bounds join work far below what any real pod needs, so the crate's own
/// fixtures exceed it.
fn tiny() -> ClosureLimits {
    let mut limits = default_auth_limits();
    limits.max_join_rows = 2_000;
    limits
}

/// An allowance whose PER-ROUND join-row bound is untouched but whose cumulative one is far
/// below what the fixtures spend over their rounds.
fn tiny_total() -> ClosureLimits {
    let mut limits = default_auth_limits();
    limits.max_total_join_rows = 2_000;
    limits
}

/// Every quad of every stored graph, the reserved authorization view included.
fn all_quads(store: &PodStore) -> Vec<String> {
    let mut out = Vec::new();
    for (name, g) in std::iter::once((String::from("<default>"), store.graph()))
        .chain(store.graph().named().iter().map(|(n, g)| (format!("{}", n), g)))
    {
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

/// Whether the store grants ANYTHING to anyone, across every mode: the sessions of the
/// fixture's principals plus an anonymous one.
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

/// The crate's largest fixtures, under an allowance far below what they need: each is
/// refused, the refusal names the limit it reached, nothing is installed and nothing is
/// granted. The same fixture materializes under the default allowance, so the refusal is the
/// allowance's doing and not the fixture's.
#[test]
fn a_pod_over_the_allowance_is_refused_naming_the_limit_and_grants_nothing() {
    for (name, nquads, acp) in [
        ("wac", sparq_solid::wac_fixture(), false),
        ("acp", sparq_solid::acp_fixture(), true),
    ] {
        let mut store = PodStore::new(Graph::load_dataset(&nquads, "nquads").expect("parses"));
        store.set_auth_limits(tiny());
        let before = all_quads(&store);
        let err = if acp {
            store.materialize_acp()
        } else {
            store.materialize_wac()
        }
        .expect_err("the fixture is over the allowance");

        assert_eq!(
            auth_allowance_refusal(&err),
            Some(AuthAllowance::JoinRows),
            "{}: {}",
            name,
            err
        );
        assert!(err.contains(AUTH_ALLOWANCE_REFUSAL), "{}: {}", name, err);
        assert!(
            err.contains("max_join_rows") && err.contains("2000"),
            "{}: the refusal names the limit it reached and its value: {}",
            name,
            err
        );
        assert_eq!(all_quads(&store), before, "{}: a refusal installed something", name);
        assert!(!grants_anything(&store), "{}: a refused pod granted something", name);

        // Raise the allowance back: the same pod materializes, so `tiny()` is what refused it.
        store.set_auth_limits(default_auth_limits());
        if acp {
            store.materialize_acp().expect("materializes under the default allowance");
        } else {
            store.materialize_wac().expect("materializes under the default allowance");
        }
        assert!(grants_anything(&store), "{}: the fixture does grant", name);
    }
}

/// A rebuild the store makes on its own — here the one an update that writes an access-control
/// document needs — is refused by the allowance too, and takes the same fail-closed route any
/// other failed rebuild takes: the update is refused, no quad changes, and the view in force
/// grants exactly what it granted before. Nothing is left wider than it was.
#[test]
fn a_rebuild_refused_by_the_allowance_leaves_the_view_no_wider() {
    let acl = "https://pod.ex/.acl";
    let doc = "https://pod.ex/d";
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let nquads = format!(
        "<{doc}#it> <https://ex.dev/ns#k> \"v\" <{doc}> .
<{acl}#a> <{rdf_type}> <{ACL}Authorization> <{acl}> .
<{acl}#a> <{ACL}accessTo> <https://pod.ex/> <{acl}> .
<{acl}#a> <{ACL}default> <https://pod.ex/> <{acl}> .
<{acl}#a> <{ACL}agent> <{ALICE}> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Read> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Write> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Control> <{acl}> .
"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nquads, "nquads").expect("parses"));
    store.materialize_wac().expect("materializes");
    let readable = store.accessible(&alice(), Mode::Read);
    assert!(!readable.is_empty(), "alice reads her pod");
    let before = all_quads(&store);

    // An allowance this pod's own rules cannot be rebuilt under.
    let mut limits = default_auth_limits();
    limits.max_join_rows = 1;
    store.set_auth_limits(limits);

    let rule = format!(
        "INSERT DATA {{ GRAPH <{acl}> {{ <{acl}#a> <{ACL}mode> <{ACL}Append> }} }}"
    );
    let err = store
        .update_as(&alice(), &rule)
        .expect_err("the rules it leaves cannot be materialized under this allowance");
    assert!(err.contains("cannot be materialized"), "{}", err);
    assert!(
        err.contains(AUTH_ALLOWANCE_REFUSAL) && err.contains("max_join_rows"),
        "the refused rebuild's reason is carried out to the caller: {}",
        err
    );
    assert_eq!(all_quads(&store), before, "a refused update changed a quad");
    assert_eq!(
        store.accessible(&alice(), Mode::Read),
        readable,
        "the view in force grants exactly what it granted"
    );

    // A direct re-materialization under the same allowance is refused as well, and keeps the
    // view it could not replace — nothing is installed half way.
    let err = store.materialize_wac().expect_err("refused");
    assert!(is_auth_allowance_refusal(&err), "{}", err);
    assert_eq!(all_quads(&store), before, "a refused re-materialization changed a quad");
    assert_eq!(store.accessible(&alice(), Mode::Read), readable);
}

/// The distinction the error carries: an allowance refusal is one kind of failure, every other
/// materialization error is not, and a pod whose policies simply grant nothing is no failure at
/// all. An operator reading these three apart is the point of
/// [`is_auth_allowance_refusal`].
#[test]
fn an_allowance_refusal_is_told_from_every_other_outcome() {
    // (1) A trusted creator fact naming a reserved principal: a materialization error, not
    //     an allowance refusal.
    let quiet = "<https://pod.ex/d#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/d> .\n";
    let mut store = PodStore::new(Graph::load_dataset(quiet, "nquads").expect("parses"));
    let mut prov = sparq_solid::AccessProvenance::new();
    prov.set_creator("https://pod.ex/d", "urn:sparq:not-a-principal");
    let err = store
        .materialize_acp_with(&prov)
        .expect_err("a reserved principal in trusted provenance is refused");
    assert!(
        !is_auth_allowance_refusal(&err),
        "an ordinary materialization error must not read as an allowance refusal: {}",
        err
    );

    // (2) A pod whose policies grant nobody anything: not an error at all.
    let mut store = PodStore::new(Graph::load_dataset(quiet, "nquads").expect("parses"));
    store.materialize_wac().expect("a pod that grants nothing materializes");
    assert!(!grants_anything(&store), "and grants nothing");

    // (3) The allowance refusal itself.
    let mut store =
        PodStore::new(Graph::load_dataset(&sparq_solid::wac_fixture(), "nquads").expect("parses"));
    store.set_auth_limits(tiny());
    let err = store.materialize_wac().expect_err("over the allowance");
    assert!(is_auth_allowance_refusal(&err), "{}", err);
}

/// The default allowance bounds join work at all — the one property the reasoner's own
/// default does not have — and leaves room far above what the crate's largest fixtures need
/// (the margins are recorded against the measurement in `materialize::default_auth_limits`).
#[test]
fn the_default_allowance_is_finite_and_well_above_the_fixtures() {
    let limits = default_auth_limits();
    assert!(
        limits.max_join_rows < usize::MAX && limits.max_total_join_rows < usize::MAX,
        "the authorization path must not inherit the reasoner's unbounded join-row defaults"
    );
    assert!(limits.max_rounds < usize::MAX && limits.max_facts < usize::MAX);
    // Measured needs of the largest fixtures: ~357,200 cumulative join rows, ~82,400 in one
    // round, ~15,900 facts, 11 rounds, ~1,200 bytes per derived term. Each default is at
    // least ten times its own measured need, and the cumulative bound is what caps a whole
    // materialization.
    assert!(limits.max_total_join_rows >= 3_572_000, "{}", limits.max_total_join_rows);
    assert!(limits.max_join_rows >= 824_350, "{}", limits.max_join_rows);
    assert!(limits.max_facts >= 158_470, "{}", limits.max_facts);
    assert!(limits.max_rounds >= 110, "{}", limits.max_rounds);
    assert!(limits.max_term_bytes >= 11_970, "{}", limits.max_term_bytes);
    // And the cumulative bound is tighter than the per-round one times the rounds, which is
    // the only thing the per-round bound alone would give.
    assert!(
        limits.max_total_join_rows < limits.max_join_rows.saturating_mul(limits.max_rounds),
        "the cumulative bound must actually bind"
    );
}

/// A store materializes under the allowance it was given, and reports it.
#[test]
fn a_store_reports_and_applies_the_allowance_it_was_given() {
    let mut store = PodStore::new(Graph::default());
    assert_eq!(store.auth_limits().max_join_rows, default_auth_limits().max_join_rows);
    store.set_auth_limits(tiny());
    assert_eq!(store.auth_limits().max_join_rows, 2_000);
    // An empty pod is inside any allowance.
    store.materialize_wac().expect("an empty pod materializes");
}

/// The free functions keep the default allowance: a caller holding a `Graph` directly gets the
/// same finite bound the store applies.
#[test]
fn the_free_functions_materialize_under_the_default_allowance() {
    let mut graph =
        Graph::load_dataset(&sparq_solid::wac_fixture(), "nquads").expect("parses");
    sparq_solid::materialize_wac(&mut graph).expect("the fixture is inside the default allowance");
    let mut graph = Graph::load_dataset(&sparq_solid::acp_fixture(), "nquads").expect("parses");
    sparq_solid::materialize_acp(&mut graph).expect("the fixture is inside the default allowance");
}

/// The marker alone is not what makes a refusal: the kind TAG is, and it is read wherever
/// it sits in the message. A message that merely mentions the wording is not a refusal.
#[test]
fn only_the_kind_tag_makes_a_refusal() {
    assert!(!is_auth_allowance_refusal(&format!("{}: no tag", AUTH_ALLOWANCE_REFUSAL)));
    assert!(!is_auth_allowance_refusal("update denied: no grant covers it"));
    assert_eq!(
        auth_allowance_refusal(&format!(
            "wrapped: {} [allowance=join-rows]: n3: …",
            AUTH_ALLOWANCE_REFUSAL
        )),
        Some(AuthAllowance::JoinRows)
    );
}

/// The tag is read only where the refusal WORDING precedes it, so an error that is not a
/// refusal but quotes the tag literal — inside a pod's own IRI, say — is not read as one.
#[test]
fn a_quoted_tag_literal_is_not_a_refusal() {
    assert_eq!(
        auth_allowance_refusal(
            "update denied: <https://pod.ex/x%20[allowance=join-rows]%20y> is not a resource"
        ),
        None,
        "a tag quoted inside an IRI is not a refusal"
    );
    assert!(!is_auth_allowance_refusal("n3: [allowance=input-bytes]: no wording"));
    // The real shape, wrapped in a sentence, still reads.
    assert_eq!(
        auth_allowance_refusal(&format!(
            "update denied: … ({} [allowance=input-bytes]: …); nothing was changed",
            AUTH_ALLOWANCE_REFUSAL
        )),
        Some(AuthAllowance::InputBytes)
    );
}

/// The CUMULATIVE allowance binds on its own: with the per-round bound left at its default,
/// a total far below what a fixture spends over its rounds refuses it, naming that limit.
#[test]
fn the_cumulative_allowance_refuses_a_pod_whose_rounds_add_up() {
    let mut store =
        PodStore::new(Graph::load_dataset(&sparq_solid::wac_fixture(), "nquads").expect("parses"));
    store.set_auth_limits(tiny_total());
    let before = all_quads(&store);
    let err = store.materialize_wac().expect_err("over the cumulative allowance");
    assert_eq!(auth_allowance_refusal(&err), Some(AuthAllowance::TotalJoinRows), "{}", err);
    assert!(err.contains("max_total_join_rows"), "{}", err);
    assert_eq!(all_quads(&store), before, "a refusal installed something");
    assert!(!grants_anything(&store), "a refused pod granted something");
    assert_eq!(AuthAllowance::TotalJoinRows.limit_name(), "max_total_join_rows");
}

/// A refusal the WRITE path wraps in its own sentence is still recognized: the kind is read
/// from the middle of the message, not from its start. This is the case a leading-prefix
/// check gets wrong.
#[test]
fn a_refusal_wrapped_by_the_update_path_is_still_recognized() {
    let acl = "https://pod.ex/.acl";
    let doc = "https://pod.ex/d";
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let nquads = format!(
        "<{doc}#it> <https://ex.dev/ns#k> \"v\" <{doc}> .
<{acl}#a> <{rdf_type}> <{ACL}Authorization> <{acl}> .
<{acl}#a> <{ACL}accessTo> <https://pod.ex/> <{acl}> .
<{acl}#a> <{ACL}default> <https://pod.ex/> <{acl}> .
<{acl}#a> <{ACL}agent> <{ALICE}> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Read> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Write> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Control> <{acl}> .
"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nquads, "nquads").expect("parses"));
    store.materialize_wac().expect("materializes");
    let readable = store.accessible(&alice(), Mode::Read);
    let before = all_quads(&store);

    let mut limits = default_auth_limits();
    limits.max_total_join_rows = 1;
    store.set_auth_limits(limits);
    let rule =
        format!("INSERT DATA {{ GRAPH <{acl}> {{ <{acl}#a> <{ACL}mode> <{ACL}Append> }} }}");
    let err = store.update_as(&alice(), &rule).expect_err("the rebuild is refused");

    // The update path's own sentence comes first, so the marker is NOT leading.
    assert!(!err.starts_with(AUTH_ALLOWANCE_REFUSAL), "{}", err);
    assert!(err.starts_with("update denied"), "{}", err);
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::TotalJoinRows),
        "the wrapped refusal still reads as one: {}",
        err
    );
    assert!(is_auth_allowance_refusal(&err), "{}", err);
    assert_eq!(all_quads(&store), before, "a refused update changed a quad");
    assert_eq!(store.accessible(&alice(), Mode::Read), readable);
}

/// Every kind the refusal can carry round-trips through its tag, and an unknown tag reads as
/// `Unspecified` rather than as "not a refusal".
#[test]
fn every_refusal_kind_round_trips() {
    for kind in [
        AuthAllowance::TotalJoinRows,
        AuthAllowance::JoinRows,
        AuthAllowance::RoundsOrFacts,
        AuthAllowance::TermBytes,
        AuthAllowance::Unspecified,
    ] {
        assert!(!kind.limit_name().is_empty());
    }
    assert_eq!(
        auth_allowance_refusal("update denied: … (authorization reasoning allowance exceeded \
                                [allowance=from-a-later-build]: n3: …); nothing was changed"),
        Some(AuthAllowance::Unspecified)
    );
    assert_eq!(auth_allowance_refusal("update denied: no grant covers it"), None);
}
