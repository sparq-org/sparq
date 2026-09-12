//! [FABLE-5] sq-pbz04.1.2 (epic sq-pbz04.1, substrate seam 3) — the ORDERING-PARITY
//! oracle for the `substrate-compare` feature: the reasoner-side comparator
//! (`sparq_reason::compare::sort_ids`, the shared `sparq_substrate::compare` total order
//! over dictionary ids) must order an ENTAILED solution multiset BYTE-IDENTICALLY to a
//! REAL SPARQL-engine `ORDER BY` over the same materialised closure.
//!
//! Non-vacuous by construction: the fixture packs the pairs each comparator arm alone
//! decides — numeric-vs-lexical divergences (`9` < `10` by value, `"10"` < `"9"`
//! lexically), a cross-timezone `xsd:dateTime` pair whose TIMELINE order is the reverse
//! of its lexical order (pins `strict_cmp`) PLUS the sq-2k5py witness shape — two zoned
//! dateTimes at the SAME instant in opposite offsets (`ex:t1`/`ex:t3`, timeline-Equal) and
//! a tz-less one (`ex:t4`) that is lexically BETWEEN them and XPath-INDETERMINATE against
//! both, so the two sides can only agree if BOTH extend the timeline order to a total one
//! (`Timeline::cmp_tl_total`) instead of falling back lexically —
//! a beyond-2^53 integer pair sharing one f64
//! PLUS the double carrying their collapse image (pins the sq-wjl8i exact mixed-tier
//! tie on BOTH engine paths: the numeric sort-cell recheck and `compare_values`), a
//! decimal/double exact-equal pair, `NaN` / `INF` / `-INF` (pins the NaN-first
//! totalisation), the plain digit-string `"11"` against integers `10`/`2`-adjacent
//! values (pins the kind-first rank that replaced the intransitive lexical fallback),
//! inline and stored integer ids, RDF 1.2 triple terms (pins the component-wise recursion),
//! plus blanks / IRIs / language-tagged / boolean / date / gYear / unknown-datatype
//! literals for the class and kind ranks and the string fallback.  Mis-wire any arm and
//! the two sequences diverge.  The `exact_cmp` collapse-recheck for distinct integers
//! beyond 2^53 is pinned by the `exact_recheck_orders_big_integers_beyond_2_53` unit
//! test in `compare.rs`, not by this RDFS-entailment multiset (mutation-verified).
//!
//! ## Full-multiset ordering parity (main parity test)
//!
//! `entailed_solution_order_matches_engine_order_by` asserts byte-identity of the full
//! solution MULTISET produced by `sort_ids` against the engine's `SELECT ?o … ORDER BY ?o`
//! over the same materialised closure — no deduplication on either side.  The RDFS
//! materialiser is triple-level duplicate-free (via `dedup_derived`), so any duplicate
//! OBJECT VALUES arise from distinct triples sharing that object, and BOTH sides see the
//! same triples; the ORDER BY total order is deterministic, so the multisets match.
//! The separate `multiset_duplicate_order_matches_engine_order_by` test pins
//! duplicate-VALUE ordering in a no-entailment fixture where duplicate values are
//! deliberately asserted. [SONNET-4.6]
//!
//! (Compiled only with `--features substrate-compare` — a Cargo `required-features`
//! target. sparq-engine is a dev-dependency here purely as the parity oracle.)

use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_reason::compare::sort_ids;
use sparq_reason::{materialize, Profile};

/// Mixed-term fixture WITH an RDFS schema, so materialisation genuinely ENTAILS new
/// solutions (domain/range/subClassOf/subPropertyOf firings) that participate in the
/// ordered set.  The RDFS materialiser is triple-level duplicate-free; both sides see
/// the same triples, and the full-multiset ORDER BY is deterministic.
const FIXTURE: &str = r#"
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

ex:knows rdfs:domain ex:Person .
ex:knows rdfs:range ex:Agent .
ex:Person rdfs:subClassOf ex:LivingThing .
ex:knows rdfs:subPropertyOf ex:related .

