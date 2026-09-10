//! SPARQL **UPDATE** differential fuzzer vs Oxigraph (beads sq-3dyje.4, sq-hodke). [FABLE-5]
//!
//! The query-side differential fuzzer (`fuzz.rs`) has zero UPDATE coverage, and
//! `sparq-engine/src/update.rs` is otherwise guarded by a single mechanism family
//! (unit tests + the fixed W3C conformance corpus). This module closes that gap with
//! a seeded generator of update sequences applied step-by-step to THREE independent
//! implementations:
//!
//!   1. sparq's rebuild path        (`sparq_engine::update` — decode / apply / rebuild),
//!   2. sparq's delta-overlay path  (`sparq_engine::update_in_place`),
//!   3. in-process Oxigraph 0.5     (`Store::update`).
//!
//! After EVERY step it asserts, per the AGENTS.md oracle-strength rule (term
//! structure and full answer sets, never row counts):
//!
//!   * **canonical dataset equality** — each store rendered to N-Quads lines and
//!     compared under the canonical form described in *Canonical comparison* below;
//!   * **probe SELECT equality** — the full sorted binding set of
//!     `SELECT ?s ?p ?o { ?s ?p ?o }` and `SELECT ?s ?p ?o ?g { GRAPH ?g { ?s ?p ?o } }`
//!     through each engine's own query path, under the same canonical form.
//!
//! A per-step compare means any divergence is localized to the exact offending
//! operation, and the final-store comparison is the last step's compare.
//!
//! ## Generated subset
//!
//! Deterministic by construction — no `NOW()`/`RAND()`/`UUID()`/`BNODE()`. (The
//! `BNODE()` FUNCTION is what would be non-deterministic; blank node LABELS written
//! into a data block or template are a pure function of the seed, and the labels the
//! engines then mint are exactly what the isomorphism-aware compare exists to absorb.)
//! `INSERT DATA` / `DELETE DATA` (with `GRAPH` blocks), `DELETE/INSERT … WHERE`
//! (ground and variable templates, `WITH`, `USING`, variable graph names,
//! `DELETE WHERE`), `CLEAR` / `DROP` / `CREATE` / `COPY` / `MOVE` / `ADD` (always
//! `SILENT`, so absent-graph error behaviour — which the spec leaves
//! implementation-defined — cannot masquerade as a semantic divergence; the state
//! compare is the whole oracle), plus occasional two-operation compound requests
//! (`op ; op`) for the within-one-update sequencing path.
//!
//! v2 (sq-hodke) adds four term/operation families on top of the v1 ground-term subset:
//!
//!   * **non-canonical numeric lexicals** (`"05"^^xsd:integer`, `"+7"^^xsd:integer`) —
//!     RDF term identity is lexical-form identity, so these must survive a store
//!     round-trip as terms DISTINCT from their canonical siblings. sparq's dictionary
//!     inlines only the canonical form (`dict::try_inline_lit`), so the non-canonical
//!     one takes the general path. This is the one family where Oxigraph is NOT a
//!     usable reference (see *Adjudicated divergences* below), so distinctness is
//!     asserted by sparq's two update paths against EACH OTHER — that compare is never
//!     adjudicated — and pinned directly by
//!     `tests::non_canonical_integer_lexicals_are_distinct_terms`.
//!   * **blank nodes** in `INSERT DATA` blocks and in `INSERT` templates (fresh per
//!     operation, SPARQL 1.1 §3.1.1; fresh per solution, §3.1.3) — which is what forces the
//!     isomorphism-aware comparison. Blank nodes never enter `DELETE DATA` or a
//!     ground `WHERE` condition (the spec forbids the former, and a blank node's
//!     label is not re-referenceable across operations anyway).
//!   * **`LOAD`** of a local `file://` document — see *LOAD* below.
//!   * **RDF-1.2 triple terms** — `<<( s p o )>>` (object position only; the parser
//!     rejects one as a subject, which is RDF 1.2's rule) in data blocks, in `INSERT`
//!     templates, and nested one level deep, plus the `<< s p o >>` reifier form
//!     (which desugars to a fresh blank node + `rdf:reifies`).
//!
//! Graph EXISTENCE (an empty named graph created by `CREATE`) is invisible to a
//! quad-level compare and remains out of scope.
//!
//! ## Canonical comparison
//!
//! With v1's ground terms, sorted N-Quads WAS a canonical form. With blank nodes it
//! is not: sparq labels them `_:fbN` off a process-wide counter, Oxigraph off a hash —
//! and sparq's OWN two update paths therefore disagree with each other. So when either
//! side carries a blank node, both snapshots are relabelled to their **RDFC-1.0**
//! canonical form (`sparq_canon`) before comparison, which decides RDF isomorphism.
//! Triple terms are outside W3C RDFC-1.0, so the comparator uses that crate's
//! CONSTRAINED `*_ground_terms` entry points: exactly RDFC-1.0 with triple terms as
//! opaque constants, failing closed if a blank node is ever nested inside a triple
//! term. The generator upholds that invariant (the triple-term template carries an
//! `isBlank` FILTER guard) and `tests::no_nested_blank_nodes_in_triple_terms` pins it,
//! so the non-standard nested-bnode descent is never reachable from here.
//!
//! Canonicalization deduplicates, so the RAW line counts are compared separately: a
//! store yielding the same quad twice is an invariant break worth failing on, and that
//! check would otherwise be lost.
//!
//! ## LOAD
//!
//! The bead's premise — "a local `file://` doc both engines can fetch" — does not hold,
//! and the code says so: Oxigraph 0.5's `eval_load` is HTTP-only and, without the
//! `http-client` feature this harness deliberately does not enable, returns
//! *"HTTP client is not available"* for ANY source; sparq's `load_document` is the exact
//! mirror image (`file://` only, and only under an allowlisted base). The two engines'
//! LOAD source surfaces are DISJOINT, so a same-request LOAD differential is impossible.
//! `tests::oxigraph_cannot_load_a_local_file` pins that reason so it is machine-checked
//! rather than merely asserted here.
//!
//! What the harness does instead: sparq's two paths get the real
//! `LOAD <file://doc.nt> [INTO GRAPH g]`, and the reference engine gets the
//! semantically-equivalent `INSERT DATA` of the SAME ground triples. The oracle is
//! therefore "sparq's LOAD produces exactly the dataset the reference engine reaches by
//! inserting that document's contents", plus a true two-implementation differential
//! between sparq's rebuild and delta-overlay LOAD paths. The document is written under a
//! per-run temporary directory allowlisted via `sparq_engine::with_load_base`, and the
//! generated IRI is RELATIVE (`<file://doc0.nt>`) so the request text — and hence the
//! seed repro — is independent of that directory's absolute path.
//!
//! ## Adjudicated divergences
//!
//! The mechanism mirrors `fuzz.rs`: the comparator consults
//! `bench/differential-divergences.json` for classes whose id starts with `update-`. A
//! listed class without a detector in this file stays STRICT, with a loud warning —
//! never a silent skip.
//!
//! One class is adjudicated today,
//! `update-oxigraph-integer-lexical-canonicalization`: Oxigraph's storage layer parses
//! every `xsd:integer` into a native integer and re-renders it canonically
//! (`storage/numeric_encoder.rs`), so `"05"^^xsd:integer` and `"5"^^xsd:integer` become
//! the SAME stored term. RDF 1.1 Concepts §3.3 makes them different terms (literal
//! equality is lexical-form equality), so sparq is spec-correct and Oxigraph is lossy.
//! It is NOT a blind skip: the comparator re-derives Oxigraph's normalization
//! independently — rewriting every `xsd:integer` literal on BOTH sides to its canonical
//! lexical form — and absorbs the step ONLY when that rewrite is injective on terms,
//! preserves rows and blank nodes, and the two datasets agree exactly under it.
//! Any residual difference still FAILS. The sparq-vs-sparq
//! compare never consults the allowlist: both sides are sparq, so any lexical
//! disagreement between them is a real bug.
//!
//! To keep that class narrow, non-canonical lexicals are generated only in INSERT
//! positions. An exact-term `DELETE DATA` of one (where sparq removes `"05"` and leaves
//! `"5"` while Oxigraph, holding one merged term, removes both) is a cascade no
//! normalization can undo; that shape is covered instead by the sparq-internal
//! `tests::non_canonical_integer_lexicals_are_distinct_terms`, which is the correct
//! oracle given Oxigraph cannot serve as a reference for it.
//!
//! [GPT-6 ASTRA] #5183 also exposed a solution-multiplicity cascade: coexisting `8`
//! and `"008"^^xsd:integer` produce two template blank nodes in sparq, but one in the
//! lossy reference. The comparator correctly rejects this. The reference corpus now
//! uses disjoint canonical/noncanonical integer value pools, with one spelling per
//! value globally, including LOAD and nested triple terms. This narrows coverage;
//! the exact historical sequence remains a sparq-only regression. Noncanonical
//! terms still reach storage, queries, templates and the reference comparison.
//! This correspondence is limited to the generated BGP/graph/isBlank operations
//! and full-quad probes: it does not justify lexical-sensitive STR/sameTerm queries.
//! A changed generator changes a seed's input; it does not resolve other historical
//! failures merely because their seed numbers pass under the new generator.
//!
//! KNOWN SHARED-ORACLE BLIND SPOT (honest boundary): both engines parse updates with
//! `spargebra`, so a parser-level desugaring bug (e.g. in COPY/MOVE/ADD expansion, or in
//! the `<< s p o >>` reifier desugaring) would affect both sides identically and cannot
//! be caught here — only evaluation-layer divergence is observable.
//!
//! Usage: `sparq-bench update-fuzz --seed-start N --seed-count M`
//! Every case is reproducible from its seed; on failure the log carries
//! `MISMATCH seed=N` lines plus a `FIRST FAILING CASE:` block (the same
//! machine-parseable contract as `fuzz.rs`, consumed by
//! `scripts/ci-file-differential-failure.py`).

use oxigraph::store::Store;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, Quad, Term, Triple};
use sparq_core::Graph;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The `xsd:integer` IRI in the N-Triples spelling the generator emits.
const XSD_INTEGER: &str = "<http://www.w3.org/2001/XMLSchema#integer>";

// [GPT-6 ASTRA] Keep the small canonical pool for exact DELETE/WHERE hits. The
// noncanonical pool starts above it; LOAD must use this same canonical bound.
const CANONICAL_INTEGER_VALUES: u64 = 20;

fn gen_canonical_integer(rng: &mut Rng) -> u64 {
    rng.below(CANONICAL_INTEGER_VALUES)
}

// ── deterministic RNG ────────────────────────────────────────────────────────────

/// Deterministic SplitMix64 — no clock/entropy, so every case is reproducible from
/// its seed. (Same generator as `fuzz.rs`; duplicated because the bead keeps this
/// file's edits disjoint from the query fuzzer's.)
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

// ── generator ────────────────────────────────────────────────────────────────────
//
// Small term pools so random DELETE DATA / WHERE conditions collide with previously
// inserted data at high probability (a delete that never matches exercises nothing).

fn gen_subject(rng: &mut Rng) -> String {
    format!("<http://ex/s{}>", rng.below(6))
}

fn gen_predicate(rng: &mut Rng) -> String {
    format!("<http://ex/p{}>", rng.below(4))
}

/// A simple ground object: IRI / plain string / canonical `xsd:integer` (the
/// dictionary's inline-id path) / language-tagged string. No triple term — this is
/// what nests INSIDE a triple term, and what the reifier form quotes.
fn gen_simple_object(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 | 1 => format!("<http://ex/o{}>", rng.below(6)),
        2 | 3 => format!("\"lit{}\"", rng.below(5)),
        4 => format!("{}", gen_canonical_integer(rng)),
        _ => format!("\"tag{}\"@en", rng.below(3)),
    }
}

/// An RDF-1.2 triple term `<<( s p o )>>`. `depth` bounds further nesting (the
/// canonicalizer enforces a depth bound of its own, and deep nesting buys nothing
/// the one-level case does not).
fn gen_triple_term(rng: &mut Rng, depth: u32) -> String {
    let o = if depth > 0 && rng.chance(1, 4) {
        gen_triple_term(rng, depth - 1)
    } else {
        gen_simple_object(rng)
    };
    format!("<<( {} {} {} )>>", gen_subject(rng), gen_predicate(rng), o)
}

/// A POOLABLE ground object — ground, blank-node-free, and in canonical lexical
/// form, so the enclosing quad can later be reused verbatim as an exact-term
/// `DELETE DATA` quad or a ground `WHERE` condition (see [`GenQuad::poolable`]).
fn gen_object(rng: &mut Rng) -> String {
    if rng.chance(1, 8) {
        gen_triple_term(rng, 1)
    } else {
        gen_simple_object(rng)
    }
}

