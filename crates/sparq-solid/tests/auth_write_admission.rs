//! Control-write admission keeps all update entry points atomic at existing caps.
use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_solid::{
    auth_allowance_refusal, default_auth_budget, AuthAllowance, Mode, PodStore, Session,
};

const DOC: &str = "https://pod.ex/.acr";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ALICE: &str = "https://alice.ex/me";

type Entry = fn(&mut PodStore, &Session, &str) -> Result<(), String>;

fn store(acp: bool) -> PodStore {
    let source = format!(
        "<https://pod.ex/d#s> <urn:p> <urn:o> <https://pod.ex/d> .\n\
         <{DOC}> <{ACP}accessControl> <{DOC}#c> <{DOC}> .\n\
         <{DOC}> <{ACP}memberAccessControl> <{DOC}#c> <{DOC}> .\n\
         <{DOC}#c> <{ACP}apply> <{DOC}#p> <{DOC}> .\n\
         <{DOC}#p> <{ACP}allow> <http://www.w3.org/ns/auth/acl#Control> <{DOC}> .\n\
         <{DOC}#p> <{ACP}allow> <http://www.w3.org/ns/auth/acl#Read> <{DOC}> .\n\
         <{DOC}#p> <{ACP}allow> <http://www.w3.org/ns/auth/acl#Write> <{DOC}> .\n\
         <{DOC}#p> <{ACP}anyOf> <{DOC}#m> <{DOC}> .\n\
         <{DOC}#m> <{ACP}agent> <{ALICE}> <{DOC}> .\n\
         <{DOC}.acr> <{ACP}accessControl> <{DOC}.acr#c> <{DOC}.acr> .\n\
         <{DOC}.acr#c> <{ACP}apply> <{DOC}.acr#p> <{DOC}.acr> .\n\
         <{DOC}.acr#p> <{ACP}allow> <http://www.w3.org/ns/auth/acl#Write> <{DOC}.acr> .\n\
         <{DOC}.acr#p> <{ACP}anyOf> <{DOC}.acr#m> <{DOC}.acr> .\n\
         <{DOC}.acr#m> <{ACP}agent> <{ALICE}> <{DOC}.acr> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <{ALICE}> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> ."
    );
    let mut store = PodStore::new(Graph::load_dataset(&source, "nquads").unwrap());
    if acp {
        store.materialize_acp().unwrap();
    } else {
        store.materialize_wac().unwrap();
    }
    store
}

fn snapshot(store: &PodStore) -> Vec<String> {
    let mut out = Vec::new();
    for (name, graph) in std::iter::once((String::new(), store.graph())).chain(
        store
            .graph()
            .named()
            .iter()
            .map(|(n, g)| (n.to_string(), g)),
    ) {
        out.push(format!("graph {name}"));
        let scan = graph.store().scan(&[None, None, None]);
        out.extend(scan.rows.iter().map(|r| {
            let [s, p, o] = scan.to_spo(r);
            format!(
                "{name} {} {} {}",
                graph.dict().term(s),
                graph.dict().term(p),
                graph.dict().term(o)
            )
        }));
    }
    out.sort();
    out
}

