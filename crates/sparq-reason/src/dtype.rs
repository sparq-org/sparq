//! D-entailment (datatype / value-space) materialization — RDF 1.1 Semantics §7-8.
//!
//! [OPUS-4.8] sq-e5atd (epic sq-pbz04). OPT-IN, behind the `d-entail` cargo
//! feature — the production materializer never carries it, so the lean default
//! build (and the wasm bundle, which never depends on this code) is byte-identical.
//!
//! ## What D-entailment adds beyond simple entailment
//!
//! A *recognized datatype map* D fixes, for each datatype IRI it covers, a lexical
//! space, a value space, and the lexical-to-value mapping (RDF 1.1 Semantics §7).
//! The D-entailment lemma then sanctions, on top of simple entailment, the
//! **datatype typing rule rdfD1**: a well-formed literal `"l"^^d` of a recognized
//! datatype `d` is an instance of `d`, i.e. it entails `"l"^^d rdf:type d`. (In the
//! abstract semantics the literal is replaced by a *surrogate* allocated for its
//! value; the materializer keeps the literal in subject position — a GENERALIZED
//! triple — and lets the downstream regime restriction decide whether a surrogate
//! may surface in a query answer. This mirrors the conformance harness's
//! `harness_rules` rdfD1 arm.)
//!
//! The load-bearing invariant is **value-space equality**: two literals whose
//! values coincide are interchangeable under D — e.g. `"1"^^xsd:integer` and
//! `"1.0"^^xsd:decimal` denote the SAME value (the integers are a subset of the
//! decimals), so they must compare equal. [`d_value_key`] is the canonical
//! comparison: equal keys mean equal D-values, across lexical forms AND across the
//! integer/decimal value spaces that coincide.
//!
//! ## Why NOT an f64 fast path
//!
//! Collapsing the numeric value to `f64` is UNSOUND for D-equality: `f64` cannot
//! represent every `xsd:integer` (anything past 2^53 rounds) nor every
//! `xsd:decimal` exactly, so distinct values would alias and equal values past the
//! mantissa would diverge — silent precision loss in a SEMANTIC equality. Instead
//! the integer/decimal value space is compared as a CANONICAL DECIMAL STRING
//! (sign + minimal integer digits + minimal fraction digits, parsed exactly): this
//! is the correct typed comparison, exact at any magnitude.
//!
//! ## sparq-substrate seam (the shared value-space comparator)
//!
//! [FABLE-5] sq-pbz04.6.3 (epic sq-pbz04.6, seam 2 — design record
//! `research/d-entailment-datatype-map.md` §4). Datatype recognition
//! (`is_integer_datatype`) and temporal value parsing (`temporal::Temporal`) come
//! from `sparq-core`; the integer/decimal canonical-key split + normalization now
//! DELEGATE to `sparq_substrate::numeric::split_decimal` — the SAME pure-string,
//! unbounded-magnitude splitter the SPARQL engine's exact decimal-string compare
//! uses — so the reasoner and the engine can never diverge on which decimal lexicals
//! are well-formed nor on the canonical form. sparq-reason still depends ONLY on
//! sparq-core + (behind `d-entail`) sparq-substrate, never sparq-engine; the lean
//! default build links none of it.
//!
//! ### What migrated, and what deliberately STAYED LOCAL (byte-identical ledger)
//!
//! The seam is BEHAVIOUR-NEUTRAL by construction — the `dtype` unit matrix and the
//! `D_ENTAIL_FLOOR` ratchet are byte-identical before/after. Two pieces stay local
//! by DESIGN, not omission (design record §4 keeps facet validation dtype-resident):
//!
//! - **`integer_subtype_ok` (bounded-range facets) stays local.** The substrate
//!   `Num::of_literal` parses magnitude only; it does NOT reject `"200"^^xsd:byte`
//!   (out of the `byte` value space). rdfD1 must not type an out-of-range literal, so
//!   the `i128` parse + range-facet reject is applied HERE before the canonical key.
//! - **`parse_xsd_double` (local double/float parser) MIGRATED in sq-s3b10 [SONNET-4.6].**
//!   The local blocklist (`contains("inf")`, case-sensitive) was replaced by the shared
//!   `sparq_substrate::numeric::parse_xsd_f64` / `parse_xsd_f32`, so dtype.rs and the
//!   SPARQL evaluator now use IDENTICAL XSD-conformant acceptance. `Infinity` /
//!   `-Infinity` / `NAN` / `nan` are REJECTED by both (XSD forbids them; the valid
//!   specials are `INF` / `-INF` / `NaN` only). This is a **documented Some→None
//!   tightening** for those four spellings — the reasoner was more lenient than the
//!   evaluator, the exact anti-pattern the module-doc warned against. D-entailment
//!   conformance suite stays green (no W3C test uses those forbidden spellings).
//! - **The integer/decimal KEY stays `canon_decimal`; it does NOT delegate to
//!   `Num::cmp_relational` (sq-fvxko, issue #3137) [SONNET-4.6].** That follow-on was
//!   proposed as behaviour-neutral. It is not, in three measured ways — pinned by
//!   `tests::cmp_relational_delegation_would_change_behaviour`:
//!   1. **Magnitude.** `as_numeric` routes `xsd:decimal` through `Dec::parse_lexical`,
//!      whose `i128` mantissa overflows past ~38 significant digits and yields `None`.
//!      `canon_decimal` is pure-string and unbounded, so a 43-digit decimal keys today
//!      and would silently LOSE its D-value under the delegation.
//!   2. **Value-space disjointness.** `cmp_relational` implements the XPath promotion
//!      tower behind SPARQL `=`, so it reports `"1"^^xsd:integer` = `"1.0"^^xsd:double`.
//!      Under D these are DIFFERENT value spaces (`DValue::Decimal` vs `DValue::F64` —
//!      see "Why NOT an f64 fast path" above); equating them is exactly the unsound
//!      aliasing this module is built to avoid.
//!   3. **Range facets.** `as_numeric` parses magnitude only, so `cmp_relational`
//!      equates `"200"^^xsd:byte` with `"200"^^xsd:integer` — but 200 is outside the
//!      `byte` value space, so rdfD1 must not type it (see `integer_subtype_ok` below).
//!
//!   There is also a structural blocker: `d_value_key` must return a standalone `Eq` KEY
//!   (`DValue`), and a pairwise `Option<Ordering>` comparator cannot produce one — only
//!   `d_value_eq` could delegate at all. The seam that IS shared, and is the right one, is
//!   `split_decimal`: the reasoner and the engine already agree on which decimal
//!   lexicals are well-formed and on their canonical form, WITHOUT the reasoner
//!   inheriting the engine's `=` semantics where D requires value identity.
//!
//! ## Single-table discipline
//!
//! [SONNET-4.6] sq-pbz04.6.2: `DTYPE_TABLE` is the **single source of truth** for
//! which XSD datatypes are (a) members of `Recognized::standard()` and (b) carry a
//! local value mapping.  Before this refactor, `Recognized::standard()`,
//! `has_value_mapping`, and `d_value_key` maintained three independent lists that
//! could drift. Now any new datatype requires exactly ONE entry in `DTYPE_TABLE`; the
//! recognised set and the has-value-mapping predicate are both derived from it
//! automatically. `d_value_key` still contains the per-type key logic but does not
//! maintain a membership list.
//!
//! ## Deferral ledger — datatypes NOT in the map (honest incompleteness)
//!
//! The following datatypes are deliberately absent from the D map. Their absence is
//! a recorded design decision, not an oversight. Fail-closed behaviour applies: no
//! rdfD1 typing, no value-equality claim, no clash detection.
//!
//! | Datatype | Reason deferred |
//! |---|---|
//! | `xsd:time` | `sparq-core::temporal` deliberately excludes it. A sound value mapping needs the reference-day model plus the `24:00:00` ≡ `00:00:00` and floating-vs-zoned rules. Adding it by value in D while the engine stays lexical would break the §4 parity chain. Revisit only together with an engine/temporal upgrade. |
//! | `xsd:gYear`, `xsd:gMonth`, `xsd:gDay`, `xsd:gYearMonth`, `xsd:gMonthDay` | No parser in `sparq-core`; timezone offsets make ordering partial and equality subtle; negligible corpus incidence. |
//! | `xsd:duration`, `xsd:yearMonthDuration`, `xsd:dayTimeDuration` | Equality is definable as a (months, seconds) pair, but there is no parser and no consumer; the value space is famously partially ordered (`P1M` vs `P30D`), so a naive relational mapping is unsound. |
//! | `rdf:XMLLiteral`, `rdf:HTML` | Value mapping requires DOM/C14N canonicalization — a heavyweight dependency for near-zero test coverage. |
//!
//! Not deferred but EXCLUDED: `xsd:QName`, `xsd:ENTITY`/`ENTITIES`, `xsd:ID`,
//! `xsd:IDREF`/`IDREFS`, `xsd:NOTATION`, and the list types are outside the
//! RDF-compatible XSD subset (RDF 1.1 Concepts §5.1).
//!
//! [SONNET-4.6] sq-pbz04.6.2 — design record: `research/d-entailment-datatype-map.md` §3.2.

use std::sync::OnceLock;

use crate::Vocab;
use rustc_hash::FxHashSet;
use sparq_core::dict::{self, Dict, Id, TermParts};
use sparq_core::{is_integer_datatype, temporal};
// [FABLE-5] sq-pbz04.6.3 (epic sq-pbz04.6, substrate seam 2): the SHARED value-space
// comparator. The integer/decimal canonical-key parsing + normalization delegate to
// `sparq_substrate::numeric::split_decimal` (pure-string, unbounded magnitude — the SAME
// splitter the engine's exact decimal-string compare uses), so the reasoner and the engine
// cannot diverge on which decimal lexicals are well-formed nor on the canonical form.
// [SONNET-4.6] sq-s3b10: the double/float lexical parser NOW ALSO delegates to the SHARED
// `sparq_substrate::numeric::parse_xsd_f64` / `parse_xsd_f32`, so the reasoner and the
// evaluator use the IDENTICAL XSD-conformant acceptance set — "Infinity"/"-Infinity"/"NAN"/
// "nan" are REJECTED by both (XSD forbids them; only "INF"/"-INF"/"NaN" are valid specials).
// Only this module is behind `d-entail`, which pulls the `numeric` slice; the default/lean
// build links none of it.
use sparq_substrate::numeric::{parse_xsd_f32, parse_xsd_f64, split_decimal};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

