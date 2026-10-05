//! Serialize RDF as JSON-LD — RDF dataset → expanded document (JSON-LD 1.1 API §8.1).
//!
//! [FABLE-5] (sq-oy1f.28) The native **Deserialize RDF as JSON-LD** algorithm
//! (<https://www.w3.org/TR/json-ld11-api/#deserialize-rdf-as-json-ld-algorithm>): the
//! entry point [`from_rdf`] converts an RDF dataset (a slice of [`RdfQuad`]s over the
//! crate-local, dependency-free [`RdfTerm`] model) into the equivalent **expanded**
//! JSON-LD document, honouring the §8.1 processing options carried by
//! [`FromRdfOptions`]:
//!
//! * **`rdfDirection`** — both modes: `i18n-datatype` (an
//!   `https://www.w3.org/ns/i18n#<lang>_<dir>` datatype IRI decodes back into
//!   `@language`/`@direction`) and `compound-literal` (a blank node carrying
//!   `rdf:value`/`rdf:language`/`rdf:direction` collapses back into a value object).
//! * **`@json` literals** — an `rdf:JSON`-typed literal is parsed with the crate's
//!   strict [`Json::parse`] and emitted as `{"@value": <json>, "@type": "@json"}`;
//!   a malformed literal raises the spec's `invalid JSON literal` error code.
//! * **`rdf:List` reconstruction** — well-formed `rdf:first`/`rdf:rest`/`rdf:nil`
//!   chains collapse into `@list` arrays (including nested lists); chains that are
//!   not well-formed (a cell referenced more than once, or carrying extra properties)
//!   stay as plain node objects, exactly per spec. One liberty: a cell referenced from
//!   a *different* graph also stays put. The REC's walk follows the global
//!   referenced-once map across graphs, which would move the cell's triples into the
//!   referencing graph; the W3C suite has no such case.
//! * **`useNativeTypes` / `useRdfType`** — native JSON scalar coercion for
//!   `xsd:boolean`/`xsd:integer`/`xsd:double` (invalid or non-finite lexical forms
//!   honestly stay typed strings), and `rdf:type`-as-property instead of `@type`.
//!
//! # Implementation notes (spec fidelity)
//!
//! The REC's list conversion relies on *shared mutable references*: converting a
//! nested inner list mutates a value object that an already-converted outer `@list`
//! aliases (W3C test `fromRdf/li03`). This implementation is reference-free and
//! order-independent instead: list detection registers each converted chain against
//! the **slot** (graph, node, property, index) holding its head reference, and the
//! final emission phase resolves slots recursively — a slot registered as a list head
//! renders as `{"@list": […]}` whose items are themselves resolved slots. Because
//! every chain cell is consumed by at most one chain (the `rdf:rest` walk-back
//! follows *referenced-exactly-once* links, and the single-valued well-formedness
//! conditions prevent two walks from merging), the reachable slot graph is a forest
//! and the recursion terminates.
//!
//! Two deliberate, documented liberties where the W3C suite pins no behaviour:
//! `@json` / `i18n-datatype` decoding is checked *before* the `useNativeTypes` block
//! (the reference-implementation order; no suite case combines them), and
//! value-object deduplication uses structural [`Json`] equality (member-order
//! sensitive), which is only observable for `@json` literals that are equal as JSON
//! but differ in member order.
//!
//! Inputs that are not RDF are tolerated, not error-raised: a quad with a literal
//! subject, literal predicate, or literal graph name is **ignored** (the RDF data
//! model cannot produce one; the term model merely permits writing it), and
//! duplicate quads are deduplicated (an RDF dataset is a *set* of quads — W3C test
//! `fromRdf/0022` depends on this). Blank-node *predicates* (generalized RDF) are
//! accepted and keyed as `_:label` properties.

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

use crate::error::{JsonLdError, JsonLdErrorCode};
use crate::json::Json;
use crate::options::{JsonLdOptions, ProcessingMode, RdfDirection};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
const RDF_LIST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#List";
const RDF_JSON: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#JSON";
const RDF_VALUE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#value";
const RDF_LANGUAGE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#language";
const RDF_DIRECTION: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#direction";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const I18N_NS: &str = "https://www.w3.org/ns/i18n#";
/// The sentinel graph key for the default graph (the spec's `@default`).
const DEFAULT_GRAPH: &str = "@default";