#[test]
fn oversized_control_updates_are_refused_before_commit_for_every_entry_point() {
    let entries: &[(bool, Entry)] = &[
        (false, |p, s, q| p.update_as(s, q)),
        (true, |p, s, q| p.update_as_acp(s, q)),
        (false, |p, s, q| {
            p.update_as_with_budget(s, q, &QueryBudget::unlimited())
        }),
        (true, |p, s, q| {
            p.update_as_acp_with_budget(s, q, &QueryBudget::unlimited())
        }),
    ];
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    for &(acp, entry) in entries {
        for cap in ["document", "facts", "term", "dependencies"] {
            if !acp && cap == "dependencies" {
                continue;
            }
            let mut pod = store(acp);
            let doc = if acp { DOC } else { "https://pod.ex/.acl" };
            let mut triple = format!("<{doc}#p> <urn:pad> \"{}\"", "x".repeat(512));
            if cap == "dependencies" {
                let policy = Graph::load_str(
                    "<https://pod.ex/policy.acr#p> <urn:pad> \"policy\" .",
                    "ntriples",
                )
                .unwrap();
                pod.graph_unchecked_mut()
                    .add_named(
                        oxrdf::NamedNode::new("https://pod.ex/policy.acr")
                            .unwrap()
                            .into(),
                        policy,
                    )
                    .unwrap();
                pod.materialize_acp().unwrap();
                triple = format!("<{doc}#p> <{ACP}allOf> <https://pod.ex/policy.acr#m>");
            }
            let mut query = format!("INSERT DATA {{ GRAPH <{doc}> {{ {triple} }} }}");
            if cap == "document" {
                // Parsed terms fit. Only serialized input exceeds the configured cap,
                // so candidate materialization cannot mask a missing admission check.
                query = format!(
                    "#{}\n{query}",
                    "x".repeat(default_auth_budget().max_document_bytes)
                );
            }
            let before = snapshot(&pod);
            let mut budget = default_auth_budget();
            let kind = match cap {
                "document" => {
                    budget.max_document_bytes = query.len() - 1;
                    AuthAllowance::InputBytes
                }
                "facts" => {
                    budget.max_input_facts = 0;
                    AuthAllowance::InputFacts
                }
                "term" => {
                    budget.max_input_term_bytes = 256;
                    AuthAllowance::InputTermBytes
                }
                "dependencies" => {
                    // The modified ACR fits; its dependency consumes the remainder.
                    let graph = pod
                        .graph()
                        .named_graph(&oxrdf::NamedNode::new(doc).unwrap().into())
                        .unwrap();
                    let mut core =
                        sparq_core::meter::Ledger::new(sparq_core::meter::Limits::default());
                    let scan = graph.store().scan(&[None, None, None]);
                    for row in scan.rows.iter() {
                        for id in scan.to_spo(row) {
                            core.stored_term(graph.dict(), id).unwrap();
                        }
                    }
                    let terms = Graph::load_str(&format!("{triple} ."), "ntriples").unwrap();
                    let scan = terms.store().scan(&[None, None, None]);
                    for row in scan.rows.iter() {
                        for id in scan.to_spo(row) {
                            core.stored_term(terms.dict(), id).unwrap();
                        }
                    }
                    budget.max_input_bytes = core.spent(sparq_core::meter::Dim::Bytes);
                    AuthAllowance::InputBytes
                }
                _ => unreachable!(),
            };
            pod.set_auth_budget(budget);
            let error = entry(&mut pod, &session, &query).unwrap_err();
            assert_eq!(auth_allowance_refusal(&error), Some(kind), "{cap}: {error}");
            assert_eq!(
                snapshot(&pod),
                before,
                "{cap}: update committed before admission"
            );
            assert!(pod.decide(&session, "https://pod.ex/d", Mode::Read).allow);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn candidate_admission_keeps_its_deadline() {
    let mut pod = store(true);
    let before = snapshot(&pod);
    let mut budget = default_auth_budget();
    budget.max_materialize_time = Some(std::time::Duration::ZERO);
    pod.set_auth_budget(budget);
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    let error = pod
        .update_as_acp(
            &session,
            &format!("INSERT DATA {{ GRAPH <{DOC}> {{ <{DOC}#p> <urn:pad> \"small\" }} }}"),
        )
        .unwrap_err();
    assert_eq!(
        auth_allowance_refusal(&error),
        Some(AuthAllowance::Deadline),
        "{error}"
    );
    assert_eq!(snapshot(&pod), before);
}

#[test]
fn removal_only_control_updates_do_not_charge_serialized_input() {
    let entries: &[(bool, Entry)] = &[
        (false, |p, s, q| p.update_as(s, q)),
        (true, |p, s, q| p.update_as_acp(s, q)),
        (false, |p, s, q| {
            p.update_as_with_budget(s, q, &QueryBudget::unlimited())
        }),
        (true, |p, s, q| {
            p.update_as_acp_with_budget(s, q, &QueryBudget::unlimited())
        }),
    ];
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    for &(acp, entry) in entries {
        let mut pod = store(acp);
        let mut budget = default_auth_budget();
        // Remaining control documents fit; only the serialized removal is over this cap.
        budget.max_document_bytes = budget.max_input_term_bytes;
        pod.set_auth_budget(budget);
        let doc = if acp { DOC } else { "https://pod.ex/.acl" };
        let query = format!(
            "#{}\nCLEAR GRAPH <{doc}>",
            "x".repeat(2 * budget.max_document_bytes)
        );
        assert!(pod.decide(&session, "https://pod.ex/d", Mode::Read).allow);
        entry(&mut pod, &session, &query).unwrap();
        let graph = pod
            .graph()
            .named_graph(&oxrdf::NamedNode::new(doc).unwrap().into())
            .unwrap();
        assert!(graph.store().scan(&[None, None, None]).rows.is_empty());
        assert!(!pod.decide(&session, "https://pod.ex/d", Mode::Read).allow);
    }
}

#[test]
fn rejected_candidate_checks_dependencies_once_at_the_input_byte_boundary() {
    let mut pod = store(true);
    let policy = "https://pod.ex/policy.acr";
    let management = format!("{policy}.acr");
    let seed = format!(
        "INSERT DATA {{
          GRAPH <{DOC}> {{ <{DOC}#c> <{ACP}apply> <{policy}#p> }}
          GRAPH <{policy}> {{ <{policy}#p> <urn:pad> \"known-empty policy\" }}
          GRAPH <{management}> {{
            <{management}> <{ACP}accessControl> <{management}#c> .
            <{management}#c> <{ACP}apply> <{management}#p> .
            <{management}#p> <{ACP}allow> <http://www.w3.org/ns/auth/acl#Write>;
                <{ACP}anyOf> <{management}#m> .
            <{management}#m> <{ACP}agent> <{ALICE}>
          }}
        }}"
    );
    sparq_engine::update_in_place(pod.graph_unchecked_mut(), &seed).unwrap();
    pod.materialize_acp().unwrap();
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    assert!(pod
        .accessible(&session, Mode::Write)
        .iter()
        .any(|g| g.as_str() == policy));
    // The removal makes an existing root reference unresolved. The added reference
    // names a sound, still-loaded document and may be stored even though the root
    // is consequently withheld. Excluding that rejected root must not scan again.
    let query = format!(
        "DROP GRAPH <{policy}>; INSERT DATA {{ GRAPH <{DOC}> {{
          <{DOC}#p> <{ACP}allOf> <{DOC}.acr#m>
        }} }}"
    );
    let candidate = sparq_engine::update(pod.graph(), &query).unwrap();
    let mut size = sparq_core::meter::Ledger::new(sparq_core::meter::Limits::default());
    size.close_document();
    for (name, graph) in candidate.named() {
        let oxrdf::Term::NamedNode(name) = name else {
            continue;
        };
        if !name.as_str().ends_with(".acr") {
            continue;
        }
        size.charge(sparq_core::meter::Dim::Bytes, name.as_str().len())
            .unwrap();
        let scan = graph.store().scan(&[None, None, None]);
        for row in scan.rows.iter() {
            for id in scan.to_spo(row) {
                size.stored_term(graph.dict(), id).unwrap();
            }
        }
    }
    let mut budget = default_auth_budget();
    budget.max_input_bytes = size.spent(sparq_core::meter::Dim::Bytes);
    pod.set_auth_budget(budget);
    pod.update_as_acp(&session, &query)
        .expect("a single dependency scan fits exactly");
    assert!(pod
        .graph()
        .named_graph(&oxrdf::NamedNode::new(policy).unwrap().into())
        .is_none());
    assert!(
        !pod.decide(&session, "https://pod.ex/d", Mode::Read).allow,
        "the unresolved existing policy reference must keep the root withheld"
    );
}
