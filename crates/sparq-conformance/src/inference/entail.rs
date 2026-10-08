//! Graph-entailment machinery for the reasoning suites: regime closures
//! (computed THROUGH `sparq-reason` — the materializer under test — plus the
//! finite axiomatic/reflexive augmentation the conformance regimes demand),
//! the simple-entailment check (blank-node homomorphism from the conclusion
//! into the closure), D-entailment literal value comparison, and the
//! datatype-clash inconsistency check.
//!
//! Everything works on GENERALIZED triples (`[Term; 3]` — literals may appear
//! in subject position) because rdfD1-style typing puts literals on the left
//! of `rdf:type`: the conclusion's blank nodes may map onto literals, which is
//! exactly how the RDF 1.1 `lg`/`gl` literal-generalization rules surface in a
//! homomorphism-based checker.

use oxrdf::vocab::{rdf, rdfs};
use oxrdf::{NamedNode, Term, Triple};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Dict;

pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// A generalized triple row.
pub type Row = [Term; 3];

/// Entailment regime of a test (what closure to compute).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Regime {
    Simple,
    Rdf,
    Rdfs,
    OwlRl,
    /// [OPUS-4.8] sq-e5atd — D-entailment (datatype / value-space, RDF 1.1
    /// Semantics §7-8): the rdfD1 datatype-typing closure under the recognized
    /// datatype map, computed THROUGH `sparq_reason::Profile::D` (the materializer
    /// under test). OPT-IN, behind the crate's `d-entail` feature.
    #[cfg(feature = "d-entail")]
    D,
}

pub fn triple_row(t: &Triple) -> Row {
    [
        Term::from(t.subject.clone()),
        Term::NamedNode(t.predicate.clone()),
        t.object.clone(),
    ]
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s))
}

/// The recognized-datatype set of a test (D). `xsd:string` and
/// `rdf:langString` are always recognized in RDF 1.1.
#[derive(Default, Clone)]
pub struct Recognized(pub FxHashSet<String>);

impl Recognized {
    pub fn with_defaults(mut iris: FxHashSet<String>) -> Recognized {
        iris.insert(format!("{XSD}string"));
        iris.insert(format!("{RDF_NS}langString"));
        Recognized(iris)
    }
    pub fn contains(&self, dt: &str) -> bool {
        self.0.contains(dt)
    }
    /// The standard recognized set (every datatype this checker has a value
    /// mapping for) — what an OWL 2 datatype map always includes.
    pub fn standard() -> Recognized {
        let mut iris: FxHashSet<String> = [
            "string",
            "normalizedString",
            "token",
            "boolean",
            "integer",
            "long",
            "int",
            "short",
            "byte",
            "nonNegativeInteger",
            "positiveInteger",
            "nonPositiveInteger",
            "negativeInteger",
            "unsignedLong",
            "unsignedInt",
            "unsignedShort",
            "unsignedByte",
            "decimal",
            "float",
            "double",
        ]
        .iter()
        .map(|l| format!("{XSD}{l}"))
        .collect();
        iris.insert(RDF_XML_LITERAL.to_string());
        Recognized::with_defaults(iris)
    }
}

