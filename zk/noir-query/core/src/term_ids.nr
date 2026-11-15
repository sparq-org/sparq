//! Literal identifiers avoid compiler-dependent evaluation of constant hashes.
use std::collections::bounded_vec::BoundedVec;
use crate::suite_constants::{DATATYPE_DOMAIN, DT_BOOLEAN, DT_BYTE, DT_DATE, DT_DATETIME, DT_DECIMAL, DT_DOUBLE, DT_FLOAT, DT_INT, DT_INTEGER, DT_LANGSTRING, DT_LONG, DT_NEGATIVEINTEGER, DT_NONNEGATIVEINTEGER, DT_NONPOSITIVEINTEGER, DT_OTHER, DT_POSITIVEINTEGER, DT_SHORT, DT_STRING, DT_UNSIGNEDBYTE, DT_UNSIGNEDINT, DT_UNSIGNEDLONG, DT_UNSIGNEDSHORT, LANGUAGE_DOMAIN, LANG_EMPTY, LANG_EN};
use crate::term_key::equals;
pub(crate) fn datatype<let L: u32>(bytes: BoundedVec<u8,L>) -> Field {
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#integer".as_bytes()) { DT_INTEGER } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#decimal".as_bytes()) { DT_DECIMAL } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#double".as_bytes()) { DT_DOUBLE } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#float".as_bytes()) { DT_FLOAT } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#dateTime".as_bytes()) { DT_DATETIME } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#boolean".as_bytes()) { DT_BOOLEAN } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#string".as_bytes()) { DT_STRING } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#date".as_bytes()) { DT_DATE } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#nonPositiveInteger".as_bytes()) { DT_NONPOSITIVEINTEGER } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#negativeInteger".as_bytes()) { DT_NEGATIVEINTEGER } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#long".as_bytes()) { DT_LONG } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#int".as_bytes()) { DT_INT } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#short".as_bytes()) { DT_SHORT } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#byte".as_bytes()) { DT_BYTE } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#nonNegativeInteger".as_bytes()) { DT_NONNEGATIVEINTEGER } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#unsignedLong".as_bytes()) { DT_UNSIGNEDLONG } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#unsignedInt".as_bytes()) { DT_UNSIGNEDINT } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#unsignedShort".as_bytes()) { DT_UNSIGNEDSHORT } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#unsignedByte".as_bytes()) { DT_UNSIGNEDBYTE } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#positiveInteger".as_bytes()) { DT_POSITIVEINTEGER } else
    if equals(bytes, "http://www.w3.org/2001/XMLSchema#other".as_bytes()) { DT_OTHER } else
    if equals(bytes, "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes()) { DT_LANGSTRING } else
    { crate::term::hash_bytes(DATATYPE_DOMAIN, bytes) }
}
pub(crate) fn language<let L: u32>(bytes: BoundedVec<u8,L>) -> Field {
    if bytes.len() == 0 { LANG_EMPTY } else
    if equals(bytes, "en".as_bytes()) { LANG_EN } else
    { crate::term::hash_bytes(LANGUAGE_DOMAIN, bytes) }
}
