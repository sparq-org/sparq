//! sq-leg8n — integration tests for `V("phrase")` lexical-first concept
//! resolution behind the §6 soundness envelope (design research
//! `llm-ergonomic-sparql-surface.md`). These exercise the REAL resolution path against a
//! real `sparq-core` Graph + a real `sparq-vectors` store — not a mock — and assert the
//! load-bearing invariants: always-canonical output, the silent-rewrite canary, the echoed
//! resolution, the confidence/ambiguity gate (loud-fail beats silent-wrong), and the
//! mandatory staleness guard.
#![cfg(feature = "vectors")]

use sparq_core::Graph;
use sparq_terse::{terse_to_sparql_with, Method, ResolveCtx, ResolveGate, TerseError};
use sparq_vectors::store::VectorStore;

/// A small PKG-like graph: a few concepts with labels (for lexical linking) and one
/// ambiguous pair (two entities sharing a label substring).
fn graph() -> Graph {
    let ttl = r#"
        @prefix ex: <http://example.org/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        @prefix skos: <http://www.w3.org/2004/02/skos/core#> .
        ex:cardEst   a ex:Topic ; rdfs:label "cardinality estimation" .
        ex:joinOrder a ex:Topic ; rdfs:label "join order optimisation" .
        ex:cat       a ex:Animal ; rdfs:label "Cat" .
        ex:catalog   a ex:Thing ; rdfs:label "Catalogue" .
        ex:finding1  ex:about ex:cardEst .
        # sq-26fdp regression: two concepts share the token "discipline"; one prefLabel is
        # punctuated. A V() of the verbatim punctuated label must resolve unambiguously.
        ex:mergeDisc a ex:Topic ; skos:prefLabel "Merge discipline" .
        ex:zkDisc    a ex:Topic ; skos:prefLabel "ZK/MPC claim + circuit discipline" .
    "#;
    Graph::load_str(ttl, "turtle").expect("graph parses")
}

#[test]
fn lexical_exact_match_resolves_and_splices_canonical_iri() {
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let exp = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"cardinality estimation\") }",
        &ctx,
        |_p| None,
    )
    .expect("an exact label match resolves");

    // The canonical SPARQL has the resolved IRI spliced in (no V() left).
    assert!(
        exp.canonical_sparql.contains("<http://example.org/cardEst>"),
        "expected the resolved IRI, got: {}",
        exp.canonical_sparql
    );
    assert!(!exp.canonical_sparql.contains("V("), "no V() must remain");

    // The resolution is echoed for the agent (design §6.2).
    assert_eq!(exp.resolutions.len(), 1);
    let r = &exp.resolutions[0];
    assert_eq!(r.phrase, "cardinality estimation");
    assert_eq!(r.iri, "http://example.org/cardEst");
    assert_eq!(r.method, Method::Lexical);
    assert!(r.confidence > 0.0);
}

#[test]
fn verbatim_punctuated_preflabel_resolves_through_the_public_surface() {
    // sq-26fdp: V("ZK/MPC claim + circuit discipline") is a VERBATIM prefLabel that shares
    // the token "discipline" with a sibling concept. Before the fix the token path scored
    // both equally and the §6 envelope loud-failed AMBIGUOUS on a phrase that IS an exact
    // label. The exact-prefLabel-first match must now splice the right IRI, unambiguously.
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let exp = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"ZK/MPC claim + circuit discipline\") }",
        &ctx,
        |_p| None,
    )
    .expect("a verbatim prefLabel must resolve, not loud-fail as ambiguous");

    assert!(
        exp.canonical_sparql.contains("<http://example.org/zkDisc>"),
        "expected the zk-discipline IRI, got: {}",
        exp.canonical_sparql
    );
    assert!(!exp.canonical_sparql.contains("V("), "no V() must remain");
    assert_eq!(exp.resolutions.len(), 1);
    let r = &exp.resolutions[0];
    assert_eq!(r.iri, "http://example.org/zkDisc");
    assert_eq!(r.method, Method::Lexical);
    assert_eq!(r.score, 1.0);
    assert!(r.runner_up.is_none(), "the verbatim label is the sole candidate");
    assert_eq!(r.confidence, 1.0);
}

#[test]
fn output_is_always_canonical_sparql() {
    // The spliced output must re-parse under spargebra (the canary runs inside
    // terse_to_sparql_with; here we additionally parse it ourselves to be sure).
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let exp = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"cardinality estimation\") }",
        &ctx,
        |_p| None,
    )
    .unwrap();
    spargebra::SparqlParser::new()
        .parse_query(&exp.canonical_sparql)
        .expect("the expanded output is conformant SPARQL");
}

#[test]
fn unresolvable_phrase_fails_loudly_not_silently() {
    // A phrase with no lexical match and no vector fallback must error, never guess
    // (design §6.3 — loud-fail beats silent-wrong).
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let err = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"zzz nonexistent concept\") }",
        &ctx,
        |_p| None,
    )
    .expect_err("an unmatched phrase must not silently bind");
    assert!(matches!(err, TerseError::Unresolved { .. }), "got {err:?}");
}

