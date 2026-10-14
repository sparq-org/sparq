//! sq-7d3dj.34.2.2 — differential streaming-equivalence harness for the
//! SELECT-JSON emit core (`exec::eval_select_json_emit`).
//!
//! 🤖 SPARQ agent — this is the design record's §10 B2 deliverable
//! (`research/lazy-pull-incremental-emission.md`): the load-bearing safety net for the
//! whole lazy-pull / incremental-emission program (sq-7d3dj.34.2). Every later child
//! bead (the classifier + block driver of .3/.4/.5) is admitted ONLY if it keeps the
//! §8.1 invariant — **the CONCATENATION of the chunks handed to `eval_select_json_emit`
//! is BYTE-IDENTICAL to the buffered `eval_select_json`** for the same query. This
//! harness pins that invariant NOW, before the streaming path exists, so a regression
//! that alters a single response byte goes red.
//!
//! ## What this asserts, and why byte-level
//!
//! The `#1745` API contract (`eval_select_json_emit` rustdoc) is: *the concatenation of
//! the chunks is byte-identical to `eval_select_json`/`eval_select_json_chunks` for the
//! same `flush`; only the chunk boundaries may differ.* The invariant is therefore
//! **byte-level and order-sensitive, NOT multiset**: two results with the same set of
//! solutions but a different serialised order (or different whitespace / escaping / value
//! formatting) are a FAILURE, because a streaming HTTP consumer sees the raw bytes and
//! parses them positionally. A multiset check would silently pass an order-shuffling
//! bug — exactly the class of bug the M1 mode (§5) exists to forbid.
//!
//! ## How the private emit core is reached (real path, not a mock)
//!
//! `exec` is a private module, so an integration test cannot call
//! `exec::eval_select_json_emit` by name. It drives the SAME emit core through its two
//! public front doors, so the code under test is the real one:
//!
//! * **`flush = Some(64 KiB)`** — `query_json_chunks_with_budget` (collected `Vec`) and
//!   `query_json_stream_with_budget` (live `FnMut` sink) both call
//!   `eval_select_json_emit(.., Some(JSON_CHUNK_BYTES = 64 KiB), ..)`. Testing BOTH covers
//!   the buffered-`Vec` layout AND the streaming-sink layout the design is actually about.
//! * **`flush = None`** — `query_json` returns `join_chunks(eval_select_json_chunks(.., None))`,
//!   i.e. the `eval_select_json_emit(.., None, ..)` concatenation. This is ALSO the buffered
//!   reference (`eval_select_json`), so for `flush = None` the emit-concat and the reference
//!   coincide by construction; the harness still exercises it as a distinct observation and
//!   pins it to a valid, non-empty SPARQL-results document.
//!
//! ## The corpus (design record §10 B2)
//!
//! * a multi-variable `SELECT DISTINCT` over a 3+-pattern join (the **q04 class** — the
//!   shape #1747's distinct-pushdown REJECTS because the projection is multi-variable, so
//!   it falls through to the streaming pipeline);
//! * a `UNION` of BGPs;
//! * `LIMIT` / `OFFSET` slices;
//! * residual `FILTER`s;
//! * `ORDER BY` (the buffered pipeline-breaker floor, §5 ladder step 3);
//! * empty results;
//!
//! each under **both** `flush = Some(64 KiB)` and `flush = None`.
//!
//! ## Non-vacuity (empirically verified, not by inspection)
//!
//! `mutation_witness_a_perturbed_expected_byte_goes_red` deliberately perturbs one byte
//! of the expected reference and asserts the byte-diff catches it — proving the harness
//! is not vacuously green. It is `#[should_panic]`, so the test run itself demonstrates
//! (empirically) that a single wrong byte fails the equality the other tests rely on.

use sparq_core::Graph;
use sparq_engine::{query_json, query_json_chunks_with_budget, query_json_stream_with_budget, QueryBudget};

// ─── corpus construction ──────────────────────────────────────────────────────

