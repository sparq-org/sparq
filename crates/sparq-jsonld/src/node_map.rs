//! Node Map Generation (JSON-LD 1.1 API §7.2) + Merge Node Maps (§7.3) + the
//! Generate Blank Node Identifier helper (§7.4).
//!
//! [FABLE-5] (sq-oy1f.26) The **node map** is the shared intermediate that the
//! [`flatten`](mod@crate::flatten) module (and, later, `from_rdf`) projects from. It indexes
//! every node object of an **expanded** document by graph name and `@id`, assigns fresh
//! blank-node identifiers to unlabelled nodes and named graphs (in first-encounter order,
//! so the labels are deterministic), and re-materialises every value under its subject so
//! the graph is "flat": no value is nested inside another node's property.
//!
//! The public entry is [`generate_node_map`], which runs the recursive algorithm over an
//! expanded element and returns the finished [`NodeMap`]. The input MUST already be
//! expanded (the output of [`crate::expand::expand`]); node-map generation does not itself
//! expand terms or resolve contexts.
//!
//! Spec: <https://www.w3.org/TR/json-ld11-api/#node-map-generation>.

use crate::fx::{FxMap, FxSet};
use crate::json::Json;
use std::collections::BTreeMap;

/// A blank-node identifier issuer (§7.4 *Generate Blank Node Identifier*). Maps an input
/// identifier (a node's existing `@id`, or `None` for a truly anonymous node) to a freshly
/// minted `_:bN` label, minting labels in first-request order so the output is
/// deterministic. A previously seen identifier returns the same label.
#[derive(Default)]
pub struct BlankNodeIssuer {
    map: BTreeMap<String, String>,
    counter: usize,
}

impl BlankNodeIssuer {
    /// Constructs a fresh issuer whose first minted label is `_:b0`.
    pub fn new() -> Self {
        BlankNodeIssuer {
            map: BTreeMap::new(),
            counter: 0,
        }
    }

    /// **Generate Blank Node Identifier** (§7.4). Returns the label for `identifier`:
    /// - if `identifier` is `Some(id)` already issued, the previously minted label;
    /// - if `identifier` is `Some(id)` not yet seen, a fresh label recorded under `id`;
    /// - if `identifier` is `None` (an anonymous node), always a fresh label.
    pub fn issue(&mut self, identifier: Option<&str>) -> String {
        if let Some(id) = identifier {
            if let Some(existing) = self.map.get(id) {
                return existing.clone();
            }
        }
        // [FABLE-5] positional format arg (avoids the CodeQL rust/unused-variable
        // false positive on inline-captured identifiers).
        let label = format!("_:b{}", self.counter);
        self.counter += 1;
        if let Some(id) = identifier {
            self.map.insert(id.to_string(), label.clone());
        }
        label
    }
}

/// The set of node objects in one graph, keyed by subject (`@id`) and preserving
/// first-insertion order of the subjects (so blank-node subjects appear in the order
/// node-map generation created them — the sort into `@id` order happens later, in
/// flattening).
#[derive(Default)]
pub struct GraphMap {
    /// subject `@id` → its node object (a `Json::Obj`), in first-insertion order.
    nodes: Vec<(String, Json)>,
    /// subject `@id` → its position in `nodes`, so lookups don't scan.
    index: FxMap<String, usize>,
    /// (node position, property) → the serialized values of that property, kept once
    /// the property holds [`LINEAR_DEDUP`] values so duplicate checks stay constant-time.
    seen: FxMap<(usize, String), FxSet<String>>,
}

/// Up to this many values, a property's duplicate check scans its array.
const LINEAR_DEDUP: usize = 16;

/// A value's identity for duplicate checks: its serialization.
fn value_key(value: &Json) -> String {
    let mut s = String::new();
    value.write(&mut s);
    s
}

impl GraphMap {
    fn new() -> Self {
        GraphMap::default()
    }

