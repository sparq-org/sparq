//! A control document that names a node no loaded document defines withholds access
//! through every referrer.
//!
//! An `.acr` may name an access control, a policy or a matcher defined in another
//! document. When that document IS loaded — in this pod or another — the reference applies
//! and the referring document delegates to whoever may edit it; that is the pod owner's
//! own choice and is exercised here too. When it is NOT loaded (another pod's document, or
//! a document a partial load left out), nothing is known about what it says: it may hold
//! an `acp:deny`, an `acp:noneOf` exception, or a condition on an allow. So the referring
//! document contributes nothing, like one the loader rejects, and the resources it governs
//! get no grant from any source.

use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const R: &str = "https://pod.ex/docs/r";
const ACR: &str = "https://pod.ex/docs/r.acr";
const R2: &str = "https://pod.ex/docs/r2";
const ACR2: &str = "https://pod.ex/docs/r2.acr";
/// A loaded document of this pod that defines shared policies and matchers.
const SHARED: &str = "https://pod.ex/shared.acr";
/// A loaded document of ANOTHER pod.
const CROSS: &str = "https://other.ex/shared.acr";
/// A document no fixture ever stores.
const ABSENT: &str = "https://pod.ex/absent.acr";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// One n-quad line per non-empty line of `body`, in graph `doc`.
fn doc(doc: &str, body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| format!("{} <{}> .\n", l.trim_end_matches('.').trim(), doc))
        .collect()
}

/// The two resources, as stored content.
fn content() -> String {
    [R, R2]
        .iter()
        .map(|r| format!("<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n"))
        .collect()
}

fn store(nq: &str) -> PodStore {
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture loads"));
    store
        .materialize_acp_with(&AccessProvenance::new())
        .expect("acp materializes");
    store
}

fn session(agent: Option<&str>) -> Session<'_> {
    Session {
        agent,
        ..Session::default()
    }
}

/// Whether `agent` reads `r` — by the per-resource decision, by the readable set, and by a
/// query over the resource's own graph, which must all agree.
fn reads(store: &PodStore, agent: Option<&str>, r: &str) -> bool {
    let s = session(agent);
    let decided = store.decide(&s, r, Mode::Read).allow;
    let listed = store
        .accessible(&s, Mode::Read)
        .iter()
        .any(|g| g.as_str() == r);
    let query = format!(
        "SELECT ?v WHERE {{ GRAPH <{}> {{ <{}#it> <https://ex.dev/ns#k> ?v }} }}",
        r, r
    );
    let rows = store
        .query_as(&s, Mode::Read, &query)
        .expect("query")
        .rows
        .len();
    assert_eq!(decided, listed, "decide and accessible disagree on {}", r);
    assert_eq!(decided, rows == 1, "decide and query disagree on {}", r);
    decided
}

/// A policy in `ACR` that allows Read to everyone, so something stands to be withheld.
fn public_read(name: &str) -> String {
    format!(
        "<{ACR}#{name}> <{ACP}allow> <{ACL}Read>
         <{ACR}#{name}> <{ACP}anyOf> <{ACR}#anyone>
         <{ACR}#anyone> <{ACP}agent> <{ACP}PublicAgent>"
    )
}

/// An `acp:noneOf` exception naming a matcher of a document that is not loaded: the
/// exception cannot be evaluated, so the allow it qualifies does not apply — and neither
/// does any other grant on the resource.
#[test]
fn a_none_of_naming_an_absent_documents_matcher_withholds_access() {
    let nq = content()
        + &doc(
            ACR,
            &format!(
                "<{ACR}> <{ACP}resource> <{R}>
                 <{ACR}> <{ACP}accessControl> <{ACR}#c>
                 <{ACR}#c> <{ACP}apply> <{ACR}#p>
                 <{ACR}#p> <{ACP}allow> <{ACL}Read>
                 <{ACR}#p> <{ACP}anyOf> <{ACR}#anyone>
                 <{ACR}#p> <{ACP}noneOf> <{ABSENT}#blocked>
                 <{ACR}#anyone> <{ACP}agent> <{ACP}PublicAgent>"
            ),
        );
    let s = store(&nq);
    assert!(
        !reads(&s, Some(ALICE), R),
        "an allow under an exception the view cannot read"
    );
    assert!(
        !reads(&s, None, R),
        "the same for an unauthenticated session"
    );
}