/// A deterministic graph shaped like the SP2Bench q04 neighbourhood: `article`s each
/// have a `creator`, a `journal`, and a `year`; some articles share a creator (so a
/// multi-variable DISTINCT over the creator×journal join has genuine duplicates to
/// remove). `n_articles` scales the result so the JSON exceeds one 64 KiB chunk.
fn q04_graph(n_articles: usize) -> Graph {
    let mut nt = String::new();
    for i in 0..n_articles {
        let art = format!("<http://ex/article/{}>", i);
        // Deliberately few distinct creators / journals so DISTINCT removes rows and the
        // join fans out (multiple articles per creator, multiple per journal).
        let creator = i % 7;
        let journal = i % 5;
        let year = 1990 + (i % 30);
        nt.push_str(&format!("{} <http://ex/creator> <http://ex/person/{}> .\n", art, creator));
        nt.push_str(&format!("{} <http://ex/journal> <http://ex/journal/{}> .\n", art, journal));
        nt.push_str(&format!(
            "{} <http://ex/year> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            art, year
        ));
        // A padding literal so string escaping + value formatting are exercised in the bytes.
        nt.push_str(&format!(
            "{} <http://ex/title> \"Article {} — a \\\"quoted\\\" title, padding padding padding\" .\n",
            art, i
        ));
    }
    Graph::load_str(&nt, "ntriples").expect("load q04-class graph")
}

/// A tiny graph for the empty-result and small-shape cases (single chunk, `flush` never trips).
fn small_graph() -> Graph {
    let nt = "\
<http://ex/a> <http://ex/p> <http://ex/b> .
<http://ex/a> <http://ex/q> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://ex/b> <http://ex/p> <http://ex/c> .
<http://ex/b> <http://ex/q> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://ex/c> <http://ex/p> <http://ex/a> .
<http://ex/c> <http://ex/q> \"3\"^^<http://www.w3.org/2001/XMLSchema#integer> .
";
    Graph::load_str(nt, "ntriples").expect("load small graph")
}

const PFX: &str = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

/// The corpus: `(label, &graph, query)` triples covering every shape §10 B2 names. Each
/// is run under both flush modes by the driver below.
fn corpus<'a>(q04: &'a Graph, small: &'a Graph) -> Vec<(&'static str, &'a Graph, String)> {
    let mut c: Vec<(&'static str, &Graph, String)> = Vec::new();

    // 1. q04 class — multi-variable SELECT DISTINCT over a 3-pattern join. The
    //    projection is (?creator, ?journal), which #1747's single-var distinct-pushdown
    //    REJECTS, so this falls through to the streaming pipeline the design targets.
    //    ORDER BY makes the row order deterministic so the byte-level reference is stable.
    c.push((
        "q04_distinct_multivar_join",
        q04,
        format!(
            "{PFX}SELECT DISTINCT ?creator ?journal WHERE {{ \
             ?a ex:creator ?creator . ?a ex:journal ?journal . ?a ex:year ?year \
             }} ORDER BY ?creator ?journal"
        ),
    ));
    // The same shape WITHOUT the stabilising ORDER BY — a self-consistency check confirms
    // the engine is deterministic on it (see `driver` self-check), so the cross-path
    // byte-diff is meaningful even for the unordered DISTINCT case.
    c.push((
        "q04_distinct_multivar_join_unordered",
        q04,
        format!(
            "{PFX}SELECT DISTINCT ?creator ?journal WHERE {{ \
             ?a ex:creator ?creator . ?a ex:journal ?journal . ?a ex:year ?year }}"
        ),
    ));

    // 2. UNION of BGPs.
    c.push((
        "union_of_bgps",
        q04,
        format!(
            "{PFX}SELECT ?a ?v WHERE {{ \
             {{ ?a ex:creator ?v }} UNION {{ ?a ex:journal ?v }} \
             }} ORDER BY ?a ?v"
        ),
    ));

    // 3. LIMIT / OFFSET.
    c.push((
        "limit_offset",
        q04,
        format!("{PFX}SELECT ?a ?year WHERE {{ ?a ex:year ?year }} ORDER BY ?year ?a LIMIT 137 OFFSET 41"),
    ));

    // 4. Residual FILTER (a join with a post-join scalar filter).
    c.push((
        "residual_filter",
        q04,
        format!(
            "{PFX}SELECT ?a ?year WHERE {{ \
             ?a ex:year ?year . ?a ex:creator ?creator \
             FILTER(?year > 2005 && ?year < 2015) \
             }} ORDER BY ?a"
        ),
    ));

    // 5. ORDER BY over the full join (the buffered pipeline-breaker floor, §5 ladder 3).
    c.push((
        "order_by_full",
        q04,
        format!(
            "{PFX}SELECT ?creator ?journal ?year WHERE {{ \
             ?a ex:creator ?creator . ?a ex:journal ?journal . ?a ex:year ?year \
             }} ORDER BY DESC(?year) ?creator ?journal"
        ),
    ));

    // 6. Empty results — a matched shape that yields zero rows, and a projected-empty.
    c.push((
        "empty_no_match",
        small,
        format!("{PFX}SELECT ?s ?o WHERE {{ ?s ex:nonexistent ?o }}"),
    ));
    c.push((
        "empty_join_no_solutions",
        small,
        format!("{PFX}SELECT ?s WHERE {{ ?s ex:p ?mid . ?mid ex:p ?s FILTER(?s = ex:nonexistent) }}"),
    ));

    // A couple of small deterministic shapes so `flush = None` (single-chunk) and the
    // fast single-pattern scan are both in the corpus.
    c.push((
        "small_star",
        small,
        format!("{PFX}SELECT ?s ?p ?o WHERE {{ ?s ?p ?o }} ORDER BY ?s ?p ?o"),
    ));

    c
}

