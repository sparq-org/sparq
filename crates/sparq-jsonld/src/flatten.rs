//! Flattening Algorithm (JSON-LD 1.1 API §7.1).
//!
//! (sq-oy1f.26) Flattening turns a JSON-LD document into a single, deterministic
//! flat form: one `@graph` array of node objects, each node object labelled and appearing
//! exactly once, with every value re-parented under its subject (no node nested inside
//! another node's property — nested nodes become references). It is the normal form the
//! W3C `flatten` conformance category checks.
//!
//! The public entry is the [`flatten()`](fn@flatten) function: it expands `input` (via
//! [`crate::expand::expand`]), generates the [`NodeMap`](crate::node_map::NodeMap) (§7.2),
//! merges every named graph into its default-graph node under `@graph` (§7.1), sorts nodes
//! by `@id`, drops empty `{ "@id": … }`-only nodes, and returns the flattened **expanded**
//! document — an array of node objects.
//!
//! Post-flatten **compaction** against a caller `@context` (the `flatten(input, context)`
//! form that produces the `{ "@context": …, "@graph": [ … ] }` shape) is the document-level
//! Compaction Algorithm, delivered by its own bead (`sq-oy1f.27`); until it lands,
//! [`flatten()`](fn@flatten) emits the expanded flattened form only, and the conformance
//! lane SKIPs the single suite case that supplies a compaction context (recorded in the
//! flatten floor, not counted as a pass or a fail).
//!
//! Spec: <https://www.w3.org/TR/json-ld11-api/#flattening-algorithm>.

use crate::error::JsonLdError;
use crate::expand::expand;
use crate::json::Json;
use crate::loader::DocumentLoader;
use crate::node_map::generate_node_map;
use crate::options::JsonLdOptions;

/// **Flattening** (JSON-LD 1.1 API §7.1). Expands `input` against `options`, then returns
/// its flattened **expanded** form: a JSON array of node objects, one per subject, sorted by
/// `@id`, with named graphs folded into their default-graph node under `@graph`.
///
/// Remote `@context` / `@import` references reachable during the initial expansion are
/// dereferenced only through `loader` (deny-by-default via
/// [`NoopLoader`](crate::loader::NoopLoader)). Returns the first spec
/// [`JsonLdError`] raised by expansion on invalid input.
///
/// This is the un-compacted form. The compacted `flatten(input, context)` shape
/// (`{ "@context": …, "@graph": [ … ] }`) is produced by composing the document-level
/// Compaction Algorithm over this output (bead `sq-oy1f.27`).
///
/// (sq-oy1f.26)
pub fn flatten(
    input: &Json,
    options: &JsonLdOptions,
    loader: &dyn DocumentLoader,
) -> Result<Json, JsonLdError> {
    let expanded = expand(input, options, loader)?;
    Ok(flatten_expanded(&expanded))
}

