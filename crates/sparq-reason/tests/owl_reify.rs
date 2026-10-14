//! [Kern] End-to-end quoted-triple (RDF 1.2 reifier) RL inference — the integration
//! suite for the `quoted-triples` feature (a Cargo `required-features` target;
//! kern/quoted-triple-infer, first increment). Fixtures are Turtle 1.2 strings run
//! through the REAL loader (`Graph::parse_to_triples` — reifiedTriple `<< … >>`,
//! `~ :r` reifiers, `{| … |}` annotation blocks, `<<( … )>>` triple terms), so the
//! rules are exercised against exactly the reifier representation sparq-core mints.
//! Expected answers are asserted positively AND negatively: the opacity
//! non-entailments (a quotation never asserts its quoted triple) and the
//! finite-Herbrand-base pins (no unguarded or nested-quotation term is ever minted)
//! matter as much as the derivations. See src/reify.rs for the rule table and the
//! termination argument.

use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id, NO_ID};
use sparq_core::Graph;
use sparq_reason::{
    materialize_owl_rl, materialize_owl_rl_reify, MaterializedOwlGraph, OwlMode, ReifyMode,
};

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const EX: &str = "http://ex/";

fn parse(ttl: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(ttl, "turtle").expect("fixture parses")
}

/// Materialize the OWL-RL (+ reify) closure of a fixture; returns dict + closure set.
fn closure(ttl: &str) -> (Dict, FxHashSet<[Id; 3]>) {
    let (mut dict, mut triples) = parse(ttl);
    materialize_owl_rl(&mut dict, &mut triples);
    let set = triples.iter().copied().collect();
    (dict, set)
}

/// Resolve an ALREADY-interned IRI (RDF-namespace fragments spelled `rdf:frag`,
/// everything else under `http://ex/`); `NO_ID` if absent — absence itself proves the
/// term occurs in no triple.
fn id(dict: &Dict, name: &str) -> Id {
    let iri = match name.strip_prefix("rdf:") {
        Some(frag) => format!("{RDF}{frag}"),
        None => format!("{EX}{name}"),
    };
    dict.lookup(&Term::NamedNode(NamedNode::new_unchecked(iri)))
}

/// Resolve the triple term `<<( s p o )>>` over resolver-style names, `NO_ID` if it
/// was never minted (o as an IRI name, or `#n` for an xsd:integer literal).
fn tt(dict: &Dict, s: &str, p: &str, o: &str) -> Id {
    let n = |name: &str| NamedNode::new_unchecked(format!("{EX}{name}"));
    let object = match o.strip_prefix('#') {
        Some(v) => Term::Literal(Literal::new_typed_literal(v, xsd::INTEGER)),
        None => Term::NamedNode(n(o)),
    };
    dict.lookup(&Term::Triple(Box::new(oxrdf::Triple::new(
        n(s),
        n(p),
        object,
    ))))
}

fn assert_has(set: &FxHashSet<[Id; 3]>, t: [Id; 3], what: &str) {
    assert!(
        t.iter().all(|&x| x != NO_ID),
        "a term of the expected triple was never interned: {what}"
    );
    assert!(set.contains(&t), "expected ENTAILED but absent: {what}");
}

fn assert_lacks(set: &FxHashSet<[Id; 3]>, t: [Id; 3], what: &str) {
    // A NO_ID component already proves non-membership (no triple contains id 0).
    if t.iter().all(|&x| x != NO_ID) {
        assert!(
            !set.contains(&t),
            "OVER-entailed — must NOT be in the closure: {what}"
        );
    }
}