/// A policy of a document that is not loaded may deny. The resource gets no grant — not
/// from a second policy of its own `.acr`, and not from an allow that survives elsewhere
/// on the same resource (here its container's member access control).
#[test]
fn a_policy_in_an_absent_document_withholds_every_allow_on_the_resource() {
    let container = "https://pod.ex/docs/.acr";
    let nq = content()
        + &doc(
            ACR,
            &format!(
                "<{ACR}> <{ACP}resource> <{R}>
                 <{ACR}> <{ACP}accessControl> <{ACR}#c>
                 <{ACR}#c> <{ACP}apply> <{ABSENT}#p>
                 <{ACR}#c> <{ACP}apply> <{ACR}#own>
                 {}",
                public_read("own")
            ),
        )
        + &doc(
            container,
            &format!(
                "<{container}> <{ACP}memberAccessControl> <{container}#c>
                 <{container}#c> <{ACP}apply> <{container}#p>
                 <{container}#p> <{ACP}allow> <{ACL}Read>
                 <{container}#p> <{ACP}anyOf> <{container}#anyone>
                 <{container}#anyone> <{ACP}agent> <{ACP}PublicAgent>"
            ),
        )
        + &doc(
            ACR2,
            &format!(
                "<{ACR2}> <{ACP}resource> <{R2}>
                 <{ACR2}> <{ACP}accessControl> <{ACR2}#c>
                 <{ACR2}#c> <{ACP}apply> <{ACR2}#p>
                 <{ACR2}#p> <{ACP}allow> <{ACL}Read>
                 <{ACR2}#p> <{ACP}anyOf> <{ACR2}#anyone>
                 <{ACR2}#anyone> <{ACP}agent> <{ACP}PublicAgent>"
            ),
        );
    let s = store(&nq);
    assert!(
        !reads(&s, Some(ALICE), R),
        "a policy of a document the view cannot read"
    );
    assert!(
        !reads(&s, None, R),
        "the same for an unauthenticated session"
    );
    assert!(
        reads(&s, Some(ALICE), R2),
        "a sibling resource whose own references all resolve is untouched"
    );
}

/// The same for the two conjunctive/disjunctive matcher roles: a condition the view cannot
/// read is not treated as absent.
#[test]
fn an_all_of_or_any_of_matcher_in_an_absent_document_withholds_access() {
    for role in ["allOf", "anyOf"] {
        let nq = content()
            + &doc(
                ACR,
                &format!(
                    "<{ACR}> <{ACP}resource> <{R}>
                     <{ACR}> <{ACP}accessControl> <{ACR}#c>
                     <{ACR}#c> <{ACP}apply> <{ACR}#p>
                     <{ACR}#c> <{ACP}apply> <{ACR}#own>
                     <{ACR}#p> <{ACP}allow> <{ACL}Read>
                     <{ACR}#p> <{ACP}{role}> <{ABSENT}#m>
                     {}",
                    public_read("own")
                ),
            );
        let s = store(&nq);
        assert!(
            !reads(&s, Some(ALICE), R),
            "{}: a matcher of a document the view cannot read",
            role
        );
        assert!(!reads(&s, None, R), "{}: the same unauthenticated", role);
    }
}

/// The design point the fail-closed rule must not break: an `.acr` may name a policy of
/// another pod's document, and when that document IS loaded the policy applies. Naming it
/// delegates to whoever may edit that document — intended, and the pod owner's choice.
#[test]
fn a_loaded_cross_pod_policy_still_grants() {
    let nq = content()
        + &doc(
            ACR,
            &format!(
                "<{ACR}> <{ACP}resource> <{R}>
                 <{ACR}> <{ACP}accessControl> <{ACR}#c>
                 <{ACR}#c> <{ACP}apply> <{CROSS}#p>"
            ),
        )
        + &doc(
            CROSS,
            &format!(
                "<{CROSS}#p> <{ACP}allow> <{ACL}Read>
                 <{CROSS}#p> <{ACP}anyOf> <{CROSS}#m>
                 <{CROSS}#m> <{ACP}agent> <{ALICE}>"
            ),
        );
    let s = store(&nq);
    assert!(
        reads(&s, Some(ALICE), R),
        "a loaded cross-pod policy grants"
    );
    assert!(!reads(&s, Some(BOB), R), "and only to the agent it names");
}

