// #7031: the OWL 2 DL reasoner (`owl2dl::check`, `owl2dl::Reasoner`).
//
// Hand-written cases pin each rule of the calculus (one consistent and one inconsistent
// ontology per construct). The differential test runs a seeded generator of random SHIQ
// ontologies and compares consistency, unsatisfiable classes and every entailed named
// subsumption with answers recorded from an independent OWL 2 DL reasoner
// (`tests/data/owl2dl_oracle_expected.txt`). To regenerate those answers, dump the corpus with
// `OWL2DL_ORACLE_DUMP=<dir> cargo test -p sparq-reason-dl --test owl2dl_reasoner oracle`,
// run the reference reasoner over `<dir>/*.ttl` in seed order, and write its answers in the
// same line format (see `render`).

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_dl::owl2dl::{check, extract, Budget, Consistency, DlReport, UnknownReason};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn run(body: &str) -> (Dict, DlReport) {
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").expect("parse");
    let o = extract(&d, &t).unwrap_or_else(|e| panic!("extract: {e}"));
    let r = check(&d, &o, Budget::default());
    (d, r)
}

fn ex(d: &Dict, frag: &str) -> Id {
    d.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

fn consistent(body: &str) -> bool {
    match run(body).1.consistency {
        Consistency::Consistent => true,
        Consistency::Inconsistent => false,
        Consistency::Unknown(r) => panic!("unknown: {r:?}"),
    }
}

fn entails_sub(body: &str, a: &str, b: &str) -> bool {
    let (d, r) = run(body);
    let c = r.classification.expect("consistent");
    c.subsumptions.contains(&(ex(&d, a), ex(&d, b)))
}

fn unsat(body: &str, a: &str) -> bool {
    let (d, r) = run(body);
    let c = r.classification.expect("consistent");
    c.unsatisfiable.contains(&ex(&d, a))
}

fn some(r: &str, c: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:someValuesFrom {c} ]")
}

fn all(r: &str, c: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:allValuesFrom {c} ]")
}

fn max(n: u32, r: &str, c: &str) -> String {
    format!(
        "[ a owl:Restriction ; owl:onProperty {r} ; owl:maxQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onClass {c} ]"
    )
}

fn min(n: u32, r: &str, c: &str) -> String {
    format!(
        "[ a owl:Restriction ; owl:onProperty {r} ; owl:minQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onClass {c} ]"
    )
}

const DECL: &str = ":A a owl:Class . :B a owl:Class . :C a owl:Class . :D a owl:Class .
    :r a owl:ObjectProperty . :s a owl:ObjectProperty . :t a owl:ObjectProperty .
    :a a owl:NamedIndividual . :b a owl:NamedIndividual . :c a owl:NamedIndividual .";

// ---- consistency ------------------------------------------------------------------------

#[test]
fn empty_ontology_is_consistent() {
    assert!(consistent(DECL));
}

#[test]
fn disjoint_classes_clash_on_a_shared_individual() {
    assert!(!consistent(&format!(
        "{DECL} :A owl:disjointWith :B . :a a :A , :B ."
    )));
    assert!(consistent(&format!(
        "{DECL} :A owl:disjointWith :B . :a a :A . :b a :B ."
    )));
}

#[test]
fn union_branches_and_backtracks() {
    // a ∈ (B ⊔ C), both disjoint with A, a ∈ A: every branch closes.
    let body =
        format!("{DECL} :a a [ owl:unionOf ( :B :C ) ] , :A . :A owl:disjointWith :B , :C .");
    assert!(!consistent(&body));
    let body = format!("{DECL} :a a [ owl:unionOf ( :B :C ) ] , :A . :A owl:disjointWith :B .");
    assert!(consistent(&body));
}

#[test]
fn universal_restriction_reaches_a_successor() {
    let body = format!(
        "{DECL} :a a {} . :a :r :b . :b a [ owl:complementOf :B ] .",
        all(":r", ":B")
    );
    assert!(!consistent(&body));
}