/// A NON-CANONICAL `xsd:integer` lexical. Every form here is non-canonical by
/// construction (`v` is non-negative and something is always prepended), so it must
/// round-trip as a term DISTINCT from `"{v}"^^xsd:integer`.
fn gen_noncanonical_integer(rng: &mut Rng) -> String {
    // Two draws, as before. Encoding the style in the value gives every value one
    // spelling while retaining all three prefix families and their probabilities.
    let offset = rng.below(CANONICAL_INTEGER_VALUES);
    let style = rng.below(3);
    let v = CANONICAL_INTEGER_VALUES + 3 * offset + style;
    let lexical = match style {
        0 => format!("0{}", v),
        1 => format!("+{}", v),
        _ => format!("00{}", v),
    };
    format!("\"{}\"^^{}", lexical, XSD_INTEGER)
}

/// A ground object spelled so the SAME text is valid in BOTH N-Triples (the `LOAD`
/// document) and SPARQL (the reference engine's equivalent `INSERT DATA`). That rules
/// out the bare-integer Turtle shorthand `4`, which N-Triples does not accept; the
/// integer arm writes the typed literal out in full, in CANONICAL lexical form so the
/// loaded quads stay poolable and outside the adjudicated numeric class.
fn gen_ntriples_object(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 | 1 => format!("<http://ex/o{}>", rng.below(6)),
        2 | 3 => format!("\"lit{}\"", rng.below(5)),
        4 => format!("\"{}\"^^{}", gen_canonical_integer(rng), XSD_INTEGER),
        _ => format!("\"tag{}\"@en", rng.below(3)),
    }
}

/// A blank node label. A 3-label pool means one `INSERT DATA` block can reference the
/// same blank node from several quads, giving the isomorphism check real structure to
/// adjudicate instead of isolated leaves.
///
/// `ns` is the label NAMESPACE of the operation being generated. SPARQL 1.1 §19.6
/// forbids a blank node label from being shared by two operations of one request
/// (spargebra rejects it outright: "The blank node _:b0 cannot be shared by multiple
/// blocks"), so the two halves of a compound `op ; op` draw from disjoint namespaces.
fn gen_bnode(ns: &str, rng: &mut Rng) -> String {
    format!("_:{}{}", ns, rng.below(3))
}

/// One of the 3 named graphs. A small pool keeps CLEAR/DROP/COPY/MOVE/ADD landing on
/// graphs that actually hold data.
fn gen_graph_iri(rng: &mut Rng) -> String {
    format!("<http://ex/g{}>", rng.below(3))
}

/// Generator state threaded through one sequence: the ground quads emitted by
/// earlier `INSERT DATA` / `LOAD` operations, as `(graph IRI or None-for-default,
/// "s p o .")`. `DELETE DATA` (and the ground `WHERE` condition) sample mostly from
/// this pool so deletions/conditions actually HIT live data — a purely random quad
/// almost never matches the store, which was measured to let a neutered DELETE DATA
/// path slip through a 200-seed window undetected. Still a pure function of the seed.
///
/// ONLY [`GenQuad::poolable`] quads are recorded: a blank node's label is not
/// re-referenceable across operations (a second mention makes a FRESH node), and a
/// non-canonical numeric lexical must stay out of `DELETE DATA` / `WHERE` to keep the
/// adjudicated Oxigraph-canonicalization class narrow (see the module docs).
#[derive(Default)]
struct GenState {
    inserted: Vec<(Option<String>, String)>,
}

/// A generated quad plus whether it may enter the reuse pool.
struct GenQuad {
    slot: Option<String>,
    triple: String,
    poolable: bool,
}

fn gen_slot(rng: &mut Rng) -> Option<String> {
    if rng.chance(2, 5) {
        Some(gen_graph_iri(rng))
    } else {
        None
    }
}

/// A random fresh ground, canonical, blank-node-free quad — always poolable.
fn gen_quad(rng: &mut Rng) -> GenQuad {
    let triple = format!(
        "{} {} {} .",
        gen_subject(rng),
        gen_predicate(rng),
        gen_object(rng)
    );
    GenQuad {
        slot: gen_slot(rng),
        triple,
        poolable: true,
    }
}

/// The INSERT-side quad. On top of [`gen_quad`] it may carry a blank node (subject
/// and/or object), a non-canonical numeric lexical, or the `<< s p o >>` reifier form
/// (which desugars to a fresh blank node + `rdf:reifies`) — none of which is poolable.
fn gen_insert_quad(ns: &str, rng: &mut Rng) -> GenQuad {
    let (subject, subject_poolable) = if rng.chance(1, 8) {
        (gen_bnode(ns, rng), false)
    } else {
        (gen_subject(rng), true)
    };
    // The non-canonical-lexical arm is deliberately the RAREST: every step for which
    // one is live in the store compares against Oxigraph under the adjudicated
    // normalization rather than byte-for-byte, so keeping it rare keeps the strictest
    // regime dominant while still covering the family in every seed window (pinned by
    // `tests::generator_emits_every_v2_family`).
    let (object, object_poolable) = match rng.below(64) {
        0 => (gen_bnode(ns, rng), false),
        1 => (gen_noncanonical_integer(rng), false),
        2 | 3 => (
            format!(
                "<< {} {} {} >>",
                gen_subject(rng),
                gen_predicate(rng),
                gen_simple_object(rng)
            ),
            false,
        ),
        _ => (gen_object(rng), true),
    };
    GenQuad {
        slot: gen_slot(rng),
        triple: format!("{} {} {} .", subject, gen_predicate(rng), object),
        poolable: subject_poolable && object_poolable,
    }
}

/// Renders a quad into a data block: bare triple or `GRAPH`-wrapped.
fn push_quad(block: &mut String, slot: &Option<String>, t: &str) {
    match slot {
        Some(g) => block.push_str(&format!(" GRAPH {} {{ {} }}", g, t)),
        None => block.push_str(&format!(" {}", t)),
    }
}

/// The `INSERT DATA` block: 1..=5 fresh quads; the poolable ones are recorded.
fn gen_insert_block(ns: &str, rng: &mut Rng, st: &mut GenState) -> String {
    let n = 1 + rng.below(5);
    let mut s = String::new();
    for _ in 0..n {
        let q = gen_insert_quad(ns, rng);
        push_quad(&mut s, &q.slot, &q.triple);
        if q.poolable {
            st.inserted.push((q.slot, q.triple));
        }
    }
    s
}

/// The `DELETE DATA` block: 1..=5 quads, each drawn from the (poolable-only) pool
/// with probability 7/10 — a hit unless an intervening op already removed it — and
/// fresh random otherwise (the delete-of-absent-quad no-op path).
fn gen_delete_block(rng: &mut Rng, st: &GenState) -> String {
    let n = 1 + rng.below(5);
    let mut s = String::new();
    for _ in 0..n {
        let (slot, t) = if !st.inserted.is_empty() && rng.chance(7, 10) {
            st.inserted[rng.below(st.inserted.len() as u64) as usize].clone()
        } else {
            let q = gen_quad(rng);
            (q.slot, q.triple)
        };
        push_quad(&mut s, &slot, &t);
    }
    s
}

/// A `DELETE/INSERT … WHERE` operation. Templates are ground or variable; the WHERE
/// is a single BGP (optionally GRAPH-wrapped / re-scoped). The query-side differential
/// owns complex WHERE evaluation; the shapes here exist to drive the update-side
/// template instantiation, `WITH`/`USING` re-scoping, and variable-graph-name paths.
fn gen_where_op(ns: &str, rng: &mut Rng, st: &GenState) -> String {
    match rng.below(11) {
        // Conditional ground insert: fires iff the (ground) condition triple exists.
        // The condition samples the inserted pool half the time so both the
        // condition-holds and condition-fails branches are exercised. The condition is
        // always a POOLABLE (canonical, blank-node-free) triple; the inserted template
        // object may be a non-canonical lexical.
        0 => {
            let cond = if !st.inserted.is_empty() && rng.chance(1, 2) {
                let (slot, t) = &st.inserted[rng.below(st.inserted.len() as u64) as usize];
                match slot {
                    Some(g) => format!("GRAPH {} {{ {} }}", g, t),
                    None => t.clone(),
                }
            } else {
                format!(
                    "{} {} {} .",
                    gen_subject(rng),
                    gen_predicate(rng),
                    gen_object(rng)
                )
            };
            let object = if rng.chance(1, 16) {
                gen_noncanonical_integer(rng)
            } else {
                gen_object(rng)
            };
            format!(
                "INSERT {{ {} {} {} }} WHERE {{ {} }}",
                gen_subject(rng),
                gen_predicate(rng),
                object,
                cond
            )
        }
        // Remove every `p` edge from the default graph.
        1 => {
            let p = gen_predicate(rng);
            format!("DELETE {{ ?s {} ?o }} WHERE {{ ?s {} ?o }}", p, p)
        }
        // Predicate rename in the default graph (delete + insert see the same pre-state).
        2 => {
            let p = gen_predicate(rng);
            let p2 = gen_predicate(rng);
            format!(
                "DELETE {{ ?s {} ?o }} INSERT {{ ?s {} ?o }} WHERE {{ ?s {} ?o }}",
                p, p2, p
            )
        }
        // Copy the default graph into a named graph (template GRAPH block).
        3 => format!(
            "INSERT {{ GRAPH {} {{ ?s ?p ?o }} }} WHERE {{ ?s ?p ?o }}",
            gen_graph_iri(rng)
        ),
        // Cross-graph delete with a VARIABLE graph name in the template.
        4 => {
            let p = gen_predicate(rng);
            format!(
                "DELETE {{ GRAPH ?g {{ ?s {} ?o }} }} WHERE {{ GRAPH ?g {{ ?s {} ?o }} }}",
                p, p
            )
        }
        // WITH-scoped predicate rename inside one named graph.
        5 => {
            let g = gen_graph_iri(rng);
            let p = gen_predicate(rng);
            let p2 = gen_predicate(rng);
            format!(
                "WITH {} DELETE {{ ?s {} ?o }} INSERT {{ ?s {} ?o }} WHERE {{ ?s {} ?o }}",
                g, p, p2, p
            )
        }
        // USING re-scoping: WHERE evaluates against the named graph, the deletion
        // template applies to the default graph.
        6 => format!(
            "DELETE {{ ?s ?p ?o }} USING {} WHERE {{ ?s ?p ?o }}",
            gen_graph_iri(rng)
        ),
        // Pull a named graph's triples into the default graph.
        7 => format!(
            "INSERT {{ ?s ?p ?o }} WHERE {{ GRAPH {} {{ ?s ?p ?o }} }}",
            gen_graph_iri(rng)
        ),
        // Blank node in an INSERT template: SPARQL 1.1 §3.1.3 makes it FRESH per
        // solution, so N matching rows must yield N distinct nodes on every engine —
        // pure isomorphism structure, invisible to a label-sensitive compare. Scoped
        // to one predicate to keep the produced node count small.
        8 => {
            let p = gen_predicate(rng);
            format!(
                "INSERT {{ ?s {} _:{}t }} WHERE {{ ?s {} ?o }}",
                gen_predicate(rng),
                ns,
                p
            )
        }
        // RDF-1.2 triple term built from the solution. The `isBlank` FILTER is LOAD-
        // BEARING, not decoration: it keeps blank nodes out of the quoted triple, which
        // is the precondition of the constrained RDFC-1.0 profile the comparator uses
        // (see the module docs and `tests::no_nested_blank_nodes_in_triple_terms`).
        9 => {
            let p = gen_predicate(rng);
            format!(
                "INSERT {{ ?s {} <<( ?s ?p ?o )>> }} WHERE {{ ?s ?p ?o \
                 FILTER(!isBlank(?s) && !isBlank(?o)) }}",
                p
            )
        }
        // The DELETE WHERE shorthand (its own grammar production).
        _ => {
            let p = gen_predicate(rng);
            format!("DELETE WHERE {{ ?s {} ?o }}", p)
        }
    }
}

