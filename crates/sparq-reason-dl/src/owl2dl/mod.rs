//! OWL 2 DL: the full structural model, its fail-closed RDF mapping, and the OWL 2 DL
//! profile check (issue #7031).
//!
//! This module reads the whole of OWL 2, not only the ALCH fragment the
//! [`crate::tableau`] decides: [`extract()`] maps an RDF graph into the
//! [`model::Ontology`] of the OWL 2 Structural Specification (inverse properties,
//! cardinality, nominals, data properties and datatypes, keys, chains, the full ABox), and
//! [`check_dl`] reports every OWL 2 DL restriction the ontology breaks — the typing
//! constraints, simple properties, regularity of the property hierarchy, anonymous-individual
//! cycles, and datatype and facet validity — each with the triples it is about.
//!
//! The same fail-closed contract as the ALCH layer holds: [`extract()`] understands a graph
//! in full or refuses it, naming the triple it could not read.

pub(crate) mod clausify;
pub mod data_ranges;
pub mod datatypes;
pub mod dl_profile;
pub mod extract;
pub(crate) mod hypertableau;
pub mod model;
pub mod reasoner;
pub(crate) mod vocab;

pub use dl_profile::{check_dl, DlProfileReport, Unchecked, Violation, ViolationKind};
pub use extract::{extract, ExtractError, ExtractErrorKind};
pub use model::Ontology;
pub use reasoner::{
    check, Budget, Classification, Consistency, DlReport, Reasoner, UnknownReason, MAX_CARDINALITY,
};
