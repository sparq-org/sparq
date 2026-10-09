//! Experimental native authentication of public RDF query support.
//!
//! [GPT-6] This detached research API is not externally audited. It proves
//! successful support, not complete query results. Its experimental BBS+ RDF
//! encoding is separate from W3C credential suites and the tuple executable.

#![forbid(unsafe_code)]

#[cfg(feature = "native-rdf")]
pub mod rdf;