#[test]
fn existential_creates_a_witness() {
    // a ∈ ∃r.B, everything r-reachable is ¬B: inconsistent without any ABox successor.
    let body = format!(
        "{DECL} :a a {} , {} .",
        some(":r", ":B"),
        all(":r", "[ owl:complementOf :B ]")
    );
    assert!(!consistent(&body));
}

#[test]
fn inverse_roles_propagate_back() {
    // a ∈ ∃r.(∀r⁻.B) entails a ∈ B.
    let body = format!(
        "{DECL} :a a {} , [ owl:complementOf :B ] .",
        some(":r", &all("[ owl:inverseOf :r ]", ":B"))
    );
    assert!(!consistent(&body));
}

#[test]
fn functional_property_merges_and_clashes() {
    let base = format!("{DECL} :r a owl:FunctionalProperty . :a :r :b , :c .");
    assert!(consistent(&base));
    assert!(!consistent(&format!("{base} :b owl:differentFrom :c .")));
    assert!(!consistent(&format!(
        "{base} :b a :A . :c a :B . :A owl:disjointWith :B ."
    )));
}

#[test]
fn at_most_restriction_counts_distinct_successors() {
    let body = format!(
        "{DECL} :a a {} , {} .",
        max(1, ":r", ":B"),
        min(2, ":r", ":B")
    );
    assert!(!consistent(&body));
    let body = format!(
        "{DECL} :a a {} , {} .",
        max(2, ":r", ":B"),
        min(2, ":r", ":B")
    );
    assert!(consistent(&body));
}

#[test]
fn transitive_role_carries_universal_restrictions() {
    let body = format!(
        "{DECL} :r a owl:TransitiveProperty . :a a {} . :a :r :b . :b :r :c . :c a [ owl:complementOf :B ] .",
        all(":r", ":B")
    );
    assert!(!consistent(&body));
}

#[test]
fn role_hierarchy_feeds_super_roles() {
    let body = format!(
        "{DECL} :r rdfs:subPropertyOf :s . :a a {} . :a :r :b . :b a [ owl:complementOf :B ] .",
        all(":s", ":B")
    );
    assert!(!consistent(&body));
}

#[test]
fn same_and_different_individuals() {
    assert!(!consistent(&format!(
        "{DECL} :a owl:sameAs :b . :a owl:differentFrom :b ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :a owl:sameAs :b . :a a :A . :b a :B . :A owl:disjointWith :B ."
    )));
}

#[test]
fn negative_property_assertion() {
    let npa = "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty :r ; owl:targetIndividual :b ]";
    assert!(consistent(&format!("{DECL} {npa} .")));
    assert!(!consistent(&format!("{DECL} {npa} . :a :r :b .")));
    assert!(!consistent(&format!(
        "{DECL} {npa} . :r rdfs:subPropertyOf :s . :s rdfs:subPropertyOf :r . :a :s :b ."
    )));
}

#[test]
fn role_characteristics() {
    assert!(!consistent(&format!(
        "{DECL} :r a owl:IrreflexiveProperty . :a :r :a ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r a owl:AsymmetricProperty . :a :r :b . :b :r :a ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r owl:propertyDisjointWith :s . :a :r :b . :a :s :b ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r a owl:ReflexiveProperty . :r a owl:IrreflexiveProperty . :a a owl:Thing ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r a owl:SymmetricProperty . :a :r :b . :b a {} .",
        all(":r", "owl:Nothing")
    )));
}

#[test]
fn tbox_only_inconsistency() {
    assert!(!consistent(&format!(
        "{DECL} owl:Thing rdfs:subClassOf owl:Nothing ."
    )));
    assert!(!consistent(&format!(
        "{DECL} owl:Thing rdfs:subClassOf {} , {} .",
        some(":r", ":B"),
        all(":r", "[ owl:complementOf :B ]")
    )));
}