/// reif-dtr over the loader's `~ :r` reifier syntax: the reifier's components are
/// recovered into the classic vocabulary and the reifier is typed rdf:Statement.
/// The literal component `30` resolves to an inline-integer id — pinned on purpose
/// (the rules must be inline-id-clean).
#[test]
fn destructure_recovers_components_from_reifier_syntax() {
    let (dict, set) = closure(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           :alice :age 30 ~ :r .
        "#,
    );
    let thirty = dict.lookup(&Term::Literal(Literal::new_typed_literal(
        "30",
        xsd::INTEGER,
    )));
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:subject"), id(&dict, "alice")],
        "r rdf:subject :alice",
    );
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:predicate"), id(&dict, "age")],
        "r rdf:predicate :age",
    );
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:object"), thirty],
        "r rdf:object 30",
    );
    assert_has(
        &set,
        [
            id(&dict, "r"),
            id(&dict, "rdf:type"),
            id(&dict, "rdf:Statement"),
        ],
        "r a rdf:Statement",
    );
    // The `~` syntax also asserts the annotated triple itself — present, but from the
    // LOADER, not from any unquoting rule (the opacity twin below proves that).
    assert_has(
        &set,
        [id(&dict, "alice"), id(&dict, "age"), thirty],
        ":alice :age 30",
    );
}

/// Opacity + annotation reasoning: RL rules apply to the reifier's ANNOTATIONS
/// (domain-typing here) and to the DESTRUCTURED components, while the quoted triple
/// itself is never leaked into the base closure.
#[test]
fn annotations_reason_without_leaking_the_quoted_triple() {
    let (dict, set) = closure(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
           :certainty rdfs:domain :Assertion .
           :suspects rdfs:domain :Suspicion .
           :r rdf:reifies <<( :bob :age 25 )>> .
           :r :certainty 0.9 .
        "#,
    );
    // Annotation reasoning: the domain rule types the reifier.
    assert_has(
        &set,
        [
            id(&dict, "r"),
            id(&dict, "rdf:type"),
            id(&dict, "Assertion"),
        ],
        "r a :Assertion",
    );
    // Destructured components are ordinary triples.
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:subject"), id(&dict, "bob")],
        "r rdf:subject :bob",
    );
    // THE opacity pin: quotation never asserts the quoted triple.
    let twentyfive = dict.lookup(&Term::Literal(Literal::new_typed_literal(
        "25",
        xsd::INTEGER,
    )));
    assert_lacks(
        &set,
        [id(&dict, "bob"), id(&dict, "age"), twentyfive],
        ":bob :age 25 (quoted only)",
    );
}

/// reif-ctr fires exactly for classic-reification descriptions of EXISTING triples;
/// for a described-but-absent triple neither the reifies triple NOR the triple term
/// itself may come into existence (the finite-Herbrand-base restriction 1).
#[test]
fn construct_only_for_existing_triples() {
    let (dict, set) = closure(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           :alice :age :x .
           :r rdf:subject :alice . :r rdf:predicate :age . :r rdf:object :x .
           :r2 rdf:subject :alice . :r2 rdf:predicate :age . :r2 rdf:object :y .
           :y :unrelated :thing .
        "#,
    );
    let q = tt(&dict, "alice", "age", "x");
    assert_ne!(q, NO_ID, "the guarded quotation was minted");
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:reifies"), q],
        "r rdf:reifies <<( :alice :age :x )>>",
    );
    // r2 describes (alice age y), which does NOT hold: no firing, no term minted.
    assert_eq!(
        tt(&dict, "alice", "age", "y"),
        NO_ID,
        "no triple term for a non-existing combination"
    );
    // And constructing never asserts components beyond its conclusion: r2 stays untyped.
    assert_lacks(
        &set,
        [
            id(&dict, "r2"),
            id(&dict, "rdf:type"),
            id(&dict, "rdf:Statement"),
        ],
        "r2 a rdf:Statement",
    );
}

