//! SHACL / OWL **prior extractor** — read enum / datatype / cardinality priors out of a parsed
//! SHACL shapes model (and `owl:FunctionalProperty` from the data graph) into the layout the
//! structure-aware encoders consume (structure-aware vectorisation **P2**).
//!
//! sq-0wo9e.3 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §2 + §7 "sparq-shacl already evaluates `sh:datatype`, `sh:in`, `sh:not`, `min/maxCount` — the
//! prior extractor reads these from `model.rs` (no SHACL changes; a new reader in the opt-in
//! crate)"). It is **opt-in** behind the `structure-shacl` cargo feature (off by default) — the
//! ONLY feature that pulls `sparq-shacl` into this crate's graph, so the default build (and the
//! `structure` feature) carry zero SHACL code and gain no new required dep.
//!
//! # What it extracts (per predicate)
//!
//! For each **property shape** whose `sh:path` is a single predicate, the reader maps the SHACL
//! constraints to the structure-aware priors of design §2:
//!
//! | SHACL / OWL fact | Prior |
//! |---|---|
//! | `sh:in (m1 m2 …)` | a **[`Codebook`]** over the enum members — encoded by slot match, out-of-enum is the reserved invalid code (design §2 "enum equality is a slot match"). |
//! | `sh:datatype dt` | the **router-confirm** [`Encoder`] for `dt` (cross-checks the datatype router against the declared datatype — a *declared* type the encoder can trust over a guessed one). |
//! | `sh:maxCount 1` **or** `owl:FunctionalProperty` | [`Cardinality::Functional`] → a **single deterministic slot** (one value per subject). |
//! | otherwise (multi-valued) | [`Cardinality::Multi`] → a **permutation-invariant pooled** block (design §2 "multi-valued → a permutation-invariant pooled block"). |
//! | `sh:minCount` / `sh:maxCount` | recorded on [`PredicatePrior`] so a caller knows the declared multiplicity. |
//!
//! `owl:FunctionalProperty` is an **OWL** axiom, not a SHACL constraint, so it is read from the
//! **data/ontology graph** (a `rdf:type owl:FunctionalProperty` triple), not the shapes model —
//! exactly the split design §2/§7 describes. The shapes model is read **read-only**; no SHACL
//! behaviour changes.
//!
//! # Honesty
//!
//! Enum/boolean exactness and the datatype-confirm are **provable** (slot match, exact dispatch).
//! Whether these priors lift downstream retrieval is **empirical and ablation-gated** (design §6.B)
//! — no accuracy claim is made here. A predicate with **no** declared shape simply has **no** prior
//! (the caller falls back to the datatype router on the literal alone) — fail-open, never a guess.

use crate::codebook::Codebook;
use crate::encode::{route, Encoder};
use oxrdf::Term;
use rustc_hash::FxHashMap;
use sparq_shacl::model::{Component, ShapesModel};
use sparq_shacl::Path;

/// `rdf:type` and `owl:FunctionalProperty` IRIs (the OWL functional-property axiom is read from the
/// data graph, not the shapes model).
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const OWL_FUNCTIONAL_PROPERTY: &str = "http://www.w3.org/2002/07/owl#FunctionalProperty";

/// The **pooling rule** a predicate's cardinality dictates (design §2 "Cardinality → a POOLING
/// RULE"). It tells the structured-object builder whether the predicate occupies one deterministic
/// slot or a permutation-invariant pooled block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinality {
    /// At most one value per subject (`sh:maxCount 1` or `owl:FunctionalProperty`) → a **single
    /// deterministic slot**.
    Functional,
    /// Possibly many values per subject → a **permutation-invariant pooled** block (the order of
    /// the values must not change the encoding).
    Multi,
}