/// Computes the closure of `premise` under `regime`. The core rule engine is
/// `sparq_reason::materialize` (the code under test); this function adds only
/// what the conformance regimes require beyond it (and which the production
/// materializer deliberately omits as store-exploding): the FINITE restriction
/// of the axiomatic triples (`rdf:_n` limited to the container-membership
/// properties occurring in premise ∪ conclusion — the standard finite-closure
/// device), the reflexive rules rdfs4a/4b/6/8/10, rdfs12/13, rdfD1/rdfD2 and
/// rdfs1 datatype recognition.
pub fn close(premise: &[Row], conclusion: &[Row], regime: Regime, d: &Recognized) -> Vec<Row> {
    let mut set: FxHashSet<Row> = premise.iter().cloned().collect();
    match regime {
        Regime::Simple => return set.into_iter().collect(),
        Regime::OwlRl => {
            materialize_through_dict(&mut set, sparq_reason::Profile::OwlRl);
            return set.into_iter().collect();
        }
        // [OPUS-4.8] sq-e5atd — D-entailment: simple entailment PLUS the rdfD1
        // datatype-typing closure, computed THROUGH the materializer under test
        // (`sparq_reason::materialize_d`) with the test's recognized datatype map.
        // No RDF/RDFS axiomatic layer (D-entailment is over simple, not RDFS).
        #[cfg(feature = "d-entail")]
        Regime::D => {
            materialize_d_through_dict(&mut set, d);
            return set.into_iter().collect();
        }
        Regime::Rdf | Regime::Rdfs => {}
    }

    // Container-membership properties occurring anywhere (finite axiomatic
    // restriction for the infinite rdf:_1, rdf:_2, … axiom family).
    let mut cmps: FxHashSet<String> = FxHashSet::default();
    for row in premise.iter().chain(conclusion) {
        for t in row {
            if let Term::NamedNode(n) = t {
                if n.as_str().starts_with(&format!("{RDF_NS}_")) {
                    cmps.insert(n.as_str().to_string());
                }
            }
        }
    }

    // RDF axiomatic triples (RDF 1.1 Semantics §8, finite part).
    let ty = || iri(rdf::TYPE.as_str());
    let property = || iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#Property");
    for p in [
        rdf::TYPE.as_str(),
        rdf::SUBJECT.as_str(),
        rdf::PREDICATE.as_str(),
        rdf::OBJECT.as_str(),
        rdf::FIRST.as_str(),
        rdf::REST.as_str(),
        rdf::VALUE.as_str(),
    ] {
        set.insert([iri(p), ty(), property()]);
    }
    set.insert([iri(rdf::NIL.as_str()), ty(), iri(rdf::LIST.as_str())]);
    for cmp in &cmps {
        set.insert([iri(cmp), ty(), property()]);
    }

    if regime == Regime::Rdfs {
        // RDFS axiomatic triples (RDF 1.1 Semantics §9.3, finite part).
        let dom = || iri(rdfs::DOMAIN.as_str());
        let rng = || iri(rdfs::RANGE.as_str());
        let sc = || iri(rdfs::SUB_CLASS_OF.as_str());
        let sp = || iri(rdfs::SUB_PROPERTY_OF.as_str());
        let resource = || iri(rdfs::RESOURCE.as_str());
        let class = || iri(rdfs::CLASS.as_str());
        let literal = || iri(rdfs::LITERAL.as_str());
        let axioms: &[Row] = &[
            [iri(rdf::TYPE.as_str()), dom(), resource()],
            [iri(rdf::TYPE.as_str()), rng(), class()],
            [dom(), dom(), property()],
            [dom(), rng(), class()],
            [rng(), dom(), property()],
            [rng(), rng(), class()],
            [sp(), dom(), property()],
            [sp(), rng(), property()],
            [sc(), dom(), class()],
            [sc(), rng(), class()],
            [
                iri(rdf::SUBJECT.as_str()),
                dom(),
                iri(rdf::STATEMENT.as_str()),
            ],
            [iri(rdf::SUBJECT.as_str()), rng(), resource()],
            [
                iri(rdf::PREDICATE.as_str()),
                dom(),
                iri(rdf::STATEMENT.as_str()),
            ],
            [iri(rdf::PREDICATE.as_str()), rng(), resource()],
            [
                iri(rdf::OBJECT.as_str()),
                dom(),
                iri(rdf::STATEMENT.as_str()),
            ],
            [iri(rdf::OBJECT.as_str()), rng(), resource()],
            [iri(rdfs::MEMBER.as_str()), dom(), resource()],
            [iri(rdfs::MEMBER.as_str()), rng(), resource()],
            [iri(rdf::FIRST.as_str()), dom(), iri(rdf::LIST.as_str())],
            [iri(rdf::FIRST.as_str()), rng(), resource()],
            [iri(rdf::REST.as_str()), dom(), iri(rdf::LIST.as_str())],
            [iri(rdf::REST.as_str()), rng(), iri(rdf::LIST.as_str())],
            [iri(rdfs::SEE_ALSO.as_str()), dom(), resource()],
            [iri(rdfs::SEE_ALSO.as_str()), rng(), resource()],
            [iri(rdfs::IS_DEFINED_BY.as_str()), dom(), resource()],
            [iri(rdfs::IS_DEFINED_BY.as_str()), rng(), resource()],
            [
                iri(rdfs::IS_DEFINED_BY.as_str()),
                sp(),
                iri(rdfs::SEE_ALSO.as_str()),
            ],
            [iri(rdfs::COMMENT.as_str()), dom(), resource()],
            [iri(rdfs::COMMENT.as_str()), rng(), literal()],
            [iri(rdfs::LABEL.as_str()), dom(), resource()],
            [iri(rdfs::LABEL.as_str()), rng(), literal()],
            [iri(rdf::VALUE.as_str()), dom(), resource()],
            [iri(rdf::VALUE.as_str()), rng(), resource()],
            [iri(rdf::ALT.as_str()), sc(), iri(rdfs::CONTAINER.as_str())],
            [iri(rdf::BAG.as_str()), sc(), iri(rdfs::CONTAINER.as_str())],
            [iri(rdf::SEQ.as_str()), sc(), iri(rdfs::CONTAINER.as_str())],
            [
                iri(rdfs::CONTAINER_MEMBERSHIP_PROPERTY.as_str()),
                sc(),
                property(),
            ],
            [
                iri(rdf::XML_LITERAL.as_str()),
                ty(),
                iri(rdfs::DATATYPE.as_str()),
            ],
            [iri(rdf::XML_LITERAL.as_str()), sc(), literal()],
            [iri(rdfs::DATATYPE.as_str()), sc(), class()],
        ];
        set.extend(axioms.iter().cloned());
        for cmp in &cmps {
            set.insert([
                iri(cmp),
                ty(),
                iri(rdfs::CONTAINER_MEMBERSHIP_PROPERTY.as_str()),
            ]);
        }
    }

    // Fixpoint: the sparq-reason RDFS materializer (rules rdfs2/3/5/7/9/11) ⋈
    // the harness-side axiomatic/reflexive/datatype rules, until stable.
    loop {
        let before = set.len();
        if regime == Regime::Rdfs {
            materialize_through_dict(&mut set, sparq_reason::Profile::Rdfs);
        }
        harness_rules(&mut set, regime, d);
        if set.len() == before {
            break;
        }
    }
    set.into_iter().collect()
}