/// One policy document, two `.acr`s applying it. While it is loaded both resources decide
/// by it; when the matcher it names is not loaded, both referrers withhold — the rule
/// propagates through a shared policy to every document that uses it.
#[test]
fn several_acrs_sharing_one_policy() {
    let acrs = |target: &str| {
        content()
            + &doc(
                ACR,
                &format!(
                    "<{ACR}> <{ACP}resource> <{R}>
                     <{ACR}> <{ACP}accessControl> <{ACR}#c>
                     <{ACR}#c> <{ACP}apply> <{SHARED}#p>"
                ),
            )
            + &doc(
                ACR2,
                &format!(
                    "<{ACR2}> <{ACP}resource> <{R2}>
                     <{ACR2}> <{ACP}accessControl> <{ACR2}#c>
                     <{ACR2}#c> <{ACP}apply> <{SHARED}#p>"
                ),
            )
            + &doc(
                SHARED,
                &format!(
                    "<{SHARED}#p> <{ACP}allow> <{ACL}Read>
                     <{SHARED}#p> <{ACP}anyOf> <{SHARED}#anyone>
                     <{SHARED}#p> <{ACP}noneOf> <{target}>
                     <{SHARED}#anyone> <{ACP}agent> <{ACP}PublicAgent>
                     <{SHARED}#bob> <{ACP}agent> <{BOB}>"
                ),
            )
    };
    let resolved = store(&acrs(&format!("{SHARED}#bob")));
    assert!(
        reads(&resolved, Some(ALICE), R),
        "r reads by the shared policy"
    );
    assert!(
        reads(&resolved, Some(ALICE), R2),
        "r2 reads by the same policy"
    );
    assert!(!reads(&resolved, Some(BOB), R), "and its exception applies");

    let s = store(&acrs(&format!("{ABSENT}#m")));
    assert!(
        !reads(&s, Some(ALICE), R),
        "r withheld through the shared policy"
    );
    assert!(
        !reads(&s, Some(ALICE), R2),
        "r2 withheld through the same policy"
    );
}

/// One matcher, two policies. Both decide by it while it is loaded; when its document is
/// not loaded, both policies' modes are withheld.
#[test]
fn one_matcher_reused_across_policies() {
    let acr = |target: &str| {
        content()
            + &doc(
                ACR,
                &format!(
                    "<{ACR}> <{ACP}resource> <{R}>
                     <{ACR}> <{ACP}accessControl> <{ACR}#c>
                     <{ACR}#c> <{ACP}apply> <{ACR}#read>
                     <{ACR}#c> <{ACP}apply> <{ACR}#write>
                     <{ACR}#read> <{ACP}allow> <{ACL}Read>
                     <{ACR}#read> <{ACP}anyOf> <{ACR}#anyone>
                     <{ACR}#read> <{ACP}noneOf> <{target}>
                     <{ACR}#write> <{ACP}allow> <{ACL}Write>
                     <{ACR}#write> <{ACP}anyOf> <{ACR}#anyone>
                     <{ACR}#write> <{ACP}noneOf> <{target}>
                     <{ACR}#anyone> <{ACP}agent> <{ACP}PublicAgent>"
                ),
            )
            + &doc(SHARED, &format!("<{SHARED}#m> <{ACP}agent> <{BOB}>"))
    };
    let resolved = store(&acr(&format!("{SHARED}#m")));
    let s = session(Some(ALICE));
    assert!(
        resolved.decide(&s, R, Mode::Read).allow,
        "read by the reused matcher"
    );
    assert!(
        resolved.decide(&s, R, Mode::Write).allow,
        "write by the reused matcher"
    );
    let bob = session(Some(BOB));
    assert!(
        !resolved.decide(&bob, R, Mode::Read).allow,
        "bob excepted from read"
    );
    assert!(
        !resolved.decide(&bob, R, Mode::Write).allow,
        "bob excepted from write"
    );

    let withheld = store(&acr(&format!("{ABSENT}#m")));
    assert!(!withheld.decide(&s, R, Mode::Read).allow, "read withheld");
    assert!(!withheld.decide(&s, R, Mode::Write).allow, "write withheld");
}