/// The structure-aware priors mined for one predicate from the SHACL shapes (and OWL axioms).
/// Every field is `Option`/best-effort: a missing constraint means "no prior", never a guess.
#[derive(Clone, Debug)]
pub struct PredicatePrior {
    /// The predicate IRI these priors are for.
    pub predicate: String,
    /// `Some(codebook)` when the predicate declares an `sh:in` enumeration → encode by slot match
    /// ([`Codebook`]); `None` otherwise.
    pub enum_codebook: Option<Codebook>,
    /// `Some(encoder)` when the predicate declares an `sh:datatype` — the **router-confirmed**
    /// encoder family for that *declared* datatype (a declared type the encoder can trust). `None`
    /// when no `sh:datatype` is declared. A `sh:datatype` list with mixed encoder families resolves
    /// to `Encoder::Other` (no single trusted lane).
    pub datatype_encoder: Option<Encoder>,
    /// The pooling rule from the predicate's cardinality (functional vs multi-valued).
    pub cardinality: Cardinality,
    /// The declared `sh:minCount`, if any.
    pub min_count: Option<u64>,
    /// The declared `sh:maxCount`, if any.
    pub max_count: Option<u64>,
}

impl PredicatePrior {
    /// Is this predicate functional (one value per subject) — `sh:maxCount 1` or
    /// `owl:FunctionalProperty`? Convenience over [`cardinality`](Self::cardinality).
    pub fn is_functional(&self) -> bool {
        self.cardinality == Cardinality::Functional
    }
}

/// All per-predicate priors mined from a shapes model + the functional-property axioms of a data
/// graph. Keyed by predicate IRI.
#[derive(Clone, Debug, Default)]
pub struct ShaclPriors {
    by_predicate: FxHashMap<String, PredicatePrior>,
}

impl ShaclPriors {
    /// Mine priors from a parsed [`ShapesModel`] **only** (no OWL functional-property axioms — a
    /// predicate is functional iff it declares `sh:maxCount 1`). Use
    /// [`from_model_and_data`](Self::from_model_and_data) to also honour `owl:FunctionalProperty`
    /// from a data graph.
    pub fn from_model(model: &ShapesModel) -> ShaclPriors {
        Self::extract(model, &Default::default())
    }

    /// Mine priors from a parsed [`ShapesModel`] **and** the `owl:FunctionalProperty` axioms of a
    /// data/ontology [`Graph`](sparq_core::Graph). A predicate is [`Cardinality::Functional`] when
    /// it declares `sh:maxCount 1` **or** is typed `owl:FunctionalProperty` in `data` (the OWL/SHACL
    /// split of design §2/§7 — the OWL axiom comes from the data graph, the SHACL constraints from
    /// the model).
    pub fn from_model_and_data(model: &ShapesModel, data: &sparq_core::Graph) -> ShaclPriors {
        let functional = functional_properties(data);
        Self::extract(model, &functional)
    }

    /// The mined prior for `predicate`, if any (`None` when the predicate has no declared shape).
    pub fn get(&self, predicate: &str) -> Option<&PredicatePrior> {
        self.by_predicate.get(predicate)
    }

    /// Every mined predicate prior.
    pub fn priors(&self) -> impl Iterator<Item = &PredicatePrior> {
        self.by_predicate.values()
    }

    /// The number of predicates with at least one mined prior.
    pub fn len(&self) -> usize {
        self.by_predicate.len()
    }

    /// Is the prior set empty (no predicate had a usable shape)?
    pub fn is_empty(&self) -> bool {
        self.by_predicate.is_empty()
    }

