use sparq_core::Graph;
use sparq_solid::{PodStore, Session};
use sparq_solid::fixture::{wac_fixture, acp_fixture, ALICE};
use std::time::Instant;

fn main() {
    let mut wac = PodStore::new(Graph::load_dataset(&wac_fixture(), "nquads").unwrap());
    let mut acp = PodStore::new(Graph::load_dataset(&acp_fixture(), "nquads").unwrap());
    wac.materialize_wac().unwrap();
    acp.materialize_acp().unwrap();
    let session = Session { agent: Some(ALICE), ..Default::default() };
    let mut edits = 0;
    for line in std::io::stdin().lines() {
        let line = line.unwrap();
        if line == "quit" { break; }
        let start = Instant::now();
        match line.as_str() {
            "wac" => { std::hint::black_box(wac.materialize_wac().unwrap()); }
            "acp" => { std::hint::black_box(acp.materialize_acp().unwrap()); }
            "edit" => {
                edits += 1;
                let acl = "https://pod.ex/priv0/c0/.acl";
                let comment = "http://www.w3.org/2000/01/rdf-schema#comment";
                let edit = format!("DELETE {{ GRAPH <{acl}> {{ ?r <{comment}> ?c }} }} INSERT {{ GRAPH <{acl}> {{ ?r <{comment}> \"v{edits}\" }} }} WHERE {{ GRAPH <{acl}> {{ ?r <http://www.w3.org/ns/auth/acl#agent> <{ALICE}> OPTIONAL {{ ?r <{comment}> ?c }} }} }}");
                wac.update_as(&session, &edit).unwrap();
            }
            _ => panic!("unknown operation"),
        }
        println!("{}", start.elapsed().as_nanos());
    }
}
