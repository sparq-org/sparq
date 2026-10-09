//! [OPUS-4.8] (sq-bif.13) PARALLEL-vs-SERIAL load differential for the in-memory ingest
//! path. `sparq-core`'s parser dispatch (`Graph::parse_to_triples`) takes a DIFFERENT code
//! path per feature state:
//!
//!   * `--features parallel` (the default) — N-Triples splits at newline boundaries into
//!     rayon shards (`parse_ntriples_parallel`) and Turtle splits at top-level statement
//!     terminators with a serial mis-split fallback (`parse_turtle_parallel`); the per-shard
//!     partial dictionaries are then consolidated;
//!   * `--no-default-features` (serial) — one `nt::parse_chunk` / `TurtleParser` pass, no
//!     sharding, no terminator pre-scan.
//!
//! The same `Graph::load_str` / `load_reader` / `load_reader_parallel` call therefore
//! exercises the parallel ingest in the default build and the serial ingest with
//! `--no-default-features`. This test pins, in BOTH states, that the loaded graph is
//! equivalent to a feature-INDEPENDENT reference graph rebuilt term-by-term from the SAME
//! logical triple set (`Graph::from_parts`, which interns serially regardless of features).
//!
//! Because the two parse paths intern terms in a different ORDER, the assigned ids legitimately
//! differ — so the oracle compares at the TERM level: triple count, distinct-term count, the
//! full sorted term dump, `contains` for every loaded triple, and per-pattern scan result SETS
//! (every bound/unbound shape). A regression in the parallel path — a triple dropped or
//! duplicated at a shard boundary, a Turtle statement mis-terminated, a term mis-routed during
//! dictionary consolidation — makes the dump / counts / scans diverge from the reference and
//! FAILS this test under `--features parallel`, while the serial state proves the oracle itself
//! is faithful. Modelled on the term-level differential idiom in `fork_differential.rs`.

use oxrdf::{NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::store::Pattern;
use sparq_core::Graph;

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s.to_string()))
}

/// Sorted term-level dump of a graph's default-graph triples — the equality oracle.
fn dump(g: &Graph) -> Vec<[String; 3]> {
    let mut v: Vec<[String; 3]> = g
        .iter_ids()
        .map(|t| {
            [
                g.dict.term(t[0]).to_string(),
                g.dict.term(t[1]).to_string(),
                g.dict.term(t[2]).to_string(),
            ]
        })
        .collect();
    v.sort();
    v
}

/// Build the feature-independent reference graph from a logical triple set: terms interned
/// SERIALLY via `Dict::intern` + `Graph::from_parts`, with no parse-path dependence at all.
fn reference_graph(triples: &[[Term; 3]]) -> Graph {
    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[Id; 3]> = triples
        .iter()
        .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
        .collect();
    Graph::from_parts(dict, ids)
}