/// An RDF term in the crate-local, dependency-free model consumed by [`from_rdf`].
///
/// [FABLE-5] (sq-oy1f.28) The crate carries **zero mandatory dependencies** (design
/// record §3.1), so it models the RDF term algebra itself rather than borrowing a
/// store's types; callers convert from their own quad model (the conformance lane
/// converts from `oxrdf`, the engine cutover bead `sq-oy1f.41` from `sparq-core`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RdfTerm {
    /// An IRI (absolute; resolution is the caller's concern).
    Iri(String),
    /// A blank node, carrying the bare label **without** the `_:` prefix
    /// (`RdfTerm::blank("b0")` is the N-Quads term `_:b0`).
    BlankNode(String),
    /// An RDF literal. `language: Some(_)` means an `rdf:langString` (any `datatype`
    /// is then ignored); `datatype: None` means `xsd:string`.
    Literal {
        /// The lexical form.
        lexical: String,
        /// The datatype IRI; `None` means `xsd:string`.
        datatype: Option<String>,
        /// The language tag, for `rdf:langString` literals.
        language: Option<String>,
    },
}

impl RdfTerm {
    /// An IRI term.
    pub fn iri(iri: impl Into<String>) -> RdfTerm {
        RdfTerm::Iri(iri.into())
    }

    /// A blank node term from its bare label (no `_:` prefix).
    pub fn blank(label: impl Into<String>) -> RdfTerm {
        RdfTerm::BlankNode(label.into())
    }

    /// A plain `xsd:string` literal.
    pub fn literal(lexical: impl Into<String>) -> RdfTerm {
        RdfTerm::Literal {
            lexical: lexical.into(),
            datatype: None,
            language: None,
        }
    }

    /// A typed literal.
    pub fn typed_literal(lexical: impl Into<String>, datatype: impl Into<String>) -> RdfTerm {
        RdfTerm::Literal {
            lexical: lexical.into(),
            datatype: Some(datatype.into()),
            language: None,
        }
    }

    /// A language-tagged string (`rdf:langString`).
    pub fn lang_literal(lexical: impl Into<String>, language: impl Into<String>) -> RdfTerm {
        RdfTerm::Literal {
            lexical: lexical.into(),
            datatype: None,
            language: Some(language.into()),
        }
    }

    /// The node-map identifier for a non-literal term: the IRI itself, or the
    /// `_:`-prefixed blank node label. `None` for literals.
    fn node_id(&self) -> Option<Cow<'_, str>> {
        match self {
            RdfTerm::Iri(i) => Some(Cow::Borrowed(i)),
            RdfTerm::BlankNode(b) => Some(Cow::Owned(format!("_:{}", b))),
            RdfTerm::Literal { .. } => None,
        }
    }
}

/// One RDF quad over [`RdfTerm`]s. `graph: None` places the triple in the default
/// graph; `Some(Iri(_) | BlankNode(_))` in that named graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RdfQuad {
    /// The subject (IRI or blank node; a literal subject is ignored by [`from_rdf`]).
    pub subject: RdfTerm,
    /// The predicate (an IRI; a blank node is accepted as generalized RDF).
    pub predicate: RdfTerm,
    /// The object (any term).
    pub object: RdfTerm,
    /// The graph name; `None` is the default graph.
    pub graph: Option<RdfTerm>,
}

impl RdfQuad {
    /// A quad; pass `graph: None` for the default graph.
    pub fn new(
        subject: RdfTerm,
        predicate: RdfTerm,
        object: RdfTerm,
        graph: Option<RdfTerm>,
    ) -> RdfQuad {
        RdfQuad {
            subject,
            predicate,
            object,
            graph,
        }
    }
}