/// Declarative single-table for the D-entailment datatype map.
///
/// Each entry is `(xsd_local_name, has_value_mapping)`.  A `true` flag means this
/// module has a **sound** value mapping for the datatype — `d_value_key` never
/// accepts an ill-formed lexical form, but some entries are intentionally
/// incomplete (e.g. `Name`/`NCName`/`NMTOKEN` reject well-formed non-ASCII input
/// by design; `anyURI` applies similar restrictions).  See the per-type fn docs
/// for the precise contract of each entry.
///
/// This table is the single source of truth for `Recognized::standard()` and
/// `has_value_mapping()` — add a datatype here to enroll it in BOTH.  The key
/// logic itself lives in `d_value_key`.
///
/// `rdf:langString` and `xsd:string` are always-recognized (by `Recognized::default()`
/// and this table respectively); `rdf:langString` is not in the XSD namespace so it
/// cannot appear here, but `has_value_mapping` handles it as a special case.
///
/// [SONNET-4.6] sq-pbz04.6.2: one-table refactor + D2 broadening.
const DTYPE_TABLE: &[(&str, bool)] = &[
    // ── String family ──────────────────────────────────────────────────────────
    ("string", true),
    ("normalizedString", true),
    ("token", true),
    // [SONNET-4.6] sq-pbz04.6.2: derived-from-token string types.
    ("language", true),
    ("Name", true),
    ("NCName", true),
    ("NMTOKEN", true),
    // ── Boolean ────────────────────────────────────────────────────────────────
    ("boolean", true),
    // ── Integer family (must match is_integer_datatype from sparq-core) ────────
    ("integer", true),
    ("long", true),
    ("int", true),
    ("short", true),
    ("byte", true),
    ("nonNegativeInteger", true),
    ("positiveInteger", true),
    ("nonPositiveInteger", true),
    ("negativeInteger", true),
    ("unsignedLong", true),
    ("unsignedInt", true),
    ("unsignedShort", true),
    ("unsignedByte", true),
    // ── Numeric non-integer ────────────────────────────────────────────────────
    ("decimal", true),
    ("double", true),
    ("float", true),
    // ── Temporal ───────────────────────────────────────────────────────────────
    ("dateTime", true),
    ("dateTimeStamp", true),
    ("date", true),
    // ── URI ─────────────────────────────────────────────────────────────────────
    // [SONNET-4.6] sq-pbz04.6.2: anyURI.
    ("anyURI", true),
    // ── Binary ──────────────────────────────────────────────────────────────────
    // [SONNET-4.6] sq-pbz04.6.2: hexBinary + base64Binary share the Octets key.
    ("hexBinary", true),
    ("base64Binary", true),
];

/// The recognized-datatype set D for a materialization. `xsd:string` and
/// `rdf:langString` are always recognized in RDF 1.1; callers add the others a
/// test or dataset declares (the `sd:recognizedDatatypes` / OWL 2 datatype map).
/// The empty constructor (only the always-recognized pair) is the conservative
/// default — under it the D closure adds NOTHING beyond what is asserted, so it is
/// safe to materialize over an arbitrary graph.
#[derive(Clone, Debug)]
pub struct Recognized {
    iris: FxHashSet<String>,
}

impl Default for Recognized {
    fn default() -> Self {
        let mut iris = FxHashSet::default();
        iris.insert(format!("{}string", XSD));
        iris.insert(RDF_LANG_STRING.to_string());
        Recognized { iris }
    }
}

impl Recognized {
    /// Recognize exactly `iris` (plus the always-recognized `xsd:string` /
    /// `rdf:langString`).
    pub fn new<I: IntoIterator<Item = String>>(iris: I) -> Recognized {
        let mut r = Recognized::default();
        r.iris.extend(iris);
        r
    }

    /// The standard datatype map — every XSD datatype this module has a value
    /// mapping for (the OWL 2 datatype map's numeric/boolean/string/temporal core,
    /// plus the D2-broadened types: `anyURI`, the token-derived string types, and
    /// the binary types).
    ///
    /// Derived from `DTYPE_TABLE` — the single source of truth.
    /// Only entries with `has_value_mapping = true` are included (behaviour is
    /// identical today — all entries are true — but the filter enforces the API
    /// contract so a future fail-closed entry cannot silently slip through).
    /// [SONNET-4.6] sq-pbz04.6.2.
    pub fn standard() -> Recognized {
        Recognized::new(
            DTYPE_TABLE
                .iter()
                .filter(|&&(_, has_map)| has_map)
                .map(|&(l, _)| format!("{}{}", XSD, l)),
        )
    }

    /// Is `dt` in the recognized set?
    pub fn contains(&self, dt: &str) -> bool {
        self.iris.contains(dt)
    }
}