_:alice ex:knows ex:bob .
ex:bob ex:name "bob" .
ex:bob ex:label "zebra"@en .
ex:bob ex:motto "bonjour"@fr .
ex:bob ex:age "42"^^xsd:integer .
ex:bob ex:debt "-3"^^xsd:integer .
ex:bob ex:tiny "9"^^xsd:integer .
ex:bob ex:score "9.5"^^xsd:decimal .
ex:bob ex:big1 "9007199254740992"^^xsd:integer .
ex:bob ex:big2 "9007199254740993"^^xsd:integer .
ex:bob ex:big3 "9007199254740992E0"^^xsd:double .
ex:bob ex:height "1.75E0"^^xsd:double .
ex:bob ex:heightDec "1.75"^^xsd:decimal .
ex:bob ex:nan "NaN"^^xsd:double .
ex:bob ex:inf "INF"^^xsd:double .
ex:bob ex:ninf "-INF"^^xsd:double .
ex:bob ex:code "11" .
ex:bob ex:ok "true"^^xsd:boolean .
ex:bob ex:no "false"^^xsd:boolean .
ex:bob ex:t1 "2024-03-15T14:00:00+01:00"^^xsd:dateTime .
ex:bob ex:t2 "2024-03-15T13:30:00Z"^^xsd:dateTime .
ex:bob ex:t3 "2024-03-15T12:00:00-01:00"^^xsd:dateTime .
ex:bob ex:t4 "2024-03-15T13:00:00"^^xsd:dateTime .
ex:bob ex:d "2024-03-14"^^xsd:date .
ex:bob ex:y "2020"^^xsd:gYear .
ex:bob ex:y2 "2021"^^xsd:gYear .
ex:bob ex:odd "weird"^^ex:custom .
# [GPT-6] Invalid numeric value spaces retain RDF identity and comparator parity.
ex:bob ex:invalidByte "1200"^^xsd:byte .
ex:bob ex:invalidInteger "5.0"^^xsd:integer .
ex:bob ex:invalidUnsigned "-1"^^xsd:unsignedLong .
ex:bob ex:invalidWhitespace " 5"^^xsd:integer .

"#;

/// The reasoner-side sort of the full ENTAILED object multiset equals the engine's real
/// `SELECT ?o … ORDER BY ?o` over the identical materialised closure, term-for-term.
/// The closure is triple-level duplicate-free; this test pins full-multiset ordering
/// parity of entailed solutions.  The separate `multiset_duplicate_order_matches_engine_order_by`
/// test pins duplicate-VALUE ordering in a no-entailment fixture.
#[test]
fn entailed_solution_order_matches_engine_order_by() {
    let (mut dict, mut triples) = Graph::parse_to_triples(FIXTURE, "turtle").expect("fixture parses");

    // Two RDF 1.2 quoted-triple objects (structural dict terms; differ in their object
    // component, one of which is an inline-integer id) — interned exactly as a loader
    // would, so the reasoner sees them as ordinary opaque object ids.
    let quoted = |dict: &mut sparq_core::dict::Dict, o: &str| {
        dict.intern(&oxrdf::Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::NamedNode::new_unchecked("http://ex/qa"),
            oxrdf::NamedNode::new_unchecked("http://ex/qp"),
            oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
                o,
                oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
            )),
        ))))
    };
    let bob = dict.intern_iri("http://ex/bob");
    let asserts = dict.intern_iri("http://ex/asserts");
    let tt1 = quoted(&mut dict, "1");
    let tt2 = quoted(&mut dict, "2");
    triples.push([bob, asserts, tt1]);
    triples.push([bob, asserts, tt2]);

    // Materialise the RDFS closure: the ordered answer set must contain ENTAILED rows.
    let added = materialize(Profile::Rdfs, &mut dict, &mut triples);
    assert!(added > 0, "the fixture must actually entail new solutions (got {} added)", added);

    // Reasoner side: sort the full object multiset (duplicates preserved — the closure
    // is triple-level duplicate-free, so any duplicate object values come from distinct
    // triples, and both sides see the same triples).
    let mut objects: Vec<Id> = triples.iter().map(|t| t[2]).collect();
    sort_ids(&dict, &mut objects);
    let reason_order: Vec<String> = objects.iter().map(|&id| dict.term(id).to_string()).collect();

    // Engine side: SAME closure, full multiset ORDER BY (no DISTINCT — both sides are
    // triple-level duplicate-free and must match without deduplication).
    let graph = Graph::from_parts(dict, triples);
    let res = sparq_engine::query(
        &graph,
        "SELECT ?o WHERE { ?s ?p ?o } ORDER BY ?o",
    )
    .expect("engine query runs");
    let engine_order: Vec<String> = res
        .rows
        .iter()
        .map(|r| r[0].as_ref().expect("?o is always bound").to_string())
        .collect();

    assert_eq!(
        reason_order.len(),
        engine_order.len(),
        "both sides must produce the same solution multiset"
    );
    assert_eq!(
        reason_order, engine_order,
        "entailed-solution ordering diverged from the engine's ORDER BY total order"
    );
}

