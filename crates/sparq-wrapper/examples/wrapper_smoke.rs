// sq-1rg2q: deterministic public-API correctness smoke.

use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_wrapper::Store;

fn main() {
    let graph = Graph::load_str(
        "@prefix ex: <http://example.org/> . ex:a ex:knows ex:b . ex:b ex:name \"B\" .",
        "turtle",
    )
    .expect("valid fixture");
    let store = Store::borrowed(&graph);
    let a = NamedNode::new("http://example.org/a").expect("valid IRI");
    let knows = NamedNode::new("http://example.org/knows").expect("valid IRI");
    let name = NamedNode::new("http://example.org/name").expect("valid IRI");
    let friend = store.node(a).out(&knows).next().expect("friend");
    let name_node = friend.out(&name).next().expect("name");
    let label = name_node.as_str().expect("string literal");
    assert_eq!(label, "B");
    println!("wrapper_smoke=ok");
}