    /// The subject ids present in this graph, in first-insertion order.
    pub fn subjects(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().map(|(k, _)| k.as_str())
    }

    /// Borrows the node object for `subject`, or `None` if absent.
    pub fn get(&self, subject: &str) -> Option<&Json> {
        self.index.get(subject).map(|&pos| &self.nodes[pos].1)
    }

    /// Ensures a node object exists for `subject`, creating `{ "@id": subject }` if absent,
    /// and returns a mutable reference to it (§7.2 the "reference a node" step).
    fn ensure(&mut self, subject: &str) -> &mut Json {
        let pos = self.position(subject);
        &mut self.nodes[pos].1
    }

    /// The position in `nodes` of `subject`'s node object, created as in [`Self::ensure`].
    fn position(&mut self, subject: &str) -> usize {
        match self.index.get(subject) {
            Some(&pos) => pos,
            None => {
                let node = Json::Obj(vec![("@id".to_string(), Json::Str(subject.to_string()))]);
                self.nodes.push((subject.to_string(), node));
                self.index.insert(subject.to_string(), self.nodes.len() - 1);
                self.nodes.len() - 1
            }
        }
    }

    /// Adds `value` to `subject`'s `property` array unless an equal value is already
    /// there; list objects are always added (equal lists are distinct list nodes).
    fn merge(&mut self, subject: &str, property: &str, value: Json) {
        let dedup = !is_list_object(&value);
        let pos = self.position(subject);
        let Json::Obj(members) = &mut self.nodes[pos].1 else { return };
        let Some(slot) = members.iter_mut().find(|(k, _)| k == property) else {
            members.push((property.to_string(), Json::Arr(vec![value])));
            return;
        };
        let Json::Arr(items) = &mut slot.1 else { return };
        if dedup {
            if items.len() < LINEAR_DEDUP {
                if items.contains(&value) {
                    return;
                }
            } else {
                let seen = self.seen.entry((pos, property.to_string())).or_insert_with(|| {
                    items.iter().filter(|v| !is_list_object(v)).map(value_key).collect()
                });
                if !seen.insert(value_key(&value)) {
                    return;
                }
            }
        }
        items.push(value);
    }
}

/// The finished node map: graph name (`@default` or a graph `@id`) → its [`GraphMap`],
/// preserving first-insertion order of graph names.
#[derive(Default)]
pub struct NodeMap {
    graphs: Vec<(String, GraphMap)>,
    /// graph name → its position in `graphs`.
    index: FxMap<String, usize>,
}

impl NodeMap {
    fn new() -> Self {
        let mut nm = NodeMap::default();
        nm.graph_mut("@default");
        nm
    }

    fn graph_mut(&mut self, name: &str) -> &mut GraphMap {
        let pos = match self.index.get(name) {
            Some(&pos) => pos,
            None => {
                self.graphs.push((name.to_string(), GraphMap::new()));
                self.index.insert(name.to_string(), self.graphs.len() - 1);
                self.graphs.len() - 1
            }
        };
        &mut self.graphs[pos].1
    }

    /// Borrows the [`GraphMap`] for graph `name`, or `None` if it has no entry.
    pub fn graph(&self, name: &str) -> Option<&GraphMap> {
        self.index.get(name).map(|&pos| &self.graphs[pos].1)
    }

    /// Consumes the map, yielding each graph's name and its `(subject, node)` pairs, in
    /// first-insertion order.
    pub fn into_graphs(self) -> impl Iterator<Item = (String, Vec<(String, Json)>)> {
        self.graphs.into_iter().map(|(name, g)| (name, g.nodes))
    }

    /// The graph names present, in first-insertion order (always includes `@default`).
    pub fn graph_names(&self) -> impl Iterator<Item = &str> {
        self.graphs.iter().map(|(k, _)| k.as_str())
    }
}