// ─── the differential driver ──────────────────────────────────────────────────

/// The two flush modes the harness exercises. `Some(64 KiB)` is the value both public
/// chunked entry points hand to `eval_select_json_emit`; `None` is the single-string
/// (`eval_select_json`) layout.
#[derive(Clone, Copy, Debug)]
enum Flush {
    /// `eval_select_json_emit(.., Some(JSON_CHUNK_BYTES = 64 KiB), ..)`, driven through
    /// both `query_json_chunks_with_budget` (Vec) and `query_json_stream_with_budget` (sink).
    SixtyFourKiB,
    /// `eval_select_json_emit(.., None, ..)` — the single-string `eval_select_json` layout.
    None,
}

/// Collect the emit-core chunks for `q` under `flush`, using the *streaming sink* front
/// door (`query_json_stream_with_budget`) for the `Some(64 KiB)` case.
fn stream_concat(g: &Graph, q: &str) -> String {
    let b = QueryBudget::unlimited();
    let mut out = String::new();
    query_json_stream_with_budget(g, q, &b, |c| {
        out.push_str(&c);
        std::ops::ControlFlow::Continue(())
    })
    .expect("stream query error");
    out
}

/// The emit-core concatenation under `flush`.
fn emit_concat(g: &Graph, q: &str, flush: Flush) -> String {
    let b = QueryBudget::unlimited();
    match flush {
        // Both `query_json_chunks_with_budget` (Vec) and the streaming sink go through
        // `eval_select_json_emit(.., Some(64 KiB), ..)`. Assert they AGREE, then return one.
        Flush::SixtyFourKiB => {
            let via_vec = query_json_chunks_with_budget(g, q, &b).expect("chunks query error").concat();
            let via_sink = stream_concat(g, q);
            assert_eq!(
                via_vec, via_sink,
                "the Vec and live-sink front doors of eval_select_json_emit(Some(64KiB)) must agree byte-for-byte"
            );
            via_vec
        }
        // `query_json` == join_chunks(eval_select_json_chunks(.., None)) == the
        // eval_select_json_emit(.., None, ..) concatenation (and the buffered reference).
        Flush::None => query_json(g, q).expect("buffered query error"),
    }
}

/// The buffered reference: `eval_select_json` (== `query_json`). Byte-identity is asserted
/// against THIS for every flush mode.
fn reference(g: &Graph, q: &str) -> String {
    query_json(g, q).expect("reference query error")
}

/// Run the full corpus under both flush modes, asserting byte-level order-sensitive
/// equality of the emit-concat against the buffered reference. Also self-checks that the
/// engine is DETERMINISTIC on each query (two reference evaluations agree) — otherwise a
/// cross-path byte-diff would be meaningless.
#[test]
fn stream_emit_concat_is_byte_identical_to_buffered_over_corpus() {
    let q04 = q04_graph(4000); // > 64 KiB of JSON on the wide shapes → multi-chunk
    let small = small_graph();
    let cases = corpus(&q04, &small);

    for (label, g, q) in &cases {
        // Self-check: the reference must be deterministic (order-preserving parallel
        // collection, design record §2) so byte-level cross-path equality is well-defined.
        let ref1 = reference(g, q);
        let ref2 = reference(g, q);
        assert_eq!(ref1, ref2, "reference is non-deterministic for [{label}] — byte-diff would be meaningless: {q}");

        // A valid, closed SPARQL-results document (the empty cases still close correctly).
        assert!(ref1.starts_with("{\"head\""), "[{label}] reference must open a results doc: {}", &ref1[..ref1.len().min(40)]);
        assert!(ref1.ends_with("]}}"), "[{label}] reference must close the results doc");

        for flush in [Flush::None, Flush::SixtyFourKiB] {
            let got = emit_concat(g, q, flush);
            assert_eq!(
                got, ref1,
                "[{label}] emit-concat under {flush:?} is NOT byte-identical to the buffered reference: {q}"
            );
        }
    }
}

