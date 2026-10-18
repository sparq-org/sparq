//! The WALL-CLOCK deadline of one authorization materialization
//! ([`sparq_solid::AuthBudget::max_materialize_time`]).
//!
//! Every other field of the budget bounds a COUNT, which is what makes a refusal a property
//! of the pod and so reproducible. Counts do not bound the TIME they take, though: a pod
//! inside every count limit can still occupy the thread that materializes it for longer
//! than the caller has. The deadline is that backstop. It is taken once per
//! materialization, shared by every stage of it, and checked at the COARSE boundaries —
//! once per document and per resource while the input is assembled, once at each stratum
//! boundary, and once per closure round inside the reasoner
//! (`sparq_reason::ClosureLimits::deadline`, whose own round-boundary refusal is tested in
//! that crate).
//!
//! The refusal is fail-closed and typed like the rest of the family: no view is installed,
//! nothing new is granted, the view in force is left alone, and the error carries
//! [`AuthAllowance::Deadline`].

use sparq_core::Graph;
use sparq_solid::{
    auth_allowance_refusal, default_auth_budget, AuthAllowance, AuthBudget, Mode, PodStore, Session,
};

const ALICE: &str = "https://alice.ex/card#me";

fn alice() -> Session<'static> {
    Session {
        agent: Some(ALICE),
        ..Session::default()
    }
}

/// A budget whose every COUNT limit is the default and whose deadline is `d` — so a refusal
/// under it can only be the deadline's.
fn with_deadline(d: std::time::Duration) -> AuthBudget {
    let mut budget = default_auth_budget();
    budget.max_materialize_time = d;
    budget
}

/// The largest pod this crate ships, which materializes comfortably inside every default.
fn sized_fixture() -> String {
    sparq_solid::fixture::wac_fixture_sized(50)
}

fn fresh(nquads: &str, budget: AuthBudget) -> PodStore {
    let mut store = PodStore::new(Graph::load_dataset(nquads, "nquads").expect("fixture parses"));
    store.set_auth_budget(budget);
    store
}

/// Whether anything at all is granted — the view's presence, observed through `decide`.
fn grants_anything(store: &PodStore) -> bool {
    let graph = store.graph();
    graph.named.iter().any(|(name, _)| {
        if let oxrdf::Term::NamedNode(n) = name {
            store.decide(&alice(), n.as_str(), Mode::Read).allow
        } else {
            false
        }
    })
}

/// A materialization that is past its deadline is REFUSED, with the typed kind, and nothing
/// is installed.
///
/// A zero deadline is already past when the materialization starts, so this pins the
/// refusal itself: the stage names differ by where the run happened to be, the kind does
/// not.
#[test]
fn a_materialization_past_its_deadline_is_refused() {
    let mut store = fresh(&sized_fixture(), with_deadline(std::time::Duration::ZERO));

    let err = store
        .materialize_wac()
        .expect_err("a materialization past its deadline must be refused");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::Deadline),
        "the refusal is the typed deadline one: {}",
        err
    );
    assert!(
        err.contains("max_materialize_time"),
        "the refusal names the limit that refused it: {}",
        err
    );
    // Fail-closed: no view, so nothing is granted.
    assert!(
        !grants_anything(&store),
        "a refused materialization granted something"
    );
}

/// The SAME pod under the SAME count limits materializes when the deadline is the generous
/// default — so it is the deadline that refused it above, not the pod or any count.
#[test]
fn the_same_pod_materializes_under_the_default_deadline() {
    let nquads = sized_fixture();
    let mut refused = fresh(&nquads, with_deadline(std::time::Duration::ZERO));
    assert!(
        refused.materialize_wac().is_err(),
        "the zero deadline refuses"
    );

    let mut ok = fresh(&nquads, default_auth_budget());
    ok.materialize_wac()
        .expect("the same pod materializes under the default deadline");
    assert!(
        grants_anything(&ok),
        "the materialized view grants what the pod's documents state"
    );
}

/// The fixtures are nowhere near the default deadline, and the view they materialize under
/// it is the SAME one they materialize with the deadline switched off entirely: the default
/// bounds a run that hangs without touching a legitimate pod.
#[test]
fn the_fixtures_are_unchanged_by_the_default_deadline() {
    for (label, nquads) in [
        ("wac", sparq_solid::wac_fixture()),
        ("wac50", sized_fixture()),
    ] {
        let mut capped = fresh(&nquads, default_auth_budget());
        capped
            .materialize_wac()
            .expect("the fixture is inside the default deadline");
        // A deadline a century out is one no run reaches, which is as close to "off" as the
        // budget can express.
        let mut open = fresh(&nquads, with_deadline(std::time::Duration::from_secs(1 << 32)));
        open.materialize_wac()
            .expect("and inside an unreachable one");
        assert_eq!(
            quads(&capped),
            quads(&open),
            "{}: the deadline changed the view",
            label
        );
    }
}

/// Every quad of every stored graph, the reserved authorization view included.
fn quads(store: &PodStore) -> Vec<String> {
    let mut out = Vec::new();
    for (name, g) in std::iter::once((String::from("<default>"), store.graph())).chain(
        store
            .graph()
            .named
            .iter()
            .map(|(n, g)| (format!("{}", n), g)),
    ) {
        let scan = g.store.scan(&[None, None, None]);
        for r in scan.rows.iter() {
            let [s, p, o] = scan.to_spo(r);
            out.push(format!(
                "{} {} {} {}",
                name,
                g.dict.term(s),
                g.dict.term(p),
                g.dict.term(o)
            ));
        }
    }
    out.sort();
    out
}

/// A REBUILD past its deadline is refused the same way, and the view in force is left in
/// place: the pod keeps answering the decisions it answered before, rather than losing its
/// view to a slow rebuild.
#[test]
fn a_rebuild_past_its_deadline_keeps_the_view_in_force() {
    let nquads = sized_fixture();
    let mut store = fresh(&nquads, default_auth_budget());
    store.materialize_wac().expect("the fixture materializes");
    let granted: Vec<String> = store
        .graph()
        .named
        .iter()
        .filter_map(|(name, _)| match name {
            oxrdf::Term::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        })
        .filter(|iri| store.decide(&alice(), iri, Mode::Read).allow)
        .collect();
    assert!(!granted.is_empty(), "the fixture grants alice something");

    store.set_auth_budget(with_deadline(std::time::Duration::ZERO));
    let err = store
        .materialize_wac()
        .expect_err("a rebuild past its deadline must be refused");
    assert_eq!(auth_allowance_refusal(&err), Some(AuthAllowance::Deadline));
    for iri in &granted {
        assert!(
            store.decide(&alice(), iri, Mode::Read).allow,
            "the view in force was dropped by the refused rebuild: {}",
            iri
        );
    }
}
