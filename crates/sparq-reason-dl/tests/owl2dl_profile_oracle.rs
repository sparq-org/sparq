// #7031: differential test of the OWL 2 DL profile check (`owl2dl::check_dl`) against an
// independent OWL 2 DL profile checker.
//
// A seeded generator writes small ontologies that mix valid constructs with the violations
// the check must catch (undeclared and punned entities, reserved vocabulary, non-simple
// properties under restricted constructs, irregular chains, ill-typed literals, bad facets,
// undefined datatypes, anonymous-individual cycles). The reference checker's verdict for each
// seed, and the kinds of violation it reports, are recorded in
// `tests/data/owl2dl_profile_expected.txt`. Whenever we give a verdict it must agree, and so
// must the SET of violation kinds (the reference's class names mapped onto ours by
// `reference_kind`). "Unknown" (a restriction we do not check) is allowed, a different
// definite answer is not. Two kinds of exception are listed, each with its reason:
// violations the reference does not check (`REFERENCE_UNCHECKED`; each is pinned by its own
// test in `owl2dl_extract.rs`), and per-seed kind differences that come from the reference's
// RDF parser rather than from the profile (`REFERENCE_ARTIFACTS`).
//
// To regenerate the answers, dump the corpus with
// `OWL2DL_PROFILE_DUMP=<dir> cargo test -p sparq-reason-dl --test owl2dl_profile_oracle`, run
// the reference checker over `<dir>/*.ttl` in seed order, and write `FILE <name>` /
// `INDL true|false` / `V <violation class>` blocks.