/// Writing such a reference into a control document would withhold that document too,
/// Control included, so the writer could never repair it: the write is refused. Storing the
/// referenced document first makes the same write acceptable.
#[test]
fn a_write_naming_a_node_of_an_unloaded_document_is_refused() {
    let mut s = store(&content());
    let body = format!(
        "<{ACR}> <{ACP}accessControl> <{ACR}#c> .
         <{ACR}#c> <{ACP}apply> <{SHARED}#p> .\n"
    );
    let err = s
        .put_acl_acp(ACR, &body, "ntriples")
        .expect_err("the referenced document is not stored");
    assert!(err.contains("may not use a node of another"), "{}", err);
    assert!(!reads(&s, Some(ALICE), R), "and nothing was changed");

    let shared = format!(
        "<{SHARED}#p> <{ACP}allow> <{ACL}Read> .
         <{SHARED}#p> <{ACP}anyOf> <{SHARED}#m> .
         <{SHARED}#m> <{ACP}agent> <{ALICE}> .\n"
    );
    s.put_acl_acp(SHARED, &shared, "ntriples")
        .expect("the policy document is stored");
    s.put_acl_acp(ACR, &body, "ntriples")
        .expect("now the reference resolves");
    assert!(reads(&s, Some(ALICE), R), "and the policy applies");
}

/// Before/after equivalence: a policy set whose every reference resolves — within the
/// document, to another document of this pod, and to another pod's — decides exactly as it
/// did before unresolved references were withheld, over the whole (agent × resource ×
/// mode) matrix.
#[test]
fn references_that_all_resolve_decide_unchanged() {
    let nq = content()
        + &doc(
            ACR,
            &format!(
                "<{ACR}> <{ACP}resource> <{R}>
                 <{ACR}> <{ACP}accessControl> <{ACR}#c>
                 <{ACR}#c> <{ACP}apply> <{ACR}#own>
                 <{ACR}#c> <{ACP}apply> <{SHARED}#append>
                 <{ACR}#c> <{ACP}apply> <{CROSS}#deny>
                 <{ACR}#own> <{ACP}allow> <{ACL}Read>
                 <{ACR}#own> <{ACP}anyOf> <{ACR}#anyone>
                 <{ACR}#own> <{ACP}noneOf> <{SHARED}#bob>
                 <{ACR}#anyone> <{ACP}agent> <{ACP}PublicAgent>"
            ),
        )
        + &doc(
            ACR2,
            &format!(
                "<{ACR2}> <{ACP}resource> <{R2}>
                 <{ACR2}> <{ACP}accessControl> <{ACR2}#c>
                 <{ACR2}#c> <{ACP}apply> <{SHARED}#write>
                 <{ACR2}#c> <{ACP}apply> <{ACR2}#p>
                 <{ACR2}#p> <{ACP}allow> <{ACL}Read>
                 <{ACR2}#p> <{ACP}anyOf> <{SHARED}#bob>"
            ),
        )
        + &doc(
            SHARED,
            &format!(
                "<{SHARED}#append> <{ACP}allow> <{ACL}Append>
                 <{SHARED}#append> <{ACP}anyOf> <{SHARED}#authed>
                 <{SHARED}#write> <{ACP}allow> <{ACL}Write>
                 <{SHARED}#write> <{ACP}allOf> <{SHARED}#bob>
                 <{SHARED}#bob> <{ACP}agent> <{BOB}>
                 <{SHARED}#authed> <{ACP}agent> <{ACP}AuthenticatedAgent>"
            ),
        )
        + &doc(
            CROSS,
            &format!(
                "<{CROSS}#deny> <{ACP}deny> <{ACL}Read>
                 <{CROSS}#deny> <{ACP}allOf> <{CROSS}#alice>
                 <{CROSS}#alice> <{ACP}agent> <{ALICE}>"
            ),
        );
    let s = store(&nq);
    let mut matrix = String::new();
    for agent in [Some(ALICE), Some(BOB), None] {
        for r in [R, R2] {
            for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
                matrix.push(if s.decide(&session(agent), r, mode).allow {
                    '1'
                } else {
                    '0'
                });
            }
            matrix.push(' ');
        }
        matrix.push('|');
    }
    assert_eq!(
        matrix,
        // alice: Read on r denied by the cross-pod policy, Append as an authenticated
        // agent; nothing on r2. bob: excepted from r's public Read by the shared matcher,
        // Append there, and Read + Write on r2 by the shared policies. unauthenticated:
        // public Read on r only.
        "0010 0000 |0010 1100 |1000 0000 |",
        "a policy set whose every reference resolves decides as before",
    );
}