/// The core oracle: `loaded` (whatever parse path produced it) must answer every term-level
/// probe identically to a reference graph rebuilt from `reference` triples. `ctx` names the
/// case so a divergence pinpoints the input + parse path.
fn assert_loaded_matches_reference(loaded: &Graph, reference: &[[Term; 3]], ctx: &str) {
    // Logical triple set (deduped) the reference graph should hold.
    let mut expected: Vec<[String; 3]> = reference
        .iter()
        .map(|[s, p, o]| [s.to_string(), p.to_string(), o.to_string()])
        .collect();
    expected.sort();
    expected.dedup();

    let reference_g = reference_graph(reference);

    // (1) Counts + full dump.
    assert_eq!(loaded.len(), expected.len(), "{ctx}: triple count != deduped reference");
    assert_eq!(loaded.dict.len(), reference_g.dict.len(), "{ctx}: distinct-term count diverges");
    assert_eq!(dump(loaded), expected, "{ctx}: loaded dump != expected triple set");
    assert_eq!(dump(loaded), dump(&reference_g), "{ctx}: loaded dump != reference-graph dump");

    // (2) Every loaded triple is `contains`-resolvable through the loaded dict (the ids the
    // parse path actually assigned). For IRI/blank subjects we also cross-check that the
    // reference resolves the same term (literal probes are exercised via the dump set above).
    for [s, p, o] in &expected {
        let resolve = |g: &Graph, t: &str| probe_term(t).and_then(|term| g.id_of(&term));
        if let (Some(ls), Some(lp), Some(lo)) = (resolve(loaded, s), resolve(loaded, p), resolve(loaded, o)) {
            assert!(loaded.store.contains([ls, lp, lo]), "{ctx}: loaded missing triple {s} {p} {o}");
        }
        if let Some(term) = probe_term(s) {
            assert_eq!(
                loaded.id_of(&term).is_some(),
                reference_g.id_of(&term).is_some(),
                "{ctx}: subject term resolvability diverges for {s}"
            );
        }
    }

    // (3) Per-pattern scan SETS match the reference for every bound/unbound shape. Build the
    // probe-term set from the first few reference triples plus a guaranteed miss.
    let mut probe: Vec<Term> = Vec::new();
    for [s, p, o] in reference.iter().take(6) {
        probe.push(s.clone());
        probe.push(p.clone());
        probe.push(o.clone());
    }
    probe.push(iri("http://nowhere.invalid/absent"));
    let opts: Vec<Option<&Term>> = std::iter::once(None).chain(probe.iter().map(Some)).collect();

    let resolve_opt = |g: &Graph, t: Option<&Term>| -> Option<Option<Id>> {
        match t {
            None => Some(None),
            Some(t) => g.id_of(t).map(Some),
        }
    };
    let term_rows = |g: &Graph, sc: &sparq_core::store::Scan, rows: &[[Id; 3]]| -> Vec<[String; 3]> {
        let mut v: Vec<[String; 3]> = rows
            .iter()
            .map(|r| {
                let t = sc.to_spo(r);
                [
                    g.dict.term(t[0]).to_string(),
                    g.dict.term(t[1]).to_string(),
                    g.dict.term(t[2]).to_string(),
                ]
            })
            .collect();
        v.sort();
        v
    };
    for &s in &opts {
        for &p in &opts {
            for &o in &opts {
                let (Some(ls), Some(lp), Some(lo)) = (
                    resolve_opt(loaded, s),
                    resolve_opt(loaded, p),
                    resolve_opt(loaded, o),
                ) else {
                    continue;
                };
                let (Some(rs), Some(rp), Some(ro)) = (
                    resolve_opt(&reference_g, s),
                    resolve_opt(&reference_g, p),
                    resolve_opt(&reference_g, o),
                ) else {
                    continue;
                };
                let lpat: Pattern = [ls, lp, lo];
                let rpat: Pattern = [rs, rp, ro];
                let lscan = loaded.store.scan(&lpat);
                let rscan = reference_g.store.scan(&rpat);
                assert_eq!(
                    lscan.rows.len(),
                    rscan.rows.len(),
                    "{ctx}: scan cardinality diverges for {lpat:?}"
                );
                assert_eq!(
                    term_rows(loaded, &lscan, &lscan.rows),
                    term_rows(&reference_g, &rscan, &rscan.rows),
                    "{ctx}: scan result set diverges for shape {lpat:?}"
                );
                assert_eq!(
                    loaded.store.estimate(&lpat),
                    reference_g.store.estimate(&rpat),
                    "{ctx}: estimate diverges for {lpat:?}"
                );
            }
        }
    }
}

/// Recognise the IRI/blank-node string forms the dump emits (`<...>` / `_:...`) back into a
/// `Term` for the resolvability cross-check. Literal lexical forms (`"..."`) return `None`:
/// they are covered by the full dump-set equality, not the per-term id_of probe.
fn probe_term(s: &str) -> Option<Term> {
    if let Some(inner) = s.strip_prefix('<').and_then(|x| x.strip_suffix('>')) {
        return Some(Term::NamedNode(NamedNode::new_unchecked(inner.to_string())));
    }
    s.strip_prefix("_:")
        .map(|b| Term::BlankNode(oxrdf::BlankNode::new_unchecked(b.to_string())))
}