/// Round-trips the row set through `sparq_reason::materialize` (dictionary ids
/// in, entailed rows out). Term-level lexical forms survive: the dictionary
/// inlines only CANONICAL small integers, so `"0010"^^xsd:integer` keeps its
/// own id and round-trips verbatim.
fn materialize_through_dict(set: &mut FxHashSet<Row>, profile: sparq_reason::Profile) {
    let mut dict = Dict::new();
    let mut triples: Vec<[sparq_core::dict::Id; 3]> = Vec::with_capacity(set.len());
    for [s, p, o] in set.iter() {
        triples.push([dict.intern(s), dict.intern(p), dict.intern(o)]);
    }
    sparq_reason::materialize(profile, &mut dict, &mut triples);
    for [s, p, o] in triples {
        set.insert([dict.term(s), dict.term(p), dict.term(o)]);
    }
}

/// [OPUS-4.8] sq-e5atd — round-trips the row set through
/// `sparq_reason::materialize_d` (the D-entailment materializer under test), with
/// the test's recognized datatype map `d` translated into the reasoner's
/// [`sparq_reason::Recognized`]. The rdfD1 typing triples it emits are GENERALIZED
/// (literal in subject position), exactly the shape the homomorphism checker
/// expects (`lg`/`gl` literal generalization). Like `materialize_through_dict`,
/// term-level lexical forms survive the dictionary round-trip.
#[cfg(feature = "d-entail")]
fn materialize_d_through_dict(set: &mut FxHashSet<Row>, d: &Recognized) {
    let recognized = sparq_reason::Recognized::new(d.0.iter().cloned());
    let mut dict = Dict::new();
    let mut triples: Vec<[sparq_core::dict::Id; 3]> = Vec::with_capacity(set.len());
    for [s, p, o] in set.iter() {
        triples.push([dict.intern(s), dict.intern(p), dict.intern(o)]);
    }
    sparq_reason::materialize_d(&recognized, &mut dict, &mut triples);
    for [s, p, o] in triples {
        set.insert([dict.term(s), dict.term(p), dict.term(o)]);
    }
}