/// **Node Map Generation** (JSON-LD 1.1 API §7.2). Runs the algorithm over an already
/// **expanded** `element` and returns the finished [`NodeMap`] (with its `@default` graph
/// plus one entry per named graph). Blank-node identifiers are minted deterministically in
/// first-encounter order.
///
/// `element` must be the output of [`crate::expand::expand`] (an array of node
/// objects, value objects, or lists). Node-map generation performs no context processing.
///
/// [FABLE-5] (sq-oy1f.26)
pub fn generate_node_map(element: &Json) -> NodeMap {
    let mut node_map = NodeMap::new();
    let mut issuer = BlankNodeIssuer::new();
    // The top-level id (if the whole document is a single node) is not needed here.
    let _ = node_map_gen(
        element,
        &mut node_map,
        "@default",
        None,
        None,
        &mut None,
        &mut issuer,
    );
    node_map
}

/// The recursive core (§7.2 steps). `active_graph` is the graph currently being built;
/// `active_subject`/`active_property` are the node/property this element is a value of (both
/// `None` at the top level); `list` is `Some(items)` when we are collecting the members of a
/// `@list` (the algorithm's `list` reference argument).
///
/// Returns `Some(id)` when `element` was a node object (the id it was materialised under), so
/// the `@reverse` handler can attach the back-reference to the SAME node without minting a
/// duplicate. Returns `None` for arrays, value objects, list objects, and scalars.
fn node_map_gen(
    element: &Json,
    node_map: &mut NodeMap,
    active_graph: &str,
    active_subject: Option<&str>,
    active_property: Option<&str>,
    list: &mut Option<Vec<Json>>,
    issuer: &mut BlankNodeIssuer,
) -> Option<String> {
    match element {
        // step 1: an array — recurse over each item.
        Json::Arr(items) => {
            for item in items {
                node_map_gen(
                    item,
                    node_map,
                    active_graph,
                    active_subject,
                    active_property,
                    list,
                    issuer,
                );
            }
            None
        }
        Json::Obj(members) => {
            // A value object or a list object is a *value*, not a node — append it to the
            // active subject's property (or the active list), unchanged for a value object,
            // recursively flattened for a list.
            if is_value_object(members) {
                let value = element.clone();
                append_value(
                    node_map, active_graph, active_subject, active_property, list, value,
                );
                return None;
            }
            if let Some(list_items) = members.iter().find(|(k, _)| k == "@list").map(|(_, v)| v) {
                // step 6: a list object. Generate a fresh list, recurse its items into it,
                // then append `{ "@list": result }` to the active property.
                let mut inner: Option<Vec<Json>> = Some(Vec::new());
                node_map_gen(
                    list_items,
                    node_map,
                    active_graph,
                    active_subject,
                    active_property,
                    &mut inner,
                    issuer,
                );
                let result = Json::Obj(vec![(
                    "@list".to_string(),
                    Json::Arr(inner.unwrap_or_default()),
                )]);
                append_value(
                    node_map,
                    active_graph,
                    active_subject,
                    active_property,
                    list,
                    result,
                );
                return None;
            }

            // Otherwise `element` is a node object (step 5). Determine/mint its @id.
            let id = node_identifier(members, issuer);

            // Add a reference `{ "@id": id }` to the active subject's property (or list)
            // when this node is nested under one (step 5.2 / 5.3).
            if active_subject.is_some() || list.is_some() {
                let reference = Json::Obj(vec![("@id".to_string(), Json::Str(id.clone()))]);
                append_value(
                    node_map,
                    active_graph,
                    active_subject,
                    active_property,
                    list,
                    reference,
                );
            }

            // Ensure the node exists in the active graph (step 5.4).
            node_map.graph_mut(active_graph).ensure(&id);

            process_node_members(members, node_map, active_graph, &id, issuer);
            Some(id)
        }
        // A bare scalar (string/raw) reaching here is a value under a property — append it.
        _ => {
            append_value(
                node_map,
                active_graph,
                active_subject,
                active_property,
                list,
                element.clone(),
            );
            None
        }
    }
}

