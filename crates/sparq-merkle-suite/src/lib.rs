//! Research-grade, externally unaudited RDF Merkle credential suites.
//!
//! New callers use this crate. The frozen detached evaluator still carries its
//! original SHA-256 implementation; the parity test exercises the existing vectors.

pub mod keys;
pub mod metadata;
pub mod poseidon;
#[cfg(feature = "os-random")]
pub mod random;
pub mod sha256;
#[cfg(feature = "sha256-salted")]
pub mod sha256_salted;

pub use keys::{comparison_key, key_class};
pub use sparq_zk::Fr;

/// One class byte followed by a thirty-byte payload.
pub const KEY_BYTES: usize = 31;
/// SHA-256 member's tree-salt byte width.
pub const SALT_BYTES: usize = 32;

/// Tags in the typed byte encoding. These differ from the older commitment methods.
pub mod term_kind {
    /// Bound default graph.
    pub const DEFAULT_GRAPH: u8 = 0;
    /// IRI.
    pub const IRI: u8 = 1;
    /// Blank node.
    pub const BLANK_NODE: u8 = 2;
    /// Literal.
    pub const LITERAL: u8 = 3;
}

/// Suite input was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub &'static str);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

/// Canonical, ordered N-Quads and proof configuration, with signature and salt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedCredential {
    /// Canonical quads in suite order.
    pub document: String,
    /// RDFC-1.0 canonical proof configuration.
    pub proof_config: String,
    /// Member-specific signature followed by the private salt.
    pub signature: Vec<u8>,
}

fn canonical_config(text: &str, suite: &str) -> Result<String, Error> {
    let text = sparq_canon::canonicalize_nquads(text)
        .map_err(|_| Error("configuration canonicalization rejected"))?;
    check_cryptosuite(&config_quads(&text)?, |literal| literal.value() == suite)?;
    Ok(text)
}

fn config_quads(text: &str) -> Result<Vec<oxrdf::Quad>, Error> {
    sparq_canon::parse_nquads(text).map_err(|_| Error("configuration parsing rejected"))
}

fn check_cryptosuite(
    quads: &[oxrdf::Quad],
    matches_suite: impl FnOnce(&oxrdf::Literal) -> bool,
) -> Result<(), Error> {
    use oxrdf::Term;
    let mut suites = quads
        .iter()
        .filter(|quad| quad.predicate.as_str() == "https://w3id.org/security#cryptosuite");
    match (suites.next().map(|quad| &quad.object), suites.next()) {
        (Some(Term::Literal(literal)), None) if matches_suite(literal) => Ok(()),
        _ => Err(Error(
            "configuration must name this cryptosuite exactly once",
        )),
    }
}