/// DERIVED triples are quotable too (restriction 1 is closure membership, not a
/// base-snapshot — the reify layer alternates with the RL closure), and the joint
/// fixpoint is IDEMPOTENT: a second materialization adds nothing. (Under a
/// base-snapshot guard this exact fixture would derive MORE on the second call.)
#[test]
fn derived_triples_are_reifiable_and_materialization_is_idempotent() {
    let (mut dict, mut triples) = parse(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
           :p rdfs:subPropertyOf :q .
           :a :p :b .
           :r rdf:subject :a . :r rdf:predicate :q . :r rdf:object :b .
        "#,
    );
    materialize_owl_rl(&mut dict, &mut triples);
    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    // rdfs7 derives (a q b); reif-ctr then quotes the DERIVED triple.
    assert_has(
        &set,
        [id(&dict, "a"), id(&dict, "q"), id(&dict, "b")],
        ":a :q :b (rdfs7)",
    );
    let q = tt(&dict, "a", "q", "b");
    assert_ne!(q, NO_ID);
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:reifies"), q],
        "r rdf:reifies <<( :a :q :b )>>",
    );
    // Idempotency (the `materialize` API contract): a second call adds nothing.
    assert_eq!(
        materialize_owl_rl(&mut dict, &mut triples),
        0,
        "materialization must be idempotent"
    );
}

/// The opacity BOUNDARY, pinned precisely: no rule rewrites inside a quotation
/// (the original triple term keeps its as-written components), but the transparent
/// bridge composition (reif-dtr, eq-rep on the classic vocabulary, reif-ctr) may
/// derive a reifies triple for a sameAs-VARIANT spelling when that variant triple
/// itself holds. See the opacity section of src/reify.rs.
#[test]
fn same_as_variants_arrive_only_via_the_transparent_bridge() {
    let (dict, set) = closure(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           @prefix owl: <http://www.w3.org/2002/07/owl#> .
           :alice owl:sameAs :al .
           :alice :knows :bob .
           :r rdf:reifies <<( :alice :knows :bob )>> .
        "#,
    );
    // The ORIGINAL quotation is intact (nothing rewrote its inside).
    let original = tt(&dict, "alice", "knows", "bob");
    assert_ne!(original, NO_ID);
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:reifies"), original],
        "the as-written quotation stays",
    );
    // The variant arrives ONLY because (al knows bob) itself holds (eq-rep) and the
    // destructured components are transparent — the documented bridge composition.
    assert_has(
        &set,
        [id(&dict, "al"), id(&dict, "knows"), id(&dict, "bob")],
        ":al :knows :bob (eq-rep)",
    );
    let variant = tt(&dict, "al", "knows", "bob");
    assert_ne!(
        variant, NO_ID,
        "bridge composition constructs the variant quotation"
    );
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:reifies"), variant],
        "r rdf:reifies <<( :al :knows :bob )>>",
    );
}

/// sq-afun3 (second increment): STRICT OPACITY end-to-end.
/// `materialize_owl_rl_reify(…, ReifyMode::DestructureOnly)` on the exact sameAs
/// fixture of `same_as_variants_arrive_only_via_the_transparent_bridge`: the
/// destructured components and the eq-rep variant TRIPLE still arrive (reif-dtr and
/// the RL core are untouched), but the variant QUOTATION is never minted — the
/// bridge composition the full Bridge mode allows is suppressed, and inference
/// introduces no triple term at all.
#[test]
fn strict_opacity_suppresses_the_bridge_composition() {
    let (mut dict, mut triples) = parse(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           @prefix owl: <http://www.w3.org/2002/07/owl#> .
           :alice owl:sameAs :al .
           :alice :knows :bob .
           :r rdf:reifies <<( :alice :knows :bob )>> .
        "#,
    );
    materialize_owl_rl_reify(&mut dict, &mut triples, ReifyMode::DestructureOnly);
    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    // reif-dtr is unchanged: the as-written quotation is destructured…
    let original = tt(&dict, "alice", "knows", "bob");
    assert_ne!(original, NO_ID, "the as-written quotation exists (loader)");
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:reifies"), original],
        "the as-written quotation stays",
    );
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:subject"), id(&dict, "alice")],
        "r rdf:subject :alice (reif-dtr still fires)",
    );
    // …and so is the RL core: eq-rep still derives the variant TRIPLE.
    assert_has(
        &set,
        [id(&dict, "al"), id(&dict, "knows"), id(&dict, "bob")],
        ":al :knows :bob (eq-rep)",
    );
    // THE strict-opacity pin: the variant QUOTATION is never minted (in Bridge
    // mode this exact fixture constructs it — the sibling test above).
    assert_eq!(
        tt(&dict, "al", "knows", "bob"),
        NO_ID,
        "strict mode must not construct the sameAs-variant quotation"
    );
    // Idempotent, like the Bridge mode (the `materialize` API contract).
    assert_eq!(
        materialize_owl_rl_reify(&mut dict, &mut triples, ReifyMode::DestructureOnly),
        0,
        "strict materialization must be idempotent"
    );
}