/// Determine the `@id` of a node object, minting a blank-node label when absent or when it
/// is already a blank node (so the whole document shares one issuer namespace).
fn node_identifier(members: &[(String, Json)], issuer: &mut BlankNodeIssuer) -> String {
    match members.iter().find(|(k, _)| k == "@id").map(|(_, v)| v) {
        Some(Json::Str(id)) if id.starts_with("_:") => issuer.issue(Some(id)),
        Some(Json::Str(id)) => id.clone(),
        _ => issuer.issue(None),
    }
}

/// Process every member of a node object (steps 5.5–5.7), re-parenting values under the
/// node's entry in `active_graph`, recursing `@type`, `@reverse`, `@included`, `@graph`, and
/// ordinary properties.
fn process_node_members(
    members: &[(String, Json)],
    node_map: &mut NodeMap,
    active_graph: &str,
    id: &str,
    issuer: &mut BlankNodeIssuer,
) {
    for (key, value) in members {
        match key.as_str() {
            "@id" => {}
            // step 5.5: @type — relabel blank-node types via the issuer, then merge into the
            // node's @type array (deduplicated).
            "@type" => {
                let relabelled = relabel_types(value, issuer);
                for t in relabelled {
                    merge_into_property(node_map, active_graph, id, "@type", t);
                }
            }
            // step 5.6: @included — a set of node objects to fold into this graph.
            "@included" => {
                node_map_gen(
                    value,
                    node_map,
                    active_graph,
                    None,
                    None,
                    &mut None,
                    issuer,
                );
            }
            // step 5.7 (@reverse): each reverse property points from its objects back at
            // this node. Materialise each reverse value ONCE via node-map generation
            // (which mints its id and captures its own properties), then attach the
            // back-reference `{ "@id": id }` to that SAME node under the reverse property.
            // (Minting via a separate `node_identifier` call AND recursing would create a
            // duplicate node — the 0039 bug.)
            "@reverse" => {
                if let Json::Obj(reverse_map) = value {
                    let reference =
                        Json::Obj(vec![("@id".to_string(), Json::Str(id.to_string()))]);
                    for (property, values) in reverse_map {
                        for v in as_array(values) {
                            // Recurse as a top-level node (no active subject/property): this
                            // mints the reverse object's id and folds its own members in.
                            if let Some(sub) = node_map_gen(
                                &v,
                                node_map,
                                active_graph,
                                None,
                                None,
                                &mut None,
                                issuer,
                            ) {
                                merge_into_property(
                                    node_map,
                                    active_graph,
                                    &sub,
                                    property,
                                    reference.clone(),
                                );
                            }
                        }
                    }
                }
            }
            // step 5.8 (@graph): recurse into a new graph named by this node's @id.
            "@graph" => {
                node_map.graph_mut(id);
                node_map_gen(value, node_map, id, None, None, &mut None, issuer);
            }
            // @index on a node object is preserved verbatim on the node.
            "@index" => {
                set_scalar_member(node_map, active_graph, id, "@index", value.clone());
            }
            // step 5.9: an ordinary property. Ensure it exists (even if empty), then recurse
            // its values as being under (active_graph, id, key).
            _ => {
                ensure_property(node_map, active_graph, id, key);
                let mut no_list = None;
                node_map_gen(
                    value,
                    node_map,
                    active_graph,
                    Some(id),
                    Some(key),
                    &mut no_list,
                    issuer,
                );
            }
        }
    }
}

/// Relabel a node's `@type` value(s), minting fresh labels for blank-node types.
fn relabel_types(value: &Json, issuer: &mut BlankNodeIssuer) -> Vec<Json> {
    as_array(value)
        .into_iter()
        .map(|t| match &t {
            Json::Str(s) if s.starts_with("_:") => Json::Str(issuer.issue(Some(s))),
            _ => t,
        })
        .collect()
}