/// Expand `triples` in place with the D-entailment closure for the recognized
/// datatype map `d`: for every well-formed literal `"l"^^t` whose datatype `t` is
/// recognized AND has a value mapping here, add the rdfD1 typing triple
/// `("l"^^t rdf:type t)`. Returns the number of NEW triples added. Idempotent: a
/// second call adds nothing (the typing triples are already present).
///
/// The triple is GENERALIZED — its subject is a literal id — which the regular
/// triple store cannot index; callers that feed the result to a SPARQL query must
/// drop literal-subject rows (they can never be a query answer), exactly the
/// existing entailment-harness contract. The materializer's job is the SOUND
/// closure; answer-shaping is the regime restriction's.
pub fn materialize_d(d: &Recognized, dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> usize {
    let v = Vocab::intern(dict);
    let asserted: FxHashSet<[Id; 3]> = triples.iter().copied().collect();

    // Phase 1 (immutable borrow of `dict`): find every literal id of a recognized,
    // well-formed, value-mapped datatype, recording the literal id + its datatype
    // IRI as an owned String. We do NOT call `intern_iri` here — that needs a
    // mutable borrow while a `TermParts` borrow is live — so the datatype IRI is
    // copied out and interned in phase 2.
    let mut to_type: Vec<(Id, String)> = Vec::new();
    let mut literal_ids: FxHashSet<Id> = FxHashSet::default();
    for &[s, _, o] in triples.iter() {
        for lit_id in [s, o] {
            if !literal_ids.insert(lit_id) {
                continue; // a literal seen once needs typing once
            }
            // Inline ids encode a canonical NON-NEGATIVE small `xsd:integer` (value
            // = id - INLINE_BASE) with NO dictionary record, so `term_parts` cannot
            // resolve them — handle them directly. They are always well-formed and,
            // when xsd:integer is recognized, typed by rdfD1.
            if dict::is_inline(lit_id) {
                if d.contains(XSD_INTEGER) {
                    to_type.push((lit_id, XSD_INTEGER.to_string()));
                }
                continue;
            }
            if let TermParts::Lit { value, datatype, lang } = dict.term_parts(lit_id) {
                // A language-tagged literal's datatype is rdf:langString; rdfD1 adds
                // no useful value-space typing for it, so skip.
                if lang.is_some() {
                    continue;
                }
                if !d.contains(datatype) || !has_value_mapping(datatype) {
                    continue;
                }
                // Ill-formed for its (recognized) datatype: rdfD1 does NOT type it —
                // that case is a D-CLASH (the inconsistency checker's concern), not
                // a typing produced by this monotone closure.
                if d_value_key(value, datatype).is_none() {
                    continue;
                }
                to_type.push((lit_id, datatype.to_string()));
            }
        }
    }

    // Phase 2 (mutable borrow): intern each datatype IRI (idempotent — returns the
    // id the data already uses) and emit the rdfD1 typing triple, deduplicated
    // against what is already asserted. Sort the new rows for deterministic output.
    let mut new_rows: FxHashSet<[Id; 3]> = FxHashSet::default();
    for (lit_id, dt_iri) in to_type {
        let dt_id = dict.intern_iri(&dt_iri);
        let row = [lit_id, v.ty, dt_id];
        if !asserted.contains(&row) {
            new_rows.insert(row);
        }
    }
    let added = new_rows.len();
    let mut new_sorted: Vec<[Id; 3]> = new_rows.into_iter().collect();
    new_sorted.sort_unstable();
    triples.extend(new_sorted);
    added
}

/// Datatypes this module has a value mapping for (so rdfD1 / value comparison can
/// judge well-formedness). Derived from `DTYPE_TABLE` — the single source of truth.
///
/// `rdf:langString` is handled as a special case (not in the XSD namespace).
/// An UNRECOGNIZED-shaped IRI returns false (we cannot judge it, so we never type
/// or clash on it — conservative).
///
/// [SONNET-4.6] sq-pbz04.6.2: refactored to use `DTYPE_TABLE`.
pub fn has_value_mapping(dt: &str) -> bool {
    if dt == RDF_LANG_STRING {
        return true;
    }
    if let Some(local) = dt.strip_prefix(XSD) {
        return DTYPE_TABLE.iter().any(|&(l, has_map)| l == local && has_map);
    }
    false
}

/// A CANONICAL, value-space comparison key for a typed literal: two literals denote
/// the same D-value iff their keys are equal. `None` = ill-formed for its datatype
/// (no value), or a datatype with no value mapping here.
///
/// Numbers in the integer/decimal value space share ONE key form
/// ([`DValue::Decimal`], a canonical decimal STRING — never an `f64`), so
/// `"1"^^xsd:integer`, `"01"^^xsd:integer`, `"1.0"^^xsd:decimal` and
/// `"+1.00"^^xsd:decimal` all key-equal. `xsd:float` / `xsd:double` are a DISTINCT
/// value space (IEEE-754) and key by the rounded bit pattern. Temporal types key by
/// the sparq-core `Temporal` instant.
///
/// `xsd:hexBinary` and `xsd:base64Binary` share [`DValue::Octets`] (the same XSD 1.1
/// value space: "finite-length sequences of binary octets" — §3.3.15/§3.3.16). So
/// `"61"^^xsd:hexBinary` and `"YQ=="^^xsd:base64Binary` both decode to `[0x61]` and
/// key-equal.
///
/// `xsd:anyURI` uses [`DValue::Uri`], which is DISJOINT from `DValue::Str`:
/// XSD 1.1 §3.3.17 defines anyURI as a distinct primitive; no cross-type key-equality
/// with `xsd:string` is sanctioned.
///
/// [FABLE-5] sq-pbz04.6.3: the integer/decimal canonical key delegates to the shared
/// `sparq_substrate::numeric::split_decimal` (see the private `canon_decimal` helper).
/// [SONNET-4.6] sq-s3b10: the double/float lexical parser NOW DELEGATES to the shared
/// `sparq_substrate::numeric::parse_xsd_f64` / `parse_xsd_f32` — the local
/// `parse_xsd_double` helper is removed; see the module doc's ledger for the tightening.
/// `integer_subtype_ok` stays local (facet validation is dtype-resident by design).
///
/// [SONNET-4.6] sq-pbz04.6.2: added anyURI, language/Name/NCName/NMTOKEN,
/// hexBinary/base64Binary.
pub fn d_value_key(lex: &str, dt: &str) -> Option<DValue> {
    if dt == RDF_LANG_STRING {
        return None; // language-tagged: keyed by (lex, lang) elsewhere, not here
    }
    if dt == format!("{}string", XSD) {
        // xsd:string: any Unicode string is in the lexical space (no restriction).
        return Some(DValue::Str(lex.to_string()));
    }
    if dt == format!("{}normalizedString", XSD) {
        // [SONNET-4.6] sq-pbz04.6.1: normalizedString forbids TAB/LF/CR (XSD 1.1
        // §3.3.2). A lexical form containing one is ill-formed → no D-value.
        if lex.contains(['\t', '\n', '\r']) {
            return None;
        }
        return Some(DValue::Str(lex.to_string()));
    }
    if dt == format!("{}token", XSD) {
        // [SONNET-4.6] sq-pbz04.6.1: token is the COLLAPSED form of
        // normalizedString (XSD 1.1 §3.3.3): no TAB/LF/CR, no leading or trailing
        // space, and no internal run of 2+ spaces. Anything else is ill-formed.
        if lex.contains(['\t', '\n', '\r'])
            || lex.starts_with(' ')
            || lex.ends_with(' ')
            || lex.contains("  ")
        {
            return None;
        }
        return Some(DValue::Str(lex.to_string()));
    }
    // ── Token-derived string types (§3.1 D2) ──────────────────────────────────
    // [SONNET-4.6] sq-pbz04.6.2: language, Name, NCName, NMTOKEN are all derived
    // from xsd:token by restriction → their values ARE string values → share
    // DValue::Str. Pattern-facet validation is required before accepting.
    if dt == format!("{}language", XSD) {
        // XSD 1.1 §3.3.6: pattern [a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*.
        // Case-significant: "EN" ≠ "en" as values. The token constraints (no
        // TAB/LF/CR, no leading/trailing spaces, no double spaces) are implied by
        // the language pattern (which forbids spaces entirely).
        if !is_valid_language(lex) {
            return None;
        }
        return Some(DValue::Str(lex.to_string()));
    }
    if dt == format!("{}Name", XSD) {
        // XSD 1.1 §3.4.7: xml:Name — NameStartChar NameChar*.
        // Conservative ASCII subset (see `is_valid_xml_name`).
        if !is_valid_xml_name(lex) {
            return None;
        }
        return Some(DValue::Str(lex.to_string()));
    }
    if dt == format!("{}NCName", XSD) {
        // XSD 1.1 §3.4.8: xml:NCName — like Name but no colon ':' allowed.
        // Conservative ASCII subset (see `is_valid_xml_ncname`).
        if !is_valid_xml_ncname(lex) {
            return None;
        }
        return Some(DValue::Str(lex.to_string()));
    }
    if dt == format!("{}NMTOKEN", XSD) {
        // XSD 1.1 §3.4.6: xml:NMTOKEN — one or more NameChar.
        // Conservative ASCII subset (see `is_valid_xml_nmtoken`).
        if !is_valid_xml_nmtoken(lex) {
            return None;
        }
        return Some(DValue::Str(lex.to_string()));
    }
    // ── Boolean ────────────────────────────────────────────────────────────────
    if dt == format!("{}boolean", XSD) {
        return match lex {
            "true" | "1" => Some(DValue::Bool(true)),
            "false" | "0" => Some(DValue::Bool(false)),
            _ => None,
        };
    }
    // ── Integer family ─────────────────────────────────────────────────────────
    if is_integer_datatype(dt) {
        let v: i128 = lex.parse().ok()?;
        // Integer-subtype range facets (the value must be IN the datatype's space).
        if !integer_subtype_ok(dt, v) {
            return None;
        }
        return Some(DValue::Decimal(canon_decimal(&v.to_string())?));
    }
    if dt == format!("{}decimal", XSD) {
        return Some(DValue::Decimal(canon_decimal(lex)?));
    }
    if dt == format!("{}double", XSD) {
        // [SONNET-4.6] sq-s3b10: delegate to the SHARED substrate parser (XSD-conformant
        // allowlist) so dtype.rs and the evaluator agree exactly on which double lexicals
        // are well-formed. Tightening: "Infinity"/"-Infinity"/"NAN"/"nan" → None (correct
        // per XSD; only "INF"/"-INF"/"NaN" are valid specials).
        let f = parse_xsd_f64(lex)?;
        return Some(DValue::F64(if f.is_nan() {
            f64::NAN.to_bits()
        } else {
            f.to_bits()
        }));
    }
    if dt == format!("{}float", XSD) {
        // [SONNET-4.6] sq-s3b10: same tightening as double — delegate to the shared
        // substrate parse_xsd_f32 (which in turn calls parse_xsd_f64 → sparq_core).
        let f = parse_xsd_f32(lex)?;
        return Some(DValue::F32(if f.is_nan() {
            f32::NAN.to_bits()
        } else {
            f.to_bits()
        }));
    }
    if dt == format!("{}dateTime", XSD)
        || dt == format!("{}dateTimeStamp", XSD)
        || dt == format!("{}date", XSD)
    {
        let t = temporal::Temporal::of_lit(lex, dt)?;
        // Key by the comparable instant (sparq-core's correct temporal value),
        // tagged with the temporal family and tz-presence so a `date` and a
        // `dateTime` at the same instant (DISJOINT value spaces) never key-equal,
        // and a floating vs zoned same-instant pair stays distinguishable.
        let kind = matches!(t.kind, temporal::TemporalKind::Date);
        return Some(DValue::Temporal(t.instant.to_bits(), t.has_tz, kind));
    }
    // ── URI (§3.1 D2) ─────────────────────────────────────────────────────────
    if dt == format!("{}anyURI", XSD) {
        // XSD 1.1 §3.3.17: the lexical space of anyURI is the set of finite-length
        // sequences of Unicode characters — any string is a valid anyURI lexical form.
        // Value = the character sequence; equality = codepoint equality, NO escaping
        // normalization: "a b" ≠ "a%20b" (XSD 1.1 §3.3.17 note 2).
        // DISJOINT from DValue::Str (conservative per design record §3.1): anyURI is
        // defined as its own primitive in XSD 1.1; cross-type key-equality with
        // xsd:string is not sanctioned.
        // NOTE (incomplete-but-sound): anyURI carries a whiteSpace=collapse facet;
        // whitespace-variant forms (e.g. "a  b" vs "a b") get DISTINCT keys here —
        // they are never incorrectly equated, but two canonically-collapsed-equal
        // values may not merge. This is deliberate: collapse is not applied.
        // [SONNET-4.6] sq-pbz04.6.2.
        return Some(DValue::Uri(lex.to_string()));
    }
    // ── Binary (§3.1 D2) ──────────────────────────────────────────────────────
    if dt == format!("{}hexBinary", XSD) {
        // XSD 1.1 §3.3.15: value space = "finite-length sequences of binary octets".
        // Lexical space: even-length strings over [0-9A-Fa-f] (case-insensitive).
        // Decoded octet sequence is the canonical key — shared with base64Binary.
        // [SONNET-4.6] sq-pbz04.6.2.
        return decode_hex_binary(lex).map(DValue::Octets);
    }
    if dt == format!("{}base64Binary", XSD) {
        // XSD 1.1 §3.3.16: value space = "finite-length sequences of binary octets"
        // — IDENTICAL to hexBinary's value space (§3.3.15/§3.3.16). Equal octet
        // sequences are equal D-values ACROSS the two datatypes (the binary analogue
        // of integer ⊂ decimal). XSD allows embedded whitespace in the lexical form.
        // [SONNET-4.6] sq-pbz04.6.2.
        return decode_base64_binary(lex).map(DValue::Octets);
    }
    None
}

/// True D-value equality for two typed literals under recognized-map semantics.
/// Equal iff both have a value mapping AND their canonical value keys coincide.
pub fn d_value_eq(lex_a: &str, dt_a: &str, lex_b: &str, dt_b: &str) -> bool {
    match (d_value_key(lex_a, dt_a), d_value_key(lex_b, dt_b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// A canonical D-value, comparable for equality across lexical forms (and across
/// the integer/decimal value spaces, which coincide). NOT an `f64`-collapsed view
/// for the exact value spaces — see the module doc on why the f64 fast path is
/// unsound for semantic equality.
///
/// [SONNET-4.6] sq-pbz04.6.2: added `Uri` and `Octets` variants.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum DValue {
    /// Canonical decimal string (sign + minimal int digits + minimal frac digits)
    /// — the shared key for `xsd:integer` (+ subtypes) and `xsd:decimal`.
    Decimal(String),
    /// `xsd:double` value as its IEEE-754 bit pattern (NaN canonicalized).
    F64(u64),
    /// `xsd:float` value as its IEEE-754 bit pattern (NaN canonicalized).
    F32(u32),
    Bool(bool),
    Str(String),
    /// A temporal value: (instant f64 bits, tz-present, is-date-family). The
    /// instant is sparq-core's `Temporal::instant` (which orders the value space);
    /// the family flag keeps `xsd:date` and `xsd:dateTime` — DISJOINT value spaces
    /// — from key-aliasing at a shared instant.
    Temporal(u64, bool, bool),
    /// URI reference value (`xsd:anyURI`) — codepoint equality, no escaping
    /// normalization. DISJOINT from `Str` (anyURI is a distinct XSD 1.1 primitive).
    /// [SONNET-4.6] sq-pbz04.6.2.
    Uri(String),
    /// Binary octet sequence — shared key for `xsd:hexBinary` and
    /// `xsd:base64Binary`. XSD 1.1 §3.3.15/§3.3.16 define BOTH value spaces as
    /// "finite-length sequences of binary octets", so equal octet sequences are
    /// equal D-values across the two datatypes.
    /// [SONNET-4.6] sq-pbz04.6.2.
    Octets(Vec<u8>),
}

/// The integer-subtype range facet check: the value must be inside the bounded
/// derived type's value space (e.g. `xsd:byte` is [-128, 127]). [SONNET-4.6]
/// sq-pbz04.6.1: every derived integer type carries BOTH its sign facet AND its
/// magnitude bounds. A value like `"200"^^xsd:byte` parses fine as `i128` but is
/// outside the `byte` value space, so it is ill-formed and must NOT be typed by
/// rdfD1; likewise `"4294967296"^^xsd:unsignedInt` exceeds the `unsignedInt` upper
/// bound. Only genuinely-unbounded `xsd:integer` (and any unrecognized-shaped IRI)
/// falls through to the permissive `_` arm. Ranges use `RangeInclusive::contains`
/// so the two-sided bound stays `clippy::manual_range_contains`-clean.
fn integer_subtype_ok(dt: &str, v: i128) -> bool {
    let Some(local) = dt.strip_prefix(XSD) else {
        return true;
    };
    match local {
        // Sign-only facets (no magnitude bound in the value space). [SONNET-4.6]
        "nonNegativeInteger" => v >= 0,
        "positiveInteger" => v > 0,
        "nonPositiveInteger" => v <= 0,
        "negativeInteger" => v < 0,
        // Bounded signed derived integers. [SONNET-4.6]
        "long" => (i64::MIN as i128..=i64::MAX as i128).contains(&v),
        "int" => (i32::MIN as i128..=i32::MAX as i128).contains(&v),
        "short" => (-32768..=32767).contains(&v),
        "byte" => (-128..=127).contains(&v),
        // Bounded unsigned derived integers (lower bound 0 AND an upper bound). [SONNET-4.6]
        "unsignedLong" => (0..=18446744073709551615_i128).contains(&v),
        "unsignedInt" => (0..=4294967295).contains(&v),
        "unsignedShort" => (0..=65535).contains(&v),
        "unsignedByte" => (0..=255).contains(&v),
        _ => true,
    }
}

/// Canonicalize a decimal lexical form to (sign)(minimal-int).(minimal-frac);
/// `None` when ill-formed. `"-0.0"` canonicalizes to `"0"`; `"1.0"` to `"1"`; so
/// the integer 1 and the decimal 1.0 share the key `"1"`.
///
/// [FABLE-5] sq-pbz04.6.3: the split + normalization delegates to the SHARED
/// `sparq_substrate::numeric::split_decimal` (pure string, no `i128` bound — so an
/// arbitrary-magnitude decimal like a 40-digit lexical still keys, exactly as the
/// pre-migration hand-rolled version did; the substrate `Dec::parse` path would have
/// overflowed and DROPPED such values, a behaviour change we deliberately avoid). The
/// final canonical STRING assembly stays here because it is the D-value key form, not a
/// comparator concern. `split_decimal` trims leading/trailing ASCII whitespace whereas the
/// old hand-rolled splitter did NOT; to keep behaviour byte-identical (a padded decimal
/// lexical was rejected before), reject any surrounding whitespace up front.
///
/// [SONNET-4.6] sq-fvxko (issue #3137): do NOT "simplify" this into a delegation to
/// `Num::cmp_relational`. That comparator loses arbitrary-magnitude decimals, promotes
/// across the decimal/IEEE-754 boundary D keeps disjoint, and ignores the integer-subtype
/// range facets — see the module doc's ledger and
/// `tests::cmp_relational_delegation_would_change_behaviour`, which turns red on each.
fn canon_decimal(lex: &str) -> Option<String> {
    // Preserve the pre-migration no-trim contract: `split_decimal` calls `s.trim()`, so
    // `" 1"` would become `"1"` (Some) there; the old splitter's digit-check rejected the
    // space (None). Reject surrounding whitespace so the accept/reject set is unchanged.
    if lex.trim() != lex {
        return None;
    }
    let (neg, int_trim, frac_trim) = split_decimal(lex)?;
    let int_c = if int_trim.is_empty() { "0" } else { int_trim };
    let neg = neg && !(int_c == "0" && frac_trim.is_empty());
    Some(format!(
        "{}{}{}{}",
        if neg { "-" } else { "" },
        int_c,
        if frac_trim.is_empty() { "" } else { "." },
        frac_trim
    ))
}

// ── Pattern-validation helpers for token-derived string types ─────────────────────
//
// All four types (language, Name, NCName, NMTOKEN) are derived from xsd:token by
// restriction, so their VALUES are string values keyed by DValue::Str. The pattern
// facet determines well-formedness. Validators return `true` iff the lexical form
// is a valid member of the type's lexical space.
//
// [SONNET-4.6] sq-pbz04.6.2.

/// Validate `xsd:language` (XSD 1.1 §3.3.6).
///
/// Pattern: `[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*`.  Case-significant: "EN" ≠ "en".
/// Uses a OnceLock-cached `Regex` (the `regex` crate is already in this crate's
/// dependency tree for the N3 `string:matches` builtin).
/// [SONNET-4.6] sq-pbz04.6.2.
fn is_valid_language(lex: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*$")
            .expect("valid xsd:language pattern")
    });
    re.is_match(lex)
}

/// Validate `xsd:NMTOKEN` (XSD 1.1 §3.4.6 — xml:NMTOKEN production).
///
/// NMTOKEN = NameChar+.  This implementation uses a conservative **ASCII subset**
/// of the NameChar production: `[a-zA-Z0-9._\-:]`.  Full NameChar includes many
/// non-ASCII Unicode ranges; rejecting those is incomplete-but-sound (may reject
/// some valid non-ASCII NMTOKENs, but will never ACCEPT an invalid one).
/// [SONNET-4.6] sq-pbz04.6.2.
fn is_valid_xml_nmtoken(lex: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^[a-zA-Z0-9._\-:]+$").expect("valid xsd:NMTOKEN pattern")
    });
    re.is_match(lex)
}