/// The conformance-only rules the production materializer deliberately omits:
/// rdfD1/rdfD2 + rdfs1 (datatype recognition), rdfs4a/4b (everything is an
/// `rdfs:Resource`), rdfs6/8/10 (reflexives), rdfs12 (container-membership →
/// `rdfs:member`), rdfs13 (datatypes are literal classes).
fn harness_rules(set: &mut FxHashSet<Row>, regime: Regime, d: &Recognized) {
    let ty = rdf::TYPE.as_str();
    let mut add: Vec<Row> = Vec::new();
    for [s, p, o] in set.iter() {
        // rdfD2: every predicate is an rdf:Property.
        add.push([
            p.clone(),
            iri(ty),
            iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#Property"),
        ]);
        // rdfD1: a well-formed literal of a recognized datatype is an instance
        // of it (the literal itself stands in for the allocated blank node).
        for t in [s, o] {
            if let Term::Literal(l) = t {
                let dt = l.datatype();
                if d.contains(dt.as_str()) && xsd_value(l.value(), dt.as_str()).is_some() {
                    add.push([t.clone(), iri(ty), iri(dt.as_str())]);
                }
            }
        }
        if regime == Regime::Rdfs {
            // rdfs4a/4b.
            add.push([s.clone(), iri(ty), iri(rdfs::RESOURCE.as_str())]);
            add.push([o.clone(), iri(ty), iri(rdfs::RESOURCE.as_str())]);
            if let (Term::NamedNode(pn), Term::NamedNode(on)) = (p, o) {
                if pn.as_str() == ty {
                    match on.as_str() {
                        // rdfs6 / rdfs8 + rdfs10 / rdfs12 / rdfs13
                        "http://www.w3.org/1999/02/22-rdf-syntax-ns#Property" => {
                            add.push([s.clone(), iri(rdfs::SUB_PROPERTY_OF.as_str()), s.clone()]);
                        }
                        "http://www.w3.org/2000/01/rdf-schema#Class" => {
                            add.push([
                                s.clone(),
                                iri(rdfs::SUB_CLASS_OF.as_str()),
                                iri(rdfs::RESOURCE.as_str()),
                            ]);
                            add.push([s.clone(), iri(rdfs::SUB_CLASS_OF.as_str()), s.clone()]);
                        }
                        "http://www.w3.org/2000/01/rdf-schema#ContainerMembershipProperty" => {
                            add.push([
                                s.clone(),
                                iri(rdfs::SUB_PROPERTY_OF.as_str()),
                                iri(rdfs::MEMBER.as_str()),
                            ]);
                        }
                        "http://www.w3.org/2000/01/rdf-schema#Datatype" => {
                            add.push([
                                s.clone(),
                                iri(rdfs::SUB_CLASS_OF.as_str()),
                                iri(rdfs::LITERAL.as_str()),
                            ]);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if regime == Regime::Rdfs {
        // rdfs1: recognized datatypes are rdfs:Datatypes (for the IRIs that occur).
        let occurring: Vec<String> = set
            .iter()
            .flat_map(|r| r.iter())
            .filter_map(|t| match t {
                Term::NamedNode(n) if d.contains(n.as_str()) => Some(n.as_str().to_string()),
                Term::Literal(l) if d.contains(l.datatype().as_str()) => {
                    Some(l.datatype().as_str().to_string())
                }
                _ => None,
            })
            .collect();
        for dt in occurring {
            add.push([
                iri(&dt),
                iri(rdf::TYPE.as_str()),
                iri(rdfs::DATATYPE.as_str()),
            ]);
        }
    }
    set.extend(add);
}

// ---------------------------------------------------------------------------
// XSD literal values (the D-entailment fragment the suites exercise).
// ---------------------------------------------------------------------------

/// A parsed XSD value, comparable across lexical forms (and across
/// integer/decimal, which share one value space).
#[derive(PartialEq, Debug, Clone)]
pub enum XsdVal {
    /// Canonical decimal representation: (sign, integer digits, fraction digits).
    Num(String),
    /// xsd:float — the value is the rounded f32 (bit pattern, NaN canonicalized).
    F32(u32),
    /// xsd:double.
    F64(u64),
    Bool(bool),
    Str(String),
    /// rdf:langString — (lexical form, lowercased language tag).
    LangStr(String, String),
    /// rdf:XMLLiteral — the (well-formed) lexical form.
    Xml(String),
}

const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
const RDF_XML_LITERAL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#XMLLiteral";

/// The value of a literal term (handles language-tagged strings).
fn lit_value(l: &oxrdf::LiteralRef<'_>) -> Option<XsdVal> {
    match l.language() {
        Some(lang) => Some(XsdVal::LangStr(
            l.value().to_string(),
            lang.to_ascii_lowercase(),
        )),
        None => xsd_value(l.value(), l.datatype().as_str()),
    }
}

/// Parses a lexical form against a known datatype; `None` = ill-formed OR a
/// datatype this checker has no value mapping for. NOTE: no whitespace
/// trimming — RDF typed literals must be IN the lexical space (the XSD
/// whiteSpace facet applies to XML attribute processing, not to RDF literals;
/// see rdf-mt xmlsch-02).
pub fn xsd_value(lex: &str, dt: &str) -> Option<XsdVal> {
    if dt == RDF_XML_LITERAL {
        // Well-formedness: the lexical form must be well-formed XML content
        // (quick-xml is too lenient — a bare `<` passes — so scan directly).
        return if xml_content_well_formed(lex) {
            Some(XsdVal::Xml(lex.to_string()))
        } else {
            None
        };
    }
    let local = dt.strip_prefix(XSD)?;
    match local {
        "string" | "normalizedString" | "token" => Some(XsdVal::Str(lex.to_string())),
        "boolean" => match lex {
            "true" | "1" => Some(XsdVal::Bool(true)),
            "false" | "0" => Some(XsdVal::Bool(false)),
            _ => None,
        },
        "integer" | "long" | "int" | "short" | "byte" | "nonNegativeInteger"
        | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" | "unsignedLong"
        | "unsignedInt" | "unsignedShort" | "unsignedByte" => {
            let v: i128 = lex.parse().ok()?;
            match local {
                "nonNegativeInteger" | "unsignedLong" | "unsignedInt" | "unsignedShort"
                | "unsignedByte"
                    if v < 0 =>
                {
                    None
                }
                "positiveInteger" if v <= 0 => None,
                "nonPositiveInteger" if v > 0 => None,
                "negativeInteger" if v >= 0 => None,
                _ => Some(XsdVal::Num(canon_decimal(&v.to_string())?)),
            }
        }
        "decimal" => Some(XsdVal::Num(canon_decimal(lex)?)),
        "float" => {
            // Parsed DIRECTLY at f32 — parsing at f64 and narrowing double-rounds.
            // [OPUS-5] issue #3796
            let f = parse_xsd_float_single(lex)?;
            Some(XsdVal::F32(if f.is_nan() {
                f32::NAN.to_bits()
            } else {
                f.to_bits()
            }))
        }
        "double" => {
            let f = parse_xsd_float(lex)?;
            Some(XsdVal::F64(if f.is_nan() {
                f64::NAN.to_bits()
            } else {
                f.to_bits()
            }))
        }
        _ => None,
    }
}

/// Minimal XML-content well-formedness scan: `<` must open a tag / comment /
/// CDATA / PI, element tags must balance, `&` must form an entity reference.
/// Approximate (no attribute-syntax validation) but strict where the test
/// suite needs it (a bare `<` or unbalanced tag is ill-formed).
fn xml_content_well_formed(lex: &str) -> bool {
    let b = lex.as_bytes();
    let name_start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c >= 0x80;
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' => {
                if i + 1 >= b.len() {
                    return false;
                }
                if b[i + 1..].starts_with(b"!--") {
                    match lex[i..].find("-->") {
                        Some(e) => i += e + 3,
                        None => return false,
                    }
                } else if b[i + 1..].starts_with(b"![CDATA[") {
                    match lex[i..].find("]]>") {
                        Some(e) => i += e + 3,
                        None => return false,
                    }
                } else if b[i + 1] == b'?' {
                    match lex[i..].find("?>") {
                        Some(e) => i += e + 2,
                        None => return false,
                    }
                } else {
                    let closing = b[i + 1] == b'/';
                    let mut j = i + 1 + closing as usize;
                    let start = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/'
                    {
                        j += 1;
                    }
                    if start == j || !name_start(b[start]) {
                        return false;
                    }
                    let name = lex[start..j].to_string();
                    let end = match lex[j..].find('>') {
                        Some(e) => j + e,
                        None => return false,
                    };
                    if closing {
                        if stack.pop().as_deref() != Some(name.as_str()) {
                            return false;
                        }
                    } else if !lex[j..end].ends_with('/') {
                        stack.push(name);
                    }
                    i = end + 1;
                }
            }
            b'&' => {
                let end = match lex[i..].find(';') {
                    Some(e) if e > 1 && e < 12 => i + e,
                    _ => return false,
                };
                if !lex[i + 1..end]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'#')
                {
                    return false;
                }
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    stack.is_empty()
}

/// The non-special part of the XSD float/double lexical space: xsd float/double syntax is a
/// subset of Rust's, so reject the forms Rust accepts but XSD does not. Shared by
/// [`parse_xsd_float`] and [`parse_xsd_float_single`] so the two widths cannot disagree on
/// which lexicals are well-formed. [OPUS-5] issue #3796
///
/// This is an ALLOWLIST of the byte classes XSD `floatRep`/`doubleRep` can contain, and it is
/// byte-for-byte the gate `sparq_core::parse_xsd_f64` applies. It replaced a BLOCKLIST
/// (`!(contains(['x','X']) || ends_with(['f','F','d','D']) || contains("inf"))`) which was
/// case-sensitive and only caught the lowercase `inf` prefix, so this crate — the one that
/// SCORES conformance — accepted six Rust-`FromStr`-only spellings the XSD lexical spaces
/// forbid, in the direction that INFLATES the score: `Infinity`, `INFINITY`, `+Infinity`,
/// `-Infinity`, `nan`, `NAN` all parsed to a value (measured, issue #3808). The engine's own
/// parser was already strict, so the scorer was more lenient than the thing it scores.
/// `tests::xsd_float_acceptance_rejects_non_xsd_spellings` pins the rejections directly AND
/// differentially against `sparq_core::parse_xsd_f64` so the two seams cannot drift apart.
/// Every valid `floatRep` lexical is spelled from these bytes, so nothing well-formed is lost.
/// [OPUS-5] issue #3808
#[inline]
fn xsd_float_body_wellformed(lex: &str) -> bool {
    lex.bytes()
        .all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E'))
}

fn parse_xsd_float(lex: &str) -> Option<f64> {
    match lex {
        "INF" | "+INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        _ if xsd_float_body_wellformed(lex) => lex.parse::<f64>().ok(),
        _ => None,
    }
}

/// SINGLE-precision twin of [`parse_xsd_float`], for `xsd:float` literals.
///
/// The lexical is parsed `&str -> f32` DIRECTLY. The pre-fix path parsed at `f64` and then
/// narrowed (`parse_xsd_float(lex)? as f32`), which rounds twice, and two roundings are not
/// the correctly-rounded single conversion XSD requires — so an `xsd:float` literal could be
/// ingested one ULP off and, in THIS crate, scored as conformant. Witness (verified by exact
/// rational arithmetic): `"4611686293305294849"` has correctly-rounded `f32` bits
/// `0x5E80_0001`, but parsing at `f64` and narrowing yields `0x5E80_0000`.
/// [OPUS-5] issue #3796
fn parse_xsd_float_single(lex: &str) -> Option<f32> {
    match lex {
        "INF" | "+INF" => Some(f32::INFINITY),
        "-INF" => Some(f32::NEG_INFINITY),
        "NaN" => Some(f32::NAN),
        _ if xsd_float_body_wellformed(lex) => lex.parse::<f32>().ok(),
        _ => None,
    }
}

/// Canonicalizes a decimal lexical form: returns `None` when ill-formed.
fn canon_decimal(lex: &str) -> Option<String> {
    let (sign, rest) = match lex.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, lex.strip_prefix('+').unwrap_or(lex)),
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((i, f)) => (i, f),
        None => (rest, ""),
    };
    if (int_part.is_empty() && frac_part.is_empty())
        || !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let int_trim = int_part.trim_start_matches('0');
    let frac_trim = frac_part.trim_end_matches('0');
    let int_c = if int_trim.is_empty() { "0" } else { int_trim };
    let neg = sign && !(int_c == "0" && frac_trim.is_empty());
    Some(format!(
        "{}{}{}{}",
        if neg { "-" } else { "" },
        int_c,
        if frac_trim.is_empty() { "" } else { "." },
        frac_trim
    ))
}

/// D-entailment literal equality: identical terms, or both datatypes
/// recognized with the same value.
fn lit_eq(a: &oxrdf::LiteralRef<'_>, b: &oxrdf::LiteralRef<'_>, d: &Recognized) -> bool {
    if a == b {
        return true;
    }
    if !d.contains(a.datatype().as_str()) || !d.contains(b.datatype().as_str()) {
        return false;
    }
    match (lit_value(a), lit_value(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

fn term_eq(a: &Term, b: &Term, d: &Recognized) -> bool {
    match (a, b) {
        (Term::Literal(x), Term::Literal(y)) => lit_eq(&x.as_ref(), &y.as_ref(), d),
        _ => a == b,
    }
}

// ---------------------------------------------------------------------------
// Simple entailment: bnode homomorphism from the conclusion into the closure.
// ---------------------------------------------------------------------------

/// Does `closure` simply entail `conclusion`? Conclusion blank nodes may map
/// (non-injectively) onto ANY closure term — IRI, bnode or literal; ground
/// terms must occur (literals modulo D-value equality). Backtracking search;
/// conclusion graphs in the suites are tiny.
pub fn entails(closure: &[Row], conclusion: &[Row], d: &Recognized) -> bool {
    let mut map: FxHashMap<String, Term> = FxHashMap::default();
    // Most-constrained-first: ground triples, then by bnode count — keeps the
    // backtracker shallow on conclusion graphs with many blank nodes.
    let mut ordered: Vec<&Row> = conclusion.iter().collect();
    let bnodes = |r: &Row| r.iter().filter(|t| matches!(t, Term::BlankNode(_))).count();
    ordered.sort_by_key(|r| bnodes(r));
    let mut steps = 0usize;
    solve(0, &ordered, closure, &mut map, d, &mut steps)
}

/// Backtracking budget — exceeding it reports non-entailment (a conservative
/// answer that surfaces as a FAIL to investigate, never a silent pass).
const MAX_STEPS: usize = 5_000_000;

fn solve(
    i: usize,
    conclusion: &[&Row],
    closure: &[Row],
    map: &mut FxHashMap<String, Term>,
    d: &Recognized,
    steps: &mut usize,
) -> bool {
    if i == conclusion.len() {
        return true;
    }
    let pat = conclusion[i];
    for row in closure {
        *steps += 1;
        if *steps > MAX_STEPS {
            return false;
        }
        let mut bound: Vec<String> = Vec::new();
        let ok = (0..3).all(|k| pat_match(&pat[k], &row[k], map, &mut bound, d));
        if ok && solve(i + 1, conclusion, closure, map, d, steps) {
            return true;
        }
        for b in bound {
            map.remove(&b);
        }
    }
    false
}

fn pat_match(
    pat: &Term,
    actual: &Term,
    map: &mut FxHashMap<String, Term>,
    bound: &mut Vec<String>,
    d: &Recognized,
) -> bool {
    match pat {
        Term::BlankNode(b) => {
            let key = b.as_str().to_string();
            match map.get(&key) {
                Some(t) => t == actual,
                None => {
                    map.insert(key.clone(), actual.clone());
                    bound.push(key);
                    true
                }
            }
        }
        _ => term_eq(pat, actual, d),
    }
}

// ---------------------------------------------------------------------------
// D-inconsistency detection.
// ---------------------------------------------------------------------------

/// Is `v`'s value a member of datatype `dt`'s value space? (`None` = cannot
/// judge — unknown datatype.) Integer values are members of xsd:decimal and
/// vice versa when fraction-free; strings/booleans/floats are disjoint spaces.
fn value_in_space(v: &XsdVal, dt: &str) -> Option<bool> {
    if dt == RDF_LANG_STRING {
        return Some(matches!(v, XsdVal::LangStr(_, _)));
    }
    if dt == RDF_XML_LITERAL {
        return Some(matches!(v, XsdVal::Xml(_)));
    }
    if matches!(v, XsdVal::LangStr(_, _) | XsdVal::Xml(_)) {
        return Some(false);
    }
    let local = dt.strip_prefix(XSD)?;
    Some(match (v, local) {
        (XsdVal::Num(_), "decimal") => true,
        (XsdVal::Num(c), "integer") => !c.contains('.'),
        (
            XsdVal::Num(c),
            "long" | "int" | "short" | "byte" | "unsignedLong" | "unsignedInt" | "unsignedShort"
            | "unsignedByte" | "nonNegativeInteger" | "positiveInteger" | "nonPositiveInteger"
            | "negativeInteger",
        ) => !c.contains('.') && xsd_value(c, &format!("{XSD}{local}")).is_some(),
        (XsdVal::Num(_), _) => false,
        (XsdVal::Str(_), "string" | "normalizedString" | "token") => true,
        (XsdVal::Str(_), _) => false,
        (XsdVal::Bool(_), "boolean") => true,
        (XsdVal::Bool(_), _) => false,
        (XsdVal::F32(_), "float") => true,
        (XsdVal::F32(_), _) => false,
        (XsdVal::F64(_), "double") => true,
        (XsdVal::F64(_), _) => false,
        // LangStr/Xml handled by the early returns above.
        _ => false,
    })
}

/// Is `sub` ⊆ `sup` as value spaces? `None` = cannot judge. Used for the
/// intensional-semantics datatype tests: asserting `d1 rdfs:subClassOf d2` is
/// D-inconsistent when the value spaces are provably NOT in a subset relation.
fn dt_subset(sub: &str, sup: &str) -> Option<bool> {
    let (a, b) = (sub.strip_prefix(XSD)?, sup.strip_prefix(XSD)?);
    if a == b {
        return Some(true);
    }
    Some(matches!(
        (a, b),
        ("integer", "decimal")
            | ("nonNegativeInteger", "integer")
            | ("nonNegativeInteger", "decimal")
            | ("positiveInteger", "integer")
            | ("positiveInteger", "decimal")
    ))
}

/// Detects D/RDFS-inconsistency of a closed graph: an ill-typed literal of a
/// recognized datatype, a literal typed (via rdfs2/3/9 closure) by a
/// recognized datatype whose value space excludes its value, or a subclass
/// assertion between recognized datatypes whose value spaces are provably not
/// in a subset relation (the intensional check).
pub fn inconsistency(closure: &[Row], d: &Recognized) -> Option<String> {
    let ty = rdf::TYPE.as_str();
    for [s, p, o] in closure {
        for t in [s, o] {
            if let Term::Literal(l) = t {
                let dt = l.datatype();
                // Only judge datatypes we have a value mapping for.
                if d.contains(dt.as_str())
                    && has_value_mapping(dt.as_str())
                    && lit_value(&l.as_ref()).is_none()
                {
                    return Some(format!(
                        "ill-typed literal \"{}\"^^<{}>",
                        l.value(),
                        dt.as_str()
                    ));
                }
            }
        }
        if let Term::NamedNode(pn) = p {
            if pn.as_str() == ty {
                if let (Term::Literal(l), Term::NamedNode(cls)) = (s, o) {
                    if d.contains(cls.as_str()) && d.contains(l.datatype().as_str()) {
                        if let Some(v) = lit_value(&l.as_ref()) {
                            if value_in_space(&v, cls.as_str()) == Some(false) {
                                return Some(format!(
                                    "literal \"{}\"^^<{}> typed by disjoint datatype <{}>",
                                    l.value(),
                                    l.datatype().as_str(),
                                    cls.as_str()
                                ));
                            }
                        }
                    }
                }
            } else if pn.as_str() == rdfs::SUB_CLASS_OF.as_str() {
                if let (Term::NamedNode(sub), Term::NamedNode(sup)) = (s, o) {
                    if d.contains(sub.as_str())
                        && d.contains(sup.as_str())
                        && dt_subset(sub.as_str(), sup.as_str()) == Some(false)
                    {
                        return Some(format!(
                            "datatype subclass clash: <{}> ⊄ <{}> (value spaces)",
                            sub.as_str(),
                            sup.as_str()
                        ));
                    }
                }
            }
        }
    }
    None
}

fn has_value_mapping(dt: &str) -> bool {
    dt == RDF_XML_LITERAL
        || dt == RDF_LANG_STRING
        || matches!(
            dt.strip_prefix(XSD),
            Some(
                "string"
                    | "normalizedString"
                    | "token"
                    | "boolean"
                    | "integer"
                    | "long"
                    | "int"
                    | "short"
                    | "byte"
                    | "nonNegativeInteger"
                    | "positiveInteger"
                    | "nonPositiveInteger"
                    | "negativeInteger"
                    | "unsignedLong"
                    | "unsignedInt"
                    | "unsignedShort"
                    | "unsignedByte"
                    | "decimal"
                    | "float"
                    | "double"
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal};

    fn lit_int(s: &str) -> Term {
        Term::Literal(Literal::new_typed_literal(
            s,
            NamedNode::new_unchecked(format!("{XSD}integer")),
        ))
    }

    #[test]
    fn value_equality_across_lexical_forms() {
        let d = Recognized::with_defaults([format!("{XSD}integer")].into_iter().collect());
        let closure = vec![[iri("http://e/a"), iri("http://e/p"), lit_int("0010")]];
        let conclusion = vec![[iri("http://e/a"), iri("http://e/p"), lit_int("10")]];
        assert!(entails(&closure, &conclusion, &d), "D value equality");
        let d0 = Recognized::default();
        assert!(
            !entails(&closure, &conclusion, &d0),
            "unrecognized: lexical only"
        );
    }

    /// `xsd:float` literals must be ingested with a SINGLE rounding to single precision.
    ///
    /// Parsing at `f64` and narrowing rounds twice, and this crate SCORES conformance — a
    /// value that is one ULP off here is recorded as conformant. The fixtures are exact-
    /// rational-verified: `H = (2m+1) * 2^38` is an f32 midpoint exactly representable in
    /// f64, so `H +/- 1` rounds to `H` in f64 and then ties-to-even to the WRONG neighbour.
    /// Asserted on `to_bits()` — the two candidates are adjacent floats that format alike.
    /// [OPUS-5] issue #3796
    #[test]
    fn xsd_float_value_is_single_rounded_bit_exact() {
        let float = format!("{XSD}float");
        // true value ABOVE the midpoint -> correct rounding is UP (0x5E800001)
        assert_eq!(
            xsd_value("4611686293305294849", &float),
            Some(XsdVal::F32(0x5E80_0001))
        );
        assert_eq!(
            xsd_value("4.611686293305294849E18", &float),
            Some(XsdVal::F32(0x5E80_0001))
        );
        // true value BELOW the midpoint -> correct rounding is DOWN (also 0x5E800001)
        assert_eq!(
            xsd_value("4611686843061108735", &float),
            Some(XsdVal::F32(0x5E80_0001))
        );
        // The f64-then-narrow route lands on the two ADJACENT floats instead — pinned so
        // the fixtures cannot silently stop witnessing the defect.
        assert_eq!(
            (parse_xsd_float("4611686293305294849").expect("well-formed") as f32).to_bits(),
            0x5E80_0000
        );
        assert_eq!(
            (parse_xsd_float("4611686843061108735").expect("well-formed") as f32).to_bits(),
            0x5E80_0002
        );
    }

    /// The XSD float/double lexical spaces admit ONLY `INF`/`+INF`/`-INF`/`NaN` plus a
    /// digits/sign/point/exponent body — every other spelling Rust's `FromStr` happens to
    /// accept must be REJECTED by this crate, which SCORES conformance.
    ///
    /// This test used to assert only that the f32 and f64 parsers AGREE. That is a fail-OPEN
    /// shape: both share `xsd_float_body_wellformed`, so they agree trivially even when the
    /// predicate is wrong, and replacing its whole body with `true` left the test GREEN. It
    /// did not agree with reality either — the blocklist it was written against really did
    /// accept `Infinity`/`INFINITY`/`nan`/`NAN` (issue #3808), which the old fixture list
    /// mislabelled as "spellings XSD rejects". So assert the REJECTIONS directly, at both
    /// widths, and pin the seam differentially against the engine's own strict parser.
    /// [OPUS-5] issue #3808
    #[test]
    fn xsd_float_acceptance_rejects_non_xsd_spellings() {
        // Rust-`FromStr`-only spellings outside XSD `floatRep`/`doubleRep`. Every one of
        // `Infinity`/`INFINITY`/`+Infinity`/`-Infinity`/`nan`/`NAN` PARSED to a value before
        // the #3808 fix; `inf`/`infinity`/`0x1p3`/`1.0f`/`2.0d` were already rejected, and
        // are kept so a future rewrite cannot lose them.
        for lex in [
            "inf",
            "+inf",
            "-inf",
            "infinity",
            "Inf",
            "iNf",
            "Infinity",
            "INFINITY",
            "+Infinity",
            "-Infinity",
            "nan",
            "NAN",
            "nAn",
            "0x1p3",
            "0X1P3",
            "1.0f",
            "2.0d",
            "1_000",
            "1.5 ",
            " 1.5",
            "abc",
            "",
        ] {
            assert_eq!(
                parse_xsd_float(lex),
                None,
                "f64 parser accepted the non-XSD lexical {:?}",
                lex
            );
            assert_eq!(
                parse_xsd_float_single(lex).map(f32::to_bits),
                None,
                "f32 parser accepted the non-XSD lexical {:?}",
                lex
            );
        }

        // The XSD-legal spellings must still be ACCEPTED — a predicate that rejects
        // everything would pass the loop above, so pin the other direction too.
        for lex in [
            "INF",
            "+INF",
            "-INF",
            "0",
            "-0",
            "+0",
            "1",
            "1.",
            ".5",
            "1.0E3",
            "-2.5",
            "1e-7",
            "+1.5E+3",
            "12345678901234567890",
        ] {
            assert!(
                parse_xsd_float(lex).is_some(),
                "f64 parser rejected the XSD-legal lexical {:?}",
                lex
            );
            assert!(
                parse_xsd_float_single(lex).is_some(),
                "f32 parser rejected the XSD-legal lexical {:?}",
                lex
            );
        }
        // `NaN` is legal but never `is_some()`-comparable by value; assert it separately.
        assert!(parse_xsd_float("NaN").expect("NaN").is_nan());
        assert!(parse_xsd_float_single("NaN").expect("NaN").is_nan());

        // ANTI-DRIFT: this crate's acceptance must equal the engine's own strict parser's
        // over the whole matrix. Before #3808 the scorer was strictly MORE lenient than the
        // engine it scores — exactly the direction that inflates a conformance number.
        for lex in [
            "inf",
            "infinity",
            "Infinity",
            "INFINITY",
            "+Infinity",
            "-Infinity",
            "nan",
            "NAN",
            "0x1p3",
            "1.0f",
            "2.0d",
            "1_000",
            "abc",
            "",
            "INF",
            "+INF",
            "-INF",
            "NaN",
            "0",
            "-0",
            "1",
            "1.",
            ".5",
            "1.0E3",
            "-2.5",
            "1e-7",
            "+1.5E+3",
            "12345678901234567890",
        ] {
            assert_eq!(
                parse_xsd_float(lex).is_some(),
                sparq_core::parse_xsd_f64(lex).is_some(),
                "conformance scorer and engine parser disagree on {:?}",
                lex
            );
            assert_eq!(
                parse_xsd_float_single(lex).is_some(),
                sparq_core::parse_xsd_f64(lex).is_some(),
                "conformance scorer (f32) and engine parser disagree on {:?}",
                lex
            );
        }

        assert_eq!(parse_xsd_float_single("-INF"), Some(f32::NEG_INFINITY));
        assert_eq!(
            parse_xsd_float_single("-0.0").map(f32::to_bits),
            Some((-0.0f32).to_bits())
        );
    }

    #[test]
    fn bnode_maps_to_literal() {
        let d = Recognized::default();
        let closure = vec![[iri("http://e/a"), iri("http://e/p"), lit_int("1")]];
        let conclusion = vec![[
            iri("http://e/a"),
            iri("http://e/p"),
            Term::BlankNode(BlankNode::new_unchecked("x")),
        ]];
        assert!(entails(&closure, &conclusion, &d));
    }

    #[test]
    fn rdfs_closure_through_materializer() {
        // p domain C; s p o ⊢ s type C, plus the axiomatic/reflexive layer.
        let rows = vec![
            [
                iri("http://e/p"),
                iri(rdfs::DOMAIN.as_str()),
                iri("http://e/C"),
            ],
            [iri("http://e/s"), iri("http://e/p"), iri("http://e/o")],
        ];
        let d = Recognized::default();
        let closure = close(&rows, &[], Regime::Rdfs, &d);
        let want = [
            iri("http://e/s"),
            iri(rdf::TYPE.as_str()),
            iri("http://e/C"),
        ];
        assert!(closure.contains(&want), "rdfs2");
        let res = [
            iri("http://e/s"),
            iri(rdf::TYPE.as_str()),
            iri(rdfs::RESOURCE.as_str()),
        ];
        assert!(closure.contains(&res), "rdfs4a");
    }

    #[test]
    fn ill_typed_literal_is_inconsistent() {
        let d = Recognized::with_defaults([format!("{XSD}integer")].into_iter().collect());
        let rows = vec![[iri("http://e/a"), iri("http://e/p"), lit_int("abc")]];
        assert!(inconsistency(&rows, &d).is_some());
    }

    #[test]
    fn parse_xsd_float_rejects_non_xsd_rust_spellings() {
        for lex in ["inf", "infinity", "Infinity", "INFINITY", "nan", "NAN"] {
            assert_eq!(parse_xsd_float(lex), None, "{}", lex);
            assert_eq!(xsd_value(lex, &format!("{}float", XSD)), None, "{}", lex);
            assert_eq!(xsd_value(lex, &format!("{}double", XSD)), None, "{}", lex);
        }

        for lex in ["INF", "+INF", "-INF", "NaN", "1", "-1.5", "2E+3"] {
            assert!(parse_xsd_float(lex).is_some(), "{}", lex);
        }
    }
}