    fn extract(
        model: &ShapesModel,
        functional_axioms: &rustc_hash::FxHashSet<String>,
    ) -> ShaclPriors {
        let mut by_predicate: FxHashMap<String, PredicatePrior> = FxHashMap::default();

        for shape in &model.shapes {
            // Only property shapes whose path is a single predicate carry per-predicate priors. A
            // complex path (inverse / sequence / alternative) has no single predicate to key on.
            let Some(Path::Predicate(pred)) = shape.path.as_ref() else { continue };

            let mut enum_members: Option<Vec<Term>> = None;
            let mut datatype_set: Option<Vec<String>> = None;
            let mut min_count: Option<u64> = None;
            let mut max_count: Option<u64> = None;

            for comp in &shape.components {
                match comp {
                    Component::In(members) => {
                        // Last `sh:in` on a shape wins if (malformedly) repeated; clone the members.
                        enum_members = Some(members.clone());
                    }
                    Component::Datatype(dts) => datatype_set = Some(dts.clone()),
                    Component::MinCount(n) => min_count = Some(*n),
                    Component::MaxCount(n) => max_count = Some(*n),
                    _ => {}
                }
            }

            let functional =
                max_count == Some(1) || functional_axioms.contains(pred.as_str());
            let cardinality =
                if functional { Cardinality::Functional } else { Cardinality::Multi };

            let enum_codebook = enum_members.map(Codebook::new);
            let datatype_encoder = datatype_set.as_deref().map(resolve_datatype_encoder);

            // Merge into any existing prior for the same predicate (two property shapes may target
            // the same predicate): a more specific (Some) value wins; functional is sticky (once a
            // shape declares maxCount 1 the predicate is functional).
            match by_predicate.get_mut(pred) {
                Some(existing) => {
                    if existing.enum_codebook.is_none() {
                        existing.enum_codebook = enum_codebook;
                    }
                    if existing.datatype_encoder.is_none() {
                        existing.datatype_encoder = datatype_encoder;
                    }
                    if existing.min_count.is_none() {
                        existing.min_count = min_count;
                    }
                    if existing.max_count.is_none() {
                        existing.max_count = max_count;
                    }
                    if cardinality == Cardinality::Functional {
                        existing.cardinality = Cardinality::Functional;
                    }
                }
                None => {
                    by_predicate.insert(
                        pred.clone(),
                        PredicatePrior {
                            predicate: pred.clone(),
                            enum_codebook,
                            datatype_encoder,
                            cardinality,
                            min_count,
                            max_count,
                        },
                    );
                }
            }
        }

        ShaclPriors { by_predicate }
    }
}

/// Resolve a declared `sh:datatype` set to a **single** router-confirmed [`Encoder`]: if every
/// declared datatype routes to the same encoder family, that family is the confirmed lane; a
/// mixed-family list (e.g. `( xsd:string xsd:integer )`) has no single trusted lane and resolves to
/// [`Encoder::Other`]. An empty set also resolves to `Other`.
fn resolve_datatype_encoder(datatypes: &[String]) -> Encoder {
    let mut encoders = datatypes.iter().map(|dt| route(dt));
    match encoders.next() {
        None => Encoder::Other,
        Some(first) => {
            if encoders.all(|e| e == first) {
                first
            } else {
                Encoder::Other
            }
        }
    }
}