use sparq_core::Graph;
use sparq_reason_dl::owl2dl::{check_dl, extract, ViolationKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const PRE: &str = r#"@prefix : <http://ex/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

const SEEDS: u64 = 600;

/// Violations the reference checker does not detect: (kind, a fragment of our message, the
/// restriction the reference skips).
const REFERENCE_UNCHECKED: &[(ViolationKind, &str, &str)] = &[
    (
        ViolationKind::AnonymousIndividualCycle,
        "",
        "§11.2: object property assertions between anonymous individuals must form a forest",
    ),
    (
        ViolationKind::UnknownDatatype,
        "defined by exactly one DatatypeDefinition",
        "§11.2: a datatype outside the datatype map needs exactly one definition",
    ),
    (
        ViolationKind::InvalidDatatypeDefinition,
        "more than one DatatypeDefinition",
        "§11.2: a datatype outside the datatype map needs exactly one definition",
    ),
    (
        ViolationKind::InvalidFacet,
        "the value of this facet must be",
        "§4: a facet value must be in the facet space",
    ),
    (
        ViolationKind::IllTypedLiteral,
        "XMLSchema#boolean",
        "XSD 1.1 §3.3.2: the boolean lexical space is true, false, 1, 0",
    ),
    (
        ViolationKind::Undeclared,
        "as a object property",
        "§5.8.1: the reference's RDF parser declares a property it types from a property axiom",
    ),
];

/// Per-seed differences in violation KINDS that come from the reference's RDF parser, not
/// from a disagreement about OWL 2 DL: (seed, kind, why).
const REFERENCE_ARTIFACTS: &[(u64, &str, &str)] = &[
    (
        2,
        "IllegalPunning",
        PUN,
    ),
    (
        31,
        "IllegalPunning",
        PUN,
    ),
    (
        73,
        "IllegalPunning",
        PUN,
    ),
    (
        305,
        "IllegalPunning",
        PUN,
    ),
    (
        345,
        "IllegalPunning",
        PUN,
    ),
    (
        363,
        "IllegalPunning",
        PUN,
    ),
    (
        432,
        "IllegalPunning",
        PUN,
    ),
    (
        571,
        "IllegalPunning",
        PUN,
    ),
    (
        23,
        "IrregularPropertyHierarchy",
        "r1∘r0 ⊑ r2 with r1 ≡ r2 needs r1 < r2 while r2 →* r1 (§11.2); the reference treats r1 as r2",
    ),
    (
        142,
        "InsufficientOperands",
        "the reference's parser drops a DisjointObjectProperties axiom over an undeclared property",
    ),
    (
        215,
        "InsufficientOperands",
        "the reference's parser drops a DisjointObjectProperties axiom over an undeclared property",
    ),
    (
        516,
        "NonSimpleProperty",
        "the restricted property is undeclared and has a chain; the reference's parser drops the chain",
    ),
    (
        592,
        "NonSimpleProperty",
        "the undeclared property is equivalent to a transitive one; the reference's parser drops the equivalence",
    ),
];

const PUN: &str =
    "the reference's parser reads an undeclared property as two kinds and reports a pun";

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

fn ontology(seed: u64) -> String {
    let mut g = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut out = String::new();
    let _ = writeln!(out, "<http://ex/o{seed}> a owl:Ontology .");
    let (nc, nr) = (3, 3);
    // Declarations, occasionally missing one.
    for i in 0..nc {
        if !g.chance(8) {
            let _ = writeln!(out, ":C{i} a owl:Class .");
        }
    }
    for i in 0..nr {
        if !g.chance(8) {
            let _ = writeln!(out, ":r{i} a owl:ObjectProperty .");
        }
    }
    if !g.chance(8) {
        let _ = writeln!(out, ":d a owl:DatatypeProperty .");
    }
    let _ = writeln!(
        out,
        ":i0 a owl:NamedIndividual . :i1 a owl:NamedIndividual ."
    );
    for i in 0..nr {
        if g.chance(25) {
            let _ = writeln!(out, ":r{i} a owl:TransitiveProperty .");
        }
    }
    let role = |g: &mut Rng| format!(":r{}", g.below(nr));
    let class = |g: &mut Rng| format!(":C{}", g.below(nc));
    for _ in 0..(3 + g.below(6)) {
        match g.below(16) {
            0 => {
                let (a, r, b) = (class(&mut g), role(&mut g), class(&mut g));
                let _ = writeln!(out, "{a} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:someValuesFrom {b} ] .");
            }
            1 => {
                let (a, r, b) = (class(&mut g), role(&mut g), class(&mut g));
                let n = g.below(3);
                let _ = writeln!(out, "{a} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:maxQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onClass {b} ] .");
            }
            2 => {
                let (a, r) = (class(&mut g), role(&mut g));
                let _ = writeln!(out, "{a} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:hasSelf true ] .");
            }
            3 => {
                let r = role(&mut g);
                let ty = [
                    "owl:FunctionalProperty",
                    "owl:InverseFunctionalProperty",
                    "owl:IrreflexiveProperty",
                    "owl:AsymmetricProperty",
                ][g.below(4)];
                let _ = writeln!(out, "{r} a {ty} .");
            }
            4 => {
                let (a, b, c) = (role(&mut g), role(&mut g), role(&mut g));
                let _ = writeln!(out, "{a} owl:propertyChainAxiom ( {b} {c} ) .");
            }
            5 => {
                let (a, b) = (role(&mut g), role(&mut g));
                let p = if g.chance(50) {
                    "rdfs:subPropertyOf"
                } else {
                    "owl:equivalentProperty"
                };
                let _ = writeln!(out, "{a} {p} {b} .");
            }
            6 => {
                let (a, b) = (role(&mut g), role(&mut g));
                let _ = writeln!(out, "{a} owl:propertyDisjointWith {b} .");
            }
            7 => {
                // Reserved vocabulary as a user entity.
                let _ = writeln!(
                    out,
                    "{} rdfs:subClassOf owl:MadeUp{} .",
                    class(&mut g),
                    g.below(2)
                );
            }
            8 => {
                let ty = ["xsd:integer", "xsd:string", "xsd:boolean", "xsd:dateTime"][g.below(4)];
                let _ = writeln!(out, ":d rdfs:range {ty} .");
            }
            9 => {
                let lit = [
                    "\"12\"^^xsd:integer",
                    "\"twelve\"^^xsd:integer",
                    "\"true\"^^xsd:boolean",
                    "\"yes\"^^xsd:boolean",
                    "\"2020-01-01T00:00:00Z\"^^xsd:dateTime",
                    "\"2020-13-01T00:00:00Z\"^^xsd:dateTime",
                    "\"-1\"^^xsd:nonNegativeInteger",
                    "\"abc\"",
                ][g.below(8)];
                let _ = writeln!(out, ":i{} :d {lit} .", g.below(2));
            }
            10 => {
                let (facet, value, base) = [
                    ("xsd:minInclusive", "\"0\"^^xsd:integer", "xsd:integer"),
                    ("xsd:maxExclusive", "\"10\"^^xsd:integer", "xsd:integer"),
                    (
                        "xsd:minLength",
                        "\"2\"^^xsd:nonNegativeInteger",
                        "xsd:string",
                    ),
                    ("xsd:length", "\"2\"^^xsd:nonNegativeInteger", "xsd:integer"),
                    ("xsd:minInclusive", "\"a\"", "xsd:integer"),
                    ("xsd:maxLength", "\"x\"", "xsd:string"),
                ][g.below(6)];
                let _ = writeln!(out, ":d rdfs:range [ a rdfs:Datatype ; owl:onDatatype {base} ; owl:withRestrictions ( [ {facet} {value} ] ) ] .");
            }
            11 => {
                // A user datatype: defined (once), or only declared, or defined twice.
                let _ = writeln!(out, ":dt a rdfs:Datatype .");
                match g.below(3) {
                    0 => {}
                    1 => {
                        let _ = writeln!(out, ":dt owl:equivalentClass xsd:integer .");
                    }
                    _ => {
                        let _ = writeln!(out, ":dt owl:equivalentClass xsd:integer . :dt owl:equivalentClass xsd:string .");
                    }
                }
                let _ = writeln!(out, ":d rdfs:range :dt .");
            }
            12 => {
                // Anonymous individuals: a chain, sometimes closed into a cycle.
                let r = role(&mut g);
                let _ = writeln!(out, "_:a {r} _:b . _:b {r} _:c .");
                if g.chance(40) {
                    let _ = writeln!(out, "_:c {r} _:a .");
                }
            }
            13 => {
                let (i, c) = (g.below(2), class(&mut g));
                let _ = writeln!(out, ":i{i} a {c} .");
            }
            14 => {
                let (a, b) = (g.below(2), g.below(2));
                let r = role(&mut g);
                let _ = writeln!(out, ":i{a} {r} :i{b} .");
            }
            _ => {
                let lit = ["\"x\"", "\"1\"^^xsd:integer", "\"one\"^^xsd:integer"][g.below(3)];
                let _ = writeln!(out, "{} rdfs:label {lit} .", class(&mut g));
            }
        }
    }
    out
}

/// The reference checker's violation class names, as our [`ViolationKind`] names.
fn reference_kind(name: &str) -> &'static str {
    match name {
        n if n.starts_with("UseOfUndeclared") => "Undeclared",
        n if n.starts_with("UseOfReservedVocabulary") => "ReservedVocabulary",
        n if n.starts_with("UseOfNonSimpleProperty") => "NonSimpleProperty",
        "UseOfPropertyInChainCausesCycle" => "IrregularPropertyHierarchy",
        "LexicalNotInLexicalSpace" => "IllTypedLiteral",
        "InsufficientPropertyExpressions" | "InsufficientOperands" => "InsufficientOperands",
        "UseOfIllegalFacetRestriction" => "InvalidFacet",
        "IllegalPunning" => "IllegalPunning",
        other => panic!("unmapped reference violation {other}"),
    }
}