/// The JSON-LD 1.1 API §8.1 serialization options honoured by [`from_rdf`].
///
/// `useNativeTypes` and `useRdfType` are §8.1-only options the JSON-LD API attaches
/// to `fromRdf` invocations; the pipeline-wide options ([`JsonLdOptions`]) do not
/// carry them, so this dedicated struct models the §8.1 option set (the
/// [`FromRdfOptions::from_jsonld`] bridge lifts the shared fields — options
/// *forwarding* across the whole pipeline is bead `sq-oy1f.30`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FromRdfOptions {
    /// `processingMode` — `json-ld-1.0` disables `@json` literal decoding.
    pub processing_mode: ProcessingMode,
    /// `rdfDirection` — how base direction was serialised to RDF (both modes are
    /// decoded; [`RdfDirection::None`] leaves such literals as plain typed literals).
    pub rdf_direction: RdfDirection,
    /// `useNativeTypes` — coerce valid `xsd:boolean`/`xsd:integer`/`xsd:double`
    /// lexical forms to native JSON scalars (default `false`).
    pub use_native_types: bool,
    /// `useRdfType` — keep `rdf:type` as a regular property instead of `@type`
    /// (default `false`).
    pub use_rdf_type: bool,
    /// `ordered` — deterministic code-point-ordered traversal (default `false`).
    /// This implementation *always* emits the fully ordered traversal (graphs and
    /// subjects sorted lexicographically), which satisfies `ordered: true` and is a
    /// permitted ordering when the flag is off; the field is carried for API parity.
    pub ordered: bool,
    /// Keep list cells that carry an explicit `rdf:type rdf:List` as plain node objects
    /// instead of collapsing them into `@list` (default `false`, the REC's behaviour).
    /// The REC's collapse drops the type triple, since list-to-RDF conversion recreates
    /// only `rdf:first`/`rdf:rest`; a writer that must round-trip to the same dataset
    /// sets this.
    pub keep_typed_list_cells: bool,
}

impl Default for FromRdfOptions {
    fn default() -> Self {
        FromRdfOptions {
            processing_mode: ProcessingMode::JsonLd11,
            rdf_direction: RdfDirection::None,
            use_native_types: false,
            use_rdf_type: false,
            ordered: false,
            keep_typed_list_cells: false,
        }
    }
}

impl FromRdfOptions {
    /// Lift the shared pipeline options ([`JsonLdOptions`]) into a §8.1 option set;
    /// the two §8.1-only flags start at their spec defaults (`false`).
    pub fn from_jsonld(options: &JsonLdOptions) -> Self {
        FromRdfOptions {
            processing_mode: options.processing_mode,
            rdf_direction: options.rdf_direction,
            use_native_types: false,
            use_rdf_type: false,
            ordered: options.ordered,
            keep_typed_list_cells: false,
        }
    }
}

/// A node object under construction in a graph's node map: its `@type`s plus its
/// property → value-object-array members (both in first-seen order, deduplicated).
#[derive(Default)]
struct Node {
    types: Vec<String>,
    props: Vec<(String, Vec<Json>)>,
}

impl Node {
    fn values(&self, property: &str) -> Option<&[Json]> {
        self.props
            .iter()
            .find(|(p, _)| p == property)
            .map(|(_, v)| v.as_slice())
    }

    /// Append `value` under `property` unless an equal value object is already
    /// there. Returns the value's index, or `None` when it was already present.
    fn push_value(&mut self, property: &str, value: Json) -> Option<usize> {
        if let Some(pos) = self.props.iter().position(|(p, _)| p == property) {
            let values = &mut self.props[pos].1;
            if values.contains(&value) {
                return None;
            }
            values.push(value);
            Some(values.len() - 1)
        } else {
            self.props.push((property.to_string(), vec![value]));
            Some(0)
        }
    }
}

/// A graph's node map, keyed by node identifier (IRI or `_:label`).
type SubjectMap = FxMap<String, Node>;

use crate::fx::{FxMap, FxSet};
/// All node maps, keyed by graph (`@default` or the graph's identifier). BTreeMaps
/// make every traversal the spec's `ordered: true` (code-point-sorted) one.
type DatasetMap = BTreeMap<String, SubjectMap>;
/// The address of one value object: (graph, node id, property, index-in-array).
type Slot = (String, String, String, usize);

/// One recorded reference to a blank node or to `rdf:nil`: which value slot holds it.
#[derive(Clone)]
struct Usage {
    graph: String,
    node: String,
    property: String,
    index: usize,
}

impl Usage {
    fn slot(&self) -> Slot {
        (
            self.graph.clone(),
            self.node.clone(),
            self.property.clone(),
            self.index,
        )
    }
}

/// The *referenced once* tracking (spec steps 4.6.9–4.6.10): a blank node is either
/// referenced exactly once (with the recorded usage) or shared.
enum RefState {
    Once(Usage),
    Shared,
}

/// `map[key]`, inserting a default value first; the key is only allocated on insert.
fn entry_mut<'m, V: Default, M: StrMap<V>>(map: &'m mut M, key: &str) -> &'m mut V {
    if !map.contains_key(key) {
        map.insert(key.to_string(), V::default());
    }
    map.get_mut(key).expect("inserted above")
}