/// The reasoner-side comparator preserves and correctly orders DUPLICATE bindings — the
/// multiset case.  Uses a controlled fixture with NO RDFS schema (so `materialize` adds
/// nothing and the multiset is exactly the asserted triples' objects), with explicit
/// numeric duplicates whose ORDER BY differs from lexicographic order.
///
/// Expected numeric ORDER BY: `[−3, 2, 2, 5, 10, 10]`.  A lexicographic comparator
/// would yield `["−3"…, "10"…, "10"…, "2"…, "2"…, "5"…]`, diverging from the engine.
/// This test goes RED if `sort_ids` is substituted with a lexicographic sort —
/// the mutation spot-check confirming the assertion is not vacuous. [SONNET-4.6]
#[test]
fn multiset_duplicate_order_matches_engine_order_by() {
    // No RDFS schema: every object in `triples` comes from the asserted statements only.
    // Intentional duplicates: "10"^^xsd:integer × 2, "2"^^xsd:integer × 2.
    const MULTI: &str = r#"
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:a1 ex:val "10"^^xsd:integer .
ex:a2 ex:val "10"^^xsd:integer .
ex:b1 ex:val "2"^^xsd:integer .
ex:b2 ex:val "2"^^xsd:integer .
ex:c  ex:val "-3"^^xsd:integer .
ex:d  ex:val "5"^^xsd:integer .
"#;
    let (mut dict, mut triples) =
        Graph::parse_to_triples(MULTI, "turtle").expect("fixture parses");

    // No RDFS axioms in the fixture — materialise must add nothing.
    let added = materialize(Profile::Rdfs, &mut dict, &mut triples);
    assert_eq!(added, 0, "no RDFS schema in fixture — materialise must add nothing");

    // Reasoner side: sort the full object multiset (duplicates preserved).
    let mut objects: Vec<Id> = triples.iter().map(|t| t[2]).collect();
    sort_ids(&dict, &mut objects);
    let reason_order: Vec<String> =
        objects.iter().map(|&id| dict.term(id).to_string()).collect();

    // Engine side: same multiset via real ORDER BY, no DISTINCT (duplicates kept).
    let graph = Graph::from_parts(dict, triples);
    let res = sparq_engine::query(&graph, "SELECT ?o WHERE { ?s ?p ?o } ORDER BY ?o")
        .expect("engine query runs");
    let engine_order: Vec<String> = res
        .rows
        .iter()
        .map(|r| r[0].as_ref().expect("?o is always bound").to_string())
        .collect();

    assert_eq!(
        reason_order.len(),
        engine_order.len(),
        "multiset sizes must match (duplicates preserved on both sides)"
    );
    assert_eq!(
        reason_order,
        engine_order,
        "multiset ORDER BY parity diverged for duplicate bindings"
    );
}