/// Per seed: the reference verdict and the kinds of violation it reports.
fn expected() -> BTreeMap<u64, (bool, BTreeSet<&'static str>)> {
    let text = include_str!("data/owl2dl_profile_expected.txt");
    let mut out: BTreeMap<u64, (bool, BTreeSet<&'static str>)> = BTreeMap::new();
    let mut seed = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("FILE ") {
            let stem = rest.rsplit('/').next().unwrap().trim_end_matches(".ttl");
            let s = stem.trim_start_matches("seed").parse::<u64>().unwrap();
            seed = Some(s);
            out.insert(s, (false, BTreeSet::new()));
        } else if let Some(v) = line.strip_prefix("INDL ") {
            out.get_mut(&seed.expect("FILE first")).unwrap().0 = v == "true";
        } else if let Some(v) = line.strip_prefix("V ") {
            out.get_mut(&seed.expect("FILE first"))
                .unwrap()
                .1
                .insert(reference_kind(v));
        }
    }
    out
}

#[test]
fn profile_differential() {
    if let Ok(dir) = std::env::var("OWL2DL_PROFILE_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        for seed in 0..SEEDS {
            std::fs::write(
                format!("{dir}/seed{seed:04}.ttl"),
                format!("{PRE}{}", ontology(seed)),
            )
            .unwrap();
        }
        return;
    }
    let expected = expected();
    let (mut agree, mut unknown, mut tolerated) = (0, 0, 0);
    let mut failures = Vec::new();
    for seed in 0..SEEDS {
        let body = ontology(seed);
        let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").unwrap();
        let (want, want_kinds) = expected
            .get(&seed)
            .unwrap_or_else(|| panic!("no recorded answer for seed {seed}"));
        let o = match extract(&d, &t) {
            Ok(o) => o,
            // A graph the mapping refuses is not OWL 2 DL either.
            Err(e) => {
                if *want {
                    failures.push(format!(
                        "seed {seed}: reference true, ours refused: {e}\n{body}"
                    ));
                } else {
                    agree += 1;
                }
                continue;
            }
        };
        let r = check_dl(&d, &o);
        // A kind only we report is accepted when every violation of it is one the reference
        // does not check.
        // An extraction note is ours alone: it explains how a triple was read.
        let got_kinds: BTreeSet<String> = r
            .violations
            .iter()
            .filter(|v| v.kind != ViolationKind::ExtractionNote)
            .map(|v| format!("{:?}", v.kind))
            .collect();
        let beyond_reference = |k: &str| {
            r.violations
                .iter()
                .filter(|v| format!("{:?}", v.kind) == k)
                .all(|v| {
                    REFERENCE_UNCHECKED
                        .iter()
                        .any(|(rk, frag, _)| v.kind == *rk && v.message.contains(frag))
                })
        };
        let want_kinds: BTreeSet<String> = want_kinds.iter().map(|k| k.to_string()).collect();
        let allowed = REFERENCE_ARTIFACTS
            .iter()
            .filter(|(s, _, _)| *s == seed)
            .map(|(_, k, _)| k.to_string())
            .collect::<BTreeSet<_>>();
        let diff: BTreeSet<&String> = got_kinds
            .difference(&want_kinds)
            .filter(|k| !beyond_reference(k))
            .chain(want_kinds.difference(&got_kinds))
            .filter(|k| !allowed.contains(*k))
            .collect();
        let only_beyond = !got_kinds.is_empty() && got_kinds.iter().all(|k| beyond_reference(k));
        let why = format!("{:?} {:?}", r.violations, r.unchecked);
        match r.verdict() {
            None => unknown += 1,
            Some(v) if v == *want && diff.is_empty() => agree += 1,
            Some(false) if *want && only_beyond => tolerated += 1,
            Some(v) => failures.push(format!(
                "seed {seed}: reference {want}, ours {v}; kinds differ in {diff:?}: {why}\n{body}"
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {SEEDS} seeds disagree ({agree} agree, {unknown} unknown, {tolerated} beyond the reference):\n{}",
        failures.len(),
        failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
    // The check must decide most of the corpus, not hide behind "unknown" or the tolerance.
    assert!(unknown * 20 < SEEDS as usize, "{unknown} of {SEEDS} unknown");
    assert!(
        agree * 10 > SEEDS as usize * 8,
        "only {agree} of {SEEDS} agree"
    );
}