/// The `String`-keyed maps [`entry_mut`] works over.
trait StrMap<V> {
    fn contains_key(&self, key: &str) -> bool;
    fn insert(&mut self, key: String, value: V);
    fn get_mut(&mut self, key: &str) -> Option<&mut V>;
}

impl<V> StrMap<V> for BTreeMap<String, V> {
    fn contains_key(&self, key: &str) -> bool {
        BTreeMap::contains_key(self, key)
    }
    fn insert(&mut self, key: String, value: V) {
        BTreeMap::insert(self, key, value);
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        BTreeMap::get_mut(self, key)
    }
}

impl<V> StrMap<V> for FxMap<String, V> {
    fn contains_key(&self, key: &str) -> bool {
        HashMap::contains_key(self, key)
    }
    fn insert(&mut self, key: String, value: V) {
        HashMap::insert(self, key, value);
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        HashMap::get_mut(self, key)
    }
}

/// Serialize an RDF dataset as an **expanded** JSON-LD document (JSON-LD 1.1 API
/// §8.1, *Deserialize RDF as JSON-LD*). Returns the expanded document as a
/// [`Json::Arr`] of node objects (named graphs nest under `@graph`).
///
/// Errors: `invalid JSON literal` (a malformed `rdf:JSON` literal, 1.1 mode only)
/// and `invalid language-tagged string` (a malformed `rdf:language` value on a
/// compound literal in `rdfDirection: compound-literal` mode).
pub fn from_rdf(dataset: &[RdfQuad], options: &FromRdfOptions) -> Result<Json, JsonLdError> {
    // ── Phase 1 (spec steps 1–4): group by graph → subject into node maps, convert
    // objects to value objects, and record rdf:nil usages + referenced-once state. ──
    let mut graphs: DatasetMap = BTreeMap::new();
    graphs.insert(DEFAULT_GRAPH.to_string(), SubjectMap::default());
    let mut referenced_once: FxMap<String, RefState> = FxMap::default();
    let mut nil_usages: BTreeMap<String, Vec<Usage>> = BTreeMap::new();
    let mut compound_subjects: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for quad in dataset {
        // Non-RDF shapes the term model permits but the data model cannot produce.
        let graph_key: Cow<'_, str> = match &quad.graph {
            None => Cow::Borrowed(DEFAULT_GRAPH),
            Some(g) => match g.node_id() {
                Some(id) => id,
                None => continue, // literal graph name: not RDF
            },
        };
        let (Some(subject_id), Some(predicate_id)) =
            (quad.subject.node_id(), quad.predicate.node_id())
        else {
            continue; // literal subject/predicate: not RDF
        };

        if graph_key != DEFAULT_GRAPH {
            // Spec step 4.4: the default graph gets a node for every graph name.
            let default = graphs.get_mut(DEFAULT_GRAPH).expect("default graph inserted above");
            entry_mut(default, &graph_key);
        }
        // Spec step 4.6.1: in compound-literal mode an rdf:direction triple marks
        // its subject as a compound-literal blank node.
        if options.rdf_direction == RdfDirection::CompoundLiteral && predicate_id == RDF_DIRECTION
        {
            let subs = entry_mut(&mut compound_subjects, &graph_key);
            if !subs.iter().any(|s| *s == subject_id) {
                subs.push(subject_id.to_string());
            }
        }

        let graph = entry_mut(&mut graphs, &graph_key);
        let object_id = quad.object.node_id();
        if let Some(oid) = &object_id {
            entry_mut(graph, oid);
        }
        let node = entry_mut(graph, &subject_id);

        // Spec step 4.6.4: rdf:type → @type (unless useRdfType).
        if predicate_id == RDF_TYPE && !options.use_rdf_type {
            if let Some(oid) = &object_id {
                if !node.types.iter().any(|t| t == oid) {
                    node.types.push(oid.to_string());
                }
                continue;
            }
        }

        // Spec steps 4.6.5–4.6.7: convert and append (deduplicated).
        let value = rdf_to_object(&quad.object, options)?;
        // An RDF dataset is a SET of quads (fromRdf/0022): a duplicate quad converts to
        // a value already present, and must not count as a second reference below.
        let Some(index) = node.push_value(&predicate_id, value) else {
            continue;
        };
        // The usage record is only needed for rdf:nil / blank-node objects, so it is
        // built lazily (most quads need neither).
        let usage = || Usage {
            graph: graph_key.to_string(),
            node: subject_id.to_string(),
            property: predicate_id.to_string(),
            index,
        };
        // Spec steps 4.6.8–4.6.10: rdf:nil usages + referenced-once bookkeeping.
        match &object_id {
            Some(oid) if oid == RDF_NIL => {
                entry_mut(&mut nil_usages, &graph_key).push(usage());
            }
            Some(oid) if oid.starts_with("_:") => match referenced_once.entry(oid.to_string()) {
                Entry::Vacant(v) => {
                    v.insert(RefState::Once(usage()));
                }
                Entry::Occupied(mut o) => {
                    *o.get_mut() = RefState::Shared;
                }
            },
            _ => {}
        }
    }

    // ── Phase 2 (spec step 6.1): compound-literal conversion. Replacements are
    // recorded per slot and applied during emission; the compound-literal blank
    // node itself is consumed (never emitted). ──
    let mut replacements: FxMap<Slot, Json> = FxMap::default();
    let mut consumed: FxSet<(String, String)> = FxSet::default();
    for (graph_key, subjects) in &compound_subjects {
        for cl in subjects {
            // Only a compound literal referenced exactly once is convertible.
            let Some(RefState::Once(usage)) = referenced_once.get(cl) else {
                continue;
            };
            let Some(cl_node) = graphs.get(graph_key).and_then(|g| g.get(cl)) else {
                continue;
            };
            // @value ← the first rdf:value item's @value; without one the node is
            // left as-is (the suite pins no behaviour for a value-less compound).
            let Some(value_json) = first_item_value(cl_node, RDF_VALUE) else {
                continue;
            };
            let mut members = vec![("@value".to_string(), value_json.clone())];
            if let Some(lang) = first_item_value(cl_node, RDF_LANGUAGE) {
                // Spec: a malformed rdf:language on a compound literal aborts with
                // `invalid language-tagged string`.
                match lang {
                    Json::Str(tag) if well_formed_language(tag) => {
                        members.push(("@language".to_string(), lang.clone()));
                    }
                    other => {
                        return Err(JsonLdError::with_detail(
                            JsonLdErrorCode::InvalidLanguageTaggedString,
                            format!("compound literal rdf:language is malformed: {:?}", other),
                        ));
                    }
                }
            }
            if let Some(dir) = first_item_value(cl_node, RDF_DIRECTION) {
                members.push(("@direction".to_string(), dir.clone()));
            }
            consumed.insert((graph_key.clone(), cl.clone()));
            replacements.insert(usage.slot(), Json::Obj(members));
        }
    }

    // ── Phase 3 (spec steps 6.2–6.4): list detection. Each rdf:nil usage walks the
    // rdf:rest chain BACKWARDS over referenced-exactly-once links while the cells
    // stay well-formed; the resulting chain is registered against the slot holding
    // its head reference, and its cells are consumed. Rendering is deferred (see the
    // module docs: this replaces the REC's shared-reference mutation, and makes the
    // conversion independent of usage processing order). ──
    let mut chains: FxMap<Slot, Vec<Slot>> = FxMap::default();
    for usages in nil_usages.values() {
        for start in usages {
            let mut node_graph = start.graph.clone();
            let mut node_id = start.node.clone();
            let mut property = start.property.clone();
            let mut head_slot = start.slot();
            let mut items: Vec<Slot> = Vec::new();
            let mut cells: Vec<(String, String)> = Vec::new();
            loop {
                if property != RDF_REST {
                    break;
                }
                let Some(RefState::Once(next)) = referenced_once.get(&node_id) else {
                    break;
                };
                // A cell referenced from another graph stays put: collapsing it would
                // move its triples into the referencing graph.
                if next.graph != node_graph {
                    break;
                }
                if !is_well_formed_list_node(&graphs, &node_graph, &node_id, options) {
                    break;
                }
                items.push((node_graph.clone(), node_id.clone(), RDF_FIRST.to_string(), 0));
                cells.push((node_graph.clone(), node_id.clone()));
                head_slot = next.slot();
                property = next.property.clone();
                node_graph = next.graph.clone();
                node_id = next.node.clone();
                // An IRI-identified referencer terminates the walk: the head slot
                // sits in that node.
                if !node_id.starts_with("_:") {
                    break;
                }
            }
            // Spec steps 6.4.4–6.4.7: the head reference becomes {"@list": […]}
            // (an empty array for a direct rdf:nil reference), reversed into
            // first-to-last order; the walked cells are removed from the output.
            items.reverse();
            chains.insert(head_slot, items);
            for cell in cells {
                consumed.insert(cell);
            }
        }
    }

    // ── Phase 4 (spec steps 7–9): emit the expanded document from the default
    // graph, nesting each named graph under its name node's @graph. Emitted nodes are
    // moved out of the node maps; consumed list cells and compound literals stay put
    // for the slot lookups that render them. ──
    let slotted = chains
        .keys()
        .chain(replacements.keys())
        .map(|(g, n, _, _)| (g.as_str(), n.as_str()))
        .collect();
    let renderer = Renderer {
        chains: &chains,
        replacements: &replacements,
        slotted,
    };
    let mut result: Vec<Json> = Vec::new();
    for (subject, node) in take_emitted(&mut graphs, DEFAULT_GRAPH, &consumed) {
        let named = graphs
            .contains_key(&subject)
            .then(|| (subject.clone(), take_emitted(&mut graphs, &subject, &consumed)));
        let mut jnode = renderer.render_node(&graphs, DEFAULT_GRAPH, subject, node);
        if let Some((graph_key, named)) = named {
            let mut graph_nodes: Vec<Json> = Vec::new();
            for (s, n) in named {
                let j = renderer.render_node(&graphs, &graph_key, s, n);
                if !is_only_id(&j) {
                    graph_nodes.push(j);
                }
            }
            jnode.set("@graph", Json::Arr(graph_nodes));
        }
        if !is_only_id(&jnode) {
            result.push(jnode);
        }
    }
    Ok(Json::Arr(result))
}