#[test]
fn cyclic_tbox_terminates_through_blocking() {
    // Every A has an r-successor in A: an infinite model, found finitely by blocking.
    let body = format!(
        "{DECL} :A rdfs:subClassOf {} . :a a :A . [ owl:inverseOf :r ] a owl:FunctionalProperty .",
        some(":r", ":A")
    );
    assert!(consistent(&body));
}

// ---- classification ---------------------------------------------------------------------

#[test]
fn classification_finds_entailed_subsumptions() {
    let body = format!(
        "{DECL} :A rdfs:subClassOf {} . {} rdfs:subClassOf :C . :D owl:equivalentClass :C .",
        some(":r", ":B"),
        some(":r", "owl:Thing")
    );
    assert!(entails_sub(&body, "A", "C"));
    assert!(entails_sub(&body, "A", "D"));
    assert!(!entails_sub(&body, "B", "C"));
    let (d, r) = run(&body);
    let c = r.classification.unwrap();
    // C ⊑ D and D ⊑ C are asserted; A ⊑ C and A ⊑ D are not.
    assert!(c.inferred.contains(&(ex(&d, "A"), ex(&d, "C"))));
    assert!(!c.inferred.contains(&(ex(&d, "D"), ex(&d, "C"))));
}

#[test]
fn classification_through_disjunction_needs_a_proof() {
    // A ⊑ B ⊔ C, B ⊑ D, C ⊑ D: A ⊑ D holds in every branch, not deterministically.
    let body = format!(
        "{DECL} :A rdfs:subClassOf [ owl:unionOf ( :B :C ) ] . :B rdfs:subClassOf :D . :C rdfs:subClassOf :D ."
    );
    assert!(entails_sub(&body, "A", "D"));
    assert!(!entails_sub(&body, "A", "B"));
}

#[test]
fn unsatisfiable_classes() {
    let body = format!(
        "{DECL} :A rdfs:subClassOf :B , [ owl:complementOf :B ] . :C rdfs:subClassOf {} , {} .",
        min(3, ":r", "owl:Thing"),
        max(2, ":r", "owl:Thing")
    );
    assert!(unsat(&body, "A"));
    assert!(unsat(&body, "C"));
    assert!(!unsat(&body, "B"));
}

// ---- fail-closed --------------------------------------------------------------------------

#[test]
fn out_of_fragment_constructs_answer_unknown() {
    let (_, r) = run(&format!(
        "{DECL} :A owl:equivalentClass [ owl:oneOf ( :a :b ) ] ."
    ));
    match r.consistency {
        Consistency::Unknown(UnknownReason::OutOfFragment { construct, source }) => {
            assert_eq!(construct, "ObjectOneOf");
            assert!(!source.is_empty());
        }
        other => panic!("expected OutOfFragment, got {other:?}"),
    }
    assert!(r.classification.is_none());
}

#[test]
fn non_simple_role_in_cardinality_answers_unknown() {
    let (_, r) = run(&format!(
        "{DECL} :r a owl:TransitiveProperty . :A rdfs:subClassOf {} .",
        max(1, ":r", "owl:Thing")
    ));
    assert!(matches!(
        r.consistency,
        Consistency::Unknown(UnknownReason::NotInDl(_))
    ));
}

#[test]
fn imports_downgrade_consistent_to_unknown() {
    let (_, r) = run(&format!(
        "{DECL} <http://ex/o> a owl:Ontology ; owl:imports <http://ex/other> ."
    ));
    assert!(matches!(
        r.consistency,
        Consistency::Unknown(UnknownReason::ImportsNotLoaded(_))
    ));
    assert!(r.classification.is_some());
    let (_, r) = run(&format!(
        "{DECL} <http://ex/o> a owl:Ontology ; owl:imports <http://ex/other> . owl:Thing rdfs:subClassOf owl:Nothing ."
    ));
    assert_eq!(r.consistency, Consistency::Inconsistent);
}