/// A representative N-Triples document with the shard-boundary hazards: MANY lines (so the
/// parallel path actually shards), terms repeated across distant lines (cross-shard dedup),
/// blank nodes, lang-tagged + typed + escaped literals, inline integers, and exact-duplicate
/// lines (must collapse to one triple identically in both paths).
fn synthetic_nt() -> (String, Vec<[Term; 3]>) {
    let lit = |v: &str| Term::Literal(oxrdf::Literal::new_simple_literal(v.to_string()));
    let typed = |v: &str, dt: &str| {
        Term::Literal(oxrdf::Literal::new_typed_literal(v.to_string(), NamedNode::new_unchecked(dt.to_string())))
    };
    let lang = |v: &str, l: &str| {
        Term::Literal(oxrdf::Literal::new_language_tagged_literal_unchecked(v.to_string(), l.to_string()))
    };
    let blank = |b: &str| Term::BlankNode(oxrdf::BlankNode::new_unchecked(b.to_string()));

    let mut nt = String::new();
    let mut reference: Vec<[Term; 3]> = Vec::new();
    let xsd_int = "http://www.w3.org/2001/XMLSchema#integer";
    // A fiftieth of the corpus under Miri, which cannot finish the full one inside the nightly
    // lane's per-test cap (sq-0s15k).
    let rows: u32 = if cfg!(miri) { 60 } else { 3000 };
    for i in 0..rows {
        // Clustered subjects/predicates so terms recur across many shard boundaries.
        let s = format!("http://ex/s{}", i % 211);
        let p = format!("http://ex/p{}", i % 13);
        nt.push_str(&format!("<{s}> <{p}> \"{}\"^^<{xsd_int}> .\n", i % 97));
        reference.push([iri(&s), iri(&p), typed(&(i % 97).to_string(), xsd_int)]);

        nt.push_str(&format!("<{s}> <http://ex/follows> <http://ex/s{}> .\n", (i * 7 + 3) % 211));
        reference.push([iri(&s), iri("http://ex/follows"), iri(&format!("http://ex/s{}", (i * 7 + 3) % 211))]);
    }
    // Every remaining record shape, plus an exact-duplicate line and a shared-term crosser.
    nt.push_str("<http://ex/s0> <http://ex/label> \"caf\\u00e9 \\\"q\\\"\"@fr .\n");
    reference.push([iri("http://ex/s0"), iri("http://ex/label"), lang("café \"q\"", "fr")]);
    // [OPUS-4.8] (sq-langcase / #1119) A MIXED-CASE language tag: the byte parser must lowercase
    // it (`en-US` -> `en-us`) to agree with the oxttl streaming/serial paths AND this reference
    // (whose tag is the canonical lowercase form). Pins the casing-normalisation parity here too.
    nt.push_str("<http://ex/s0> <http://ex/label> \"hello\"@en-US .\n");
    reference.push([iri("http://ex/s0"), iri("http://ex/label"), lang("hello", "en-us")]);
    nt.push_str("<http://ex/s1> <http://ex/note> \"a plain string\" .\n");
    reference.push([iri("http://ex/s1"), iri("http://ex/note"), lit("a plain string")]);
    nt.push_str("_:b0 <http://ex/about> <http://ex/s2> .\n");
    reference.push([blank("b0"), iri("http://ex/about"), iri("http://ex/s2")]);
    nt.push_str("<http://ex/s1> <http://ex/note> \"a plain string\" .\n"); // exact duplicate
    nt.push_str("<http://ex/s0> <http://ex/p0> \"0\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n"); // crosser
    reference.push([iri("http://ex/s0"), iri("http://ex/p0"), typed("0", xsd_int)]);
    (nt, reference)
}

/// Turtle with the terminator-handling traps the parallel split must survive: `@prefix`
/// preamble, predicate-object lists (`;`) and object lists (`,`), and — critically — literals
/// CONTAINING a `.` and a `;` (which must NOT be treated as statement terminators by the
/// parallel pre-scan), across many statements so the parallel path actually splits.
fn synthetic_turtle() -> (String, Vec<[Term; 3]>) {
    let lit = |v: &str| Term::Literal(oxrdf::Literal::new_simple_literal(v.to_string()));
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n");
    let mut reference: Vec<[Term; 3]> = Vec::new();
    for i in 0..1500u32 {
        let s = format!("http://ex/s{}", i % 97);
        // predicate-object list with two predicates; one object is a dotted/semicoloned literal.
        ttl.push_str(&format!(
            "ex:s{} rdfs:label \"item {}. v1; v2\" ;\n       ex:follows ex:s{} .\n",
            i % 97,
            i,
            (i * 3 + 1) % 97
        ));
        reference.push([iri(&s), iri("http://www.w3.org/2000/01/rdf-schema#label"), lit(&format!("item {i}. v1; v2"))]);
        reference.push([iri(&s), iri("http://ex/follows"), iri(&format!("http://ex/s{}", (i * 3 + 1) % 97))]);
    }
    // An object list (`,`) and a final dotted-literal terminator trap.
    ttl.push_str("ex:hub ex:links ex:a, ex:b, ex:c .\n");
    for o in ["a", "b", "c"] {
        reference.push([iri("http://ex/hub"), iri("http://ex/links"), iri(&format!("http://ex/{o}"))]);
    }
    ttl.push_str("ex:end rdfs:comment \"trailing dot inside. literal\" .\n");
    reference.push([iri("http://ex/end"), iri("http://www.w3.org/2000/01/rdf-schema#comment"), lit("trailing dot inside. literal")]);
    (ttl, reference)
}