/// Removes and returns the emitted (non-consumed) nodes of `graph`, in code-point order
/// of their ids (the emission order). An absent graph yields nothing.
fn take_emitted(
    graphs: &mut DatasetMap,
    graph: &str,
    consumed: &FxSet<(String, String)>,
) -> Vec<(String, Node)> {
    let Some(map) = graphs.get_mut(graph) else {
        return Vec::new();
    };
    let mut entries: Vec<(String, Node)> = if consumed.is_empty() {
        map.drain().collect()
    } else {
        map.extract_if(|id, _| !consumed.contains(&(graph.to_string(), id.clone())))
            .collect()
    };
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// The deferred emission resolver: renders node maps to `Json`, materialising the
/// registered `@list` chains and compound-literal replacements at their slots.
struct Renderer<'a> {
    chains: &'a FxMap<Slot, Vec<Slot>>,
    replacements: &'a FxMap<Slot, Json>,
    /// `(graph, node)` pairs owning a chain head or replacement slot; only their
    /// values need a slot lookup.
    slotted: FxSet<(&'a str, &'a str)>,
}

impl Renderer<'_> {
    fn render_node(&self, graphs: &DatasetMap, graph: &str, id: String, node: Node) -> Json {
        let mut members: Vec<(String, Json)> = Vec::with_capacity(node.props.len() + 2);
        // Slot keys are only built when a chain or replacement can match.
        let slotted = self.slotted.contains(&(graph, id.as_str())).then(|| id.clone());
        members.push(("@id".to_string(), Json::Str(id)));
        if !node.types.is_empty() {
            members.push((
                "@type".to_string(),
                Json::Arr(node.types.into_iter().map(Json::Str).collect()),
            ));
        }
        for (property, values) in node.props {
            let rendered: Vec<Json> = match &slotted {
                None => values,
                Some(id) => values
                    .into_iter()
                    .enumerate()
                    .map(|(i, stored)| {
                        let slot = (graph.to_string(), id.clone(), property.clone(), i);
                        self.render_value(graphs, slot, stored)
                    })
                    .collect(),
            };
            members.push((property, Json::Arr(rendered)));
        }
        Json::Obj(members)
    }

    fn render_value(&self, graphs: &DatasetMap, slot: Slot, stored: Json) -> Json {
        if let Some(items) = self.chains.get(&slot) {
            let rendered: Vec<Json> =
                items.iter().map(|item| self.render_slot(graphs, item)).collect();
            Json::Obj(vec![("@list".to_string(), Json::Arr(rendered))])
        } else if let Some(replacement) = self.replacements.get(&slot) {
            replacement.clone()
        } else {
            stored
        }
    }

    fn render_slot(&self, graphs: &DatasetMap, slot: &Slot) -> Json {
        let (graph, node, property, index) = slot;
        let stored = graphs
            .get(graph)
            .and_then(|g| g.get(node))
            .and_then(|n| n.values(property))
            .and_then(|v| v.get(*index));
        match stored {
            Some(stored) => self.render_value(graphs, slot.clone(), stored.clone()),
            // Unreachable by construction: chains only reference slots whose
            // existence the well-formedness check verified. Stay total regardless.
            None => Json::Obj(Vec::new()),
        }
    }
}