/// sq-afun3: in strict mode classic-reification data is never surfaced
/// as `rdf:reifies` — the `construct_only_for_existing_triples` fixture derives no
/// reifies triple and mints no triple term, while the destructure direction (and
/// `rdf:Statement` typing of real reifiers) is unaffected.
#[test]
fn strict_opacity_never_surfaces_classic_reification() {
    let (mut dict, mut triples) = parse(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           :alice :age :x .
           :r rdf:subject :alice . :r rdf:predicate :age . :r rdf:object :x .
        "#,
    );
    materialize_owl_rl_reify(&mut dict, &mut triples, ReifyMode::DestructureOnly);
    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    // The triple term for the described-and-EXISTING triple is never minted…
    assert_eq!(
        tt(&dict, "alice", "age", "x"),
        NO_ID,
        "strict mode mints no triple term even for an existing triple"
    );
    // …and r is not typed rdf:Statement either (that conclusion is reif-dtr's,
    // whose premise — an rdf:reifies triple — never comes into existence here).
    assert_lacks(
        &set,
        [
            id(&dict, "r"),
            id(&dict, "rdf:type"),
            id(&dict, "rdf:Statement"),
        ],
        "r a rdf:Statement",
    );
}

/// THE termination pin, end-to-end: the adversarial schema that makes an
/// UNRESTRICTED constructor diverge (`rdf:reifies ⊑ rdf:object` feeds each minted
/// quotation straight back into a construct premise). Restriction 2 (never quote a
/// triple term) cuts the regress at depth one — materialization terminates and the
/// nested quotation-of-a-quotation is never minted.
#[test]
fn adversarial_subproperty_cascade_terminates_at_depth_one() {
    let (mut dict, mut triples) = parse(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
           rdf:reifies rdfs:subPropertyOf rdf:object .
           :r rdf:subject :r .
           :r rdf:predicate rdf:reifies .
           :r rdf:reifies :o .
        "#,
    );
    materialize_owl_rl(&mut dict, &mut triples); // must return at all (termination)
    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    // Depth one IS derived: (r rdf:object :o) via rdfs7, and (r rdf:reifies :o)
    // holds, so reif-ctr mints <<( :r rdf:reifies :o )>>.
    let depth1 = dict.lookup(&Term::Triple(Box::new(oxrdf::Triple::new(
        NamedNode::new_unchecked(format!("{EX}r")),
        NamedNode::new_unchecked(format!("{RDF}reifies")),
        Term::NamedNode(NamedNode::new_unchecked(format!("{EX}o"))),
    ))));
    assert_ne!(
        depth1, NO_ID,
        "the depth-1 quotation of the asserted triple is minted"
    );
    assert_has(
        &set,
        [id(&dict, "r"), id(&dict, "rdf:reifies"), depth1],
        "r rdf:reifies <<( :r rdf:reifies :o )>>",
    );
    // Depth two is REFUSED: <<( :r rdf:reifies <<( … )>> )>> must never exist.
    let depth2 = dict.lookup(&Term::Triple(Box::new(oxrdf::Triple::new(
        NamedNode::new_unchecked(format!("{EX}r")),
        NamedNode::new_unchecked(format!("{RDF}reifies")),
        dict.term(depth1),
    ))));
    assert_eq!(
        depth2, NO_ID,
        "restriction 2 must cut the cascade — no quotation of a quotation"
    );
}