#[test]
fn budget_exhaustion_answers_unknown() {
    let (d, t) = Graph::parse_to_triples(
        &format!(
            "{PRE}{DECL} :A rdfs:subClassOf {} . :a a :A .",
            some(":r", ":A")
        ),
        "turtle",
    )
    .unwrap();
    let o = extract(&d, &t).unwrap();
    let r = check(
        &d,
        &o,
        Budget {
            max_steps: 3,
            max_nodes: 100,
        },
    );
    assert_eq!(
        r.consistency,
        Consistency::Unknown(UnknownReason::ResourceBudget)
    );
}

// ---- differential test against recorded reference answers ---------------------------------

/// xorshift64*: a small deterministic generator, so the corpus is code, not data.
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

struct Gen {
    rng: Rng,
    n_classes: usize,
    n_roles: usize,
    simple: Vec<bool>,
}

impl Gen {
    fn class(&mut self) -> String {
        format!(":C{}", self.rng.below(self.n_classes))
    }

    fn role(&mut self, simple_only: bool) -> Option<String> {
        let pool: Vec<usize> = (0..self.n_roles)
            .filter(|&i| !simple_only || self.simple[i])
            .collect();
        if pool.is_empty() {
            return None;
        }
        let r = pool[self.rng.below(pool.len())];
        Some(if self.rng.chance(30) {
            format!("[ owl:inverseOf :r{r} ]")
        } else {
            format!(":r{r}")
        })
    }