/// Spec §8.2 *RDF to Object Conversion*: one term → one value object / node reference.
fn rdf_to_object(term: &RdfTerm, options: &FromRdfOptions) -> Result<Json, JsonLdError> {
    let (lexical, datatype, language) = match term {
        RdfTerm::Iri(i) => {
            return Ok(Json::Obj(vec![("@id".to_string(), Json::Str(i.clone()))]));
        }
        RdfTerm::BlankNode(b) => {
            return Ok(Json::Obj(vec![(
                "@id".to_string(),
                Json::Str(format!("_:{}", b)),
            )]));
        }
        RdfTerm::Literal {
            lexical,
            datatype,
            language,
        } => (lexical, datatype, language),
    };
    // A language-tagged string (rdf:langString) short-circuits every datatype rule.
    if let Some(lang) = language {
        return Ok(Json::Obj(vec![
            ("@value".to_string(), Json::Str(lexical.clone())),
            ("@language".to_string(), Json::Str(lang.clone())),
        ]));
    }
    let dt = datatype.as_deref().unwrap_or(XSD_STRING);

    // @json literals (JSON-LD 1.1 processing mode only; strict RFC 8259 parse).
    if options.processing_mode != ProcessingMode::JsonLd10 && dt == RDF_JSON {
        let parsed = Json::parse(lexical).map_err(|e| {
            JsonLdError::with_detail(JsonLdErrorCode::InvalidJsonLiteral, format!("{}", e))
        })?;
        return Ok(Json::Obj(vec![
            ("@value".to_string(), parsed),
            ("@type".to_string(), Json::Str("@json".to_string())),
        ]));
    }
    // rdfDirection: i18n-datatype — decode https://www.w3.org/ns/i18n#<lang>_<dir>.
    if options.rdf_direction == RdfDirection::I18nDatatype && dt.starts_with(I18N_NS) {
        if let Some((lang, dir)) = dt[I18N_NS.len()..].split_once('_') {
            let mut members = vec![("@value".to_string(), Json::Str(lexical.clone()))];
            if !lang.is_empty() {
                members.push(("@language".to_string(), Json::Str(lang.to_string())));
            }
            members.push(("@direction".to_string(), Json::Str(dir.to_string())));
            return Ok(Json::Obj(members));
        }
    }
    if options.use_native_types {
        match dt {
            XSD_STRING => {}
            XSD_BOOLEAN => {
                // The xsd:boolean lexical space is {true, false, 1, 0}; anything
                // else honestly stays a typed string (fromRdf/0027).
                if let "true" | "1" = lexical.as_str() {
                    return Ok(native_value(Json::Raw("true".to_string())));
                }
                if let "false" | "0" = lexical.as_str() {
                    return Ok(native_value(Json::Raw("false".to_string())));
                }
                return Ok(typed_value(lexical, dt));
            }
            XSD_INTEGER => {
                if let Some(canonical) = canonical_integer(lexical) {
                    return Ok(native_value(Json::Raw(canonical)));
                }
                return Ok(typed_value(lexical, dt));
            }
            XSD_DOUBLE => {
                if let Some(number) = native_double(lexical) {
                    return Ok(native_value(Json::Raw(number)));
                }
                return Ok(typed_value(lexical, dt));
            }
            // Any other datatype keeps its typed-string form below.
            _ => {}
        }
    }
    if dt == XSD_STRING {
        Ok(Json::Obj(vec![(
            "@value".to_string(),
            Json::Str(lexical.clone()),
        )]))
    } else {
        Ok(typed_value(lexical, dt))
    }
}

