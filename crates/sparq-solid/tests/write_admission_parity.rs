use sparq_core::Graph;
use sparq_solid::{PodStore, Session};
const ALICE: &str = "https://alice.ex/me";
fn store() -> PodStore {
    let src = format!("<https://pod.ex/d#s> <urn:p> <urn:o> <https://pod.ex/d> .\n<https://pod.ex/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <{ALICE}> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .");
    let mut s = PodStore::new(Graph::load_dataset(&src, "nquads").unwrap());
    s.materialize_wac().unwrap();
    s
}
#[test]
fn default_budget_large_update_text_with_one_acl_triple() {
    let mut pod = store();
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    // Ordinary data: many small triples to a data graph, plus one ACL statement.
    let mut data = String::new();
    for i in 0..3000 {
        data += &format!("<https://pod.ex/d#s{i}> <urn:p> \"v{i}\" . ");
    }
    let q = format!("INSERT DATA {{ GRAPH <https://pod.ex/d> {{ {data} }} GRAPH <https://pod.ex/.acl> {{ <https://pod.ex/.acl#a> <urn:note> \"n\" }} }}");
    assert!(q.len() > 64 << 10, "{}", q.len());
    let r = pod.update_as(&session, &q);
    assert!(r.is_ok(), "{r:?}");
}
#[test]
fn default_budget_put_acl_with_large_comment() {
    let mut pod = store();
    let body = format!("# {}\n<https://pod.ex/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> .\n<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/> .\n<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <{ALICE}> .\n<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> .", "x".repeat(70_000));
    let r = pod.put_acl("https://pod.ex/.acl", &body, "turtle");
    assert!(r.is_ok(), "{r:?}");
}
#[test]
fn kind_for_one_oversized_term() {
    let mut pod = store();
    let body = format!(
        "<https://pod.ex/.acl#a> <urn:note> \"{}\" .",
        "x".repeat(70_000)
    );
    let r = pod.put_acl("https://pod.ex/.acl", &body, "ntriples");
    let e = r.unwrap_err();
    assert_eq!(
        sparq_solid::auth_allowance_refusal(&e),
        Some(sparq_solid::AuthAllowance::InputTermBytes)
    );
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    let q = format!("INSERT DATA {{ GRAPH <https://pod.ex/.acl> {{ <https://pod.ex/.acl#a> <urn:note> \"{}\" }} }}", "x".repeat(70_000));
    let e = pod.update_as(&session, &q).unwrap_err();
    assert_eq!(
        sparq_solid::auth_allowance_refusal(&e),
        Some(sparq_solid::AuthAllowance::InputTermBytes)
    );
}

#[test]
fn wac_updates_do_not_meter_inactive_acr_graphs() {
    let mut pod = store();
    let acl_doc = "https://pod.ex/unused.acr.acl";
    let acl = "http://www.w3.org/ns/auth/acl#";
    let body = format!("<{acl_doc}#a> a <{acl}Authorization>; <{acl}accessTo> <https://pod.ex/unused.acr>; <{acl}agent> <{ALICE}>; <{acl}mode> <{acl}Write> .");
    pod.graph_unchecked_mut()
        .add_named(
            oxrdf::NamedNode::new(acl_doc).unwrap().into(),
            Graph::load_str(&body, "turtle").unwrap(),
        )
        .unwrap();
    pod.materialize_wac().unwrap();
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    let query = format!("INSERT DATA {{ GRAPH <https://pod.ex/unused.acr> {{ <https://pod.ex/unused.acr#p> <urn:note> \"{}\" }} }}", "x".repeat(70_000));
    pod.update_as(&session, &query).unwrap();
    assert!(pod
        .graph()
        .named_graph(
            &oxrdf::NamedNode::new("https://pod.ex/unused.acr")
                .unwrap()
                .into()
        )
        .is_some());
    assert!(
        pod.decide(&session, "https://pod.ex/d", sparq_solid::Mode::Read)
            .allow
    );
}

#[test]
fn wac_put_does_not_meter_an_inactive_acr() {
    let mut pod = store();
    let body = format!(
        "<https://pod.ex/unused.acr#p> <urn:note> \"{}\" .",
        "x".repeat(70_000)
    );
    pod.put_acl("https://pod.ex/unused.acr", &body, "ntriples")
        .unwrap();
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    assert!(
        pod.decide(&session, "https://pod.ex/d", sparq_solid::Mode::Read)
            .allow
    );
}

#[test]
fn impossible_document_row_total_keeps_its_byte_refusal_kind() {
    let mut body = String::new();
    for i in 0..1000 {
        let value = if i == 0 {
            "x".repeat(70_000)
        } else {
            "small".into()
        };
        body += &format!("<https://pod.ex/.acl#r{i}> <urn:note> \"{value}\" .\n");
    }
    let mut pod = store();
    let error = pod
        .put_acl("https://pod.ex/.acl", &body, "ntriples")
        .unwrap_err();
    assert_eq!(
        sparq_solid::auth_allowance_refusal(&error),
        Some(sparq_solid::AuthAllowance::InputBytes)
    );
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    let query = format!("INSERT DATA {{ GRAPH <https://pod.ex/.acl> {{ {body} }} }}");
    let error = pod.update_as(&session, &query).unwrap_err();
    assert_eq!(
        sparq_solid::auth_allowance_refusal(&error),
        Some(sparq_solid::AuthAllowance::InputBytes)
    );
}