/// A graph-management operation. Always `SILENT`: the spec leaves the error-vs-no-op
/// choice for absent/existing targets implementation-defined (a store MAY succeed),
/// so a non-SILENT error-status difference between engines would not be a semantic
/// divergence — SILENT removes that ambiguity and keeps "every op must succeed on
/// every engine, and the states must match" as the whole oracle.
fn gen_structural(rng: &mut Rng) -> String {
    // DEFAULT or a named graph, for COPY/MOVE/ADD endpoints.
    fn graph_or_default(rng: &mut Rng) -> String {
        if rng.chance(1, 4) {
            "DEFAULT".to_string()
        } else {
            format!("GRAPH {}", gen_graph_iri(rng))
        }
    }
    match rng.below(8) {
        0 | 1 => {
            let target = match rng.below(5) {
                0 => "DEFAULT".to_string(),
                1 => "NAMED".to_string(),
                2 => "ALL".to_string(),
                _ => format!("GRAPH {}", gen_graph_iri(rng)),
            };
            format!("CLEAR SILENT {}", target)
        }
        2 | 3 => {
            let target = match rng.below(5) {
                0 => "DEFAULT".to_string(),
                1 => "NAMED".to_string(),
                2 => "ALL".to_string(),
                _ => format!("GRAPH {}", gen_graph_iri(rng)),
            };
            format!("DROP SILENT {}", target)
        }
        4 => format!("CREATE SILENT GRAPH {}", gen_graph_iri(rng)),
        // COPY/MOVE/ADD, including the same-source-and-destination corner (a spec
        // no-op) — both engines desugar via spargebra, so only evaluation-layer
        // handling can diverge here.
        _ => {
            let verb = ["COPY", "MOVE", "ADD"][rng.below(3) as usize];
            format!(
                "{} SILENT {} TO {}",
                verb,
                graph_or_default(rng),
                graph_or_default(rng)
            )
        }
    }
}

// ── one generated request ────────────────────────────────────────────────────────

/// A local document a `LOAD` operation reads. `name` is the file name under the
/// per-run load base (never an absolute path — see the module docs on why the
/// generated IRI is relative).
#[derive(Clone, Debug, PartialEq, Eq)]
struct LoadDoc {
    name: String,
    content: String,
}

/// One generated update request.
///
/// `sparq` is what both sparq paths execute. `oxi` is what the REFERENCE engine
/// executes: identical for every operation except `LOAD`, where it is the
/// semantically-equivalent `INSERT DATA` (Oxigraph 0.5 cannot fetch a `file://`
/// source at all — see the module docs), or `None` for a `LOAD SILENT` of an absent
/// document, whose equivalent is "apply nothing".
#[derive(Clone, Debug, PartialEq, Eq)]
struct Op {
    sparq: String,
    oxi: Option<String>,
    doc: Option<LoadDoc>,
}

impl Op {
    /// An operation both engines run verbatim.
    fn shared(text: String) -> Op {
        Op {
            oxi: Some(text.clone()),
            sparq: text,
            doc: None,
        }
    }
}

/// A `LOAD` operation plus the document it reads.
///
/// 1-in-4 loads an ABSENT document under `SILENT` (sparq's silent-failure path); that
/// name is never written, so no earlier document can accidentally satisfy it.
fn gen_load(rng: &mut Rng, st: &mut GenState) -> Op {
    let destination = if rng.chance(1, 2) {
        Some(gen_graph_iri(rng))
    } else {
        None
    };
    let into = match &destination {
        Some(g) => format!(" INTO GRAPH {}", g),
        None => String::new(),
    };
    if rng.chance(1, 4) {
        return Op {
            sparq: format!("LOAD SILENT <file://absent.nt>{}", into),
            oxi: None,
            doc: None,
        };
    }
    let name = format!("doc{}.nt", rng.below(3));
    let n = 1 + rng.below(3);
    let mut content = String::new();
    let mut block = String::new();
    for _ in 0..n {
        // N-Triples: ground, blank-node-free, canonical — so the INSERT DATA the
        // reference engine gets is exactly equivalent, and the triples are poolable.
        let triple = format!(
            "{} {} {} .",
            gen_subject(rng),
            gen_predicate(rng),
            gen_ntriples_object(rng)
        );
        content.push_str(&triple);
        content.push('\n');
        push_quad(&mut block, &destination, &triple);
        st.inserted.push((destination.clone(), triple));
    }
    let silent = if rng.chance(1, 2) { " SILENT" } else { "" };
    Op {
        sparq: format!("LOAD{} <file://{}>{}", silent, name, into),
        oxi: Some(format!("INSERT DATA {{{} }}", block)),
        doc: Some(LoadDoc { name, content }),
    }
}

/// One update request. Mostly a single operation; 1-in-10 a two-operation compound
/// (`op ; op`) to exercise within-one-request sequencing.
///
/// A `LOAD` is never compounded: its reference-engine substitute is a whole separate
/// request, and splicing two substitutes into one `;` chain would silently change the
/// sequencing the compound is there to test.
fn gen_op(rng: &mut Rng, st: &mut GenState) -> Op {
    fn single(ns: &str, rng: &mut Rng, st: &mut GenState) -> String {
        match rng.below(100) {
            0..=34 => format!("INSERT DATA {{{} }}", gen_insert_block(ns, rng, st)),
            35..=49 => format!("DELETE DATA {{{} }}", gen_delete_block(rng, st)),
            50..=74 => gen_where_op(ns, rng, st),
            _ => gen_structural(rng),
        }
    }
    if rng.chance(1, 12) {
        return gen_load(rng, st);
    }
    // Disjoint blank-node label namespaces for the two halves of a compound request
    // (see `gen_bnode`).
    let first = single("b", rng, st);
    if rng.chance(1, 10) {
        Op::shared(format!("{} ;\n{}", first, single("c", rng, st)))
    } else {
        Op::shared(first)
    }
}

/// The whole update sequence for one seed: 4..=10 update requests, applied (and
/// differentially checked) one at a time.
fn gen_sequence(rng: &mut Rng) -> Vec<Op> {
    let n = 4 + rng.below(7) as usize;
    let mut st = GenState::default();
    (0..n).map(|_| gen_op(rng, &mut st)).collect()
}

// ── dataset snapshots ────────────────────────────────────────────────────────────

/// Renders one quad as an N-Quads line. Both engines hand out `oxrdf` terms, so this
/// is byte-comparable across them.
fn nquads_line(subject: &str, predicate: &str, object: &str, graph: Option<&str>) -> String {
    match graph {
        Some(g) => format!("{} {} {} {} .", subject, predicate, object, g),
        None => format!("{} {} {} .", subject, predicate, object),
    }
}

/// A sparq `Graph` (default graph + named graphs) as SORTED N-Quads lines.
/// Duplicate lines are NOT collapsed — see the raw-count check in [`compare`].
fn sparq_nquads(g: &Graph) -> Vec<String> {
    fn triples_of(g: &Graph, graph: Option<&str>, out: &mut Vec<String>) {
        let scan = g.store.scan(&[None, None, None]);
        for r in scan.rows.iter() {
            let t = scan.to_spo(r);
            out.push(nquads_line(
                &g.dict.term(t[0]).to_string(),
                &g.dict.term(t[1]).to_string(),
                &g.dict.term(t[2]).to_string(),
                graph,
            ));
        }
    }
    let mut out = Vec::new();
    triples_of(g, None, &mut out);
    for (name, sub) in &g.named {
        triples_of(sub, Some(&name.to_string()), &mut out);
    }
    out.sort();
    out
}

/// The Oxigraph store as SORTED N-Quads lines, rendered identically.
fn oxi_nquads(store: &Store) -> Result<Vec<String>, String> {
    use oxigraph::model::GraphName;
    let mut out = Vec::new();
    for q in store.iter() {
        let q = q.map_err(|e| format!("oxigraph iter error: {}", e))?;
        let graph = match &q.graph_name {
            GraphName::DefaultGraph => None,
            g => Some(g.to_string()),
        };
        out.push(nquads_line(
            &q.subject.to_string(),
            &q.predicate.to_string(),
            &q.object.to_string(),
            graph.as_deref(),
        ));
    }
    out.sort();
    Ok(out)
}

// ── canonical comparison ─────────────────────────────────────────────────────────

/// Whether a snapshot mentions a blank node, i.e. whether sorted N-Quads has stopped
/// being a canonical form and RDFC-1.0 relabelling is required.
///
/// Deliberately a substring test. It cannot produce a FALSE NEGATIVE — every blank
/// node renders as `_:label` — and a false positive (a literal containing the text
/// `_:`) would only route a blank-node-free snapshot through the canonicalizer, which
/// on such input is just a sort-and-deduplicate. The generator emits no such literal
/// today; the check stays conservative so that if one is ever added, the comparator
/// degrades in the safe direction.
fn mentions_blank_node(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("_:"))
}

/// Re-parses N-Quads lines back into `oxrdf` quads (the input side of every
/// structural rewrite below).
fn parse_lines(lines: &[String], what: &str) -> Result<Vec<Quad>, String> {
    let doc = {
        let mut s = lines.join("\n");
        s.push('\n');
        s
    };
    sparq_canon::parse_nquads(&doc)
        .map_err(|e| format!("{}: N-Quads re-parse failed ({})", what, e))
}

/// The comparable form of one snapshot.
///
/// With `relabel` false (neither side mentions a blank node) the sorted lines ARE the
/// canonical form and are returned untouched — the strongest, byte-level compare.
/// With it true, both sides are relabelled to their RDFC-1.0 canonical form so the
/// comparison decides RDF ISOMORPHISM. The constrained `*_ground_terms` profile is
/// used deliberately: it is exactly RDFC-1.0 with triple terms as opaque constants and
/// fails closed on a blank node nested inside a triple term, so the non-standard
/// nested-bnode descent is never reachable from this harness.
fn comparable(lines: &[String], relabel: bool, what: &str) -> Result<Vec<String>, String> {
    if !relabel {
        return Ok(lines.to_vec());
    }
    let quads = parse_lines(lines, what)?;
    let canon = sparq_canon::canonicalize_rdf12_ground_terms(&quads)
        .map_err(|e| format!("{}: RDFC-1.0 canonicalization failed ({})", what, e))?;
    let mut out: Vec<String> = canon.lines().map(str::to_string).collect();
    out.sort();
    Ok(out)
}

/// Rewrites `t` the way Oxigraph's storage layer does: an `xsd:integer` literal whose
/// value fits the native integer it decodes to is re-rendered in canonical lexical
/// form. Structural, never string surgery — and it descends into triple terms, which
/// Oxigraph's recursive term encoder also normalizes.
fn oxigraph_normalized_term(t: &Term) -> Term {
    match t {
        Term::Literal(l) if l.datatype() == xsd::INTEGER => match l.value().parse::<i64>() {
            Ok(v) if v.to_string() != l.value() => {
                Term::Literal(Literal::new_typed_literal(v.to_string(), xsd::INTEGER))
            }
            _ => t.clone(),
        },
        Term::Triple(inner) => Term::Triple(Box::new(Triple::new(
            inner.subject.clone(),
            inner.predicate.clone(),
            oxigraph_normalized_term(&inner.object),
        ))),
        _ => t.clone(),
    }
}

// [GPT-6 ASTRA] Quad uniqueness alone cannot detect colliding terms at different
// predicates/graphs. Check every integer leaf, including nested triple objects.
fn record_lexical_terms(
    term: &Term,
    integers: &mut BTreeMap<i64, String>,
    blank_nodes: &mut BTreeSet<String>,
) -> Result<(), String> {
    match term {
        Term::Literal(l) if l.datatype() == xsd::INTEGER => {
            if let Ok(value) = l.value().parse::<i64>() {
                match integers.insert(value, l.value().to_string()) {
                    Some(previous) if previous != l.value() => {
                        return Err(format!(
                            "integer normalization is not term-injective: {previous:?} and {:?}",
                            l.value()
                        ));
                    }
                    _ => {}
                }
            }
        }
        Term::BlankNode(b) => {
            blank_nodes.insert(b.as_str().to_string());
        }
        Term::Triple(t) => {
            if let oxrdf::NamedOrBlankNode::BlankNode(b) = &t.subject {
                blank_nodes.insert(b.as_str().to_string());
            }
            record_lexical_terms(&t.object, integers, blank_nodes)?;
        }
        _ => {}
    }
    Ok(())
}

struct NormalizedSnapshot {
    lines: Vec<String>,
    blank_nodes: usize,
}