/// The set of predicate IRIs typed `owl:FunctionalProperty` in `data` (an OWL axiom read from the
/// data/ontology graph — not the shapes model).
fn functional_properties(data: &sparq_core::Graph) -> rustc_hash::FxHashSet<String> {
    use oxrdf::NamedNode;
    let type_id = data.id_of(&Term::NamedNode(NamedNode::new_unchecked(RDF_TYPE)));
    let fp_id = data.id_of(&Term::NamedNode(NamedNode::new_unchecked(OWL_FUNCTIONAL_PROPERTY)));
    let (Some(type_id), Some(fp_id)) = (type_id, fp_id) else {
        return rustc_hash::FxHashSet::default();
    };
    let mut out = rustc_hash::FxHashSet::default();
    for [s, p, o] in data.iter_ids() {
        if p == type_id && o == fp_id {
            // Resolve the subject id back to its IRI string (a non-IRI subject of an
            // owl:FunctionalProperty axiom is malformed and simply ignored).
            if let Term::NamedNode(n) = data.dict.term(s) {
                out.insert(n.into_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    fn shapes(ttl: &str) -> ShapesModel {
        let g = Graph::load_str(ttl, "turtle").expect("parse shapes");
        ShapesModel::parse(&g)
    }

    const SHAPES: &str = r#"
@prefix sh:   <http://www.w3.org/ns/shacl#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:   <http://ex/> .

ex:PersonShape a sh:NodeShape ;
  sh:targetClass ex:Person ;
  sh:property [ sh:path ex:age ;     sh:datatype xsd:integer ; sh:maxCount 1 ; sh:minCount 1 ] ;
  sh:property [ sh:path ex:status ;  sh:in ( ex:Active ex:Suspended ex:Closed ) ; sh:maxCount 1 ] ;
  sh:property [ sh:path ex:nickname ; sh:datatype xsd:string ] .
"#;

    #[test]
    fn extracts_enum_codebook_with_slot_match() {
        let m = shapes(SHAPES);
        let priors = ShaclPriors::from_model(&m);
        let status = priors.get("http://ex/status").expect("status prior");
        let cb = status.enum_codebook.as_ref().expect("status declares sh:in → codebook");
        assert_eq!(cb.member_count(), 3, "3 enum members");
        // Slot match, not a cosine threshold: a member is hot at its own slot; an outsider is the
        // reserved invalid code.
        let active = oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/Active"));
        assert!(cb.is_member(&active));
        let outsider = oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/Other"));
        assert!(!cb.is_member(&outsider));
        assert_eq!(cb.slot(&outsider), crate::codebook::INVALID_SLOT);
    }

    #[test]
    fn datatype_confirm_routes_declared_type() {
        let m = shapes(SHAPES);
        let priors = ShaclPriors::from_model(&m);
        // The declared sh:datatype confirms the encoder family the router would pick.
        assert_eq!(priors.get("http://ex/age").unwrap().datatype_encoder, Some(Encoder::Numeric));
        assert_eq!(
            priors.get("http://ex/nickname").unwrap().datatype_encoder,
            Some(Encoder::Other),
            "xsd:string routes to the text (Other) lane"
        );
    }

    #[test]
    fn cardinality_pooling_rule_from_maxcount() {
        let m = shapes(SHAPES);
        let priors = ShaclPriors::from_model(&m);
        // sh:maxCount 1 → Functional (one deterministic slot).
        let age = priors.get("http://ex/age").unwrap();
        assert!(age.is_functional(), "maxCount 1 → functional");
        assert_eq!(age.cardinality, Cardinality::Functional);
        assert_eq!(age.min_count, Some(1));
        assert_eq!(age.max_count, Some(1));
        // No maxCount on nickname → Multi (permutation-invariant pooled block).
        assert_eq!(priors.get("http://ex/nickname").unwrap().cardinality, Cardinality::Multi);
    }

    #[test]
    fn owl_functional_property_from_data_graph() {
        // `ex:ssn` is functional via the OWL axiom, NOT a SHACL maxCount.
        let shapes_ttl = r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix ex:  <http://ex/> .
ex:S a sh:NodeShape ; sh:property [ sh:path ex:ssn ] .
"#;
        let data_ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:ssn a owl:FunctionalProperty .
"#;
        let m = shapes(shapes_ttl);
        let data = Graph::load_str(data_ttl, "turtle").unwrap();

        // Without the data graph, ssn has no maxCount → Multi.
        assert_eq!(
            ShaclPriors::from_model(&m).get("http://ex/ssn").unwrap().cardinality,
            Cardinality::Multi
        );
        // With the data graph, the OWL axiom makes it Functional.
        let with_owl = ShaclPriors::from_model_and_data(&m, &data);
        assert!(
            with_owl.get("http://ex/ssn").unwrap().is_functional(),
            "owl:FunctionalProperty makes ex:ssn functional"
        );
    }

    #[test]
    fn predicate_with_no_shape_has_no_prior_fail_open() {
        let m = shapes(SHAPES);
        let priors = ShaclPriors::from_model(&m);
        assert!(priors.get("http://ex/undeclared").is_none(), "no shape → no prior (fail-open)");
    }

    #[test]
    fn mixed_datatype_list_has_no_single_lane() {
        let m = shapes(
            r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:  <http://ex/> .
ex:S a sh:NodeShape ;
  sh:property [ sh:path ex:mixed ; sh:datatype ( xsd:integer xsd:string ) ] .
"#,
        );
        let priors = ShaclPriors::from_model(&m);
        // numeric + string → no single trusted lane → Other.
        assert_eq!(
            priors.get("http://ex/mixed").unwrap().datatype_encoder,
            Some(Encoder::Other)
        );
    }
}