/// Append `value` to wherever the recursion is currently writing: into the active `list`
/// accumulator if one is open, otherwise onto `active_subject`'s `active_property` array in
/// `active_graph`. A top-level value with no subject and no list is dropped (it cannot occur
/// for a well-formed expanded document).
fn append_value(
    node_map: &mut NodeMap,
    active_graph: &str,
    active_subject: Option<&str>,
    active_property: Option<&str>,
    list: &mut Option<Vec<Json>>,
    value: Json,
) {
    if let Some(items) = list.as_mut() {
        items.push(value);
        return;
    }
    if let (Some(subject), Some(property)) = (active_subject, active_property) {
        merge_into_property(node_map, active_graph, subject, property, value);
    }
}

/// Ensure `subject` has a `property` member in `active_graph` initialised to an empty array
/// (§7.2 the "create a new property" step, so an empty list-valued property survives).
fn ensure_property(node_map: &mut NodeMap, active_graph: &str, subject: &str, property: &str) {
    let node = node_map.graph_mut(active_graph).ensure(subject);
    if node.get(property).is_none() {
        node.set(property, Json::Arr(Vec::new()));
    }
}

/// Merge `value` into `subject`'s `property` array in `active_graph`, avoiding duplicate
/// entries (the algorithm's "if node does not already contain value" guard). A **list
/// object** (`{ "@list": … }`) is exempt from the dedup: two structurally-equal lists are
/// distinct list nodes and must both be retained (the `list-equivalence` case #t0042).
fn merge_into_property(
    node_map: &mut NodeMap,
    active_graph: &str,
    subject: &str,
    property: &str,
    value: Json,
) {
    node_map.graph_mut(active_graph).merge(subject, property, value);
}

/// True iff `value` is a list object (`{ "@list": … }`).
fn is_list_object(value: &Json) -> bool {
    matches!(value, Json::Obj(members) if members.iter().any(|(k, _)| k == "@list"))
}

/// Set a scalar (non-array) member such as `@index` on `subject`'s node in `active_graph`.
fn set_scalar_member(
    node_map: &mut NodeMap,
    active_graph: &str,
    subject: &str,
    key: &str,
    value: Json,
) {
    node_map
        .graph_mut(active_graph)
        .ensure(subject)
        .set(key, value);
}

/// Normalise a value to an array of its items (a plain value becomes a one-element array).
fn as_array(value: &Json) -> Vec<Json> {
    match value {
        Json::Arr(items) => items.clone(),
        other => vec![other.clone()],
    }
}