    fn concept(&mut self, depth: usize) -> String {
        if depth == 0 || self.rng.chance(35) {
            return match self.rng.below(10) {
                0 => format!("[ a owl:Class ; owl:complementOf {} ]", self.class()),
                1 if self.rng.chance(30) => "owl:Thing".into(),
                _ => self.class(),
            };
        }
        let d = depth - 1;
        let card = |n: usize| format!("\"{n}\"^^xsd:nonNegativeInteger");
        match self.rng.below(11) {
            0 => format!(
                "[ a owl:Class ; owl:intersectionOf ( {} {} ) ]",
                self.concept(d),
                self.concept(d)
            ),
            1 => format!(
                "[ a owl:Class ; owl:unionOf ( {} {} ) ]",
                self.concept(d),
                self.concept(d)
            ),
            2 => format!("[ a owl:Class ; owl:complementOf {} ]", self.concept(d)),
            3 | 4 => {
                let r = self.role(false).unwrap();
                format!(
                    "[ a owl:Restriction ; owl:onProperty {r} ; owl:someValuesFrom {} ]",
                    self.concept(d)
                )
            }
            5 | 6 => {
                let r = self.role(false).unwrap();
                format!(
                    "[ a owl:Restriction ; owl:onProperty {r} ; owl:allValuesFrom {} ]",
                    self.concept(d)
                )
            }
            k @ 7..=9 => {
                let Some(r) = self.role(true) else {
                    return self.class();
                };
                let (prop, n) = match k {
                    7 => ("owl:minQualifiedCardinality", 1 + self.rng.below(3)),
                    8 => ("owl:maxQualifiedCardinality", self.rng.below(3)),
                    _ => ("owl:qualifiedCardinality", 1 + self.rng.below(2)),
                };
                format!(
                    "[ a owl:Restriction ; owl:onProperty {r} ; {prop} {} ; owl:onClass {} ]",
                    card(n),
                    self.concept(d)
                )
            }
            _ => match self.role(true) {
                Some(r) => format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:hasSelf true ]"),
                None => self.class(),
            },
        }
    }

    fn ontology(seed: u64) -> String {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n_classes = 3 + rng.below(4);
        let n_roles = 1 + rng.below(3);
        let mut out = String::new();
        let _ = writeln!(out, "<http://ex/o{seed}> a owl:Ontology .");
        for i in 0..n_classes {
            let _ = writeln!(out, ":C{i} a owl:Class .");
        }
        for i in 0..n_roles {
            let _ = writeln!(out, ":r{i} a owl:ObjectProperty .");
        }
        // Role hierarchy and transitivity first, so simplicity is known.
        let mut incl: Vec<(usize, usize)> = Vec::new();
        let mut transitive = vec![false; n_roles];
        for (i, t) in transitive.iter_mut().enumerate() {
            if rng.chance(25) {
                *t = true;
                let _ = writeln!(out, ":r{i} a owl:TransitiveProperty .");
            }
        }
        if n_roles > 1 {
            for _ in 0..rng.below(3) {
                let (a, b) = (rng.below(n_roles), rng.below(n_roles));
                if a != b {
                    incl.push((a, b));
                    if rng.chance(30) {
                        let _ = writeln!(
                            out,
                            "[ owl:inverseOf :r{a} ] rdfs:subPropertyOf [ owl:inverseOf :r{b} ] ."
                        );
                    } else {
                        let _ = writeln!(out, ":r{a} rdfs:subPropertyOf :r{b} .");
                    }
                }
            }
        }
        // A role is non-simple when a transitive role is below it.
        let mut simple: Vec<bool> = transitive.iter().map(|t| !t).collect();
        for _ in 0..n_roles {
            for &(a, b) in &incl {
                if !simple[a] {
                    simple[b] = false;
                }
            }
        }
        let mut g = Gen {
            rng,
            n_classes,
            n_roles,
            simple,
        };
        for i in 0..n_roles {
            if !g.simple[i] {
                continue;
            }
            for (pct, ty) in [
                (12, "owl:FunctionalProperty"),
                (8, "owl:InverseFunctionalProperty"),
                (6, "owl:IrreflexiveProperty"),
                (6, "owl:AsymmetricProperty"),
            ] {
                if g.rng.chance(pct) {
                    let _ = writeln!(out, ":r{i} a {ty} .");
                }
            }
        }
        for i in 0..n_roles {
            if g.rng.chance(8) {
                let _ = writeln!(out, ":r{i} a owl:SymmetricProperty .");
            }
            if g.rng.chance(5) {
                let _ = writeln!(out, ":r{i} a owl:ReflexiveProperty .");
            }
            if g.rng.chance(15) {
                let c = g.concept(1);
                let _ = writeln!(out, ":r{i} rdfs:domain {c} .");
            }
            if g.rng.chance(15) {
                let c = g.concept(1);
                let _ = writeln!(out, ":r{i} rdfs:range {c} .");
            }
        }
        for _ in 0..(2 + g.rng.below(6)) {
            match g.rng.below(10) {
                0..=5 => {
                    let (a, b) = if g.rng.chance(50) {
                        (g.class(), g.concept(2))
                    } else {
                        (g.concept(2), g.class())
                    };
                    let _ = writeln!(out, "{a} rdfs:subClassOf {b} .");
                }
                6 | 7 => {
                    let (a, b) = (g.class(), g.concept(2));
                    let _ = writeln!(out, "{a} owl:equivalentClass {b} .");
                }
                _ => {
                    let (a, b) = (g.class(), g.class());
                    if a != b {
                        let _ = writeln!(out, "{a} owl:disjointWith {b} .");
                    }
                }
            }
        }
        let n_ind = g.rng.below(4);
        for i in 0..n_ind {
            let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
        }
        if n_ind > 0 {
            for _ in 0..g.rng.below(5) {
                let a = g.rng.below(n_ind);
                match g.rng.below(8) {
                    0..=3 => {
                        let c = g.concept(1);
                        let _ = writeln!(out, ":i{a} a {c} .");
                    }
                    4 | 5 => {
                        let b = g.rng.below(n_ind);
                        let r = g.rng.below(n_roles);
                        let _ = writeln!(out, ":i{a} :r{r} :i{b} .");
                    }
                    6 => {
                        let b = g.rng.below(n_ind);
                        if a != b {
                            let _ = writeln!(out, ":i{a} owl:differentFrom :i{b} .");
                        }
                    }
                    _ => {
                        let b = g.rng.below(n_ind);
                        let r = g.rng.below(n_roles);
                        let _ = writeln!(out, "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :i{a} ; owl:assertionProperty :r{r} ; owl:targetIndividual :i{b} ] .");
                    }
                }
            }
        }
        out
    }
}

