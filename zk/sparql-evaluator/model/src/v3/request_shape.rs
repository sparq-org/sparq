// [OPUS-5.5] Host-only query-shape classification for protocol adapters; not in the guest.
//! Classifies the actual form of a V3-admitted query from its parse.
//!
//! A protocol adapter must not trust a stored form label. [`query_shape`] runs
//! the same parse, VERSION and static admission steps as [`super::admit`]
//! (under the blank-free V3 profile) and reads the form from the parsed query.
//! SELECT row order uses the evaluator's own outer-order rule, so the reported
//! shape equals the `RowOrder` V3 evaluation would produce. It changes no guest
//! relation, journal or ABI.

use crate::evaluate::{DatasetProfile, admit_query, ordered};
use crate::{MAX_QUERY_BYTES, Rejected};
use sparq_engine::{EbvSemantics, PreparedQuery};
use std::fmt;

/// Actual form of a parsed, statically admitted V3 query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QueryShape {
    /// SELECT whose V3 result is an unordered bag.
    SelectBag,
    /// SELECT whose V3 result keeps outer ORDER BY sequence order.
    SelectSequence,
    /// ASK.
    Ask,
    /// CONSTRUCT.
    Construct,
    /// DESCRIBE.
    Describe,
}

/// Why a query has no admitted V3 shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// The query exceeds [`MAX_QUERY_BYTES`]; carries the actual byte length.
    QueryBytes {
        /// Actual query length in bytes.
        len: usize,
    },
    /// The pinned parser rejected the query.
    Parse,
    /// A VERSION declaration contradicts the REC 2013 profile.
    Version,
    /// Static V3 admission rejected the query, with its existing diagnostic.
    ///
    /// The diagnostic is descriptive only; callers must not classify capacity
    /// from its text.
    NotAdmitted(Rejected),
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueryBytes { len } => write!(f, "query of {len} bytes exceeds the V3 bound"),
            Self::Parse => f.write_str("SPARQL parse rejected"),
            Self::Version => f.write_str("query VERSION contradicts REC 2013 profile"),
            Self::NotAdmitted(rejected) => fmt::Display::fmt(rejected, f),
        }
    }
}

impl std::error::Error for ShapeError {}

/// Parses and statically admits `query`, then reports its actual V3 shape.
///
/// The length bound is checked before parsing. Admission is the static
/// blank-free V3 profile of [`super::admit`]; evaluation over a dataset with
/// blank nodes applies a stricter profile, so admission here is necessary but
/// not sufficient for a later successful evaluation.
///
/// # Errors
/// Returns [`ShapeError`] for an oversized, unparsable, VERSION-contradicting
/// or statically unadmitted query.
pub fn query_shape(query: &str) -> Result<QueryShape, ShapeError> {
    if query.len() > MAX_QUERY_BYTES {
        return Err(ShapeError::QueryBytes { len: query.len() });
    }
    let prepared = PreparedQuery::parse(query).map_err(|_| ShapeError::Parse)?;
    prepared
        .resolve_ebv_semantics(Some(EbvSemantics::Rec2013))
        .map_err(|_| ShapeError::Version)?;
    admit_query(prepared.query(), DatasetProfile::GraphResultsBlankFree)
        .map_err(ShapeError::NotAdmitted)?;
    Ok(match prepared.query() {
        spargebra::Query::Select { pattern, .. } if ordered(pattern) => QueryShape::SelectSequence,
        spargebra::Query::Select { .. } => QueryShape::SelectBag,
        spargebra::Query::Ask { .. } => QueryShape::Ask,
        spargebra::Query::Construct { .. } => QueryShape::Construct,
        spargebra::Query::Describe { .. } => QueryShape::Describe,
    })
}