/// Normalizes integer spellings without merging terms or rows.
fn oxigraph_normalized(lines: &[String], what: &str) -> Result<NormalizedSnapshot, String> {
    let quads = parse_lines(lines, what)?;
    if quads.len() != lines.len() {
        return Err(format!("{what}: parsing changed the raw row count"));
    }
    let mut integers = BTreeMap::new();
    let mut blank_nodes = BTreeSet::new();
    for q in &quads {
        record_lexical_terms(&q.object, &mut integers, &mut blank_nodes)?;
        if let oxrdf::NamedOrBlankNode::BlankNode(b) = &q.subject {
            blank_nodes.insert(b.as_str().to_string());
        }
        if let oxrdf::GraphName::BlankNode(b) = &q.graph_name {
            blank_nodes.insert(b.as_str().to_string());
        }
    }
    let mut out: Vec<String> = quads
        .iter()
        .map(|q| {
            let graph = match &q.graph_name {
                oxrdf::GraphName::DefaultGraph => None,
                g => Some(g.to_string()),
            };
            nquads_line(
                &q.subject.to_string(),
                &q.predicate.to_string(),
                &oxigraph_normalized_term(&q.object).to_string(),
                graph.as_deref(),
            )
        })
        .collect();
    out.sort();
    // The fixed probes project whole quads, so duplicates are not legitimate
    // projection multiplicity. Reject before canon's set conversion can hide them.
    if out.windows(2).any(|rows| rows[0] == rows[1]) {
        return Err(format!(
            "{what}: integer normalization merges or duplicates rows"
        ));
    }
    Ok(NormalizedSnapshot {
        lines: out,
        blank_nodes: blank_nodes.len(),
    })
}

/// The lines on exactly one side — the human-readable core of a divergence report.
fn one_sided(label_a: &str, a: &[String], label_b: &str, b: &[String]) -> String {
    let bset: std::collections::BTreeSet<&String> = b.iter().collect();
    let aset: std::collections::BTreeSet<&String> = a.iter().collect();
    let mut s = String::new();
    s.push_str(&format!("only in {}:\n", label_a));
    for l in a.iter().filter(|l| !bset.contains(l)) {
        s.push_str(&format!("  {}\n", l));
    }
    s.push_str(&format!("only in {}:\n", label_b));
    for l in b.iter().filter(|l| !aset.contains(l)) {
        s.push_str(&format!("  {}\n", l));
    }
    s
}

/// The outcome of comparing two snapshots.
enum Verdict {
    /// Byte-identical, or RDF-isomorphic when blank nodes are in play.
    Same,
    /// Absorbed by the adjudicated `update-oxigraph-integer-lexical-canonicalization`
    /// class: the two datasets agree exactly once Oxigraph's numeric normalization is
    /// re-derived on both sides.
    AdjudicatedIntegerLexical,
    /// A real divergence (the string is the report body).
    Differs(String),
}

// [GPT-6 Astra] Borrow raw lines so duplicate detection also works on unsorted
// diagnostic inputs. O(N log N) comparisons and O(N) borrowed references.
fn repeated_raw_line(lines: &[String]) -> Option<(&str, usize)> {
    let mut seen = BTreeSet::new();
    let repeated = lines.iter().find(|line| !seen.insert(line.as_str()))?;
    let count = lines.iter().filter(|line| *line == repeated).count();
    Some((repeated.as_str(), count))
}

/// Compares two snapshots under the canonical form the module docs describe.
///
/// [GPT-6 Astra] Inputs are complete-quad snapshots or the full-quad rows from
/// `PROBES`. Repeated raw quads are invalid emissions, even on both sides; this
/// is not a uniqueness policy for arbitrary SPARQL projection/join/UNION bags.
/// In particular, a duplicate-bearing snapshot does not compare equal to itself.
///
/// `allow_integer_lexical` is set ONLY for a sparq-vs-Oxigraph compare, and only when
/// the allowlist enables the class. The sparq-vs-sparq compare passes false: both
/// sides are sparq, so any lexical disagreement between them is a real bug.
fn compare(
    label_a: &str,
    a: &[String],
    label_b: &str,
    b: &[String],
    allow_integer_lexical: bool,
) -> Verdict {
    let relabel = mentions_blank_node(a) || mentions_blank_node(b);
    let (ca, cb) = match (
        comparable(a, relabel, label_a),
        comparable(b, relabel, label_b),
    ) {
        (Ok(ca), Ok(cb)) => (ca, cb),
        (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),
    };
    if ca == cb {
        // Canonicalization deduplicates, so a duplicate quad on one side alone would
        // survive the compare — check the raw counts to keep that failure visible.
        if a.len() != b.len() {
            return Verdict::Differs(format!(
                "datasets are isomorphic but the raw quad COUNTS differ \
                 ({} {} vs {} {}) — one side is yielding a duplicate quad",
                label_a,
                a.len(),
                label_b,
                b.len()
            ));
        }
        for (label, lines) in [(label_a, a), (label_b, b)] {
            if let Some((line, count)) = repeated_raw_line(lines) {
                return Verdict::Differs(format!(
                    "{label}: repeated raw full-quad line occurs {count} times \
                     (raw totals: {label_a} {}, {label_b} {}):\n  {line}",
                    a.len(),
                    b.len()
                ));
            }
        }
        return Verdict::Same;
    }
    if allow_integer_lexical {
        if a.len() != b.len() {
            return Verdict::Differs(format!(
                "integer-lexical adjudication refused: raw row counts differ\n{}",
                one_sided(label_a, &ca, label_b, &cb)
            ));
        }
        let normalized = (
            oxigraph_normalized(a, label_a),
            oxigraph_normalized(b, label_b),
        );
        match normalized {
            (Ok(na), Ok(nb)) => {
                if na.blank_nodes != nb.blank_nodes {
                    return Verdict::Differs(format!(
                        "integer-lexical adjudication refused: blank-node counts differ ({} vs {})\n{}",
                        na.blank_nodes,
                        nb.blank_nodes,
                        one_sided(label_a, &ca, label_b, &cb)
                    ));
                }
                match (
                    comparable(&na.lines, relabel, label_a),
                    comparable(&nb.lines, relabel, label_b),
                ) {
                    (Ok(na), Ok(nb)) if na == nb => return Verdict::AdjudicatedIntegerLexical,
                    (Err(e), _) | (_, Err(e)) => {
                        return Verdict::Differs(format!(
                            "integer-lexical adjudication refused: {e}\n{}",
                            one_sided(label_a, &ca, label_b, &cb)
                        ));
                    }
                    _ => {}
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                return Verdict::Differs(format!(
                    "integer-lexical adjudication refused: {e}\n{}",
                    one_sided(label_a, &ca, label_b, &cb)
                ));
            }
        }
    }
    Verdict::Differs(one_sided(label_a, &ca, label_b, &cb))
}

// ── probe SELECTs (full binding-set compare, per the oracle-strength rule) ────────
//
// The projection order is N-QUADS ORDER (`?s ?p ?o [?g]`) on purpose: a row rendered
// by joining its cells is then a well-formed N-Quads line, so probe results go through
// exactly the same canonical comparison as the dataset snapshots — which is what makes
// them comparable at all once blank nodes are in play.
// [GPT-6 Astra] Each probe projects every component of one triple/quad, including
// the named graph. Adding projection loss, joins or UNION requires reassessing
// this private comparator's unique-emission contract; ordinary result bags need
// not be unique. The scope and projection tests below pin these two probes.

const PROBES: &[(&str, &[&str])] = &[
    ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"]),
    (
        "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
        &["s", "p", "o", "g"],
    ),
];

/// Renders one probe row's cells as an N-Quads line.
fn probe_line(cells: &[String]) -> String {
    format!("{} .", cells.join(" "))
}

/// The full sorted binding set of a probe through sparq's query path.
fn sparq_probe(g: &Graph, q: &str) -> Result<Vec<String>, String> {
    let r = sparq_engine::query(g, q).map_err(|e| format!("sparq probe error: {}", e))?;
    let mut rows: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            probe_line(
                &row.iter()
                    .map(|t| {
                        t.as_ref()
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "UNDEF".to_string())
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    rows.sort();
    Ok(rows)
}

/// The full sorted binding set of a probe through Oxigraph's query path.
// clippy: the differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn oxi_probe(store: &Store, q: &str, vars: &[&str]) -> Result<Vec<String>, String> {
    match store.query(q).map_err(|e| e.to_string())? {
        oxigraph::sparql::QueryResults::Solutions(s) => {
            let mut rows = Vec::new();
            for sol in s {
                let sol = sol.map_err(|e| e.to_string())?;
                rows.push(probe_line(
                    &vars
                        .iter()
                        .map(|v| {
                            sol.get(*v)
                                .map(|t| t.to_string())
                                .unwrap_or_else(|| "UNDEF".to_string())
                        })
                        .collect::<Vec<_>>(),
                ));
            }
            rows.sort();
            Ok(rows)
        }
        _ => Err("probe did not return solutions".to_string()),
    }
}

// ── the LOAD document sandbox ────────────────────────────────────────────────────

/// A temporary directory holding the documents a seed's `LOAD` operations read, and
/// which `sparq_engine::with_load_base` allowlists for the duration of that seed.
///
/// Uniquified by process id AND a run-local counter so concurrently-running seeds
/// (cargo test runs these in parallel threads) never share a document. The path never
/// appears in a generated request — the generated IRI is relative — so the seed repro
/// is unaffected by it.
struct LoadSandbox(PathBuf);

impl LoadSandbox {
    fn new() -> Result<LoadSandbox, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sparq-update-fuzz-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("LOAD sandbox {}: {}", dir.display(), e))?;
        Ok(LoadSandbox(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, doc: &LoadDoc) -> Result<(), String> {
        std::fs::write(self.0.join(&doc.name), &doc.content)
            .map_err(|e| format!("LOAD document {}: {}", doc.name, e))
    }
}

impl Drop for LoadSandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ── the per-seed differential ────────────────────────────────────────────────────

/// What one seed's run produced.
#[derive(Debug)]
struct SeedOutcome {
    /// Update requests applied.
    ops: u64,
    /// Steps absorbed by the adjudicated integer-lexical class.
    adjudicated_integer_lexical: u64,
    /// Steps whose dataset compare needed RDFC-1.0 relabelling — i.e. steps where a
    /// blank node was actually in play. Reported so the nightly log SHOWS the v2
    /// isomorphism path being exercised instead of leaving it to be assumed; a run
    /// where this is 0 has silently lost blank-node coverage.
    isomorphism_compares: u64,
}

/// Applies one seed's update sequence to all three implementations, checking the
/// canonical dataset + both probes after every step. Returns the run's counts, or the
/// divergence detail (step-localized).
///
/// `inject_divergence_at`: test-only comparator non-vacuity knob — after applying
/// step `i` to all three engines, a marker quad is inserted into the OXIGRAPH store
/// only, so the comparator MUST report a divergence at that step (see
/// `tests::injected_divergence_is_caught`). `None` in production.
fn check_seed(
    seed: u64,
    inject_divergence_at: Option<usize>,
    allow: &UpdateDivergenceAllowlist,
) -> Result<SeedOutcome, String> {
    let mut rng = Rng::new(seed);
    let ops = gen_sequence(&mut rng);
    let sandbox = if ops.iter().any(|o| o.sparq.starts_with("LOAD")) {
        Some(LoadSandbox::new()?)
    } else {
        None
    };
    match sandbox.as_ref() {
        // The allowlisted base is installed for the whole seed: `with_load_base` is a
        // thread-local guard, and a seed's steps all run on this thread.
        Some(s) => sparq_engine::with_load_base(s.path(), || {
            apply_sequence(seed, &ops, Some(s), inject_divergence_at, allow)
        }),
        None => apply_sequence(seed, &ops, None, inject_divergence_at, allow),
    }
}

fn apply_sequence(
    seed: u64,
    ops: &[Op],
    sandbox: Option<&LoadSandbox>,
    inject_divergence_at: Option<usize>,
    allow: &UpdateDivergenceAllowlist,
) -> Result<SeedOutcome, String> {
    let mut g_rebuild = Graph::new();
    let mut g_inplace = Graph::new();
    let store = Store::new().map_err(|e| format!("oxigraph store init: {}", e))?;
    let mut adjudicated_integer_lexical = 0u64;
    let mut isomorphism_compares = 0u64;

    let fail = |step: usize, op: &Op, detail: String| -> String {
        format!(
            "step={} of {}\nop: {}\n{}\nrepro: cargo run -p sparq-bench --release -- \
             update-fuzz --seed-start {} --seed-count 1\n--- full sequence ---\n{}",
            step,
            ops.len(),
            op.sparq,
            detail,
            seed,
            ops.iter()
                .enumerate()
                .map(|(i, o)| match &o.oxi {
                    Some(x) if *x != o.sparq =>
                        format!("[{}] {}\n     (reference engine ran: {})", i, o.sparq, x),
                    Some(_) => format!("[{}] {}", i, o.sparq),
                    None => format!("[{}] {}\n     (reference engine ran: nothing)", i, o.sparq),
                })
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    for (i, op) in ops.iter().enumerate() {
        // A LOAD's document must exist before the request runs.
        if let (Some(doc), Some(s)) = (&op.doc, sandbox) {
            s.write(doc).map_err(|e| fail(i, op, e))?;
        }

        // Apply to all three implementations. Every generated op is inside the
        // supported deterministic subset, so an error from ANY engine is itself a
        // divergence (strict — there is no unsupported-skip in this harness).
        g_rebuild = sparq_engine::update(&g_rebuild, &op.sparq)
            .map_err(|e| fail(i, op, format!("sparq update (rebuild path) error: {}", e)))?;
        sparq_engine::update_in_place(&mut g_inplace, &op.sparq)
            .map_err(|e| fail(i, op, format!("sparq update_in_place error: {}", e)))?;
        if let Some(oxi) = &op.oxi {
            store
                .update(oxi.as_str())
                .map_err(|e| fail(i, op, format!("oxigraph update error: {}", e)))?;
        }

        if inject_divergence_at == Some(i) {
            use oxigraph::model::{GraphName, NamedNode, Quad};
            let n = |s: &str| NamedNode::new(s).expect("valid IRI");
            let marker = Quad::new(
                n("http://ex/injected"),
                n("http://ex/injected"),
                n("http://ex/injected"),
                GraphName::DefaultGraph,
            );
            store
                .insert(&marker)
                .map_err(|e| format!("marker insert failed: {}", e))?;
        }

        // (a) Canonical dataset equality. Localizes a divergence to this exact step.
        // sparq-vs-sparq FIRST and STRICT (no adjudication): the two sparq paths must
        // agree with each other whatever the reference engine does.
        let nq_rebuild = sparq_nquads(&g_rebuild);
        let nq_inplace = sparq_nquads(&g_inplace);
        let nq_oxi = oxi_nquads(&store).map_err(|e| fail(i, op, e))?;
        if mentions_blank_node(&nq_rebuild) || mentions_blank_node(&nq_oxi) {
            isomorphism_compares += 1;
        }
        if let Verdict::Differs(detail) = compare(
            "sparq(rebuild)",
            &nq_rebuild,
            "sparq(in-place)",
            &nq_inplace,
            false,
        ) {
            return Err(fail(
                i,
                op,
                format!(
                    "canonical dataset differs BETWEEN SPARQ'S OWN UPDATE PATHS\n{}",
                    detail
                ),
            ));
        }
        for (label, nq) in [
            ("sparq(rebuild)", &nq_rebuild),
            ("sparq(in-place)", &nq_inplace),
        ] {
            match compare(label, nq, "oxigraph", &nq_oxi, allow.integer_lexical) {
                Verdict::Same => {}
                Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
                Verdict::Differs(detail) => {
                    return Err(fail(
                        i,
                        op,
                        format!(
                            "canonical dataset differs ({} vs oxigraph)\n{}",
                            label, detail
                        ),
                    ));
                }
            }
        }

        // (b) Probe SELECTs — the query-path view of the updated store, full sorted
        // binding sets (never counts). Checked for BOTH sparq graphs: the in-place
        // one reads through the live delta overlay.
        for (probe, vars) in PROBES {
            let oxi = oxi_probe(&store, probe, vars)
                .map_err(|e| fail(i, op, format!("oxigraph probe error: {}", e)))?;
            for (label, g) in [("rebuild", &g_rebuild), ("in-place", &g_inplace)] {
                let sparq = sparq_probe(g, probe).map_err(|e| fail(i, op, e))?;
                match compare("sparq", &sparq, "oxigraph", &oxi, allow.integer_lexical) {
                    Verdict::Same => {}
                    Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
                    Verdict::Differs(detail) => {
                        return Err(fail(
                            i,
                            op,
                            format!(
                                "probe {:?} binding set differs (sparq {} vs oxigraph)\n{}",
                                probe, label, detail
                            ),
                        ));
                    }
                }
            }
        }
    }
    Ok(SeedOutcome {
        ops: ops.len() as u64,
        adjudicated_integer_lexical,
        isomorphism_compares,
    })
}

// ── adjudicated-divergence allowlist (update classes) ─────────────────────────────

/// The adjudicated `update-*` divergence classes this comparator has detectors for.
/// The registry is the same `bench/differential-divergences.json` the query fuzzer
/// consumes. A listed `update-*` class WITHOUT a detector here stays strict (loud
/// warning), mirroring the fail-toward-flagging posture of `fuzz.rs`.
struct UpdateDivergenceAllowlist {
    /// `update-oxigraph-integer-lexical-canonicalization` — Oxigraph's storage layer
    /// collapses `xsd:integer` lexical forms. Not a blind skip: absorbed only when
    /// re-deriving that normalization on both sides makes the datasets agree exactly.
    integer_lexical: bool,
    /// Where the allowlist was loaded from, and its posture (for the summary line).
    state: String,
}

/// The class id this comparator has a detector for.
const INTEGER_LEXICAL_CLASS: &str = "update-oxigraph-integer-lexical-canonicalization";

impl UpdateDivergenceAllowlist {
    fn strict(path: &str, why: &str) -> Self {
        UpdateDivergenceAllowlist {
            integer_lexical: false,
            state: format!("({}): {} — STRICT (every divergence fails)", path, why),
        }
    }

    /// Load from `SPARQ_FUZZ_DIVERGENCES` (a CI/agent override), else the committed
    /// repo default resolved relative to this crate's manifest (works from any cwd).
    fn load() -> Self {
        let path = std::env::var("SPARQ_FUZZ_DIVERGENCES").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../bench/differential-divergences.json"
            )
            .to_string()
        });
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_json(&s, &path),
            Err(e) => Self::strict(&path, &format!("unreadable ({})", e)),
        }
    }

    /// Parse the allowlist JSON. An unknown `update-*` class id is IGNORED with a loud
    /// warning (fail-STRICT: the comparator has no detector for it, so that class keeps
    /// failing rather than being silently "absorbed" by nothing); malformed JSON is
    /// also strict.
    fn from_json(s: &str, path: &str) -> Self {
        let v: serde_json::Value = match serde_json::from_str(s) {
            Ok(v) => v,
            Err(e) => return Self::strict(path, &format!("invalid JSON ({})", e)),
        };
        let ids: Vec<String> = v["classes"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|c| c["id"].as_str())
            .filter(|id| id.starts_with("update-"))
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return Self::strict(path, "no adjudicated `update-*` classes");
        }
        let mut out = UpdateDivergenceAllowlist {
            integer_lexical: false,
            state: String::new(),
        };
        let mut enabled = Vec::new();
        for id in &ids {
            if id == INTEGER_LEXICAL_CLASS {
                out.integer_lexical = true;
                enabled.push(id.clone());
            } else {
                eprintln!(
                    "warning: divergence allowlist lists update class {:?} but update-fuzz has \
                     no detector for it — that class stays STRICT",
                    id
                );
            }
        }
        out.state = format!(
            "({}): adjudicated classes enabled {:?}; every other divergence fails",
            path, enabled
        );
        out
    }
}