/// `MaterializedOwlGraph` on a reify-vocabulary base: routed to the documented
/// Fallback mode, with incremental == from-scratch parity across mutations —
/// including an insert that only THEN completes a construct premise set.
#[test]
fn incremental_owl_graph_falls_back_and_stays_parity_identical() {
    let (mut dict, base) = parse(
        r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
           :certainty rdfs:domain :Assertion .
           :alice :age 30 ~ :r {| :certainty 0.9 |} .
        "#,
    );
    let oracle = |dict: &mut Dict, base: &[[Id; 3]]| -> Vec<[Id; 3]> {
        let mut all = base.to_vec();
        materialize_owl_rl(dict, &mut all);
        let mut v: Vec<[Id; 3]> = all
            .into_iter()
            .collect::<FxHashSet<_>>()
            .into_iter()
            .collect();
        v.sort_unstable();
        v
    };
    let mut g = MaterializedOwlGraph::new(&mut dict, &base);
    assert_eq!(
        g.mode(),
        OwlMode::Fallback,
        "reify vocabulary must route to Fallback"
    );
    assert_eq!(
        g.closure(),
        oracle(&mut dict, &base),
        "initial closure == from-scratch"
    );

    // Mutate: a classic-reification description of an EXISTING triple — the insert
    // must rematerialize (reify trigger ids) and derive the constructed quotation.
    let ex = |dict: &mut Dict, frag: &str| dict.intern_iri(&format!("{EX}{frag}"));
    let rdf = |dict: &mut Dict, frag: &str| dict.intern_iri(&format!("{RDF}{frag}"));
    let (r2, alice, age) = (
        ex(&mut dict, "r2"),
        ex(&mut dict, "alice"),
        ex(&mut dict, "age"),
    );
    let thirty = dict.intern_lit("30", xsd::INTEGER.as_str(), None);
    let delta = [
        [r2, rdf(&mut dict, "subject"), alice],
        [r2, rdf(&mut dict, "predicate"), age],
        [r2, rdf(&mut dict, "object"), thirty],
    ];
    g.insert(&mut dict, &delta);
    let base2: Vec<[Id; 3]> = g.base_triples().collect();
    assert_eq!(
        g.closure(),
        oracle(&mut dict, &base2),
        "post-insert closure == from-scratch"
    );
    let q = tt(&dict, "alice", "age", "#30");
    assert_ne!(q, NO_ID);
    assert!(
        g.contains(&[r2, rdf(&mut dict, "reifies"), q]),
        "the insert completed a construct premise set"
    );
}

