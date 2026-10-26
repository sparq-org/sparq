//! Interactive materialization probe. Each input line is `bounded|unbounded wac|wac-sized|acp|edit`.
//! Emits elapsed nanoseconds; measurement orchestration and results stay outside the tree.
use sparq_core::Graph;
use sparq_solid::fixture::{acp_fixture, wac_fixture, ALICE};
use sparq_solid::{default_auth_budget, Mode, PodStore, Session};
use std::time::Instant;

fn main() {
    let mut wac = PodStore::new(Graph::load_dataset(&wac_fixture(), "nquads").unwrap());
    let mut acp = PodStore::new(Graph::load_dataset(&acp_fixture(), "nquads").unwrap());
    wac.materialize_wac().unwrap();
    acp.materialize_acp().unwrap();
    let session = Session {
        agent: Some(ALICE),
        ..Default::default()
    };
    let mut wac_sized = PodStore::new(
        Graph::load_dataset(&sparq_solid::fixture::wac_fixture_sized(50), "nquads").unwrap(),
    );
    let mut edits = 0;
    for line in std::io::stdin().lines() {
        let line = line.unwrap();
        if line == "quit" {
            break;
        }
        let (clock, operation) = line.split_once(' ').expect("clock and operation");
        let mut budget = default_auth_budget();
        match clock {
            "bounded" => {}
            "unbounded" => budget.max_materialize_time = None,
            _ => panic!("unknown clock setting"),
        }
        wac.set_auth_budget(budget);
        acp.set_auth_budget(budget);
        wac_sized.set_auth_budget(budget);
        let start = Instant::now();
        match operation {
            "wac" => {
                std::hint::black_box(wac.materialize_wac().unwrap());
            }
            "wac-sized" => {
                std::hint::black_box(wac_sized.materialize_wac().unwrap());
            }
            "acp" => {
                std::hint::black_box(acp.materialize_acp().unwrap());
            }
            "edit" => {
                edits += 1;
                let acl = "https://pod.ex/priv0/c0/.acl";
                let comment = "http://www.w3.org/2000/01/rdf-schema#comment";
                let edit = format!("DELETE {{ GRAPH <{acl}> {{ ?r <{comment}> ?c }} }} INSERT {{ GRAPH <{acl}> {{ ?r <{comment}> \"v{edits}\" }} }} WHERE {{ GRAPH <{acl}> {{ ?r <http://www.w3.org/ns/auth/acl#agent> <{ALICE}> OPTIONAL {{ ?r <{comment}> ?c }} }} }}");
                wac.update_as(&session, &edit).unwrap();
            }
            _ => panic!("unknown operation"),
        }
        let elapsed = start.elapsed().as_nanos();
        if operation == "edit" {
            let observed = format!("ASK {{ GRAPH <https://pod.ex/priv0/c0/.acl> {{ ?r <http://www.w3.org/2000/01/rdf-schema#comment> \"v{edits}\" }} }}");
            assert!(
                wac.ask_as(&session, Mode::Read, &observed).unwrap(),
                "edit must change the control document"
            );
        }
        println!("{elapsed}");
    }
}