// ── entry point ──────────────────────────────────────────────────────────────────

pub fn run(seed_start: u64, count: u64) {
    let allow = UpdateDivergenceAllowlist::load();
    println!("update-fuzz divergence allowlist {}", allow.state);
    let mut checked = 0u64;
    let mut ops_applied = 0u64;
    let mut adjudicated_integer_lexical = 0u64;
    let mut isomorphism_compares = 0u64;
    let mut mismatch = 0u64;
    let mut first_repro: Option<String> = None;

    for seed in seed_start..seed_start + count {
        match check_seed(seed, None, &allow) {
            Ok(outcome) => {
                checked += 1;
                ops_applied += outcome.ops;
                adjudicated_integer_lexical += outcome.adjudicated_integer_lexical;
                isomorphism_compares += outcome.isomorphism_compares;
            }
            Err(detail) => {
                mismatch += 1;
                // One machine-greppable line per failing seed (the contract
                // scripts/ci-file-differential-failure.py parses).
                eprintln!("MISMATCH seed={}", seed);
                if first_repro.is_none() {
                    first_repro = Some(format!("seed={}\n{}", seed, detail));
                }
            }
        }
    }

    println!(
        "update-fuzz seeds {}..{} : checked={} update_requests={} \
         isomorphism_compares={} adjudicated(integer-lexical)={} mismatch={}",
        seed_start,
        seed_start + count,
        checked,
        ops_applied,
        isomorphism_compares,
        adjudicated_integer_lexical,
        mismatch
    );
    // NON-VACUITY GUARD: a window that never exercised the v2 isomorphism path has
    // lost blank-node coverage without failing anything — say so loudly.
    if mismatch == 0 && checked > 0 && isomorphism_compares == 0 {
        eprintln!(
            "warning: no step in this window needed RDFC-1.0 relabelling — the \
             blank-node isomorphism path was never exercised"
        );
    }
    if let Some(r) = first_repro {
        println!("\nFIRST FAILING CASE:\n{}", r);
        std::process::exit(1);
    }
}