/// True iff `members` describe a JSON-LD value object (it has an `@value` member).
fn is_value_object(members: &[(String, Json)]) -> bool {
    members.iter().any(|(k, _)| k == "@value")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Json {
        Json::parse(s).expect("valid JSON fixture")
    }

    /// A subject with many values keeps each distinct value once, every list, and
    /// insertion order, without a quadratic duplicate scan.
    #[test]
    fn many_values_dedup_in_linear_time() {
        let mut values: Vec<String> = (0..40_000).map(|i| format!(r#"{{"@id":"http://ex/o{}"}}"#, i % 20_000)).collect();
        values.push(r#"{"@list":[{"@value":1}]}"#.to_string());
        values.push(r#"{"@list":[{"@value":1}]}"#.to_string());
        let doc = parse(&format!(r#"[{{"@id":"http://ex/s","http://ex/p":[{}]}}]"#, values.join(",")));
        let started = std::time::Instant::now();
        let nm = generate_node_map(&doc);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let p = nm.graph("@default").and_then(|g| g.get("http://ex/s")).and_then(|n| n.get("http://ex/p")).unwrap();
        let Json::Arr(items) = p else { panic!() };
        assert_eq!(items.len(), 20_002);
        assert_eq!(items[0].get("@id").and_then(Json::as_str), Some("http://ex/o0"));
        assert_eq!(items[19_999].get("@id").and_then(Json::as_str), Some("http://ex/o19999"));
    }

    /// `BlankNodeIssuer::new` mints `_:b0` first and reuses labels per input id.
    #[test]
    fn blank_node_issuer_new_mints_and_reuses() {
        let mut issuer = BlankNodeIssuer::new();
        assert_eq!(issuer.issue(None), "_:b0");
        assert_eq!(issuer.issue(Some("_:orig")), "_:b1");
        // Same input id ⇒ same label.
        assert_eq!(issuer.issue(Some("_:orig")), "_:b1");
        // Anonymous ⇒ always fresh.
        assert_eq!(issuer.issue(None), "_:b2");
    }

    /// `BlankNodeIssuer::issue` on a `Default` issuer also starts at `_:b0`.
    #[test]
    fn blank_node_issuer_default_starts_at_zero() {
        let mut issuer = BlankNodeIssuer::default();
        assert_eq!(issuer.issue(None), "_:b0");
    }

    /// `generate_node_map` places an IRI-identified node into the `@default` graph and its
    /// `graph`/`graph_names`/`subjects`/`get` accessors expose it.
    #[test]
    fn generate_node_map_indexes_default_graph_node() {
        let expanded = parse(r#"[{"@id":"http://ex/a","http://ex/p":[{"@value":"v"}]}]"#);
        let nm = generate_node_map(&expanded);
        assert_eq!(nm.graph_names().collect::<Vec<_>>(), vec!["@default"]);
        let g = nm.graph("@default").expect("default graph present");
        assert_eq!(g.subjects().collect::<Vec<_>>(), vec!["http://ex/a"]);
        let node = g.get("http://ex/a").expect("node present");
        assert_eq!(
            node.get("http://ex/p"),
            Some(&Json::Arr(vec![Json::Obj(vec![(
                "@value".to_string(),
                Json::Str("v".to_string())
            )])]))
        );
    }

    /// `graph`/`get` return `None` for absent names/subjects.
    #[test]
    fn node_map_accessors_absent() {
        let nm = generate_node_map(&parse("[]"));
        assert!(nm.graph("http://ex/missing").is_none());
        // @default always exists but is empty.
        let g = nm.graph("@default").expect("default graph");
        assert!(g.get("http://ex/x").is_none());
        assert_eq!(g.subjects().count(), 0);
    }

    /// A blank-node node object is minted `_:b0`; a nested node it references gets `_:b1`
    /// and lands as a separate top-level node (the flattening property).
    #[test]
    fn generate_node_map_flattens_nested_blank_nodes() {
        // Expanded form of `{ "p": { "q": "v" } }` under @vocab http://ex/.
        let expanded = parse(r#"[{"http://ex/p":[{"http://ex/q":[{"@value":"v"}]}]}]"#);
        let nm = generate_node_map(&expanded);
        let g = nm.graph("@default").expect("default graph");
        let subs: Vec<&str> = g.subjects().collect();
        assert_eq!(subs, vec!["_:b0", "_:b1"]);
        // The outer node references the inner by id.
        let outer = g.get("_:b0").unwrap();
        assert_eq!(
            outer.get("http://ex/p"),
            Some(&Json::Arr(vec![Json::Obj(vec![(
                "@id".to_string(),
                Json::Str("_:b1".to_string())
            )])]))
        );
    }

    /// A `@graph` member creates a named-graph entry keyed by the enclosing node's id.
    #[test]
    fn generate_node_map_creates_named_graph_entry() {
        let expanded = parse(
            r#"[{"@id":"http://ex/g","@graph":[{"@id":"http://ex/a","http://ex/p":[{"@value":"x"}]}]}]"#,
        );
        let nm = generate_node_map(&expanded);
        let names: Vec<&str> = nm.graph_names().collect();
        assert!(
            names.contains(&"http://ex/g"),
            "named graph created: {:?}",
            names
        );
        let g = nm.graph("http://ex/g").expect("named graph");
        assert!(g.get("http://ex/a").is_some());
    }
}