fn native_value(scalar: Json) -> Json {
    Json::Obj(vec![("@value".to_string(), scalar)])
}

fn typed_value(lexical: &str, datatype: &str) -> Json {
    Json::Obj(vec![
        ("@value".to_string(), Json::Str(lexical.to_string())),
        ("@type".to_string(), Json::Str(datatype.to_string())),
    ])
}

/// Canonicalize a valid `xsd:integer` lexical form into a JSON number token of
/// arbitrary magnitude (strip the `+` sign and leading zeros); `None` if the form
/// is not in the xsd:integer lexical space.
fn canonical_integer(lexical: &str) -> Option<String> {
    let (negative, digits) = match lexical.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, lexical.strip_prefix('+').unwrap_or(lexical)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return Some("0".to_string()); // all zeros (covers "-0" → 0)
    }
    Some(if negative {
        format!("-{}", trimmed)
    } else {
        trimmed.to_string()
    })
}

/// Convert a valid, *finite* `xsd:double` lexical form to a JSON number token;
/// `None` for INF/NaN and for overflowing forms like `0.1e999999999999999`
/// (fromRdf/0027: those honestly stay typed strings — JSON has no non-finite
/// numbers), and for anything outside the xsd:double lexical space.
fn native_double(lexical: &str) -> Option<String> {
    if !valid_xsd_double(lexical) {
        return None;
    }
    let f: f64 = lexical.parse().ok()?;
    if !f.is_finite() {
        return None;
    }
    Some(format!("{}", f))
}