/// Validate `xsd:Name` (XSD 1.1 §3.4.7 — xml:Name production).
///
/// Name = NameStartChar NameChar*.  This implementation uses a conservative ASCII
/// subset: NameStartChar = `[a-zA-Z_:]`, NameChar = `[a-zA-Z0-9._\-:]`.  Colons
/// are allowed (Name permits them; NCName does not).  Non-ASCII valid start/body
/// chars are rejected — incomplete-but-sound.
/// [SONNET-4.6] sq-pbz04.6.2.
fn is_valid_xml_name(lex: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^[a-zA-Z_:][a-zA-Z0-9._\-:]*$").expect("valid xsd:Name pattern")
    });
    re.is_match(lex)
}

/// Validate `xsd:NCName` (XSD 1.1 §3.4.8 — xml:NCName production).
///
/// NCName = NCNameStartChar NCNameChar*.  Like Name but colons are FORBIDDEN.  This
/// implementation uses a conservative ASCII subset: NCNameStartChar = `[a-zA-Z_]`,
/// NCNameChar = `[a-zA-Z0-9._\-]` (no colon).  Non-ASCII valid start/body chars
/// are rejected — incomplete-but-sound.
/// [SONNET-4.6] sq-pbz04.6.2.
fn is_valid_xml_ncname(lex: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^[a-zA-Z_][a-zA-Z0-9._\-]*$").expect("valid xsd:NCName pattern")
    });
    re.is_match(lex)
}

// ── Binary decoders ───────────────────────────────────────────────────────────────
//
// Both hexBinary and base64Binary decode to Vec<u8> (the Octets key). No external
// dep required: both are straightforward decoding tasks without the error-recovery
// or streaming concerns that make a library worthwhile.
//
// [SONNET-4.6] sq-pbz04.6.2.

/// Decode an `xsd:hexBinary` lexical form to a byte sequence.
///
/// Lexical space: an even-length string over `[0-9A-Fa-f]` (case-insensitive).
/// Returns `None` for odd-length strings or strings containing non-hex characters.
/// [SONNET-4.6] sq-pbz04.6.2.
fn decode_hex_binary(lex: &str) -> Option<Vec<u8>> {
    if !lex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(lex.len() / 2);
    let lex_bytes = lex.as_bytes();
    let mut i = 0;
    while i < lex_bytes.len() {
        let hi = hex_nibble(lex_bytes[i])?;
        let lo = hex_nibble(lex_bytes[i + 1])?;
        bytes.push((hi << 4) | lo);
        i += 2;
    }
    Some(bytes)
}

/// Decode one hex nibble from an ASCII byte. Case-insensitive.
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decode an `xsd:base64Binary` lexical form to a byte sequence.
///
/// XSD 1.1 §3.3.16 allows embedded whitespace in the lexical form; this function
/// strips XML S characters (SP/TAB/CR/LF — the four characters in the XML 1.0
/// production `S`) before decoding.  Non-XML whitespace (VT 0x0B, FF 0x0C, …)
/// is NOT stripped and causes the form to be rejected as ill-formed, per XSD 1.1
/// which inherits the XML S definition.
///
/// The cleaned form must be a multiple of 4 characters from the base64 alphabet
/// (`A-Z`, `a-z`, `0-9`, `+`, `/`) with `=` padding. Returns `None` for ill-formed
/// input.
/// [SONNET-4.6] sq-pbz04.6.2.
fn decode_base64_binary(lex: &str) -> Option<Vec<u8>> {
    // Strip XML S characters only (SP/TAB/CR/LF per XML 1.0 production S).
    // `is_ascii_whitespace()` would also strip VT (0x0B) and FF (0x0C), which
    // are NOT XML whitespace; those must be rejected as ill-formed. [SONNET-4.6]
    let cleaned: Vec<u8> = lex
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .collect();
    if !cleaned.len().is_multiple_of(4) {
        return None;
    }
    let mut bytes = Vec::with_capacity(cleaned.len() * 3 / 4);
    let mut i = 0;
    while i < cleaned.len() {
        let a = b64_val(cleaned[i])?;
        let b = b64_val(cleaned[i + 1])?;
        let c_byte = cleaned[i + 2];
        let d_byte = cleaned[i + 3];
        if c_byte == b'=' {
            // Two padding bytes: "ab==" encodes 1 octet.
            if d_byte != b'=' {
                return None;
            }
            // XSD 1.1 §3.3.16: the low 4 bits of `b` are discarded bits and MUST
            // be zero in a canonical lexical form; reject non-canonical encoding.
            if (b & 0x0f) != 0 {
                return None;
            }
            bytes.push((a << 2) | (b >> 4));
            if i + 4 != cleaned.len() {
                return None; // padding must be at the end
            }
        } else {
            let c = b64_val(c_byte)?;
            if d_byte == b'=' {
                // One padding byte: "abc=" encodes 2 octets.
                // XSD 1.1 §3.3.16: the low 2 bits of `c` are discarded bits and
                // MUST be zero in a canonical lexical form; reject non-canonical.
                if (c & 0x03) != 0 {
                    return None;
                }
                bytes.push((a << 2) | (b >> 4));
                bytes.push(((b & 0x0f) << 4) | (c >> 2));
                if i + 4 != cleaned.len() {
                    return None; // padding must be at the end
                }
            } else {
                let d = b64_val(d_byte)?;
                bytes.push((a << 2) | (b >> 4));
                bytes.push(((b & 0x0f) << 4) | (c >> 2));
                bytes.push(((c & 0x03) << 6) | d);
            }
        }
        i += 4;
    }
    Some(bytes)
}