const SEEDS: u64 = 400;

/// The answer block in the reference line format: `CONSISTENT true|false`, then sorted
/// `UNSAT <iri>` and `SUB <iri> <iri>` lines.
fn render(d: &Dict, report: &DlReport, names: &BTreeMap<Id, String>) -> String {
    let mut lines = Vec::new();
    match &report.consistency {
        Consistency::Consistent => lines.push("CONSISTENT true".to_string()),
        Consistency::Inconsistent => {
            lines.push("CONSISTENT false".to_string());
            return lines.join("\n");
        }
        Consistency::Unknown(r) => return format!("UNKNOWN {r:?}"),
    }
    let _ = d;
    let c = report.classification.as_ref().expect("consistent");
    let mut body = Vec::new();
    for u in &c.unsatisfiable {
        body.push(format!("UNSAT {}", names[u]));
    }
    for (a, b) in &c.subsumptions {
        body.push(format!("SUB {} {}", names[a], names[b]));
    }
    body.sort();
    lines.extend(body);
    lines.join("\n")
}

fn expected() -> BTreeMap<u64, String> {
    let text = include_str!("data/owl2dl_oracle_expected.txt");
    let mut out = BTreeMap::new();
    let mut cur: Option<(u64, Vec<&str>)> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("FILE ") {
            if let Some((s, ls)) = cur.take() {
                out.insert(s, ls.join("\n"));
            }
            let stem = rest.rsplit('/').next().unwrap().trim_end_matches(".ttl");
            cur = Some((stem.trim_start_matches("seed").parse().unwrap(), Vec::new()));
        } else if let Some((_, ls)) = cur.as_mut() {
            ls.push(line);
        }
    }
    if let Some((s, ls)) = cur.take() {
        out.insert(s, ls.join("\n"));
    }
    out
}

#[test]
fn oracle_differential() {
    if let Ok(dir) = std::env::var("OWL2DL_ORACLE_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        for seed in 0..SEEDS {
            let ttl = format!("{PRE}{}", Gen::ontology(seed));
            std::fs::write(format!("{dir}/seed{seed:04}.ttl"), ttl).unwrap();
        }
        return;
    }
    let expected = expected();
    let mut failures = Vec::new();
    let mut skipped = 0;
    for seed in 0..SEEDS {
        let body = Gen::ontology(seed);
        let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").unwrap();
        let o = extract(&d, &t).unwrap_or_else(|e| panic!("seed {seed}: extract: {e}"));
        let report = check(&d, &o, Budget::default());
        let mut names = BTreeMap::new();
        for i in 0..8 {
            let id = ex(&d, &format!("C{i}"));
            if id != sparq_core::dict::NO_ID {
                names.insert(id, format!("http://ex/C{i}"));
            }
        }
        let got = render(&d, &report, &names);
        let want = expected
            .get(&seed)
            .unwrap_or_else(|| panic!("no recorded answer for seed {seed}"));
        // The reference itself failed on this seed (recorded as an `ERROR` line).
        if want.starts_with("ERROR") {
            skipped += 1;
            continue;
        }
        if &got != want {
            failures.push(format!(
                "seed {seed}:\n--- want\n{want}\n--- got\n{got}\n{body}"
            ));
        }
    }
    assert!(
        skipped * 50 < SEEDS,
        "the reference failed on {skipped} seeds"
    );
    assert!(
        failures.is_empty(),
        "{} of {SEEDS} seeds disagree:\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
