// Regression image only; never accepted as an authenticated-RDF evaluator.
#![no_main]

risc0_zkvm::guest::entry!(main);

type Outputs = (
    Result<String, String>,
    Result<String, String>,
    Result<String, String>,
);

fn outputs<D: sparq_canon::Digest>(text: &str) -> Outputs {
    let quads = match sparq_canon::parse_nquads(text) {
        Ok(quads) => quads,
        Err(error) => {
            let message = error.to_string();
            return (Err(message.clone()), Err(message.clone()), Err(message));
        }
    };
    let limits = sparq_canon::CanonicalizationLimits {
        max_quads: 100_000,
        max_input_bytes: 16 << 20,
        max_output_bytes: 16 << 20,
        max_hndq_calls: 4000,
        max_permutation_steps: 1 << 28,
    };
    (
        sparq_canon::canonicalize_quads_with::<D>(&quads).map_err(|e| e.to_string()),
        sparq_canon::canonicalize_quads_bounded_with::<D>(&quads, &limits)
            .map_err(|e| e.to_string()),
        sparq_canon::issue_quads_bounded_ordered_with::<D>(&quads, &limits)
            .map(|labels| {
                // Compare the issuer through its relabelled dataset: symmetric
                // nodes can have interchangeable labels in the canonical form.
                let relabel = |term: String| match term.strip_prefix("_:") {
                    Some(label) => format!("_:{}", labels[label]),
                    None => term,
                };
                let mut lines: Vec<_> = quads
                    .iter()
                    .map(|quad| {
                        let graph = if quad.graph_name.is_default_graph() {
                            String::new()
                        } else {
                            format!(" {}", relabel(quad.graph_name.to_string()))
                        };
                        format!(
                            "{} {} {}{graph} .\n",
                            relabel(quad.subject.to_string()),
                            quad.predicate,
                            relabel(quad.object.to_string())
                        )
                    })
                    .collect();
                lines.sort();
                lines.dedup();
                lines.concat()
            })
            .map_err(|e| e.to_string()),
    )
}

fn main() {
    let (text, sha384): (String, bool) = risc0_zkvm::guest::env::read();
    let output = if sha384 {
        outputs::<sha2::Sha384>(&text)
    } else {
        outputs::<sha2::Sha256>(&text)
    };
    risc0_zkvm::guest::env::commit(&output);
}