/// Map a base64 alphabet byte to its 6-bit value.
fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::vocab::rdf;
    use oxrdf::{Literal, NamedNode, Term};

    fn lit(value: &str, local: &str) -> Term {
        Term::Literal(Literal::new_typed_literal(
            value,
            NamedNode::new_unchecked(format!("{}{}", XSD, local)),
        ))
    }

    /// The load-bearing invariant: integer/decimal value-space equality across
    /// lexical forms, using the CORRECT TYPED (canonical-decimal) comparison — NOT
    /// an f64 fast path.
    #[test]
    fn integer_decimal_value_space_coincides() {
        // "1"^^xsd:integer is equivalent to "1.0"^^xsd:decimal — same value.
        assert!(d_value_eq("1", &format!("{}integer", XSD), "1.0", &format!("{}decimal", XSD)));
        // leading zeros / signs / trailing fraction zeros are all the same value.
        assert!(d_value_eq("01", &format!("{}integer", XSD), "+1", &format!("{}integer", XSD)));
        assert!(d_value_eq("1", &format!("{}integer", XSD), "1.00", &format!("{}decimal", XSD)));
        assert!(d_value_eq("-0", &format!("{}integer", XSD), "0", &format!("{}decimal", XSD)));
        // distinct values are NOT equal.
        assert!(!d_value_eq("1", &format!("{}integer", XSD), "2", &format!("{}decimal", XSD)));
        assert!(!d_value_eq("1.5", &format!("{}decimal", XSD), "1", &format!("{}integer", XSD)));
    }

    /// f64 would silently alias these past the 53-bit mantissa; the canonical
    /// decimal comparison keeps them DISTINCT (the unsound-f64 guard).
    #[test]
    fn large_integers_are_not_aliased_by_f64() {
        let big_a = "9007199254740993"; // 2^53 + 1, not representable in f64
        let big_b = "9007199254740992"; // 2^53
        assert!(!d_value_eq(
            big_a,
            &format!("{}integer", XSD),
            big_b,
            &format!("{}integer", XSD)
        ));
        // ...but each equals its own decimal spelling exactly.
        assert!(d_value_eq(
            big_a,
            &format!("{}integer", XSD),
            "9007199254740993.0",
            &format!("{}decimal", XSD)
        ));
    }

    /// float and double are a SEPARATE value space from integer/decimal.
    #[test]
    fn float_double_distinct_from_decimal() {
        assert!(!d_value_eq("1", &format!("{}integer", XSD), "1.0", &format!("{}double", XSD)));
        assert!(!d_value_eq("1.0", &format!("{}decimal", XSD), "1.0", &format!("{}float", XSD)));
        // but two double lexical forms of the same value are equal.
        assert!(d_value_eq("1.0E0", &format!("{}double", XSD), "1.0", &format!("{}double", XSD)));
    }

    #[test]
    fn integer_subtype_range_facets() {
        // 200 is out of xsd:byte [-128,127] in the XSD value space → ill-formed (no value).
        assert!(d_value_key("-1", &format!("{}nonNegativeInteger", XSD)).is_none());
        assert!(d_value_key("0", &format!("{}positiveInteger", XSD)).is_none());
        assert!(d_value_key("1", &format!("{}nonPositiveInteger", XSD)).is_none());
        // in range parses.
        assert!(d_value_key("100", &format!("{}nonNegativeInteger", XSD)).is_some());
    }

    #[test]
    fn ill_formed_literal_has_no_value() {
        assert!(d_value_key("abc", &format!("{}integer", XSD)).is_none());
        assert!(d_value_key("not-a-date", &format!("{}dateTime", XSD)).is_none());
    }

    /// rdfD1: a recognized, well-formed literal is typed by its datatype; the
    /// closure is idempotent and adds nothing for an unrecognized datatype.
    #[test]
    fn materialize_d_types_recognized_literals() {
        let mut dict = Dict::new();
        let s = dict.intern(&Term::NamedNode(NamedNode::new_unchecked("http://e/s")));
        let p = dict.intern(&Term::NamedNode(NamedNode::new_unchecked("http://e/p")));
        let one = dict.intern(&lit("1", "integer"));
        let ty = dict.intern(&Term::NamedNode(NamedNode::new_unchecked(rdf::TYPE.as_str())));
        let xsd_integer =
            dict.intern(&Term::NamedNode(NamedNode::new_unchecked(format!("{}integer", XSD))));
        let mut triples = vec![[s, p, one]];

        // Recognized: the literal is typed xsd:integer (rdfD1).
        let d = Recognized::new([format!("{}integer", XSD)]);
        let added = materialize_d(&d, &mut dict, &mut triples);
        assert_eq!(added, 1, "rdfD1 types the recognized integer literal");
        assert!(
            triples.contains(&[one, ty, xsd_integer]),
            "(\"1\"^^xsd:integer rdf:type xsd:integer)"
        );

        // Idempotent.
        let added2 = materialize_d(&d, &mut dict, &mut triples);
        assert_eq!(added2, 0, "second materialization adds nothing");
    }

    #[test]
    fn materialize_d_skips_unrecognized_datatype() {
        let mut dict = Dict::new();
        let s = dict.intern(&Term::NamedNode(NamedNode::new_unchecked("http://e/s")));
        let p = dict.intern(&Term::NamedNode(NamedNode::new_unchecked("http://e/p")));
        let one = dict.intern(&lit("1", "integer"));
        let mut triples = vec![[s, p, one]];
        // integer NOT in D (only the always-recognized string/langString).
        let d = Recognized::default();
        let added = materialize_d(&d, &mut dict, &mut triples);
        assert_eq!(added, 0, "an unrecognized datatype yields no typing");
    }

    #[test]
    fn materialize_d_skips_ill_typed_literal() {
        let mut dict = Dict::new();
        let s = dict.intern(&Term::NamedNode(NamedNode::new_unchecked("http://e/s")));
        let p = dict.intern(&Term::NamedNode(NamedNode::new_unchecked("http://e/p")));
        let bad = dict.intern(&lit("abc", "integer"));
        let mut triples = vec![[s, p, bad]];
        let d = Recognized::new([format!("{}integer", XSD)]);
        let added = materialize_d(&d, &mut dict, &mut triples);
        assert_eq!(
            added, 0,
            "ill-typed literal is not typed by rdfD1 (it is a clash, not a typing)"
        );
    }

    // [SONNET-4.6] sq-pbz04.6.1: derived-integer magnitude facets. Each of these
    // fails on the pre-fix `_ => true` / grouped-sign-only code (non-vacuous).
    #[test]
    fn byte_out_of_range_not_typed() {
        // "200"^^xsd:byte is ill-formed: byte is [-128, 127]
        // Bug: pre-fix code returns Some (wrongly typed)
        assert!(d_value_key("200", &format!("{}byte", XSD)).is_none(), "200 is outside xsd:byte");
        assert!(
            d_value_key("-129", &format!("{}byte", XSD)).is_none(),
            "-129 is outside xsd:byte"
        );
        // In-range values are well-formed
        assert!(d_value_key("127", &format!("{}byte", XSD)).is_some(), "127 is xsd:byte max");
        assert!(d_value_key("-128", &format!("{}byte", XSD)).is_some(), "-128 is xsd:byte min");
        assert!(d_value_key("0", &format!("{}byte", XSD)).is_some(), "0 is valid xsd:byte");
    }

    #[test]
    fn short_out_of_range_not_typed() {
        assert!(
            d_value_key("70000", &format!("{}short", XSD)).is_none(),
            "70000 is outside xsd:short"
        );
        assert!(
            d_value_key("32767", &format!("{}short", XSD)).is_some(),
            "32767 is xsd:short max"
        );
        assert!(
            d_value_key("-32768", &format!("{}short", XSD)).is_some(),
            "-32768 is xsd:short min"
        );
    }

    #[test]
    fn unsigned_int_out_of_range_not_typed() {
        assert!(
            d_value_key("4294967296", &format!("{}unsignedInt", XSD)).is_none(),
            "4294967296 exceeds xsd:unsignedInt max"
        );
        assert!(
            d_value_key("4294967295", &format!("{}unsignedInt", XSD)).is_some(),
            "4294967295 is xsd:unsignedInt max"
        );
        assert!(
            d_value_key("-1", &format!("{}unsignedInt", XSD)).is_none(),
            "negative is outside xsd:unsignedInt"
        );
    }

    #[test]
    fn unsigned_byte_out_of_range_not_typed() {
        assert!(
            d_value_key("256", &format!("{}unsignedByte", XSD)).is_none(),
            "256 exceeds xsd:unsignedByte max"
        );
        assert!(
            d_value_key("255", &format!("{}unsignedByte", XSD)).is_some(),
            "255 is xsd:unsignedByte max"
        );
        assert!(
            d_value_key("0", &format!("{}unsignedByte", XSD)).is_some(),
            "0 is xsd:unsignedByte min"
        );
    }

    // [SONNET-4.6] sq-pbz04.6.1: string-family lexical-space validation.
    #[test]
    fn token_lexical_space_validated() {
        // " a"^^xsd:token has a leading space — illegal for token
        // Bug: pre-fix code returns Some (wrongly typed)
        assert!(
            d_value_key(" a", &format!("{}token", XSD)).is_none(),
            "leading space is illegal for xsd:token"
        );
        assert!(
            d_value_key("a ", &format!("{}token", XSD)).is_none(),
            "trailing space is illegal for xsd:token"
        );
        assert!(
            d_value_key("a  b", &format!("{}token", XSD)).is_none(),
            "double space is illegal for xsd:token"
        );
        assert!(
            d_value_key("a\tb", &format!("{}token", XSD)).is_none(),
            "tab is illegal for xsd:token"
        );
        // Valid tokens
        assert!(
            d_value_key("a", &format!("{}token", XSD)).is_some(),
            "simple word is valid xsd:token"
        );
        assert!(
            d_value_key("a b", &format!("{}token", XSD)).is_some(),
            "two words with single space is valid xsd:token"
        );
        assert!(
            d_value_key("", &format!("{}token", XSD)).is_some(),
            "empty string is valid xsd:token"
        );
    }

    #[test]
    fn normalized_string_lexical_space_validated() {
        // normalizedString forbids \t, \n, \r
        // Bug: pre-fix code returns Some (wrongly typed)
        assert!(
            d_value_key("a\tb", &format!("{}normalizedString", XSD)).is_none(),
            "tab is illegal for xsd:normalizedString"
        );
        assert!(
            d_value_key("a\nb", &format!("{}normalizedString", XSD)).is_none(),
            "newline is illegal for xsd:normalizedString"
        );
        assert!(
            d_value_key("a\rb", &format!("{}normalizedString", XSD)).is_none(),
            "carriage-return is illegal for xsd:normalizedString"
        );
        // Valid normalizedString: leading/trailing spaces and double spaces ARE allowed
        assert!(
            d_value_key(" a", &format!("{}normalizedString", XSD)).is_some(),
            "leading space is valid for xsd:normalizedString"
        );
        assert!(
            d_value_key("a  b", &format!("{}normalizedString", XSD)).is_some(),
            "double space is valid for xsd:normalizedString (only forbidden by token)"
        );
    }

    // ── D2 broadening tests [SONNET-4.6] sq-pbz04.6.2 ─────────────────────────

    /// xsd:anyURI accepts any Unicode string and is disjoint from xsd:string.
    #[test]
    fn any_uri_value_key() {
        // Any string is a valid anyURI lexical form (XSD 1.1 §3.3.17).
        assert!(
            d_value_key("", &format!("{}anyURI", XSD)).is_some(),
            "empty string is a valid xsd:anyURI"
        );
        assert!(
            d_value_key("http://example.org/", &format!("{}anyURI", XSD)).is_some(),
            "HTTP IRI is a valid xsd:anyURI"
        );
        assert!(
            d_value_key("a b", &format!("{}anyURI", XSD)).is_some(),
            "string with space is a valid xsd:anyURI (no encoding normalization)"
        );
        // anyURI is DISJOINT from xsd:string: same lex, different value spaces.
        let uri_key = d_value_key("foo", &format!("{}anyURI", XSD));
        let str_key = d_value_key("foo", &format!("{}string", XSD));
        assert!(uri_key.is_some() && str_key.is_some());
        assert_ne!(
            uri_key, str_key,
            "xsd:anyURI and xsd:string are disjoint: same lex must NOT key-equal"
        );
    }

    /// xsd:language pattern validation + case-sensitivity.
    #[test]
    fn language_value_key() {
        // Valid
        assert!(d_value_key("en", &format!("{}language", XSD)).is_some(), "\"en\" is valid");
        assert!(d_value_key("en-US", &format!("{}language", XSD)).is_some(), "\"en-US\" is valid");
        assert!(
            d_value_key("EN", &format!("{}language", XSD)).is_some(),
            "\"EN\" is valid (case-significant)"
        );
        assert!(
            d_value_key("en-US-academic", &format!("{}language", XSD)).is_some(),
            "multiple subtags valid"
        );
        // Case-significant: "EN" ≠ "en" as values.
        assert_ne!(
            d_value_key("en", &format!("{}language", XSD)),
            d_value_key("EN", &format!("{}language", XSD)),
            "xsd:language is case-significant: EN ≠ en"
        );
        // Invalid
        assert!(
            d_value_key("", &format!("{}language", XSD)).is_none(),
            "empty string invalid for xsd:language"
        );
        assert!(
            d_value_key("toolongggg", &format!("{}language", XSD)).is_none(),
            "primary tag >8 chars is invalid"
        );
        assert!(
            d_value_key("en--US", &format!("{}language", XSD)).is_none(),
            "double dash is invalid"
        );
        assert!(
            d_value_key("en-toolongsubtag", &format!("{}language", XSD)).is_none(),
            "subtag >8 chars is invalid"
        );
    }

    /// xsd:Name: starts with NameStartChar (letter, '_', ':'), colons allowed.
    #[test]
    fn xml_name_value_key() {
        assert!(d_value_key("foo", &format!("{}Name", XSD)).is_some(), "\"foo\" is valid Name");
        assert!(
            d_value_key("foo:bar", &format!("{}Name", XSD)).is_some(),
            "colon allowed in Name"
        );
        assert!(d_value_key("_foo", &format!("{}Name", XSD)).is_some(), "underscore start valid");
        assert!(
            d_value_key(":foo", &format!("{}Name", XSD)).is_some(),
            "colon start valid for Name"
        );
        // Invalid
        assert!(
            d_value_key("1foo", &format!("{}Name", XSD)).is_none(),
            "digit start invalid for Name"
        );
        assert!(d_value_key("", &format!("{}Name", XSD)).is_none(), "empty invalid for Name");
        assert!(
            d_value_key("a b", &format!("{}Name", XSD)).is_none(),
            "space invalid for Name"
        );
    }

    /// xsd:NCName: like Name but NO colon.
    #[test]
    fn xml_ncname_value_key() {
        assert!(
            d_value_key("_foo", &format!("{}NCName", XSD)).is_some(),
            "underscore start valid NCName"
        );
        assert!(d_value_key("foo", &format!("{}NCName", XSD)).is_some(), "\"foo\" valid NCName");
        // Invalid
        assert!(
            d_value_key("foo:bar", &format!("{}NCName", XSD)).is_none(),
            "colon INVALID in NCName"
        );
        assert!(
            d_value_key("", &format!("{}NCName", XSD)).is_none(),
            "empty invalid for NCName"
        );
        assert!(
            d_value_key("1foo", &format!("{}NCName", XSD)).is_none(),
            "digit start invalid for NCName"
        );
        assert!(
            d_value_key(":foo", &format!("{}NCName", XSD)).is_none(),
            "colon start invalid for NCName"
        );
    }

    /// xsd:NMTOKEN: one or more NameChar (can start with digit).
    #[test]
    fn xml_nmtoken_value_key() {
        assert!(d_value_key("foo", &format!("{}NMTOKEN", XSD)).is_some(), "\"foo\" valid NMTOKEN");
        assert!(
            d_value_key("123", &format!("{}NMTOKEN", XSD)).is_some(),
            "digit-start valid NMTOKEN"
        );
        assert!(
            d_value_key("foo:bar", &format!("{}NMTOKEN", XSD)).is_some(),
            "colon allowed in NMTOKEN"
        );
        // Invalid
        assert!(
            d_value_key("", &format!("{}NMTOKEN", XSD)).is_none(),
            "empty invalid for NMTOKEN"
        );
        assert!(
            d_value_key("a b", &format!("{}NMTOKEN", XSD)).is_none(),
            "space invalid for NMTOKEN"
        );
    }

    /// xsd:hexBinary decoding — case-insensitive, even-length hex strings only.
    ///
    /// MUTATION SPOT CHECK: if the decoder treats hex chars as case-sensitive
    /// (e.g. by removing the `b'a'..=b'f'` arm in `hex_nibble`), the assertion
    /// `d_value_key("0FB7", …) == d_value_key("0fb7", …)` will FAIL because "0FB7"
    /// would decode correctly while "0fb7" would return None, making the keys
    /// differ. This verifies the test is non-vacuous w.r.t. case normalization.
    #[test]
    fn hex_binary_value_key() {
        // Case-insensitive: "0FB7" and "0fb7" decode to the same bytes.
        assert_eq!(
            d_value_key("0FB7", &format!("{}hexBinary", XSD)),
            d_value_key("0fb7", &format!("{}hexBinary", XSD)),
            "hexBinary is case-insensitive"
        );
        // Single byte
        assert!(d_value_key("0F", &format!("{}hexBinary", XSD)).is_some(), "single byte valid");
        // Multi-byte
        assert!(d_value_key("CAFE", &format!("{}hexBinary", XSD)).is_some(), "2-byte hex valid");
        // Empty = valid (empty octet sequence, per XSD 1.1 §3.3.15)
        assert!(
            d_value_key("", &format!("{}hexBinary", XSD)).is_some(),
            "empty hexBinary is valid (0 octets)"
        );
        // Invalid: non-hex char
        assert!(
            d_value_key("GG", &format!("{}hexBinary", XSD)).is_none(),
            "'G' is not a hex digit"
        );
        // Invalid: odd number of hex chars
        assert!(
            d_value_key("0F0", &format!("{}hexBinary", XSD)).is_none(),
            "odd-length hexBinary is invalid"
        );
    }

    /// xsd:base64Binary decoding — whitespace allowed, padding required.
    #[test]
    fn base64_binary_value_key() {
        // "YQ==" decodes to [0x61] = b'a'
        assert!(
            d_value_key("YQ==", &format!("{}base64Binary", XSD)).is_some(),
            "\"YQ==\" is valid base64"
        );
        // Empty is valid (empty octet sequence)
        assert!(
            d_value_key("", &format!("{}base64Binary", XSD)).is_some(),
            "empty base64Binary is valid (0 octets)"
        );
        // Embedded XML-S whitespace (SP/TAB/CR/LF) is allowed (XSD 1.1 §3.3.16)
        assert!(
            d_value_key("YQ ==", &format!("{}base64Binary", XSD)).is_some(),
            "embedded SP is accepted in base64Binary lexical form"
        );
        // Non-XML whitespace (FF 0x0C) is NOT XML S and must cause rejection.
        // MUTATION CHECK: revert the filter to is_ascii_whitespace() → this assert
        // goes RED because FF is then stripped and "YQ==" is accepted. [SONNET-4.6]
        assert!(
            d_value_key("YQ\x0c==", &format!("{}base64Binary", XSD)).is_none(),
            "form feed (0x0C) is not XML whitespace — must be rejected as ill-formed"
        );
        // Invalid: not a multiple of 4 after stripping whitespace
        assert!(
            d_value_key("YQ=", &format!("{}base64Binary", XSD)).is_none(),
            "3-char base64 (not multiple of 4) is invalid"
        );
        // Invalid: non-base64 char
        assert!(
            d_value_key("Y@==", &format!("{}base64Binary", XSD)).is_none(),
            "'@' is not a base64 char"
        );
    }

    /// XSD 1.1 §3.3.16: discarded bits in padded base64 MUST be zero.
    /// Non-canonical padding (non-zero discarded bits) must be rejected so that
    /// two distinct lexical forms for the same octet sequence cannot slip through.
    ///
    /// MUTATION CHECK: removing either guard (`(b & 0x0f) != 0` or `(c & 0x03) != 0`)
    /// from `decode_base64_binary` causes the corresponding `is_none()` assertion to
    /// fail because the non-canonical form gets accepted and produces `Some(...)`.
    /// Verified by reverting each guard in isolation → test RED → restoring → GREEN.
    /// [SONNET-4.6] sq-pbz04.6.2 fix.
    #[test]
    fn base64_non_canonical_padding_rejected() {
        let b64 = format!("{}base64Binary", XSD);
        // Positive control: "YQ==" IS the canonical two-pad encoding of octet 0x61 (b'a').
        // Y=24, Q=16; b & 0x0f = 16 & 0x0f = 0 → discarded bits are zero → canonical.
        assert_eq!(
            d_value_key("YQ==", &b64),
            Some(DValue::Octets(vec![0x61])),
            "\"YQ==\" is the canonical base64 encoding of octet 0x61"
        );
        // "YR==": Y=24, R=17 (0b010001); b & 0x0f = 0x01 ≠ 0 → non-canonical two-pad.
        assert!(
            d_value_key("YR==", &b64).is_none(),
            "\"YR==\" has non-zero discarded bits in two-pad branch (XSD 1.1 §3.3.16)"
        );
        // "YWJ=": Y=24, W=22, J=9 (0b001001); c & 0x03 = 0x01 ≠ 0 → non-canonical one-pad.
        assert!(
            d_value_key("YWJ=", &b64).is_none(),
            "\"YWJ=\" has non-zero discarded bits in one-pad branch (XSD 1.1 §3.3.16)"
        );
    }

    /// The binary value space is shared: hexBinary and base64Binary compare equal
    /// when they encode the same octet sequence (the binary analogue of integer ⊂ decimal).
    ///
    /// Verification: 0x61 = "61" in hexBinary; 0x61 = b'a', encoded as "YQ==" in base64.
    #[test]
    fn hex_base64_cross_type_value_equality() {
        // "61" hex = [0x61]; "YQ==" base64 = [0x61]; both must key-equal.
        assert!(
            d_value_eq(
                "61",
                &format!("{}hexBinary", XSD),
                "YQ==",
                &format!("{}base64Binary", XSD)
            ),
            "equal octet sequence must compare equal across hexBinary and base64Binary"
        );
        // Negative: different octets must NOT key-equal.
        assert!(
            !d_value_eq(
                "62",
                &format!("{}hexBinary", XSD),
                "YQ==",
                &format!("{}base64Binary", XSD)
            ),
            "different octet sequences must NOT key-equal"
        );
    }

    /// DTYPE_TABLE drives both Recognized::standard() and has_value_mapping() —
    /// verify the new D2 types appear in standard() and have a value mapping.
    #[test]
    fn dtype_table_drives_standard_and_has_value_mapping() {
        let std = Recognized::standard();
        let new_types = [
            "anyURI", "language", "Name", "NCName", "NMTOKEN", "hexBinary", "base64Binary",
        ];
        for local in new_types {
            let iri = format!("{}{}", XSD, local);
            assert!(
                std.contains(&iri),
                "Recognized::standard() must contain xsd:{} (DTYPE_TABLE-driven)",
                local
            );
            assert!(
                has_value_mapping(&iri),
                "has_value_mapping must return true for xsd:{} (DTYPE_TABLE-driven)",
                local
            );
        }
    }

    /// Internal: hex_nibble recognizes all valid hex digits including lowercase.
    #[test]
    fn hex_nibble_all_values() {
        assert_eq!(hex_nibble(b'0'), Some(0));
        assert_eq!(hex_nibble(b'9'), Some(9));
        assert_eq!(hex_nibble(b'a'), Some(10));
        assert_eq!(hex_nibble(b'f'), Some(15));
        assert_eq!(hex_nibble(b'A'), Some(10));
        assert_eq!(hex_nibble(b'F'), Some(15));
        assert_eq!(hex_nibble(b'G'), None);
        assert_eq!(hex_nibble(b' '), None);
    }

    /// Internal: b64_val covers the full base64 alphabet.
    #[test]
    fn b64_val_all_values() {
        assert_eq!(b64_val(b'A'), Some(0));
        assert_eq!(b64_val(b'Z'), Some(25));
        assert_eq!(b64_val(b'a'), Some(26));
        assert_eq!(b64_val(b'z'), Some(51));
        assert_eq!(b64_val(b'0'), Some(52));
        assert_eq!(b64_val(b'9'), Some(61));
        assert_eq!(b64_val(b'+'), Some(62));
        assert_eq!(b64_val(b'/'), Some(63));
        assert_eq!(b64_val(b'='), None);
        assert_eq!(b64_val(b' '), None);
    }

    /// Internal: decode_hex_binary round-trip.
    #[test]
    fn decode_hex_binary_round_trip() {
        assert_eq!(decode_hex_binary(""), Some(vec![]));
        assert_eq!(decode_hex_binary("61"), Some(vec![0x61]));
        assert_eq!(decode_hex_binary("CAFE"), Some(vec![0xCA, 0xFE]));
        assert_eq!(decode_hex_binary("cafe"), Some(vec![0xCA, 0xFE]));
        assert_eq!(decode_hex_binary("0"), None); // odd length
        assert_eq!(decode_hex_binary("GG"), None); // invalid hex
    }

    /// Internal: decode_base64_binary round-trip.
    #[test]
    fn decode_base64_binary_round_trip() {
        assert_eq!(decode_base64_binary(""), Some(vec![]));
        assert_eq!(decode_base64_binary("YQ=="), Some(vec![0x61]));
        // "YWI=" = [0x61, 0x62] = "ab"
        assert_eq!(decode_base64_binary("YWI="), Some(vec![0x61, 0x62]));
        // "YWJj" = [0x61, 0x62, 0x63] = "abc"
        assert_eq!(decode_base64_binary("YWJj"), Some(vec![0x61, 0x62, 0x63]));
        // whitespace stripped
        assert_eq!(decode_base64_binary("YQ =="), Some(vec![0x61]));
        assert_eq!(decode_base64_binary("YQ="), None); // not multiple of 4
    }

    // ── [FABLE-5] sq-pbz04.6.3: dtype ≡ substrate differential over the XSD matrix ──
    //
    // The seam D3 fixes: the integer/decimal canonical key now delegates to
    // `sparq_substrate::numeric::split_decimal`. These tests are the differential
    // acceptance the bead requires — the value-space equality/relational verdicts of
    // `d_value_key`/`d_value_eq` must AGREE with the substrate's own typed numeric
    // comparator (`as_numeric` + `Num::cmp_relational`) over `=`, `<`, `>`, for
    // value-equal-different-lexical pairs, plus the 2^53+1 non-aliasing guard.

    use oxrdf::Literal as OxLit;
    use sparq_substrate::numeric::{as_numeric, Num};
    use std::cmp::Ordering;

    /// Build an oxrdf literal for the substrate comparator side.
    fn slit(value: &str, local: &str) -> OxLit {
        OxLit::new_typed_literal(value, NamedNode::new_unchecked(format!("{}{}", XSD, local)))
    }

    /// The substrate's relational verdict for two numeric lexicals (the engine FILTER
    /// path's semantics: `<`/`>`/`=`), or `None` if either is not numeric there.
    fn substrate_cmp(a_lex: &str, a_dt: &str, b_lex: &str, b_dt: &str) -> Option<Ordering> {
        let a = as_numeric(&slit(a_lex, a_dt.strip_prefix(XSD).unwrap()))?;
        let b: Num = as_numeric(&slit(b_lex, b_dt.strip_prefix(XSD).unwrap()))?;
        a.cmp_relational(b)
    }

    /// `=`: dtype value-equality (via the substrate-delegated canonical key) AGREES with
    /// the substrate's own relational `Equal` verdict, over the integer⊂decimal space and
    /// across value-equal-different-lexical pairs.
    #[test]
    fn dtype_substrate_equality_agrees_over_matrix() {
        let integer = format!("{}integer", XSD);
        let decimal = format!("{}decimal", XSD);
        // (a_lex, a_dt, b_lex, b_dt): value-equal-different-lexical pairs across the
        // coinciding integer/decimal space, plus non-equal controls.
        let cases: &[(&str, &str, &str, &str)] = &[
            ("1", &integer, "1.0", &decimal),
            ("01", &integer, "+1", &integer),
            ("1", &integer, "1.00", &decimal),
            ("-0", &integer, "0", &decimal),
            ("100", &decimal, "100.000", &decimal),
            ("1", &integer, "2", &decimal),   // NOT equal
            ("1.5", &decimal, "1", &integer), // NOT equal
            ("-1.5", &decimal, "-1.50", &decimal),
        ];
        for &(al, adt, bl, bdt) in cases {
            let dtype_eq = d_value_eq(al, adt, bl, bdt);
            let substrate_eq = substrate_cmp(al, adt, bl, bdt) == Some(Ordering::Equal);
            assert_eq!(
                dtype_eq, substrate_eq,
                "dtype ≡ substrate equality disagreement on ({:?}^^{:?}, {:?}^^{:?}): \
                 dtype={} substrate={}",
                al, adt, bl, bdt, dtype_eq, substrate_eq
            );
        }
    }

    /// `<` / `>`: the ORDER of two distinct-but-close numeric values agrees between the
    /// dtype canonical key (string order over the shared canonical decimal) and the
    /// substrate's `cmp_relational`. This exercises the relational verdict, not just `=`.
    #[test]
    fn dtype_substrate_relational_order_agrees() {
        let integer = format!("{}integer", XSD);
        let decimal = format!("{}decimal", XSD);
        let ordered: &[(&str, &str, &str, &str)] = &[
            ("1", &integer, "2", &integer),
            ("1.4", &decimal, "1.5", &decimal),
            ("9", &integer, "10", &decimal),
            ("-2", &integer, "-1", &decimal),
            ("0", &integer, "0.1", &decimal),
        ];
        for &(al, adt, bl, bdt) in ordered {
            // Both well-formed → the substrate gives Less (a < b by construction).
            assert_eq!(
                substrate_cmp(al, adt, bl, bdt),
                Some(Ordering::Less),
                "fixture ({:?},{:?}) must be substrate-Less",
                al,
                bl
            );
            // dtype: distinct values → NOT key-equal; equal values → key-equal. Here they
            // are distinct, so the dtype keys must differ (the < relation manifests as
            // key-inequality in the value-space-equality contract dtype exposes).
            assert!(
                !d_value_eq(al, adt, bl, bdt),
                "dtype keys of a strictly-ordered pair must differ ({:?} vs {:?})",
                al,
                bl
            );
        }
    }

    /// The 2^53+1 NON-ALIASING guard, cross-checked against the substrate: an f64 would
    /// alias 2^53+1 with 2^53, but BOTH the dtype canonical-decimal key AND the substrate
    /// exact comparator keep them distinct (the load-bearing unsound-f64 guard, now proven
    /// on both sides of the seam).
    #[test]
    fn dtype_substrate_2p53_plus_1_non_aliasing() {
        let integer = format!("{}integer", XSD);
        let a = "9007199254740993"; // 2^53 + 1
        let b = "9007199254740992"; // 2^53
        // dtype: distinct keys.
        assert!(
            !d_value_eq(a, &integer, b, &integer),
            "dtype must NOT alias 2^53+1 with 2^53"
        );
        // substrate: NOT relational-equal (exact i128/Dec tier, no f64 collapse).
        assert_ne!(
            substrate_cmp(a, &integer, b, &integer),
            Some(Ordering::Equal),
            "substrate must NOT alias 2^53+1 with 2^53"
        );
    }

    /// Arbitrary-magnitude decimal REGRESSION guard: a 40+-digit `xsd:decimal` lexical
    /// (well past the i128 bound of `Dec::parse`) STILL keys — because the delegation is to
    /// the pure-string, unbounded `split_decimal`, NOT to `Dec::parse` (which would overflow
    /// and drop it, a behaviour change we deliberately avoided). Non-vacuous: a naive
    /// `Dec::parse`-based migration turns this `Some` into `None`.
    #[test]
    fn big_decimal_beyond_i128_still_keys() {
        let decimal = format!("{}decimal", XSD);
        let big = "123456789012345678901234567890123456789012.5"; // 43 significant digits
        assert!(
            d_value_key(big, &decimal).is_some(),
            "an arbitrary-magnitude decimal must still get a D-value (unbounded split)"
        );
        // Its canonical key round-trips its own value (equal to itself, distinct from a
        // neighbour that differs only in the last fraction digit).
        assert!(d_value_eq(big, &decimal, big, &decimal));
        let neighbour = "123456789012345678901234567890123456789012.6";
        assert!(!d_value_eq(big, &decimal, neighbour, &decimal));
    }

    /// [SONNET-4.6] sq-fvxko (issue #3137): the REFUSED-delegation guard.
    ///
    /// The follow-on proposed replacing the `canon_decimal` numeric arm with a delegation to
    /// `Num::cmp_relational`, claimed behaviour-neutral. It is NOT. This test pins the three
    /// concrete divergences so the migration cannot be re-attempted silently — each assertion
    /// is exactly the point where `d_value_key`/`d_value_eq` and `cmp_relational` disagree,
    /// and a delegating rewrite turns each one red.
    ///
    /// (The fourth objection is structural, not testable here: `d_value_key` must return a
    /// standalone `Eq` KEY — `DValue` — and a pairwise `Option<Ordering>` comparator cannot
    /// produce one. Only `d_value_eq` could delegate at all.)
    #[test]
    fn cmp_relational_delegation_would_change_behaviour() {
        let integer = format!("{}integer", XSD);
        let decimal = format!("{}decimal", XSD);
        let double = format!("{}double", XSD);
        let byte = format!("{}byte", XSD);

        // (1) UNBOUNDED MAGNITUDE. `as_numeric` routes xsd:decimal through
        // `Dec::parse_lexical`, whose i128 mantissa overflows past ~38 digits → the value is
        // not numeric there at all. `canon_decimal` is pure-string and keys it exactly.
        let big = "123456789012345678901234567890123456789012.5"; // 43 significant digits
        assert!(
            d_value_key(big, &decimal).is_some(),
            "dtype keys an arbitrary-magnitude decimal"
        );
        assert_eq!(
            as_numeric(&slit(big, "decimal")).map(|_| ()),
            None,
            "substrate cannot represent it — a delegation would DROP this D-value"
        );

        // (2) DISJOINT VALUE SPACES. Under D, xsd:decimal and xsd:double are distinct value
        // spaces (`DValue::Decimal` vs `DValue::F64`), so "1"^^xsd:integer and
        // "1.0"^^xsd:double are NOT the same D-value. `cmp_relational` implements the XPath
        // promotion tower used by SPARQL `=`, which equates them.
        assert!(
            !d_value_eq("1", &integer, "1.0", &double),
            "D keeps the decimal and IEEE-754 value spaces disjoint"
        );
        assert_eq!(
            substrate_cmp("1", &integer, "1.0", &double),
            Some(Ordering::Equal),
            "substrate promotes across the tower — a delegation would ALIAS the two spaces"
        );

        // (3) RANGE FACETS. rdfD1 must not type a literal outside its datatype's value
        // space, so "200"^^xsd:byte has NO D-value. `as_numeric` parses magnitude only, so
        // `cmp_relational` happily equates it with "200"^^xsd:integer.
        assert!(
            d_value_key("200", &byte).is_none(),
            "200 is outside the xsd:byte value space — no D-value"
        );
        assert!(
            !d_value_eq("200", &byte, "200", &integer),
            "an ill-formed literal is never D-value-equal to anything"
        );
        assert_eq!(
            substrate_cmp("200", &byte, "200", &integer),
            Some(Ordering::Equal),
            "substrate ignores the facet — a delegation would type an out-of-range literal"
        );
    }

    /// The no-trim contract of the canonical decimal key is preserved after delegating to
    /// `split_decimal` (which trims): a whitespace-padded decimal lexical is still REJECTED,
    /// byte-identically to the pre-migration hand-rolled splitter. Non-vacuous: dropping the
    /// up-front whitespace reject makes " 1" key as Some.
    #[test]
    fn padded_decimal_lexical_rejected() {
        let decimal = format!("{}decimal", XSD);
        assert!(d_value_key(" 1", &decimal).is_none(), "leading space rejected");
        assert!(d_value_key("1 ", &decimal).is_none(), "trailing space rejected");
        assert!(d_value_key("1\t", &decimal).is_none(), "trailing tab rejected");
        // A clean lexical still keys.
        assert!(d_value_key("1", &decimal).is_some());
    }

    // ── [SONNET-4.6] sq-s3b10: double/float dtype ≡ substrate parse_xsd_f64 parity ──
    //
    // Load-bearing invariant: d_value_key for xsd:double / xsd:float now delegates to
    // sparq_substrate::numeric::parse_xsd_f64 / parse_xsd_f32, so the reasoner and the
    // SPARQL evaluator accept IDENTICAL XSD lexical forms. This table pins the parity.
    //
    // XSD 1.1 §3.3.8 (double) / §3.3.7 (float) lexical space:
    //   - Accepted specials: "INF", "-INF", "NaN"  (XSD-defined; +INF also accepted)
    //   - REJECTED Rust-FromStr-only forms: "Infinity", "-Infinity", "NAN", "nan", "inf"
    //   - Accepted numbers: "1.0E3", "0", "-0", "1", "+INF" etc.
    //
    // Non-vacuous: before sq-s3b10, dtype.rs used a lowercase-only `contains("inf")`
    // blocklist that accepted "Infinity"/"-Infinity"/"NAN"/"nan" (the blocklist checked
    // for "inf" but not for "Infinity" / mixed-case "NAN"); the evaluator correctly
    // returned None. After the migration both return the SAME verdict on every row below.

    use sparq_substrate::numeric::{parse_xsd_f64 as sub_parse_f64, parse_xsd_f32 as sub_parse_f32};

    /// Parity table: dtype.rs d_value_key for xsd:double AGREES with the substrate
    /// parse_xsd_f64 on EVERY XSD-valid and XSD-invalid lexical form. [SONNET-4.6] sq-s3b10.
    #[test]
    fn double_dtype_key_parity_with_substrate_parse_xsd_f64() {
        let double = format!("{}double", XSD);
        // (lexical, expected_some: bool)
        // accepted = XSD-valid; rejected = XSD-invalid (Rust-FromStr-only or just garbage)
        let cases: &[(&str, bool)] = &[
            // XSD-valid specials
            ("INF",    true),
            ("+INF",   true),
            ("-INF",   true),
            ("NaN",    true),
            // XSD-valid numbers
            ("1.0E3",  true),
            ("0",      true),
            ("-0",     true),
            ("1",      true),
            ("+1",     true),
            ("-1.5E2", true),
            ("1.0",    true),
            // Rust-FromStr-only spellings XSD REJECTS
            ("Infinity",  false),
            ("-Infinity", false),
            ("NAN",       false),
            ("nan",       false),
            ("inf",       false),
            ("+inf",      false),
            ("-inf",      false),
            // Other ill-formed forms
            ("",    false),
            ("abc", false),
            ("1x",  false),
        ];
        for &(lex, expected) in cases {
            let dtype_some = d_value_key(lex, &double).is_some();
            let substrate_some = sub_parse_f64(lex).is_some();
            // Both must agree with the expected verdict AND with each other.
            assert_eq!(
                dtype_some, expected,
                "dtype double key mismatch for {:?}: expected some={}", lex, expected
            );
            assert_eq!(
                substrate_some, expected,
                "substrate parse_xsd_f64 mismatch for {:?}: expected some={}", lex, expected
            );
            // Cross-parity: they must agree even if both differ from the expected column
            // (guards against the test fixture itself being wrong).
            assert_eq!(
                dtype_some, substrate_some,
                "dtype vs substrate DISAGREE on double {:?}: dtype={} substrate={}",
                lex, dtype_some, substrate_some
            );
        }
    }

    /// Same parity table for xsd:float — dtype.rs d_value_key delegates to parse_xsd_f32
    /// (which itself calls parse_xsd_f64), so it must agree with the substrate on every form.
    /// [SONNET-4.6] sq-s3b10.
    #[test]
    fn float_dtype_key_parity_with_substrate_parse_xsd_f32() {
        let float = format!("{}float", XSD);
        let cases: &[(&str, bool)] = &[
            ("INF",    true),
            ("+INF",   true),
            ("-INF",   true),
            ("NaN",    true),
            ("1.0E3",  true),
            ("0",      true),
            ("-0",     true),
            ("1",      true),
            // Rust-FromStr-only spellings XSD REJECTS
            ("Infinity",  false),
            ("-Infinity", false),
            ("NAN",       false),
            ("nan",       false),
            ("inf",       false),
            // Other ill-formed
            ("",    false),
            ("abc", false),
        ];
        for &(lex, expected) in cases {
            let dtype_some = d_value_key(lex, &float).is_some();
            let substrate_some = sub_parse_f32(lex).is_some();
            assert_eq!(
                dtype_some, expected,
                "dtype float key mismatch for {:?}: expected some={}", lex, expected
            );
            assert_eq!(
                substrate_some, expected,
                "substrate parse_xsd_f32 mismatch for {:?}: expected some={}", lex, expected
            );
            assert_eq!(
                dtype_some, substrate_some,
                "dtype vs substrate DISAGREE on float {:?}: dtype={} substrate={}",
                lex, dtype_some, substrate_some
            );
        }
    }
}