/// **Flattening** over an already-expanded document (§7.1 steps 2–5). Splits out so callers
/// that already hold the expanded form (the conformance lane, `from_rdf`) skip re-expansion.
/// Returns the flattened expanded array.
///
/// (sq-oy1f.26)
pub fn flatten_expanded(expanded: &Json) -> Json {
    // §7.1 step 1–2: build the node map from the expanded input.
    let node_map = generate_node_map(expanded);

    // §7.1 step 3: fold every NON-default graph into its node in the default graph, under an
    // `@graph` member holding that graph's nodes sorted by @id (empty nodes dropped).
    // Collect the folds first (immutable borrow), then apply.
    let mut graph_folds: Vec<(String, Vec<Json>)> = Vec::new();
    for name in node_map.graph_names() {
        if name == "@default" {
            continue;
        }
        let Some(graph) = node_map.graph(name) else {
            continue;
        };
        let mut subjects: Vec<&str> = graph.subjects().collect();
        subjects.sort_unstable();
        let mut graph_nodes = Vec::new();
        for sub in subjects {
            if let Some(node) = graph.get(sub) {
                if !is_id_only(node) {
                    graph_nodes.push(node.clone());
                }
            }
        }
        graph_folds.push((name.to_string(), graph_nodes));
    }

    // Materialise the default graph's nodes, sorted by @id, then attach the folded @graph
    // arrays to the matching default-graph node (creating a bare node if the graph name has
    // no default-graph entry).
    let default = node_map.graph("@default");
    let mut default_subjects: Vec<String> = default
        .map(|g| g.subjects().map(str::to_string).collect())
        .unwrap_or_default();
    // Ensure every graph name that folds into the default graph has a node there.
    for (name, _) in &graph_folds {
        if !default_subjects.iter().any(|s| s == name) {
            default_subjects.push(name.clone());
        }
    }
    default_subjects.sort_unstable();

    let mut result = Vec::new();
    for sub in &default_subjects {
        // Start from the default-graph node object, or a bare `{ "@id": sub }`.
        let mut node = default
            .and_then(|g| g.get(sub))
            .cloned()
            .unwrap_or_else(|| Json::Obj(vec![("@id".to_string(), Json::Str(sub.clone()))]));
        // Attach this subject's folded named graph, if any.
        if let Some((_, graph_nodes)) = graph_folds.iter().find(|(n, _)| n == sub) {
            node.set("@graph", Json::Arr(graph_nodes.clone()));
        }
        // §7.1 step 4: drop nodes that carry only an @id (and no @graph fold).
        if is_id_only(&node) {
            continue;
        }
        result.push(node);
    }

    Json::Arr(result)
}

/// True iff `node` is a node object whose ONLY member is `@id` (the "empty node" the
/// flattening algorithm drops from the output).
fn is_id_only(node: &Json) -> bool {
    match node {
        Json::Obj(members) => members.len() == 1 && members[0].0 == "@id",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::NoopLoader;

    fn parse(s: &str) -> Json {
        Json::parse(s).expect("valid JSON fixture")
    }

    /// `flatten` over a single IRI-identified node with a value keeps the node and its value
    /// in expanded array form.
    #[test]
    fn flatten_single_node() {
        // Already-expanded input (flatten expands it again — expansion is idempotent on
        // expanded node objects with absolute IRIs).
        let input = parse(r#"[{"@id":"http://ex/a","http://ex/p":[{"@value":"v"}]}]"#);
        let out = flatten(&input, &JsonLdOptions::default(), &NoopLoader).expect("flatten ok");
        assert_eq!(
            out,
            parse(r#"[{"@id":"http://ex/a","http://ex/p":[{"@value":"v"}]}]"#)
        );
    }

    /// `flatten_expanded` folds a nested node into a separate top-level node, referenced by
    /// id, and sorts the two blank nodes by @id.
    #[test]
    fn flatten_expanded_nests_out() {
        let expanded = parse(r#"[{"@id":"_:b0","http://ex/p":[{"@id":"_:b1","http://ex/q":[{"@value":"v"}]}]}]"#);
        let out = flatten_expanded(&expanded);
        // Two top-level nodes, sorted by @id (_:b0 before _:b1).
        assert_eq!(
            out,
            parse(
                r#"[{"@id":"_:b0","http://ex/p":[{"@id":"_:b1"}]},{"@id":"_:b1","http://ex/q":[{"@value":"v"}]}]"#
            )
        );
    }

    /// `flatten_expanded` drops a node object that carries only an `@id` (an empty node).
    #[test]
    fn flatten_expanded_drops_id_only_node() {
        // A lone `{ "@id": … }` produces no properties ⇒ the node is dropped from the output.
        let expanded = parse(r#"[{"@id":"http://ex/a"}]"#);
        let out = flatten_expanded(&expanded);
        assert_eq!(out, Json::Arr(Vec::new()));
    }

    /// `flatten_expanded` folds a named graph into its default-graph node under `@graph`.
    #[test]
    fn flatten_expanded_named_graph_fold() {
        let expanded = parse(
            r#"[{"@id":"http://ex/g","@graph":[{"@id":"http://ex/a","http://ex/p":[{"@value":"x"}]}]}]"#,
        );
        let out = flatten_expanded(&expanded);
        assert_eq!(
            out,
            parse(
                r#"[{"@id":"http://ex/g","@graph":[{"@id":"http://ex/a","http://ex/p":[{"@value":"x"}]}]}]"#
            )
        );
    }
}