/// A dedicated case pinning that the wide q04-class result actually splits into MULTIPLE
/// chunks under `Some(64 KiB)` — otherwise the `flush = Some(64 KiB)` path would be
/// exercising only the single-chunk layout and the multi-chunk boundary logic would be
/// untested (a vacuity trap distinct from the byte-value mutation below).
#[test]
fn q04_class_result_is_genuinely_multi_chunk_under_64kib() {
    let q04 = q04_graph(4000);
    let q = format!(
        "{PFX}SELECT DISTINCT ?a ?creator ?journal ?year WHERE {{ \
         ?a ex:creator ?creator . ?a ex:journal ?journal . ?a ex:year ?year \
         }} ORDER BY ?a"
    );
    let b = QueryBudget::unlimited();

    let single = query_json(&q04, &q).unwrap();
    assert!(single.len() > 64 * 1024, "corpus must exceed one 64 KiB chunk (got {} bytes)", single.len());

    let chunks = query_json_chunks_with_budget(&q04, &q, &b).unwrap();
    assert!(chunks.len() > 1, "expected a genuinely multi-chunk stream, got {}", chunks.len());
    // First chunk opens, only the last closes — the boundary logic the byte-diff protects.
    assert!(chunks[0].starts_with("{\"head\""), "first chunk must carry the header");
    assert!(chunks.last().unwrap().ends_with("]}}"), "only the last chunk closes the document");
    assert!(!chunks[0].ends_with("]}}"), "the header chunk must not be the whole document");
    // The load-bearing invariant, on the wide multi-chunk shape.
    assert_eq!(chunks.concat(), single, "multi-chunk concat must be byte-identical to the buffered reference");
}

// ─── non-vacuity: a perturbed expected byte MUST go red ────────────────────────

/// NON-VACUITY WITNESS (design record §10 B2: *a perturbed byte goes red*).
///
/// This test deliberately corrupts ONE byte of the expected reference and then asserts
/// the emit-concat equals the corrupted expectation. The assertion MUST fail — proving,
/// EMPIRICALLY (not by inspection), that the byte-level equality the real tests rely on
/// actually catches a single wrong byte. `#[should_panic]` inverts it: the test suite is
/// GREEN precisely because the perturbed comparison went RED.
///
/// The panic message is matched so this can only pass by failing the INTENDED assertion
/// (a compile/setup panic with a different message would not satisfy `expected`).
#[test]
#[should_panic(expected = "MUTATION WITNESS: perturbed byte must differ")]
fn mutation_witness_a_perturbed_expected_byte_goes_red() {
    let g = small_graph();
    let q = format!("{PFX}SELECT ?s ?p ?o WHERE {{ ?s ?p ?o }} ORDER BY ?s ?p ?o");

    // The true emit-concat (Some(64 KiB) front door) and the true reference.
    let got = emit_concat(&g, &q, Flush::SixtyFourKiB);
    let reference = query_json(&g, &q).expect("reference query error");
    // Sanity: without perturbation they ARE equal (so the failure below is caused ONLY by
    // the deliberate perturbation, not a pre-existing mismatch).
    assert_eq!(got, reference, "pre-condition: unperturbed emit-concat must match the reference");

    // Perturb exactly one byte of the EXPECTED value. Pick an ASCII byte in the body and
    // flip it to a different ASCII byte so the corrupted expectation is still valid UTF-8
    // but differs from `got` in exactly one position.
    let mut perturbed = reference.clone().into_bytes();
    // Choose a mid-document position that is a plain ASCII letter/digit to flip.
    let pos = perturbed
        .iter()
        .position(|&b| b.is_ascii_alphanumeric() && b != b'h') // skip the "head" region start
        .map(|p| p + 8) // a little into the body, still ASCII in this corpus
        .filter(|&p| p < perturbed.len())
        .expect("corpus must contain an ASCII body byte to perturb");
    let original = perturbed[pos];
    perturbed[pos] = if original == b'z' { b'a' } else { original + 1 };
    let perturbed = String::from_utf8(perturbed).expect("single-byte ASCII flip stays valid UTF-8");

    // Confirm the perturbation actually changed a byte (guards against a no-op flip).
    assert_ne!(perturbed, reference, "internal: perturbation must change the expected string");

    // THE ASSERTION UNDER TEST: comparing the true emit-concat to the perturbed
    // expectation MUST fail — the harness's byte-level equality is non-vacuous.
    assert_eq!(got, perturbed, "MUTATION WITNESS: perturbed byte must differ from the true emit-concat");
}