#[test]
fn passthrough_with_no_v_is_byte_identical() {
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let q = "SELECT ?s WHERE { ?s <http://example.org/p> ?o }";
    let exp = terse_to_sparql_with(q, &ctx, |_p| None).unwrap();
    assert_eq!(exp.canonical_sparql, q);
    assert!(exp.resolutions.is_empty());
    assert!(exp.keywords.is_empty());
}

#[test]
fn keyword_layer_and_v_resolution_compose() {
    // sq-vfeme — lever 1 (K:<name>) and lever 3 (V("phrase")) compose through the
    // vectors entry point: the keyword expands to its frozen IRI AND the phrase resolves to a
    // concept IRI, both spliced into one canonical query, both echoed for the agent.
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let exp = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f K:about V(\"cardinality estimation\") }",
        &ctx,
        |_p| None,
    )
    .expect("keyword + V() compose");

    // Lever 1: K:about became the canonical PKG IRI and is echoed.
    assert!(
        exp.canonical_sparql.contains("<https://sparq.dev/ns/pkg#about>"),
        "got: {}",
        exp.canonical_sparql
    );
    assert_eq!(exp.keywords.len(), 1);
    assert_eq!(exp.keywords[0].keyword, "about");
    assert_eq!(exp.keywords[0].iri, "https://sparq.dev/ns/pkg#about");

    // Lever 3: the phrase resolved and was spliced; no terse token remains.
    assert!(exp.canonical_sparql.contains("<http://example.org/cardEst>"));
    assert!(!exp.canonical_sparql.contains("K:"), "no keyword token must remain");
    assert!(!exp.canonical_sparql.contains("V("), "no V() must remain");
    assert_eq!(exp.resolutions.len(), 1);

    // The whole emission is conformant SPARQL (the canary ran; verify independently).
    spargebra::SparqlParser::new()
        .parse_query(&exp.canonical_sparql)
        .expect("the composed output is conformant SPARQL");
}

#[test]
fn unknown_keyword_is_loud_even_on_the_vectors_path() {
    // The lever-1 guardrails apply identically through terse_to_sparql_with.
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let err = terse_to_sparql_with("ASK { ?s K:nope ?o }", &ctx, |_p| None)
        .expect_err("an unknown keyword must fail loudly on the vectors path too");
    assert!(matches!(err, TerseError::UnknownKeyword { .. }), "got {err:?}");
}

#[test]
fn confidence_floor_refuses_a_weak_vector_match() {
    // Build a real store keyed to the graph, then resolve a phrase that misses lexically
    // so the vector fallback fires — but with a query vector that scores below the floor.
    let g = graph();
    let store = build_store(&g);
    let gate = ResolveGate {
        min_score: 0.99, // impossibly high floor: any real cosine is below it
        min_confidence: 0.0,
    };
    let ctx = ResolveCtx::lexical(&g)
        .with_vector_store(&store)
        .with_gate(gate);
    // "fuzzy phrase" has no lexical match; supply a query vector so the fallback engages.
    let err = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"fuzzy phrase\") }",
        &ctx,
        |_p| Some(vec![1.0, 0.0, 0.0, 0.0]),
    )
    .expect_err("a below-floor vector match must be refused");
    match err {
        TerseError::Unresolved { why, candidates, .. } => {
            assert!(why.contains("floor"), "expected a floor reason, got {why}");
            assert!(!candidates.is_empty(), "candidates must be surfaced for disambiguation");
        }
        other => panic!("expected Unresolved(floor), got {other:?}"),
    }
}

#[test]
fn staleness_guard_aborts_on_a_stale_store() {
    // A store fingerprinted to graph A, queried (vector fallback) against a DIFFERENT
    // graph B, must hard-error rather than serve stale neighbours (design §6.5).
    let g_a = graph();
    let store = build_store(&g_a);

    // A structurally different graph -> a different fingerprint.
    let g_b = Graph::load_str(
        r#"@prefix ex: <http://example.org/> . ex:other a ex:Other ."#,
        "turtle",
    )
    .unwrap();
    let ctx = ResolveCtx::lexical(&g_b).with_vector_store(&store);

    let mut embed_calls = 0;
    let err = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"fuzzy phrase\") }",
        &ctx,
        |_p| {
            embed_calls += 1;
            Some(vec![1.0, 0.0, 0.0, 0.0])
        },
    )
    .expect_err("a stale store must abort the V() resolution");
    assert!(matches!(err, TerseError::StaleStore { .. }), "got {err:?}");
    // The staleness guard runs BEFORE the embedder: a store that cannot serve the fallback
    // must not cost a model/network round-trip (nor disclose the phrase to it).
    assert_eq!(
        embed_calls, 0,
        "a stale store cannot serve the fallback => the embedder must not be consulted"
    );
}