// ── tests (the per-PR fixed-window smoke + comparator non-vacuity) ────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn allowlist() -> UpdateDivergenceAllowlist {
        UpdateDivergenceAllowlist::load()
    }

    // [GPT-6 Astra] Equal raw totals must not hide canon's duplicate collapse.
    #[test]
    fn raw_duplicate_redistribution_is_rejected() {
        let p = format!("_:x <http://ex/p> \"020\"^^{XSD_INTEGER} .");
        let q = format!("_:x <http://ex/q> \"021\"^^{XSD_INTEGER} .");
        let left = vec![p.clone(), p.clone(), q.clone()];
        let right = vec![p.clone(), q.clone(), q];
        for allow in [false, true] {
            match compare("left", &left, "right", &right, allow) {
                Verdict::Differs(detail) => assert_eq!(
                    detail,
                    format!(
                        "left: repeated raw full-quad line occurs 2 times \
                         (raw totals: left 3, right 3):\n  {p}"
                    )
                ),
                Verdict::Same => panic!("equal-total raw duplicate redistribution returned Same"),
                Verdict::AdjudicatedIntegerLexical => panic!("raw duplicates were adjudicated"),
            }
        }
    }

    #[test]
    fn raw_duplicate_nonadjacent_self_comparison_is_invalid() {
        for subject in ["<http://ex/s>", "_:x"] {
            // The literal also exercises mentions_blank_node's false-positive route.
            for object in ["<http://ex/o>", "\"contains _: text\""] {
                let p = format!("{subject} <http://ex/p> {object} .");
                let q = format!("{subject} <http://ex/q> {object} .");
                let rows = vec![p.clone(), q, p.clone()];
                for allow in [false, true] {
                    match compare("first", &rows, "second", &rows, allow) {
                        Verdict::Differs(detail) => {
                            assert!(detail
                                .starts_with("first: repeated raw full-quad line occurs 2 times"));
                            assert!(detail.ends_with(&p));
                        }
                        _ => panic!("duplicate-bearing snapshot must not compare equal to itself"),
                    }
                }
            }
        }
    }

    #[test]
    fn raw_duplicate_guard_preserves_symmetry_and_graph_identity() {
        let left = vec![
            "_:a <http://ex/p> _:b .".into(),
            "_:b <http://ex/p> _:a .".into(),
        ];
        let right = vec![
            "_:y <http://ex/p> _:z .".into(),
            "_:z <http://ex/p> _:y .".into(),
        ];
        assert!(matches!(
            compare("left", &left, "right", &right, false),
            Verdict::Same
        ));
        let rows = vec![
            "_:s <http://ex/p> <http://ex/o> <http://ex/g1> .".into(),
            "_:s <http://ex/p> <http://ex/o> <http://ex/g2> .".into(),
        ];
        assert!(matches!(
            compare("left", &rows, "right", &rows, false),
            Verdict::Same
        ));
        let duplicate = vec![rows[0].clone(), rows[0].clone()];
        assert!(matches!(
            compare("left", &duplicate, "right", &duplicate, false),
            Verdict::Differs(_)
        ));
    }

    #[test]
    fn raw_duplicate_probe_projection_contract() {
        assert_eq!(
            PROBES,
            &[
                ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"][..]),
                (
                    "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
                    &["s", "p", "o", "g"][..]
                ),
            ]
        );
    }

    #[test]
    fn raw_duplicate_probe_scope_is_complete_quads_in_both_engines() {
        let triple = "<http://ex/s> <http://ex/p> <http://ex/o>";
        let update = format!(
            "INSERT DATA {{ {triple} . GRAPH <http://ex/g1> {{ {triple} }} \
             GRAPH <http://ex/g2> {{ {triple} }} }}"
        );
        let graph = sparq_engine::update(&Graph::new(), &update).unwrap();
        let store = Store::new().unwrap();
        store.update(update.as_str()).unwrap();
        let snapshots = (sparq_nquads(&graph), oxi_nquads(&store).unwrap());
        assert_eq!(snapshots.0.len(), 3);
        assert_eq!(snapshots.0, snapshots.1);
        let expected = [
            vec![format!("{triple} .")],
            vec![
                format!("{triple} <http://ex/g1> ."),
                format!("{triple} <http://ex/g2> ."),
            ],
        ];
        for ((query, vars), expected) in PROBES.iter().zip(expected) {
            let sparq = sparq_probe(&graph, query).unwrap();
            let oxi = oxi_probe(&store, query, vars).unwrap();
            assert_eq!(sparq, expected, "Sparq scope for {query}");
            assert_eq!(oxi, expected, "Oxigraph scope for {query}");
            assert!(matches!(
                compare("sparq", &sparq, "oxigraph", &oxi, false),
                Verdict::Same
            ));
        }
    }

    #[test]
    fn raw_duplicate_guard_preserves_canonicalization_errors() {
        for line in [
            "_:x not-an-iri <http://ex/o> .",
            "_:x <http://ex/p> <<( _:nested <http://ex/q> <http://ex/o> )>> .",
        ] {
            let rows = vec![line.to_string(), line.to_string()];
            match compare("left", &rows, "right", &rows, false) {
                Verdict::Differs(detail) => {
                    assert!(
                        detail.contains("re-parse failed")
                            || detail.contains("canonicalization failed")
                    );
                    assert!(!detail.contains("repeated raw full-quad"));
                }
                _ => panic!("existing parse/ground-profile error must stay strict"),
            }
        }
    }

    /// The per-PR BLOCKING smoke: a fixed seed window through the full three-way
    /// differential (both sparq update paths vs Oxigraph, canonical dataset + probe
    /// binding sets per step). The randomized soak (advancing window) is the nightly
    /// `differential-update.yml` lane, not this test — keep this window small enough
    /// to stay fast in debug builds.
    #[test]
    fn fixed_window_smoke() {
        let allow = allowlist();
        for seed in 0..40 {
            if let Err(detail) = check_seed(seed, None, &allow) {
                panic!("seed={} diverged:\n{}", seed, detail);
            }
        }
    }

    /// Comparator NON-VACUITY: a quad present in only one store MUST be reported as
    /// a divergence at exactly the injected step. If this test ever passes with the
    /// comparison logic hollowed out, the whole harness is asserting nothing.
    #[test]
    fn injected_divergence_is_caught() {
        let allow = allowlist();
        let err = check_seed(0, Some(0), &allow).expect_err("an injected extra quad must diverge");
        assert!(
            err.contains("canonical dataset differs"),
            "divergence must be reported by the canonical-dataset compare, got:\n{}",
            err
        );
        assert!(
            err.contains("step=0"),
            "divergence must be localized to the injected step, got:\n{}",
            err
        );
        assert!(
            err.contains("http://ex/injected"),
            "the report must show the offending quad, got:\n{}",
            err
        );
    }

    /// The generator is a pure function of the seed (the seed IS the corpus — the
    /// repro contract depends on this), INCLUDING the LOAD documents.
    #[test]
    fn generator_is_deterministic() {
        let a = gen_sequence(&mut Rng::new(12345));
        let b = gen_sequence(&mut Rng::new(12345));
        assert_eq!(a, b);
        assert!(a.len() >= 4 && a.len() <= 10, "sequence length in 4..=10");
    }

    /// Every v2 term/operation family actually reaches the generated corpus. Without
    /// this, a refactor that silently stopped emitting (say) triple terms would leave
    /// the whole extension untested and every seed still green.
    #[test]
    fn generator_emits_every_v2_family() {
        let mut noncanonical = 0;
        let mut bnodes = 0;
        let mut triple_terms = 0;
        let mut reifiers = 0;
        let mut loads = 0;
        let mut load_docs = 0;
        let mut bnode_templates = 0;
        for seed in 0..400 {
            for op in gen_sequence(&mut Rng::new(seed)) {
                if op.sparq.contains(XSD_INTEGER) {
                    noncanonical += 1;
                }
                if op.sparq.contains("_:b") || op.sparq.contains("_:c") {
                    bnodes += 1;
                }
                if op.sparq.contains("<<(") {
                    triple_terms += 1;
                }
                if op.sparq.contains("<< <") {
                    reifiers += 1;
                }
                if op.sparq.starts_with("LOAD") {
                    loads += 1;
                }
                if op.doc.is_some() {
                    load_docs += 1;
                }
                if op.sparq.contains("_:bt") || op.sparq.contains("_:ct") {
                    bnode_templates += 1;
                }
            }
        }
        for (what, n) in [
            ("non-canonical numeric lexicals", noncanonical),
            ("blank nodes in data blocks", bnodes),
            ("triple terms", triple_terms),
            ("reifier syntax", reifiers),
            ("LOAD operations", loads),
            ("LOAD documents", load_docs),
            ("blank-node INSERT templates", bnode_templates),
        ] {
            assert!(n > 0, "{} never generated over seeds 0..400", what);
        }
    }

    /// The generator's invariant that licenses the CONSTRAINED RDFC-1.0 profile: no
    /// blank node may end up nested inside a triple term.
    ///
    /// Both halves matter. The guard is REAL — the constrained profile rejects a nested
    /// blank node, so it is not a no-op that would pass on anything — and the generator
    /// never trips it across a seed window that demonstrably produces both blank nodes
    /// and triple terms.
    #[test]
    fn no_nested_blank_nodes_in_triple_terms() {
        // (a) the guard actually fires on the shape it exists to reject.
        let nested = vec!["<http://ex/s> <http://ex/p> \
                          <<( <http://ex/a> <http://ex/b> _:x )>> ."
            .to_string()];
        let err = comparable(&nested, true, "nested")
            .expect_err("a blank node inside a triple term must be rejected, not canonicalized");
        assert!(
            err.contains("RDFC-1.0 canonicalization failed"),
            "expected the constrained profile's fail-closed error, got: {}",
            err
        );
        // ... and the same lines minus the nesting canonicalize fine, so the rejection
        // is specific to the nesting rather than to triple terms as such.
        let ground = vec![
            "_:x <http://ex/p> <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> .".to_string(),
        ];
        comparable(&ground, true, "ground").expect("a GROUND triple term must canonicalize");

        // (b) no generated sequence ever reaches the rejected shape. Seeds that produce
        // both families are counted so this cannot pass on an empty corpus.
        let allow = allowlist();
        let mut seeds_with_both = 0;
        for seed in 0..200 {
            let ops = gen_sequence(&mut Rng::new(seed));
            if ops.iter().any(|o| o.sparq.contains("_:"))
                && ops.iter().any(|o| o.sparq.contains("<<("))
            {
                seeds_with_both += 1;
            }
            check_seed(seed, None, &allow).unwrap_or_else(|detail| {
                panic!(
                    "seed={} failed; if this mentions a nested blank node the generator \
                     has broken the constrained-RDFC-1.0 precondition:\n{}",
                    seed, detail
                )
            });
        }
        assert!(
            seeds_with_both > 20,
            "only {} of 200 seeds mixed blank nodes with triple terms — the invariant \
             is not being exercised",
            seeds_with_both
        );
    }

    /// The two snapshot renderers agree on a concrete mixed default/named-graph
    /// dataset built through BOTH engines' own update paths — the byte-level
    /// foundation the canonical compare rests on (IRIs, plain + typed + tagged
    /// literals, triple terms, named-graph rendering).
    #[test]
    fn snapshot_renderers_agree_on_known_dataset() {
        let op = "INSERT DATA { <http://ex/s0> <http://ex/p0> \"lit0\" . \
                  <http://ex/s1> <http://ex/p1> 7 . \
                  <http://ex/s3> <http://ex/p3> <<( <http://ex/a> <http://ex/b> \"z\" )>> . \
                  GRAPH <http://ex/g0> { <http://ex/s2> <http://ex/p2> \"tag0\"@en . } }";
        let g = sparq_engine::update(&Graph::new(), op).expect("sparq update");
        let store = Store::new().expect("oxigraph store");
        store.update(op).expect("oxigraph update");
        let a = sparq_nquads(&g);
        let b = oxi_nquads(&store).expect("oxigraph iter");
        assert_eq!(a, b);
        assert_eq!(a.len(), 4);
        assert!(
            a.iter().any(|l| l.ends_with("<http://ex/g0> .")),
            "named-graph quad must carry its graph IRI: {:?}",
            a
        );
        assert!(
            a.iter()
                .any(|l| l.contains("\"7\"^^<http://www.w3.org/2001/XMLSchema#integer>")),
            "bare integer must render as a typed literal on both sides: {:?}",
            a
        );
        assert!(
            a.iter().any(|l| l.contains("<<(")),
            "an RDF-1.2 triple term must render identically on both sides: {:?}",
            a
        );
    }

    /// sq-hodke (1): a non-canonical `xsd:integer` lexical survives a store round-trip
    /// as a term DISTINCT from its canonical sibling, through BOTH sparq update paths
    /// and through the query path — and an exact-term `DELETE DATA` of one removes ONLY
    /// it. This is the sparq-INTERNAL oracle for the family, and it is the right one:
    /// Oxigraph collapses the two terms, so it cannot serve as a reference here.
    #[test]
    fn non_canonical_integer_lexicals_are_distinct_terms() {
        let insert = "INSERT DATA { \
                      <http://ex/s> <http://ex/p> \"05\"^^<http://www.w3.org/2001/XMLSchema#integer> . \
                      <http://ex/s> <http://ex/p> \"+7\"^^<http://www.w3.org/2001/XMLSchema#integer> . \
                      <http://ex/s> <http://ex/p> 5 . <http://ex/s> <http://ex/p> 7 }";
        let expected = [
            "<http://ex/s> <http://ex/p> \"+7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "<http://ex/s> <http://ex/p> \"05\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "<http://ex/s> <http://ex/p> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "<http://ex/s> <http://ex/p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
        ];
        let g = sparq_engine::update(&Graph::new(), insert).expect("rebuild insert");
        assert_eq!(
            sparq_nquads(&g),
            expected,
            "rebuild path lost a lexical form"
        );
        let mut gi = Graph::new();
        sparq_engine::update_in_place(&mut gi, insert).expect("in-place insert");
        assert_eq!(
            sparq_nquads(&gi),
            expected,
            "delta-overlay path lost a lexical form"
        );
        assert_eq!(
            sparq_probe(&g, PROBES[0].0).expect("probe"),
            expected,
            "the query path must also see four distinct terms"
        );

        // Deleting the non-canonical term must NOT take its canonical sibling with it.
        let deleted = sparq_engine::update(
            &g,
            "DELETE DATA { <http://ex/s> <http://ex/p> \
             \"05\"^^<http://www.w3.org/2001/XMLSchema#integer> }",
        )
        .expect("delete");
        let after = sparq_nquads(&deleted);
        assert!(
            !after.iter().any(|l| l.contains("\"05\"")),
            "the non-canonical term must be gone: {:?}",
            after
        );
        assert!(
            after
                .iter()
                .any(|l| l.contains("\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>")),
            "its canonical sibling is a different RDF term and must survive: {:?}",
            after
        );
        assert_eq!(after.len(), 3, "exactly one term removed: {:?}", after);
    }

    /// sq-hodke (2): the isomorphism-aware compare is REAL, in both directions. Two
    /// datasets that differ only in blank-node labels must compare Same; one extra
    /// edge on a blank node must still be caught. Without the second half, the
    /// canonicalization would be a divergence-swallowing rubber stamp.
    #[test]
    fn blank_node_compare_is_isomorphism_not_relabelling_blindness() {
        let a = vec![
            "_:fb0 <http://ex/p> <http://ex/o> .".to_string(),
            "_:fb0 <http://ex/q> _:fb1 <http://ex/g> .".to_string(),
        ];
        let relabelled = vec![
            "_:zzz <http://ex/p> <http://ex/o> .".to_string(),
            "_:zzz <http://ex/q> _:aaa <http://ex/g> .".to_string(),
        ];
        assert!(
            matches!(compare("a", &a, "b", &relabelled, false), Verdict::Same),
            "a pure blank-node relabelling is the SAME dataset"
        );
        let mut extra = relabelled.clone();
        extra.push("_:zzz <http://ex/r> <http://ex/o2> .".to_string());
        extra.sort();
        assert!(
            matches!(compare("a", &a, "b", &extra, false), Verdict::Differs(_)),
            "an extra edge on a blank node is NOT an isomorphism and must fail"
        );
        // A different blank-node *shape* with the same edge count must also fail.
        let reshaped = vec![
            "_:p <http://ex/p> <http://ex/o> .".to_string(),
            "_:q <http://ex/q> _:r <http://ex/g> .".to_string(),
        ];
        assert!(
            matches!(compare("a", &a, "b", &reshaped, false), Verdict::Differs(_)),
            "splitting one blank node into two is NOT an isomorphism and must fail"
        );
    }

    /// A duplicate quad on one side alone survives canonicalization (RDFC-1.0
    /// deduplicates), so the raw-count guard must catch it. Pins the check that keeps
    /// v1's duplicate sensitivity alive under the new comparator.
    #[test]
    fn duplicate_quad_is_caught_despite_canonicalization() {
        let a = vec![
            "_:x <http://ex/p> <http://ex/o> .".to_string(),
            "_:x <http://ex/p> <http://ex/o> .".to_string(),
        ];
        let b = vec!["_:y <http://ex/p> <http://ex/o> .".to_string()];
        match compare("a", &a, "b", &b, false) {
            Verdict::Differs(d) => assert!(
                d.contains("COUNTS differ"),
                "expected the raw-count guard to fire, got:\n{}",
                d
            ),
            _ => panic!("a duplicated quad on one side must not compare equal"),
        }
    }

    /// sq-hodke (1), comparator side: the adjudicated integer-lexical class absorbs
    /// EXACTLY Oxigraph's normalization and nothing more.
    #[test]
    fn integer_lexical_adjudication_is_narrow() {
        let ncl = "<http://ex/s> <http://ex/p> \
                   \"05\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
            .to_string();
        let canonical = "<http://ex/s> <http://ex/p> \
                         \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
            .to_string();
        let collapsed = vec![ncl.clone(), canonical.clone()];
        let oxi = vec![canonical.clone()];
        assert!(
            matches!(
                compare("sparq", &collapsed, "oxigraph", &oxi, true),
                Verdict::Differs(_)
            ),
            "a reference-state collapse is no longer adjudicated"
        );
        let sparq = vec![ncl.clone()];
        assert!(
            matches!(
                compare("sparq", &sparq, "oxigraph", &oxi, true),
                Verdict::AdjudicatedIntegerLexical
            ),
            "a one-to-one lexical rewrite must still exercise the adjudicated class"
        );
        assert!(
            matches!(
                compare("sparq", &sparq, "oxigraph", &oxi, false),
                Verdict::Differs(_)
            ),
            "with the class DISABLED the same case must fail — including for every \
             sparq-vs-sparq compare, which never passes the flag"
        );
        // An unrelated missing quad alongside the lexical difference must NOT be
        // absorbed: the class only fires when normalization makes the sets agree.
        let mut sparq_plus = sparq.clone();
        sparq_plus.push("<http://ex/s> <http://ex/q> <http://ex/o> .".to_string());
        sparq_plus.sort();
        assert!(
            matches!(
                compare("sparq", &sparq_plus, "oxigraph", &oxi, true),
                Verdict::Differs(_)
            ),
            "a real missing quad must survive the adjudication"
        );
        // A lexical difference the reference engine does NOT make is not this class.
        let lang = vec!["<http://ex/s> <http://ex/p> \"a\"@en .".to_string()];
        let lang2 = vec!["<http://ex/s> <http://ex/p> \"a\"@fr .".to_string()];
        assert!(
            matches!(
                compare("sparq", &lang, "oxigraph", &lang2, true),
                Verdict::Differs(_)
            ),
            "the class must not absorb a language-tag difference"
        );
    }

    /// Pins the committed registry ⟷ this comparator's detectors. A new `update-*`
    /// class added to the JSON without a detector here must NOT silently start
    /// absorbing divergences.
    #[test]
    fn committed_allowlist_enables_exactly_the_adjudicated_classes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../bench/differential-divergences.json"
        );
        let s = std::fs::read_to_string(path).expect("committed divergence registry");
        let a = UpdateDivergenceAllowlist::from_json(&s, path);
        assert!(
            a.integer_lexical,
            "{} must be adjudicated in the committed registry",
            INTEGER_LEXICAL_CLASS
        );
        assert!(
            a.state.contains(INTEGER_LEXICAL_CLASS),
            "the summary line must name the enabled class: {}",
            a.state
        );
        // …and the file lists no `update-*` class this comparator lacks a detector for
        // (an undetectable entry would be a claimed-but-unenforced allowlisting).
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        for c in v["classes"].as_array().expect("classes array") {
            let id = c["id"].as_str().expect("string id");
            if !id.starts_with("update-") {
                continue; // a query-fuzzer class; fuzz.rs pins those.
            }
            assert_eq!(
                id, INTEGER_LEXICAL_CLASS,
                "update class {:?} in the committed file has no detector in update_fuzz.rs",
                id
            );
            assert!(
                c["bead"].as_str().is_some_and(|b| b.starts_with("sq-")),
                "class {:?} must cite its adjudication bead",
                id
            );
        }
        // An unknown update class stays strict (no detector -> no absorption).
        let unknown = r#"{"classes":[{"id":"update-not-a-real-class"}]}"#;
        let b = UpdateDivergenceAllowlist::from_json(unknown, path);
        assert!(!b.integer_lexical);
        // Malformed / absent registries are strict too.
        assert!(!UpdateDivergenceAllowlist::from_json("{", path).integer_lexical);
        assert!(UpdateDivergenceAllowlist::from_json("{", path)
            .state
            .contains("STRICT"));
    }

    /// sq-hodke (3), the premise correction, MACHINE-CHECKED: Oxigraph 0.5 as this
    /// harness builds it cannot fetch a local `file://` document — which is why the
    /// reference engine gets an equivalent `INSERT DATA` instead of the same LOAD.
    /// If a future Oxigraph (or feature change) makes this succeed, this test fails
    /// and the substitution should be revisited.
    #[test]
    fn oxigraph_cannot_load_a_local_file() {
        let sandbox = LoadSandbox::new().expect("sandbox");
        let doc = LoadDoc {
            name: "doc0.nt".to_string(),
            content: "<http://ex/s0> <http://ex/p0> <http://ex/o0> .\n".to_string(),
        };
        sandbox.write(&doc).expect("write");
        let absolute = format!("LOAD <file://{}>", sandbox.path().join(&doc.name).display());
        let store = Store::new().expect("oxigraph store");
        let err = store
            .update(absolute.as_str())
            .expect_err("Oxigraph must not be able to fetch a file:// document");
        assert!(
            err.to_string().contains("HTTP client is not available"),
            "expected Oxigraph's HTTP-only LOAD refusal, got: {}",
            err
        );

        // sparq, in contrast, loads it — through BOTH update paths, and only under an
        // allowlisted base.
        let relative = format!("LOAD <file://{}>", doc.name);
        let refused = match sparq_engine::update(&Graph::new(), &relative) {
            Err(e) => e,
            Ok(_) => panic!("file:// LOAD must be refused with no allowlisted base"),
        };
        assert!(
            refused.to_string().contains("refused"),
            "expected sparq's no-base refusal, got: {}",
            refused
        );
        sparq_engine::with_load_base(sandbox.path(), || {
            let rebuilt = sparq_engine::update(&Graph::new(), &relative).expect("rebuild LOAD");
            let mut gi = Graph::new();
            sparq_engine::update_in_place(&mut gi, &relative).expect("in-place LOAD");
            let expected = vec!["<http://ex/s0> <http://ex/p0> <http://ex/o0> .".to_string()];
            assert_eq!(sparq_nquads(&rebuilt), expected);
            assert_eq!(sparq_nquads(&gi), expected);
        });
    }

    /// sq-hodke (3), the substitution's soundness: sparq's `LOAD … INTO GRAPH` reaches
    /// exactly the dataset Oxigraph reaches from the equivalent `INSERT DATA` — the
    /// oracle the fuzzer's LOAD steps rely on.
    #[test]
    fn load_equals_the_equivalent_insert_data() {
        let sandbox = LoadSandbox::new().expect("sandbox");
        let doc = LoadDoc {
            name: "doc0.nt".to_string(),
            content: "<http://ex/s0> <http://ex/p0> <http://ex/o0> .\n\
                      <http://ex/s1> <http://ex/p1> \"lit1\" .\n"
                .to_string(),
        };
        sandbox.write(&doc).expect("write");
        let loaded = sparq_engine::with_load_base(sandbox.path(), || {
            sparq_engine::update(
                &Graph::new(),
                &format!("LOAD <file://{}> INTO GRAPH <http://ex/g0>", doc.name),
            )
            .expect("LOAD")
        });
        let store = Store::new().expect("oxigraph store");
        store
            .update(
                "INSERT DATA { GRAPH <http://ex/g0> { \
                 <http://ex/s0> <http://ex/p0> <http://ex/o0> . \
                 <http://ex/s1> <http://ex/p1> \"lit1\" . } }",
            )
            .expect("equivalent INSERT DATA");
        assert_eq!(
            sparq_nquads(&loaded),
            oxi_nquads(&store).expect("oxigraph iter")
        );
    }

    // [GPT-6 ASTRA] These checks use actual generated operations and public updates;
    // a persistent map also catches collisions across different graphs or steps.
    #[test]
    fn generated_integer_domain_is_injective_including_load() {
        // [GPT-6 ASTRA] Independent test boundaries: do not derive the expected
        // domain from the production constant or rely on a sampled collision.
        fn assert_integer_pool(term: &Term) {
            match term {
                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
                    let value = l
                        .value()
                        .parse::<u64>()
                        .expect("nonnegative generated integer");
                    if value < 20 {
                        assert_eq!(l.value(), value.to_string(), "canonical pool spelling");
                    } else {
                        assert!((20..80).contains(&value), "noncanonical pool: {value}");
                        assert_ne!(l.value(), value.to_string(), "noncanonical pool spelling");
                    }
                }
                Term::Triple(t) => assert_integer_pool(&t.object),
                _ => {}
            }
        }
        assert_eq!(CANONICAL_INTEGER_VALUES, 20);
        let mut noncanonical = 0;
        let mut loads = 0;
        let mut load_integers = 0;
        let mut nested = 0;
        for seed in 0..400 {
            let ops = gen_sequence(&mut Rng::new(seed));
            let sandbox = LoadSandbox::new().expect("sandbox");
            sparq_engine::with_load_base(sandbox.path(), || {
                let mut graph = Graph::new();
                let mut integers = BTreeMap::new();
                let mut bnodes = BTreeSet::new();
                for op in &ops {
                    if let Some(doc) = &op.doc {
                        sandbox.write(doc).expect("LOAD document");
                        let rows: Vec<_> = doc.content.lines().map(str::to_string).collect();
                        for q in parse_lines(&rows, "LOAD document").expect("valid document") {
                            assert_integer_pool(&q.object);
                            match &q.object {
                                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
                                    let value = l.value().parse::<u64>().unwrap();
                                    assert!(
                                        value < 20,
                                        "LOAD integer must stay canonical: {value}"
                                    );
                                    assert_eq!(value.to_string(), l.value());
                                    load_integers += 1;
                                }
                                _ => {}
                            }
                            record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                                .expect("LOAD shares the injective integer domain");
                        }
                        let mirror = sparq_engine::update(&Graph::new(), op.oxi.as_ref().unwrap())
                            .expect("reference INSERT mirror");
                        let loaded =
                            sparq_engine::update(&Graph::new(), &op.sparq).expect("isolated LOAD");
                        assert_eq!(sparq_nquads(&mirror), sparq_nquads(&loaded));
                        loads += 1;
                    }
                    graph = sparq_engine::update(&graph, &op.sparq)
                        .unwrap_or_else(|e| panic!("seed={seed} update={} failed: {e}", op.sparq));
                    for q in parse_lines(&sparq_nquads(&graph), "generated state").unwrap() {
                        assert_integer_pool(&q.object);
                        nested += usize::from(matches!(&q.object, Term::Triple(_)));
                        record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                            .unwrap_or_else(|e| panic!("seed={seed}: {e}"));
                    }
                }
                noncanonical += integers
                    .iter()
                    .filter(|(v, lexical)| v.to_string() != **lexical)
                    .count();
            });
        }
        assert!(noncanonical > 0 && loads > 0 && load_integers > 0 && nested > 0);
        eprintln!(
            "injective corpus: noncanonical={noncanonical}, LOAD={loads}, triple terms={nested}"
        );
    }

    #[test]
    fn integer_pools_keep_draw_counts_and_all_spelling_families() {
        let mut seen = BTreeMap::new();
        let mut styles = BTreeSet::new();
        for seed in 0..400 {
            let mut rng = Rng::new(seed);
            let mut expected = Rng::new(seed);
            expected.below(20);
            expected.below(3);
            let term = gen_noncanonical_integer(&mut rng);
            assert_eq!(rng.next(), expected.next(), "exactly two RNG draws");
            let rows = vec![format!("<http://ex/s> <http://ex/p> {term} .")];
            let q = parse_lines(&rows, "generated literal")
                .unwrap()
                .pop()
                .unwrap();
            record_lexical_terms(&q.object, &mut seen, &mut BTreeSet::new()).unwrap();
            if let Term::Literal(l) = q.object {
                let v = l.value().parse::<u64>().unwrap();
                assert!((20..80).contains(&v));
                assert_ne!(v.to_string(), l.value());
                styles.insert(if l.value().starts_with('+') {
                    0
                } else if l.value().starts_with("00") {
                    1
                } else {
                    2
                });
            }
        }
        assert_eq!(styles.len(), 3);
        assert_eq!(seen.len(), 60, "all disjoint values reached");
    }

    #[test]
    fn lexical_collisions_across_contexts_and_nested_terms_fail() {
        for second in [
            "<http://ex/s> <http://ex/q> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "<http://ex/s> <http://ex/q> <<( <http://ex/a> <http://ex/p> <<( <http://ex/b> <http://ex/r> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> )>> .",
        ] {
            let a = vec![
                format!("<http://ex/s> <http://ex/p> \"020\"^^{XSD_INTEGER} ."),
                second.to_string(),
            ];
            let b: Vec<_> = a.iter().map(|s| s.replace("\"020\"", "\"20\"")).collect();
            match compare("a", &a, "b", &b, true) {
                Verdict::Differs(e) => assert!(e.contains("not term-injective"), "{e}"),
                _ => panic!("different contexts must not hide a term collision"),
            }
        }
    }

    #[test]
    fn lexical_adjudication_preserves_rows_and_blank_node_structure() {
        // The blank-node case reaches canon's set conversion; without it the
        // ordinary vector comparison alone rejects the differing multiplicities.
        for subject in ["<http://ex/s>", "_:x"] {
            let p = format!("{subject} <http://ex/p> \"020\"^^{XSD_INTEGER} .");
            let q = format!("{subject} <http://ex/q> \"021\"^^{XSD_INTEGER} .");
            let a = vec![p.clone(), p.clone(), q.clone()];
            let b = vec![
                p.replace("020", "20"),
                q.replace("021", "21"),
                q.replace("021", "21"),
            ];
            assert!(
                matches!(compare("a", &a, "b", &b, true), Verdict::Differs(_)),
                "equal totals cannot hide duplicate redistribution for {subject}"
            );
        }
        let a = vec![
            format!("_:a <http://ex/p> \"020\"^^{XSD_INTEGER} ."),
            "_:a <http://ex/q> <http://ex/o> .".into(),
        ];
        let good = vec![
            format!("_:x <http://ex/p> \"20\"^^{XSD_INTEGER} ."),
            "_:x <http://ex/q> <http://ex/o> .".into(),
        ];
        assert!(matches!(
            compare("a", &a, "b", &good, true),
            Verdict::AdjudicatedIntegerLexical
        ));
        let split = vec![good[0].clone(), "_:y <http://ex/q> <http://ex/o> .".into()];
        match compare("a", &a, "b", &split, true) {
            Verdict::Differs(detail) => {
                assert!(
                    detail.contains("blank-node counts differ (1 vs 2)"),
                    "{detail}"
                );
                assert!(detail.contains("only in a:"), "{detail}");
                assert!(detail.contains("only in b:"), "{detail}");
                assert!(
                    detail.contains("\"020\""),
                    "original lexical missing: {detail}"
                );
                assert!(
                    detail.contains("\"20\""),
                    "reference lexical missing: {detail}"
                );
            }
            _ => panic!("different blank-node counts must fail with original dataset details"),
        }
        let extra = vec![good[0].clone(), "_:x <http://ex/r> <http://ex/o> .".into()];
        assert!(
            matches!(compare("a", &a, "b", &extra, true), Verdict::Differs(_)),
            "same node/row counts do not license a changed predicate"
        );
        for (left, right) in [
            (
                "\"20.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                "\"20\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
            ),
            ("\"x\"@en", "\"x\"@fr"),
        ] {
            let left = vec![format!("<http://ex/s> <http://ex/p> {left} .")];
            let right = vec![format!("<http://ex/s> <http://ex/p> {right} .")];
            assert!(matches!(
                compare("a", &left, "b", &right, true),
                Verdict::Differs(_)
            ));
        }
    }

    #[test]
    fn nested_noncanonical_terms_remain_exact_in_sparq() {
        let expected = vec![format!(
            "<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/q> <<( <http://ex/b> <http://ex/r> \"020\"^^{XSD_INTEGER} )>> )>> ."
        )];
        let op = format!("INSERT DATA {{ {} }}", expected[0]);
        let rebuilt = sparq_engine::update(&Graph::new(), &op).unwrap();
        let mut inplace = Graph::new();
        sparq_engine::update_in_place(&mut inplace, &op).unwrap();
        for graph in [&rebuilt, &inplace] {
            assert_eq!(sparq_nquads(graph), expected);
            assert_eq!(sparq_probe(graph, PROBES[0].0).unwrap(), expected);
        }
        let canonical: Vec<_> = expected.iter().map(|s| s.replace("020", "20")).collect();
        assert!(matches!(
            compare("sparq", &expected, "reference", &canonical, true),
            Verdict::AdjudicatedIntegerLexical
        ));
        assert!(matches!(
            compare("sparq", &expected, "other sparq", &canonical, false),
            Verdict::Differs(_)
        ));
    }

    #[test]
    fn injected_marker_fails_during_actual_lexical_adjudication() {
        let ops = vec![
            Op::shared(format!(
                "INSERT DATA {{ <http://ex/s> <http://ex/p> \"020\"^^{XSD_INTEGER} }}"
            )),
            Op::shared("INSERT { ?s <http://ex/q> _:bt } WHERE { ?s <http://ex/p> ?o }".into()),
        ];
        let allow = UpdateDivergenceAllowlist {
            integer_lexical: true,
            state: "hermetic marker-adjudication test".into(),
        };
        let positive = apply_sequence(0, &ops, None, None, &allow).unwrap();
        assert_eq!(positive.ops, 2);
        assert!(positive.adjudicated_integer_lexical > 0);
        let negative = apply_sequence(0, &ops, None, Some(1), &allow).unwrap_err();
        assert!(negative.contains("step=1") && negative.contains("http://ex/injected"));
    }

    // [GPT-6 ASTRA] Literal historical input, not regenerated after narrowing the
    // reference corpus. The reference cannot represent this lexical/cardinality case.
    #[test]
    fn historical_4141222487_preserves_lexicals_and_fresh_nodes() {
        let ops = [
            r#"INSERT DATA { GRAPH <http://ex/g0> { <http://ex/s4> <http://ex/p0> "lit2" . } <http://ex/s2> <http://ex/p3> <http://ex/o5> . <http://ex/s4> <http://ex/p3> 7 . <http://ex/s0> <http://ex/p2> "lit1" . <http://ex/s4> <http://ex/p0> <<( <http://ex/s2> <http://ex/p0> "tag0"@en )>> . }"#,
            r#"INSERT DATA { <http://ex/s5> <http://ex/p2> <<( <http://ex/s4> <http://ex/p2> 9 )>> . <http://ex/s1> <http://ex/p3> 6 . GRAPH <http://ex/g2> { <http://ex/s3> <http://ex/p0> 1 . } GRAPH <http://ex/g1> { <http://ex/s5> <http://ex/p0> "lit1" . } <http://ex/s2> <http://ex/p0> "lit1" . } ;
DELETE DATA { <http://ex/s2> <http://ex/p0> <http://ex/o2> . <http://ex/s4> <http://ex/p1> 18 . <http://ex/s0> <http://ex/p1> 8 . }"#,
            r#"DELETE DATA { <http://ex/s4> <http://ex/p0> <<( <http://ex/s2> <http://ex/p0> "tag0"@en )>> . <http://ex/s4> <http://ex/p3> 7 . <http://ex/s5> <http://ex/p2> <<( <http://ex/s4> <http://ex/p2> 9 )>> . }"#,
            r#"INSERT { ?s <http://ex/p1> <<( ?s ?p ?o )>> } WHERE { ?s ?p ?o FILTER(!isBlank(?s) && !isBlank(?o)) }"#,
            r#"INSERT DATA { <http://ex/s3> <http://ex/p1> "lit1" . <http://ex/s5> <http://ex/p2> <http://ex/o4> . <http://ex/s2> <http://ex/p1> 8 . <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> . }"#,
            r#"LOAD SILENT <file://doc1.nt>"#,
            r#"DELETE DATA { <http://ex/s0> <http://ex/p2> "lit1" . }"#,
            r#"CREATE SILENT GRAPH <http://ex/g0>"#,
            r#"INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }"#,
            r#"ADD SILENT GRAPH <http://ex/g0> TO GRAPH <http://ex/g2>"#,
        ];
        let sandbox = LoadSandbox::new().unwrap();
        sandbox.write(&LoadDoc {
            name: "doc1.nt".into(),
            content: "<http://ex/s5> <http://ex/p1> <http://ex/o3> .\n<http://ex/s0> <http://ex/p0> <http://ex/o2> .\n<http://ex/s4> <http://ex/p1> <http://ex/o5> .\n".into(),
        }).unwrap();
        sparq_engine::with_load_base(sandbox.path(), || {
            let mut rebuilt = Graph::new();
            let mut inplace = Graph::new();
            for (step, op) in ops.iter().enumerate() {
                rebuilt = sparq_engine::update(&rebuilt, op).unwrap();
                sparq_engine::update_in_place(&mut inplace, op).unwrap();
                let a = sparq_nquads(&rebuilt);
                let b = sparq_nquads(&inplace);
                assert!(
                    matches!(compare("rebuild", &a, "inplace", &b, false), Verdict::Same),
                    "internal dataset mismatch at {step}"
                );
                for (probe, _) in PROBES {
                    let a = sparq_probe(&rebuilt, probe).unwrap();
                    let b = sparq_probe(&inplace, probe).unwrap();
                    assert!(
                        matches!(
                            compare("rebuild probe", &a, "inplace probe", &b, false),
                            Verdict::Same
                        ),
                        "internal full-row mismatch at {step}"
                    );
                }
                if step == 8 {
                    for graph in [&rebuilt, &inplace] {
                        let quads = parse_lines(&sparq_nquads(graph), "historical state").unwrap();
                        let fresh: Vec<_> = quads
                            .iter()
                            .filter(|q| {
                                q.predicate.as_str() == "http://ex/p0"
                                    && matches!(&q.object, Term::BlankNode(_))
                            })
                            .collect();
                        assert_eq!(fresh.len(), 9);
                        assert_eq!(
                            fresh
                                .iter()
                                .map(|q| q.object.to_string())
                                .collect::<BTreeSet<_>>()
                                .len(),
                            9
                        );
                        assert_eq!(
                            fresh
                                .iter()
                                .filter(|q| q.subject.to_string() == "<http://ex/s2>")
                                .count(),
                            4
                        );
                        let rows = sparq_engine::query(
                            graph,
                            "SELECT ?s ?o WHERE { ?s <http://ex/p1> ?o }",
                        )
                        .unwrap();
                        assert_eq!(rows.rows.len(), 9);
                        let s2: Vec<_> = quads
                            .iter()
                            .filter(|q| {
                                q.subject.to_string() == "<http://ex/s2>"
                                    && q.predicate.as_str() == "http://ex/p1"
                            })
                            .map(|q| q.object.to_string())
                            .collect();
                        assert!(s2.contains(&format!("\"8\"^^{XSD_INTEGER}")));
                        assert!(s2.contains(&format!("\"008\"^^{XSD_INTEGER}")));
                    }
                    eprintln!(
                        "historical index8: 9 solutions, 9 fresh blank nodes, 4 for s2; both lexical terms retained"
                    );
                }
            }
            let g2: Vec<_> = sparq_nquads(&rebuilt)
                .into_iter()
                .filter(|q| q.ends_with("<http://ex/g2> ."))
                .collect();
            assert_eq!(
                g2,
                vec![
                    format!("<http://ex/s3> <http://ex/p0> \"1\"^^{XSD_INTEGER} <http://ex/g2> ."),
                    "<http://ex/s4> <http://ex/p0> \"lit2\" <http://ex/g2> .".to_string(),
                ],
                "final index9 ADD preserved existing g2 and copied g0"
            );
            eprintln!(
                "historical index9: ADD completed; all 10 original requests checked internally"
            );
        });
    }
}