#[test]
fn ntriples_load_matches_reference() {
    let (nt, reference) = synthetic_nt();
    let g = Graph::load_str(&nt, "ntriples").expect("N-Triples loads");
    assert_loaded_matches_reference(&g, &reference, "load_str ntriples");
}

#[test]
fn turtle_load_matches_reference() {
    let (ttl, reference) = synthetic_turtle();
    let g = Graph::load_str(&ttl, "turtle").expect("Turtle loads");
    assert_loaded_matches_reference(&g, &reference, "load_str turtle");
}

/// `load_reader` (serial streaming) and `load_reader_parallel` (the pipelined parallel reader,
/// present only with `parallel`) must agree with the same reference for N-Triples — the reader
/// entry the CLI/Solid ingest actually drives.
#[test]
// The parallel reader's production 32 MiB block buffer is the cost under Miri, not the corpus:
// this ran 71 minutes even at the Miri-sized corpus. The same pipeline runs under Miri at a
// small block size in the crate's `load_reader_parallel_short_reads_match_sequential`.
#[cfg_attr(miri, ignore = "32 MiB production block buffer (sq-0s15k)")]
fn ntriples_load_reader_matches_reference() {
    let (nt, reference) = synthetic_nt();
    let g = Graph::load_reader(std::io::Cursor::new(nt.clone().into_bytes()), "ntriples").expect("reader loads");
    assert_loaded_matches_reference(&g, &reference, "load_reader ntriples");

    #[cfg(feature = "parallel")]
    {
        let gp = Graph::load_reader_parallel(std::io::Cursor::new(nt.into_bytes()), "ntriples")
            .expect("parallel reader loads");
        assert_loaded_matches_reference(&gp, &reference, "load_reader_parallel ntriples");
        // And the two reader entry points agree with each other at the term level.
        assert_eq!(dump(&g), dump(&gp), "load_reader vs load_reader_parallel diverge");
    }
}

/// Guards against the empty / single-line edge that a chunk-splitting path can mishandle: an
/// empty document, a no-trailing-newline last line, and a comment-only / blank-line mix.
#[test]
fn ntriples_edge_documents_match_reference() {
    // Empty.
    let g = Graph::load_str("", "ntriples").expect("empty loads");
    assert_eq!(g.len(), 0, "empty document has no triples");
    assert_eq!(g.dict.len(), 0, "empty document interns no terms");

    // No trailing newline on the final statement.
    let doc = "<http://ex/a> <http://ex/p> <http://ex/b> .\n<http://ex/c> <http://ex/p> <http://ex/d> .";
    let g = Graph::load_str(doc, "ntriples").expect("no-trailing-newline loads");
    let reference = vec![
        [iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b")],
        [iri("http://ex/c"), iri("http://ex/p"), iri("http://ex/d")],
    ];
    assert_loaded_matches_reference(&g, &reference, "ntriples no trailing newline");

    // Blank lines and comments interleaved (the splitter must skip them).
    let doc = "# header comment\n\n<http://ex/a> <http://ex/p> <http://ex/b> .\n\n# mid comment\n<http://ex/c> <http://ex/p> <http://ex/d> .\n";
    let g = Graph::load_str(doc, "ntriples").expect("comment/blank-line mix loads");
    assert_loaded_matches_reference(&g, &reference, "ntriples comments + blank lines");
}