#[test]
fn lexical_beats_vector_when_both_could_fire() {
    // An exact lexical match must take the lexical path even with a store attached
    // (design §6.4: lexical-first). The embedder must NOT be consulted.
    let g = graph();
    let store = build_store(&g);
    let ctx = ResolveCtx::lexical(&g).with_vector_store(&store);
    let mut embed_calls = 0;
    let exp = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"cardinality estimation\") }",
        &ctx,
        |_p| {
            embed_calls += 1;
            Some(vec![0.0, 1.0, 0.0, 0.0])
        },
    )
    .unwrap();
    assert_eq!(exp.resolutions[0].method, Method::Lexical);
    assert_eq!(exp.resolutions[0].iri, "http://example.org/cardEst");
    // Embedding free text is a model/network cost the caller owns (design §9 Q5); a phrase
    // the deterministic lexical linker already binds must not pay it at all.
    assert_eq!(
        embed_calls, 0,
        "lexical-first: the embedder must NOT be consulted when lexical linking succeeds"
    );
}

#[test]
fn embedder_is_not_consulted_without_a_vector_store() {
    // Lexical-only context (no store): even a phrase that misses lexically has no vector
    // fallback to feed, so the embedder must never be called — the loud Unresolved failure
    // costs no model round-trip.
    let g = graph();
    let ctx = ResolveCtx::lexical(&g);
    let mut embed_calls = 0;
    let err = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"nothing like this exists\") }",
        &ctx,
        |_p| {
            embed_calls += 1;
            Some(vec![1.0, 0.0, 0.0, 0.0])
        },
    )
    .expect_err("an unresolvable phrase must loud-fail");
    assert!(matches!(err, TerseError::Unresolved { .. }), "got {err:?}");
    assert_eq!(
        embed_calls, 0,
        "no store attached => no vector fallback => no embedder call"
    );
}

#[test]
fn embedder_is_consulted_exactly_once_when_the_vector_fallback_fires() {
    // The complement: a phrase lexical linking misses, with a store attached, DOES pay the
    // embedder — exactly once — and binds via the vector path. The gate is opened up so the
    // assertion isolates the embedder-call contract, not the (separately tested) §6.3 gate.
    let g = graph();
    let store = build_store(&g);
    let gate = ResolveGate {
        min_score: -1.0,
        min_confidence: 0.0,
    };
    let ctx = ResolveCtx::lexical(&g)
        .with_vector_store(&store)
        .with_gate(gate);
    let mut embed_calls = 0;
    let exp = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"zzz no lexical match zzz\") }",
        &ctx,
        |_p| {
            embed_calls += 1;
            Some(vec![1.0, 0.0, 0.0, 0.0])
        },
    )
    .expect("the vector fallback must bind this phrase");
    assert_eq!(exp.resolutions[0].method, Method::Vector);
    assert_eq!(embed_calls, 1, "the fallback embeds the phrase exactly once");
}

#[test]
fn ambiguity_margin_refuses_a_too_close_runner_up() {
    // Two stored vectors near-identical to the query -> a tiny top-vs-runner-up margin ->
    // the bind is ambiguous and refused (design §6.3), not silently bound to the top.
    let g = graph();
    let store = build_store(&g);
    let gate = ResolveGate {
        min_score: -1.0,        // accept any score: isolate the ambiguity gate
        min_confidence: 0.5,    // demand a clear margin
    };
    let ctx = ResolveCtx::lexical(&g)
        .with_vector_store(&store)
        .with_gate(gate);
    let err = terse_to_sparql_with(
        "SELECT ?f WHERE { ?f <http://example.org/about> V(\"fuzzy phrase\") }",
        &ctx,
        // A query equidistant-ish from several stored vectors -> top and runner-up close.
        |_p| Some(vec![0.3, 0.3, 0.3, 0.3]),
    )
    .expect_err("an ambiguous (close-runner-up) match must be refused");
    match err {
        TerseError::Unresolved { why, .. } => {
            assert!(why.contains("ambiguous"), "expected an ambiguity reason, got {why}");
        }
        other => panic!("expected Unresolved(ambiguous), got {other:?}"),
    }
}

/// Builds a real `.spqv` store keyed to `graph`'s fingerprint, with one 4-d vector per
/// term id present, written to a unique temp path (the sparq-vectors test idiom).
fn build_store(graph: &Graph) -> VectorStore {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "sparq_terse_v_{}_{}.spqv",
        std::process::id(),
        // a per-call nonce so parallel tests do not collide
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut store = VectorStore::create(&path, 4)
        .expect("create store")
        .with_fingerprint(graph);
    // Put a distinct unit-ish vector for each dict id we can read back (ids 0..n).
    // We only need SOME vectors present for the fallback to have neighbours.
    for (i, id) in (0u32..).zip(dict_ids(graph)) {
        let v = match i % 4 {
            0 => [0.6, 0.4, 0.0, 0.0],
            1 => [0.0, 0.6, 0.4, 0.0],
            2 => [0.0, 0.0, 0.6, 0.4],
            _ => [0.4, 0.0, 0.0, 0.6],
        };
        let _ = store.put(id, &v); // ignore dup/absent ids
    }
    store.finalize().expect("finalize store");
    store
}

/// The dict ids actually present in the graph (subjects/predicates/objects), de-duplicated.
fn dict_ids(graph: &Graph) -> Vec<sparq_core::dict::Id> {
    let mut ids: Vec<sparq_core::dict::Id> = graph.iter_ids_sorted(0).flatten().collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}
