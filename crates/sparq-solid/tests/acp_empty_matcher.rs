//! [GPT-6] ACP §6.5: an attribute-free matcher never matches any context.

mod reference;

use reference::{RefDecision, RefMode};
use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

const DOC: &str = "https://pod.example/record";
const ACR: &str = "https://pod.example/record.acr";
const RECIPIENT: &str = "https://recipient.example/#me";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";

fn quad(subject: &str, predicate: &str, object: &str) -> String {
    format!("<{subject}> <{predicate}> <{object}> <{ACR}> .\n")
}

fn fixture(conditions: &str, attributes: &str) -> String {
    format!(
        r#"<{DOC}#it> <https://example.org/value> "private" <{DOC}> .
<{ACR}> <{ACP}resource> <{DOC}> <{ACR}> .
<{ACR}> <{ACP}accessControl> <{ACR}#control> <{ACR}> .
<{ACR}#control> <{ACP}apply> <{ACR}#policy> <{ACR}> .
<{ACR}#policy> <{ACP}allow> <http://www.w3.org/ns/auth/acl#Read> <{ACR}> .
<{ACR}#public> <{ACP}agent> <{ACP}PublicAgent> <{ACR}> .
<{ACR}#empty> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ACP}Matcher> <{ACR}> .
{conditions}{attributes}"#
    )
}

fn materialized(nq: &str) -> PodStore {
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
    store.materialize_acp().unwrap();
    store
}

fn assert_read(store: &PodStore, agent: Option<&str>, allowed: bool) {
    let session = Session {
        agent,
        ..Session::default()
    };
    assert_eq!(
        store
            .accessible(&session, Mode::Read)
            .iter()
            .any(|g| g.as_str() == DOC),
        allowed
    );
    let query = format!("SELECT ?value WHERE {{ GRAPH <{DOC}> {{ <{DOC}#it> <https://example.org/value> ?value }} }}");
    assert_eq!(
        store
            .query_as(&session, Mode::Read, &query)
            .unwrap()
            .rows
            .len(),
        usize::from(allowed)
    );
}

#[test]
fn empty_matchers_follow_all_any_and_none_semantics() {
    for (empty_combinator, public_combinator, expected) in [
        ("anyOf", None, false),
        ("allOf", None, false),
        ("noneOf", None, false),
        ("allOf", Some("anyOf"), false),
        ("anyOf", Some("allOf"), false),
        ("anyOf", Some("anyOf"), true),
        ("noneOf", Some("anyOf"), true),
    ] {
        let mut conditions = quad(
            &format!("{ACR}#policy"),
            &format!("{ACP}{empty_combinator}"),
            &format!("{ACR}#empty"),
        );
        if let Some(combinator) = public_combinator {
            conditions.push_str(&quad(
                &format!("{ACR}#policy"),
                &format!("{ACP}{combinator}"),
                &format!("{ACR}#public"),
            ));
        }
        let nq = fixture(&conditions, "");
        let store = materialized(&nq);
        for agent in [Some(RECIPIENT), None] {
            let reference = reference::acp::decide(
                &nq,
                &reference::acp::Request {
                    agent,
                    client: None,
                    mode: RefMode::Read,
                    resource: DOC,
                },
            )
            .unwrap();
            assert_eq!(
                reference,
                if expected {
                    RefDecision::Allow
                } else {
                    RefDecision::Deny
                }
            );
            assert_read(&store, agent, expected);
        }
    }
}

#[test]
fn deleting_last_matcher_attribute_revokes_cached_query_access() {
    let conditions = quad(
        &format!("{ACR}#policy"),
        &format!("{ACP}anyOf"),
        &format!("{ACR}#empty"),
    );
    let attribute = quad(&format!("{ACR}#empty"), &format!("{ACP}agent"), RECIPIENT);
    let mut store = materialized(&fixture(&conditions, &attribute));
    let body = format!("GRAPH <{ACR}> {{ <{ACR}#empty> <{ACP}agent> <{RECIPIENT}> }}");
    assert_read(&store, Some(RECIPIENT), true);
    assert_read(&store, None, false);

    sparq_engine::update_in_place(&mut store.graph, &format!("DELETE DATA {{ {body} }}")).unwrap();
    store.materialize_acp().unwrap();
    assert_read(&store, Some(RECIPIENT), false);
    assert_read(&store, None, false);

    // Rebuild from the stored graph as well: denial must not depend on a warm cache.
    let mut reopened = PodStore::new(store.graph.fork());
    reopened.materialize_acp().unwrap();
    assert_read(&reopened, Some(RECIPIENT), false);
    assert_read(&reopened, None, false);

    sparq_engine::update_in_place(&mut reopened.graph, &format!("INSERT DATA {{ {body} }}"))
        .unwrap();
    reopened.materialize_acp().unwrap();
    assert_read(&reopened, Some(RECIPIENT), true);
    assert_read(&reopened, None, false);
}

#[test]
fn replacing_acr_with_empty_matcher_revokes_cached_query_access() {
    let conditions = quad(
        &format!("{ACR}#policy"),
        &format!("{ACP}anyOf"),
        &format!("{ACR}#empty"),
    );
    let attribute = quad(&format!("{ACR}#empty"), &format!("{ACP}agent"), RECIPIENT);
    let mut store = materialized(&fixture(&conditions, &attribute));
    assert_read(&store, Some(RECIPIENT), true);
    assert_read(&store, None, false);

    let empty_acr = format!(
        r#"@prefix acp: <{ACP}> .
<{ACR}> acp:resource <{DOC}>; acp:accessControl <{ACR}#control> .
<{ACR}#control> acp:apply <{ACR}#policy> .
<{ACR}#policy> acp:allow <http://www.w3.org/ns/auth/acl#Read>; acp:anyOf <{ACR}#empty> .
<{ACR}#empty> a acp:Matcher ."#
    );
    store.put_acl_acp(ACR, &empty_acr, "turtle").unwrap();
    assert_read(&store, Some(RECIPIENT), false);
    assert_read(&store, None, false);
}