/// sq-afun3 (third increment): STRICT OPACITY through the INCREMENTAL handle.
/// `MaterializedOwlGraph::with_reify_mode(…, DestructureOnly)` keeps the mode across
/// re-materializations, so (a) the closure equals the MATCHING strict batch oracle
/// initially AND after a mutation, and (b) no triple term is ever minted by
/// inference — not even by the mutation that completes a construct premise set (which
/// under `new`/Bridge does mint one: the sibling test above, on this very fixture).
#[test]
fn incremental_strict_opacity_matches_the_strict_batch_oracle() {
    const FIXTURE: &str = r#"@prefix : <http://ex/> .
           @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
           @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
           :certainty rdfs:domain :Assertion .
           :alice :age 30 ~ :r {| :certainty 0.9 |} .
        "#;
    let (mut dict, base) = parse(FIXTURE);
    // The oracle is `materialize_owl_rl_reify` in the SAME mode — a Bridge oracle would
    // mint terms into this dict and destroy the never-minted assertions below.
    let oracle = |dict: &mut Dict, base: &[[Id; 3]]| -> Vec<[Id; 3]> {
        let mut all = base.to_vec();
        materialize_owl_rl_reify(dict, &mut all, ReifyMode::DestructureOnly);
        let mut v: Vec<[Id; 3]> = all
            .into_iter()
            .collect::<FxHashSet<_>>()
            .into_iter()
            .collect();
        v.sort_unstable();
        v
    };
    let mut g = MaterializedOwlGraph::with_reify_mode(&mut dict, &base, ReifyMode::DestructureOnly);
    assert_eq!(
        g.reify_mode(),
        ReifyMode::DestructureOnly,
        "the constructed reify mode is what the graph maintains"
    );
    assert_eq!(
        g.mode(),
        OwlMode::Fallback,
        "mode detection is unchanged — reify vocabulary still routes to Fallback"
    );
    assert_eq!(
        g.closure(),
        oracle(&mut dict, &base),
        "initial closure == from-scratch STRICT closure"
    );
    // reif-dtr still fires (the loader-written quotation is destructured).
    let (r, alice, age) = (
        dict.intern_iri(&format!("{EX}r")),
        dict.intern_iri(&format!("{EX}alice")),
        dict.intern_iri(&format!("{EX}age")),
    );
    let rdf = |dict: &mut Dict, frag: &str| dict.intern_iri(&format!("{RDF}{frag}"));
    let subject = rdf(&mut dict, "subject");
    assert!(
        g.contains(&[r, subject, alice]),
        "destructure is unaffected by strict mode"
    );

    // The mutation that, in Bridge mode, completes a construct premise set over an
    // EXISTING triple and mints `<<( :alice :age 30 )>>`.
    let r2 = dict.intern_iri(&format!("{EX}r2"));
    let thirty = dict.intern_lit("30", xsd::INTEGER.as_str(), None);
    let delta = [
        [r2, subject, alice],
        [r2, rdf(&mut dict, "predicate"), age],
        [r2, rdf(&mut dict, "object"), thirty],
    ];
    g.insert(&mut dict, &delta);
    let base2: Vec<[Id; 3]> = g.base_triples().collect();
    assert_eq!(
        g.closure(),
        oracle(&mut dict, &base2),
        "post-insert closure == from-scratch STRICT closure"
    );
    // THE pin: strict mode never constructs, so no reifies triple for r2 exists at all…
    let reifies = rdf(&mut dict, "reifies");
    assert!(
        !g.closure().iter().any(|t| t[0] == r2 && t[1] == reifies),
        "strict mode must derive no rdf:reifies triple for the classic-reification node"
    );
    // …and the quotation it would have minted was never interned (the loader wrote
    // `<<( :alice :age 30 )>>` nowhere in this fixture — the `~ :r` form quotes it, so
    // guard against vacuity by checking the Bridge contrast in a FRESH dict below).
    let (mut bdict, bbase) = parse(FIXTURE);
    let mut bg = MaterializedOwlGraph::new(&mut bdict, &bbase);
    let (balice, bage) = (
        bdict.intern_iri(&format!("{EX}alice")),
        bdict.intern_iri(&format!("{EX}age")),
    );
    let br2 = bdict.intern_iri(&format!("{EX}r2"));
    let bthirty = bdict.intern_lit("30", xsd::INTEGER.as_str(), None);
    let bdelta = [
        [br2, rdf(&mut bdict, "subject"), balice],
        [br2, rdf(&mut bdict, "predicate"), bage],
        [br2, rdf(&mut bdict, "object"), bthirty],
    ];
    bg.insert(&mut bdict, &bdelta);
    let breifies = rdf(&mut bdict, "reifies");
    assert!(
        bg.closure().iter().any(|t| t[0] == br2 && t[1] == breifies),
        "NON-VACUITY: the same edit sequence under the default Bridge mode DOES construct"
    );
    assert_eq!(
        bg.reify_mode(),
        ReifyMode::Bridge,
        "`new` is the full bridge"
    );
}
