// [GPT-6 Astra] Issue #6475: pinned upstream replay of the saved CI counterexample.
// No sparq crate, bridge, parser, proptest, or engine is linked in this harness.
use oxrdf::{BlankNode, GraphName, NamedNode, Quad};
use std::error::Error;

fn dataset(labels: &[&str; 6]) -> Result<Vec<Quad>, Box<dyn Error>> {
    let b = labels
        .iter()
        .map(|label| BlankNode::new(*label))
        .collect::<Result<Vec<_>, _>>()?;
    let p = NamedNode::new("http://ex/p")?;
    let q = NamedNode::new("http://ex/q")?;
    Ok(vec![
        Quad::new(
            b[4].clone(),
            q.clone(),
            b[2].clone(),
            GraphName::BlankNode(b[3].clone()),
        ),
        Quad::new(b[4].clone(), p, b[0].clone(), GraphName::DefaultGraph),
        Quad::new(
            b[0].clone(),
            q,
            b[3].clone(),
            GraphName::BlankNode(b[2].clone()),
        ),
    ])
}

fn main() -> Result<(), Box<dyn Error>> {
    let original = dataset(&["b0", "b1", "b2", "b3", "b4", "b5"])?;
    let renamed = dataset(&["zz0", "zz1", "zz4", "zz3", "zz5", "zz2"])?;
    for (name, quads) in [("original", &original), ("renamed", &renamed)] {
        for quad in quads {
            println!("input_{name}: {quad} .");
        }
    }
    let control_original = rdf_canon::canonicalize_quads(&original[1..2])?;
    let control_renamed = rdf_canon::canonicalize_quads(&renamed[1..2])?;
    println!(
        "single_edge_control_equal={}",
        control_original == control_renamed
    );
    assert_eq!(
        control_original, control_renamed,
        "positive control must canonicalize"
    );
    let left = rdf_canon::canonicalize_quads(&original)?;
    let right = rdf_canon::canonicalize_quads(&renamed)?;
    println!("original_canonical={left:?}");
    println!("renamed_canonical={right:?}");
    println!("relabel_invariant={}", left == right);
    if left != right {
        return Err(
            "saved relabeling pair violates canonicalization invariance in direct rdf-canon".into(),
        );
    }
    Ok(())
}