/// The finite `xsd:double` lexical grammar (sign? (digits ('.' digits*)? | '.'
/// digits+) exponent?). INF/NaN deliberately excluded (see [`native_double`]).
fn valid_xsd_double(lexical: &str) -> bool {
    let s = lexical.as_bytes();
    let mut i = 0;
    if matches!(s.first(), Some(b'+') | Some(b'-')) {
        i += 1;
    }
    let int_digits = count_digits(s, i);
    i += int_digits;
    let mut frac_digits = 0;
    if s.get(i) == Some(&b'.') {
        i += 1;
        frac_digits = count_digits(s, i);
        i += frac_digits;
    }
    if int_digits == 0 && frac_digits == 0 {
        return false;
    }
    if matches!(s.get(i), Some(b'e') | Some(b'E')) {
        i += 1;
        if matches!(s.get(i), Some(b'+') | Some(b'-')) {
            i += 1;
        }
        let exp_digits = count_digits(s, i);
        if exp_digits == 0 {
            return false;
        }
        i += exp_digits;
    }
    i == s.len()
}

fn count_digits(s: &[u8], from: usize) -> usize {
    s[from..].iter().take_while(|b| b.is_ascii_digit()).count()
}

/// BCP47 *well-formedness* (the language-tag shape check the spec requires for a
/// compound literal's `rdf:language`): 1–8 alphabetic characters, then `-`-separated
/// 1–8 character alphanumeric subtags.
fn well_formed_language(tag: &str) -> bool {
    let mut parts = tag.split('-');
    match parts.next() {
        Some(first)
            if (1..=8).contains(&first.len())
                && first.bytes().all(|b| b.is_ascii_alphabetic()) => {}
        _ => return false,
    }
    parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// The first item's `@value` under `property`, if any.
fn first_item_value<'a>(node: &'a Node, property: &str) -> Option<&'a Json> {
    node.values(property)?.first()?.get("@value")
}

/// The spec's list-node well-formedness: only `@id` + single-valued `rdf:first` +
/// single-valued `rdf:rest` (and optionally `@type` = exactly `[rdf:List]`, unless
/// [`FromRdfOptions::keep_typed_list_cells`]).
fn is_well_formed_list_node(
    graphs: &DatasetMap,
    graph: &str,
    id: &str,
    options: &FromRdfOptions,
) -> bool {
    let Some(node) = graphs.get(graph).and_then(|g| g.get(id)) else {
        return false;
    };
    let typed_ok = node.types.len() == 1
        && node.types[0] == RDF_LIST
        && !options.keep_typed_list_cells;
    if !(node.types.is_empty() || typed_ok) {
        return false;
    }
    if node.props.len() != 2 {
        return false;
    }
    matches!(
        (node.values(RDF_FIRST), node.values(RDF_REST)),
        (Some(first), Some(rest)) if first.len() == 1 && rest.len() == 1
    )
}

fn is_only_id(node: &Json) -> bool {
    matches!(node, Json::Obj(members) if members.len() == 1 && members[0].0 == "@id")
}
