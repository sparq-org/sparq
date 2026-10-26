//! (sq-32i5) A cheap, strongly-distinguishing **graph fingerprint** embedded in
//! the `.spqv` / `.spqg` headers so a stale store can never silently return WRONG results.
//!
//! # The foot-gun this closes
//!
//! A [`VectorStore`](crate::store::VectorStore) (`.spqv`) and a
//! [`DiskAnnIndex`](crate::diskann::DiskAnnIndex) (`.spqg`) are build-once-immutable and keyed by
//! **dictionary term id**. They carry no link to the graph they were built against. But a graph
//! mutation/rebuild can SHIFT dict ids (interning order changes), and a query entry point such as
//! [`nearest_term`](crate::diskann::DiskAnnIndex::nearest_term) resolves the query term through the
//! *caller-provided* graph's dictionary (`graph.id_of(term)`) and then looks that id up in the
//! store. If the store was built against a different generation of the graph, the id resolves to a
//! DIFFERENT term's vector — and the search returns plausible-looking but WRONG neighbours, with no
//! error. This module makes that mismatch a hard, descriptive error instead.
//!
//! # What is fingerprinted (the exact inputs — keep this comment in sync with [`Fingerprint::of`])
//!
//! Three fields, all cheap to compute at build time and on open:
//!   1. **`dict_len`** — `graph.dict().len()`, the number of real dictionary terms. O(1).
//!   2. **`triple_count`** — `graph.len()`, the number of triples in the default graph. O(1).
//!   3. **`content_hash`** — a 64-bit hash over **the dictionary term SET, folded in a
//!      dict-id-order-INDEPENDENT order** (sorted), not in id order. For each real term we
//!      compute a per-term 64-bit content hash from its *lexical* content (its [`TermParts`] — IRI
//!      prefix+suffix, literal value+datatype+lang, blank label, or — for a triple term — its three
//!      children resolved RECURSIVELY to their own lexical content rather than to their raw child
//!      ids). The per-term hashes are then **sorted** and folded in that canonical order, so the
//!      result is a function of *which terms are present*, not of *which id each landed on*. The two
//!      scalars are folded in first so a fingerprint that agrees on the hash but differs on
//!      length/count is still rejected. `FxHasher` is used because it is **deterministic with no
//!      seed** (the same property the core dictionary relies on for its own content hashing), so a
//!      fingerprint computed at build time on one machine matches one recomputed at open time on
//!      another. (sq-32i5) Every hashed input is **fixed-width** — lengths/counts as
//!      `u64`, ids as `u32`, tags as `u8`, never `usize` — so the folded byte sequence (and thus
//!      the hash) is identical on 32-bit wasm32 and 64-bit native; the cross-machine guarantee
//!      holds by construction. The cost is O(dict_len) to hash each term plus one
//!      O(dict_len·log dict_len) sort over already-resident term parts — a small fraction of the
//!      embed/build work that produced the store in the first place.
//!
//! # (sq-xhiv) Why the fold is dict-id-order-INDEPENDENT
//!
//! `sparq-core` assigns dictionary ids in a **thread-count-dependent** order: the parallel sharded
//! dict merge sizes its shard count from `rayon::current_num_threads()`, so the *same* logical graph
//! re-loaded from the *same* source RDF at a different thread count gets a different (still internally
//! consistent) id→term binding. An earlier version of this fingerprint folded every term *in ascending
//! id order, binding the id explicitly* — which made it spuriously MISMATCH a store against a graph
//! that was merely re-loaded at a different `RAYON_NUM_THREADS`, even though the graph is logically
//! identical (the failure was **fail-closed** — a descriptive error, never wrong vectors). The whole
//! point of the fingerprint is to catch a graph that genuinely *changed* (a term added/removed/edited
//! ⇒ wrong vectors keyed by the shifted ids), not a thread-count-driven id permutation of an unchanged
//! term set. Folding the term *set* in a sorted (id-order-independent) order detects exactly the
//! former and is invariant to the latter, so the same graph fingerprints identically at any thread
//! count (the regression test `fingerprint_stable_across_thread_counts` pins this), while a real term
//! change still changes the hash. See `research/dict-id-order-determinism-audit.md` (sq-xom2).
//!
//! `content_hash` is a **collision-resistance-irrelevant integrity check**, not a security MAC: it
//! exists to catch accidental staleness, not to defend against an adversary crafting a colliding
//! graph (it makes no integrity, soundness, or tamper-resistance claim against a motivated party).
//! dict_len + triple_count are folded alongside it precisely so the common shift cases
//! (terms added/removed, triples added/removed) are caught structurally even before the hash.
//!
//! # (sq-wlzi) The id-keyed STALENESS CONTRACT — serve via `Graph::open`, never a re-parse
//!
//! A passing [`check_against`] is **necessary but NOT sufficient** for a correct query, and the
//! sq-xhiv thread-count-stability above is exactly why. The store/index are keyed by **raw dict id**;
//! the fingerprint deliberately folds only the term *set*, so it is **blind to a pure id permutation
//! of an unchanged term set**. That blindness is the right call for the fingerprint's job (catch a
//! genuine term change, ignore a thread-count reshuffle), but it means the fingerprint **cannot** tell
//! a graph with the build-time `id → term` binding apart from the *same* graph re-parsed at a different
//! `RAYON_NUM_THREADS` (whose ids permuted). Both fingerprint identically, so `check_against` passes
//! BOTH — yet only the first resolves the id-keyed store correctly; the re-parse serves a **different,
//! real term's vector** under the queried term's id (a plausible-looking but WRONG neighbour, no error).
//!
//! The CONTRACT that closes this gap is a **usage discipline**, not a code check:
//!
//! > To serve a persisted `.spqv` / `.spqg`, **persist the graph it was built against**
//! > (`Graph::save`) and **reopen THAT graph** (`Graph::open`, which mmaps the **frozen** dict id
//! > order) to resolve query terms. **NEVER re-parse the source RDF** (`Graph::load_str` /
//! > `load_reader` / `load_dataset`): the parallel sharded dict merge assigns thread-count-dependent
//! > ids, so a re-parse yields a different `id → term` binding than the store was keyed against. (The
//! > `mmap` feature on `sparq-core` gates `Graph::save`/`Graph::open`.)
//!
//! An id-keyed store/index is valid ONLY against the **exact graph generation it was built against**.
//! The persisted dict is the only thread-count-independent id binding sparq-core exposes, so reopening
//! it is the only sound way to recover that generation. `check_against` remains a worthwhile guard for
//! the cases it *can* catch (a term added/removed/edited ⇒ a changed term set ⇒ a changed fingerprint),
//! but it is a backstop, not the primary safety mechanism — the discipline above is. The round-trip
//! and the re-parse trap are pinned end-to-end in `tests/staleness_contract.rs`. A larger, optional
//! redesign that would make a *logically*-identical graph queryable at any thread count — re-resolving
//! neighbours by **term** rather than raw id (a term-keyed store/index/mask) — is tracked as a separate
//! follow-up (it changes the on-disk format and the `IdMask` mask-cache key; not a doc change).

use sparq_core::dict::{is_inline, Dict, Id, TermParts, INLINE_BASE};
use sparq_core::Graph;
use std::hash::Hasher;

/// Number of bytes a [`Fingerprint`] occupies in a file header: three little-endian `u64`s.
pub const FINGERPRINT_LEN: usize = 24;

/// A graph fingerprint: dictionary length, triple count, and a content hash over the dictionary
/// term set in a dict-id-order-independent (sorted) order. See the module docs for the exact
/// inputs and rationale (incl. the sq-xhiv thread-count-stability property).
///
/// (sq-36ol) Derives `Hash` so a fingerprint can key a per-graph cache (the
/// filtered-ANN `IdMask` cache in `crate::rewrite`): two graphs with the same fingerprint
/// are treated as the same graph for caching, and any mutation that could change a derived
/// mask changes the fingerprint (and thus the key), so a stale mask is never served.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    /// `graph.dict().len()` at build time.
    pub dict_len: u64,
    /// `graph.len()` (default-graph triple count) at build time.
    pub triple_count: u64,
    /// 64-bit content hash over the dictionary term set in a dict-id-order-independent
    /// (sorted) order (see module docs).
    pub content_hash: u64,
}

impl Fingerprint {
    /// Computes the fingerprint of `graph`. Cheap: two O(1) reads, one O(dict_len) pass over the
    /// already-resident term parts to hash each term, and one O(dict_len·log dict_len) sort so the
    /// fold is **dict-id-order-independent** (the same graph fingerprints identically at any thread
    /// count — see the sq-xhiv note in the module docs). The exact inputs are documented at the
    /// module level — keep the two in sync.
    pub fn of(graph: &Graph) -> Fingerprint {
        let dict = graph.dict();
        let dict_len = dict.len() as u64;
        let triple_count = graph.len() as u64;
        // (sq-xhiv) Hash every real term by its LEXICAL content (id-order-independent),
        // collect the per-term hashes, then sort them so the fold does not depend on which id each
        // term landed on. `dict.iter()` walks ids `1..=len()`; the id itself is deliberately NOT
        // bound here (binding it is exactly what made the old fingerprint thread-count-dependent).
        let mut term_hashes: Vec<u64> = Vec::with_capacity(dict.len());
        for (_id, parts) in dict.iter() {
            term_hashes.push(hash_term_lexical(dict, &parts));
        }
        // Canonical (id-order-independent) order. Real dict terms are unique, so this is a set
        // fold; a per-term-hash tie (two distinct terms hashing equal) is astronomically unlikely
        // for a 64-bit hash and, even then, would only weaken discrimination — never a false match
        // that loses dict_len/triple_count, which are folded in first. (Integrity, not security.)
        term_hashes.sort_unstable();

        // FxHasher: deterministic, no seed — so a fingerprint computed at build time matches one
        // recomputed at open time (possibly on another machine). The dict module relies on the
        // same property for its own content hashing.
        let mut h = rustc_hash::FxHasher::default();
        // Fold the scalars first so a hash collision alone cannot mask a length/count change.
        h.write_u64(dict_len);
        h.write_u64(triple_count);
        for th in term_hashes {
            h.write_u64(th);
        }
        Fingerprint {
            dict_len,
            triple_count,
            content_hash: h.finish(),
        }
    }

    /// Serializes to `FINGERPRINT_LEN` little-endian bytes (the on-disk header layout).
    pub fn to_bytes(self) -> [u8; FINGERPRINT_LEN] {
        let mut out = [0u8; FINGERPRINT_LEN];
        out[0..8].copy_from_slice(&self.dict_len.to_le_bytes());
        out[8..16].copy_from_slice(&self.triple_count.to_le_bytes());
        out[16..24].copy_from_slice(&self.content_hash.to_le_bytes());
        out
    }

    /// Parses a fingerprint from `FINGERPRINT_LEN` little-endian bytes. `bytes` must be at least
    /// [`FINGERPRINT_LEN`] long (the caller validated the header length first).
    pub fn from_bytes(bytes: &[u8]) -> Fingerprint {
        Fingerprint {
            dict_len: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            triple_count: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            content_hash: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        }
    }

    /// (sq-32i5) Parses a fingerprint from a current-format header block, returning
    /// `None` for an **all-zero block**. A v2 artifact finalized *without* binding a graph (no
    /// `with_fingerprint`) writes a zeroed block; decoding that to `Some(Fingerprint{0,0,0})` would
    /// make [`check_against`] report a stale-graph *mismatch* (a "DIFFERENT graph" error) instead of
    /// the accurate "carries no fingerprint / unverifiable" message the `store.rs`/`diskann.rs` docs
    /// promise. Treating all-zero as `None` aligns the behaviour with those docs. (An all-zero block
    /// is unreachable from a real [`Fingerprint::of`]: `content_hash` is the FxHash of at least the
    /// two folded scalars, so a genuine empty graph still has a non-zero hash; even in the
    /// astronomically unlikely event a real fingerprint hashed to all-zero, the only consequence is
    /// that it is reported as "unverifiable" rather than checked — fail-safe, never a false match.)
    pub fn from_bytes_opt(bytes: &[u8]) -> Option<Fingerprint> {
        if bytes[0..FINGERPRINT_LEN].iter().all(|&b| b == 0) {
            None
        } else {
            Some(Fingerprint::from_bytes(bytes))
        }
    }

    /// A human-readable one-line summary for error messages.
    fn describe(&self) -> String {
        format!(
            "dict_len={} triples={} hash={:#018x}",
            self.dict_len, self.triple_count, self.content_hash
        )
    }
}

/// (sq-xhiv) The per-term LEXICAL content hash: a 64-bit FxHash over `parts`' content,
/// computed so it does NOT depend on any dict-id assignment. For an IRI / literal / blank the content
/// is already lexical (no id involved). For a **triple term** the child ids ARE thread-count-dependent,
/// so we do not hash them — we resolve each child to its own lexical content and fold THAT recursively
/// (`dict` provides the child's parts; inline-integer children decode directly). The result is stable
/// across thread counts: a pure dict-id permutation of an unchanged term set leaves every per-term hash
/// (and so the sorted fold in [`Fingerprint::of`]) untouched.
fn hash_term_lexical(dict: &Dict, parts: &TermParts<'_>) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    fold_parts_lexical(&mut h, dict, parts, 0);
    h.finish()
}

/// Bound on triple-term nesting depth, mirroring `sparq_core::dict`'s own recursion guard: a
/// well-formed dict cannot nest triple terms beyond this, and capping prevents an
/// (untampered-impossible) cyclic record from looping the fold. Beyond it we fold a fixed sentinel —
/// deterministic, never a panic.
const MAX_TRIPLE_DEPTH: u32 = 64;

/// Folds one term's LEXICAL content into the hasher. The byte/field sequence (and the leading tag)
/// mirror the discipline of `sparq_core::dict`'s own content hashing: a length-prefixed, tagged write
/// per variant, so distinct terms cannot alias by concatenation. We do not depend on the core's hash
/// VALUE (it is private); we only need a self-consistent fold that distinguishes terms here.
fn fold_parts_lexical(h: &mut rustc_hash::FxHasher, dict: &Dict, parts: &TermParts<'_>, depth: u32) {
    // (sq-32i5) All length prefixes are hashed as fixed-width `u64`, NOT `usize`:
    // `usize` is 32-bit on wasm32 and 64-bit on native, so `write_usize` would fold a different
    // byte sequence on each and the on-disk fingerprint would mismatch between a store built on
    // 64-bit native and the same graph opened on 32-bit wasm — silently breaking the documented
    // "build/open fingerprints match across machines" guarantee. `as u64` is lossless on both
    // (lengths fit in 32 bits) and produces an architecture-independent fingerprint by construction.
    match parts {
        TermParts::Iri { prefix, suffix } => {
            h.write_u8(0);
            h.write_u64(prefix.len() as u64);
            h.write(prefix.as_bytes());
            h.write(suffix.as_bytes());
        }
        TermParts::Lit {
            value,
            datatype,
            lang,
        } => {
            h.write_u8(1);
            h.write_u64(value.len() as u64);
            h.write(value.as_bytes());
            h.write_u64(datatype.len() as u64);
            h.write(datatype.as_bytes());
            match lang {
                Some(l) => {
                    h.write_u8(1);
                    h.write(l.as_bytes());
                }
                None => h.write_u8(0),
            }
        }
        TermParts::Blank(label) => {
            h.write_u8(2);
            h.write(label.as_bytes());
        }
        TermParts::Triple(ids) => {
            h.write_u8(3);
            // (sq-xhiv) Recurse into each child's LEXICAL content, not its raw id: the
            // child id is thread-count-dependent (the exact thing this fingerprint must be invariant
            // to), so folding it would re-introduce the determinism bug at one level of nesting.
            for &child in ids {
                fold_child_lexical(h, dict, child, depth);
            }
        }
    }
}

/// Folds a triple-term CHILD id by its lexical content. A child may be (a) an inline-integer id
/// (no dict entry — it decodes to an `xsd:integer` literal whose value is `id - INLINE_BASE`), or
/// (b) a real dict id resolved via [`Dict::term_parts`], itself possibly another (nested) triple term.
fn fold_child_lexical(h: &mut rustc_hash::FxHasher, dict: &Dict, child: Id, depth: u32) {
    if depth >= MAX_TRIPLE_DEPTH {
        // Untampered dicts never nest this deep; fold a fixed sentinel so the result stays
        // deterministic (and bounded) rather than recursing without limit.
        h.write_u8(0xff);
        return;
    }
    if is_inline(child) {
        // Inline integer: lexical form is the decimal value with the `xsd:integer` datatype,
        // exactly as the dict would reconstruct it via `term`. Fold the value (id-independent).
        let value = (child - INLINE_BASE).to_string();
        h.write_u8(1); // literal tag, matching the Lit arm above
        h.write_u64(value.len() as u64);
        h.write(value.as_bytes());
        h.write_u64(XSD_INTEGER.len() as u64);
        h.write(XSD_INTEGER.as_bytes());
        h.write_u8(0); // no language tag
        return;
    }
    let parts = dict.term_parts(child);
    fold_parts_lexical(h, dict, &parts, depth + 1);
}

/// The `xsd:integer` datatype IRI — the datatype an inline-integer id decodes to (see
/// `sparq_core::dict::is_inline` / `Dict::term`). Kept as a constant so the inline-child fold
/// matches the literal fold byte-for-byte.
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// The result of a stored-fingerprint vs. live-graph check, produced by a CHECKED open path.
/// `Ok(())` means the store is safe to query against the graph; `Err` carries a descriptive
/// message naming the mismatch so a stale store can never be queried silently.
pub type CheckResult = Result<(), String>;

/// (sq-32i5) Identifies which kind of artifact a [`check_against`] call is guarding, so
/// the error text is accurate at both call sites (a `.spqv` [`VectorStore`](crate::store::VectorStore)
/// and a `.spqg` [`DiskAnnIndex`](crate::diskann::DiskAnnIndex)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Artifact {
    /// A `.spqv` vector store.
    Store,
    /// A `.spqg` ANN index.
    Index,
}

impl Artifact {
    /// The noun used in error messages ("store" / "index").
    fn noun(self) -> &'static str {
        match self {
            Artifact::Store => "store",
            Artifact::Index => "index",
        }
    }
}

/// Compares a `stored` fingerprint (read from a header) against the live `graph`, returning a
/// descriptive `Err` on any mismatch. `stored` is `None` when the artifact carries no fingerprint —
/// either a pre-fingerprinting (version-1) file, or a current-format file finalized without a graph
/// binding (`with_fingerprint`); in both cases it cannot be certified, so the check fails with a
/// clear "unverified" message rather than silently passing. `artifact` selects the noun used in the
/// message (store vs. index) so it is accurate at both call sites; `origin` names the file.
pub fn check_against(
    stored: Option<Fingerprint>,
    graph: &Graph,
    artifact: Artifact,
    origin: &str,
) -> CheckResult {
    let kind = artifact.noun();
    let Some(stored) = stored else {
        return Err(format!(
            "{origin}: this {kind} carries no graph fingerprint (either a legacy file that predates \
             graph-fingerprinting, or one finalized without binding it to a graph) and so cannot be \
             verified against the graph; rebuild it with a graph binding to enable the staleness check"
        ));
    };
    let live = Fingerprint::of(graph);
    if stored == live {
        Ok(())
    } else {
        Err(format!(
            "{origin}: graph fingerprint mismatch — this {kind} was built against a DIFFERENT graph \
             (stored {}, this graph {}); querying it would silently return wrong results. Rebuild \
             the {kind} against the current graph.",
            stored.describe(),
            live.describe()
        ))
    }
}

#[cfg(test)]
mod tests {
    // (sq-32i5) fingerprint unit tests: discrimination, round-trip, and the
    // legacy (None) path. The store/diskann modules cover the end-to-end checked-open behaviour.
    use super::*;
    use sparq_core::Graph;

    fn graph(ttl: &str) -> Graph {
        Graph::load_str(ttl, "turtle").expect("load test turtle")
    }

    const A: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice ex:knows ex:bob .
        ex:bob ex:knows ex:carol .
    "#;
    // B has a different dictionary (different IRIs) AND a different triple count.
    const B: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:dave ex:likes ex:eve .
    "#;

    #[test]
    fn distinguishes_different_graphs() {
        let fa = Fingerprint::of(&graph(A));
        let fb = Fingerprint::of(&graph(B));
        assert_ne!(fa, fb, "distinct graphs must have distinct fingerprints");
    }

    #[test]
    fn stable_for_the_same_graph() {
        // Recomputed twice (different Graph instances of the same content) must agree, so a
        // build-time fingerprint matches an open-time one.
        assert_eq!(Fingerprint::of(&graph(A)), Fingerprint::of(&graph(A)));
    }

    #[test]
    fn invariant_to_pure_dict_id_permutation() {
        // (sq-xhiv) Same TERM SET, different interning order → a pure dict-id shift.
        // The fingerprint folds the term SET in a sorted (id-order-independent) order, so a pure
        // id permutation that does NOT change which terms are present must leave content_hash
        // UNCHANGED. (The old fingerprint bound the id explicitly and so changed here — that was
        // exactly the thread-count-dependence sq-xhiv removes: a thread-count change is a pure id
        // permutation, and must NOT be reported as a "different graph".)
        let g1 = graph(
            r#"@prefix ex: <http://example.org/> .
               ex:aaa ex:p ex:bbb ."#,
        );
        let g2 = graph(
            r#"@prefix ex: <http://example.org/> .
               ex:bbb ex:p ex:aaa ."#,
        );
        // Same term set and same triple count; only the interning order differs.
        assert_eq!(g1.dict().len(), g2.dict().len());
        assert_eq!(g1.len(), g2.len());
        assert_eq!(
            Fingerprint::of(&g1),
            Fingerprint::of(&g2),
            "a pure dict-id permutation of the same term set must NOT change the fingerprint"
        );
    }

    #[test]
    fn still_detects_a_genuine_term_change() {
        // (sq-xhiv) The fold is id-order-independent but NOT change-blind: adding,
        // removing, or editing a term changes the term set, so content_hash must change. This is
        // the property that keeps the staleness guard useful after the determinism fix.
        let base = graph(
            r#"@prefix ex: <http://example.org/> .
               ex:aaa ex:p ex:bbb ."#,
        );
        // Add a term (a new object) — a genuine graph change.
        let added = graph(
            r#"@prefix ex: <http://example.org/> .
               ex:aaa ex:p ex:bbb .
               ex:aaa ex:p ex:ccc ."#,
        );
        assert_ne!(
            Fingerprint::of(&base),
            Fingerprint::of(&added),
            "adding a term must change the fingerprint"
        );
        // Edit a term (rename an object) — same dict_len/triple_count, different term set.
        let edited = graph(
            r#"@prefix ex: <http://example.org/> .
               ex:aaa ex:p ex:zzz ."#,
        );
        assert_eq!(
            base.dict().len(),
            edited.dict().len(),
            "same number of terms"
        );
        assert_eq!(base.len(), edited.len(), "same triple count");
        assert_ne!(
            Fingerprint::of(&base).content_hash,
            Fingerprint::of(&edited).content_hash,
            "renaming a term (same counts, different term set) must change content_hash"
        );
    }

    #[test]
    fn invariant_to_thread_count_via_simulated_id_permutation() {
        // (sq-xhiv) Direct regression guard for the thread-count-dependence bug.
        // `RAYON_NUM_THREADS` is read once per process for the GLOBAL pool, so we cannot change
        // it mid-test; instead we drive the SAME parallel sharded merge the loader uses at two
        // explicit thread counts via a scoped rayon pool. The two graphs are the same RDF source,
        // so any difference in the fingerprint can ONLY come from the thread-count-dependent
        // dict-id assignment — which the id-order-independent fold must absorb. (This test FAILS on
        // the pre-sq-xhiv id-bound fold and PASSES after.)
        //
        // A multi-namespace N-Triples doc large enough that the merge actually shards (the shard
        // count is `(threads*2).clamp(4,64)`, so 2 vs 8 threads → 4 vs 16 shards → different ids).
        let mut nt = String::new();
        for i in 0..3000u32 {
            nt.push_str(&format!(
                "<http://ns{}.example/s{}> <http://ns{}.example/p{}> <http://ns{}.example/o{}> .\n",
                i % 7,
                i,
                i % 5,
                i % 11,
                i % 9,
                i
            ));
        }
        let load_in_pool = |threads: usize| -> Graph {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| Graph::load_str(&nt, "ntriples").expect("load ntriples"))
        };
        let g2 = load_in_pool(2);
        let g8 = load_in_pool(8);
        // Sanity: the dict-ids genuinely permuted between the two thread counts (otherwise the
        // test would pass trivially and not guard anything). Find an id whose term differs.
        assert_eq!(
            g2.dict().len(),
            g8.dict().len(),
            "same term set, just permuted"
        );
        let permuted =
            (1..=g2.dict().len() as u32).any(|id| g2.dict().term(id) != g8.dict().term(id));
        assert!(
            permuted,
            "expected the parallel sharded merge to assign different dict-ids at 2 vs 8 threads; \
             if this fails the test no longer exercises the bug"
        );
        // The fingerprint must nonetheless be IDENTICAL across the two thread counts.
        assert_eq!(
            Fingerprint::of(&g2),
            Fingerprint::of(&g8),
            "the SAME graph re-loaded at a different thread count must fingerprint identically"
        );
    }

    #[test]
    fn invariant_to_thread_count_with_triple_terms() {
        // (sq-xhiv) The triple-term path is the subtle one: a triple term carries its
        // CHILDREN's dict-ids, which are themselves thread-count-dependent, so the fold must resolve
        // the children to their LEXICAL content recursively rather than fold the child ids. This
        // guards that recursion: an RDF-1.2 reification doc loaded at 2 vs 8 threads — incl. a
        // NESTED triple term and an inline-integer child — must fingerprint identically.
        let mut nt = String::new();
        for i in 0..1500u32 {
            nt.push_str(&format!(
                "<http://ex/r{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> \
                 <<( <http://ns{}.example/s{}> <http://ns{}.example/p{}> \
                 \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n",
                i,
                i % 7,
                i,
                i % 5,
                i % 11,
                i % 13
            ));
        }
        // A nested triple term + an inline-integer object component.
        nt.push_str(
            "<http://ex/rn> <http://ex/nested> <<( <http://a.example/x> <http://b.example/q> \
             <<( <http://c.example/a> <http://d.example/b> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> )>> .\n",
        );
        let load_in_pool = |threads: usize| -> Graph {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| Graph::load_str(&nt, "ntriples").expect("load ntriples"))
        };
        let g2 = load_in_pool(2);
        let g8 = load_in_pool(8);
        assert!(!g2.dict().is_empty() && g2.dict().len() == g8.dict().len());
        assert_eq!(
            Fingerprint::of(&g2),
            Fingerprint::of(&g8),
            "a graph WITH triple terms must also fingerprint identically across thread counts"
        );
    }

    #[test]
    fn bytes_round_trip() {
        let f = Fingerprint::of(&graph(A));
        let back = Fingerprint::from_bytes(&f.to_bytes());
        assert_eq!(f, back);
    }

    #[test]
    fn check_against_matching_is_ok() {
        let f = Fingerprint::of(&graph(A));
        assert!(check_against(Some(f), &graph(A), Artifact::Store, "t").is_ok());
    }

    #[test]
    fn check_against_mismatch_is_descriptive_err() {
        let f = Fingerprint::of(&graph(A));
        let err = check_against(Some(f), &graph(B), Artifact::Store, "t")
            .expect_err("must reject a different graph");
        assert!(err.contains("fingerprint mismatch"), "err was: {err}");
        assert!(
            err.contains("wrong results"),
            "err must warn about wrong results: {err}"
        );
    }

    #[test]
    fn check_against_legacy_none_is_unverifiable_err() {
        let err = check_against(None, &graph(A), Artifact::Store, "t")
            .expect_err("legacy/unbound file must not pass");
        assert!(
            err.contains("carries no graph fingerprint"),
            "err was: {err}"
        );
    }

    #[test]
    fn check_against_error_text_names_the_artifact_kind() {
        // (sq-32i5) The same routine guards BOTH a `.spqv` store and a `.spqg` index;
        // the error noun must follow the artifact kind, not hard-code "store" at an index call site.
        let f = Fingerprint::of(&graph(A));

        // Mismatch path.
        let store_err = check_against(Some(f), &graph(B), Artifact::Store, "t")
            .expect_err("mismatch must error");
        assert!(
            store_err.contains("this store was built"),
            "err: {store_err}"
        );
        let index_err = check_against(Some(f), &graph(B), Artifact::Index, "t")
            .expect_err("mismatch must error");
        assert!(
            index_err.contains("this index was built"),
            "err: {index_err}"
        );
        // The index error must not call the artifact a "store" (the original hard-coded bug).
        // ("stored …" from describe() is fine; we forbid the *noun* phrasings.)
        assert!(
            !index_err.contains(" store") && !index_err.contains("store "),
            "index error must not call the artifact a 'store': {index_err}"
        );

        // Unverifiable (None) path.
        let index_none =
            check_against(None, &graph(A), Artifact::Index, "t").expect_err("None must error");
        assert!(
            index_none.contains("this index carries no"),
            "err: {index_none}"
        );
        assert!(
            !index_none.contains(" store") && !index_none.contains("store "),
            "index error must not call the artifact a 'store': {index_none}"
        );
    }

    #[test]
    fn golden_fingerprint_is_architecture_independent() {
        // (sq-32i5, fold updated sq-xhiv) Regression guard against any width/endianness
        // drift in the fingerprint computation. The fingerprint of graph A is fixed by construction:
        //   * lengths/counts are hashed as fixed-width `u64` (never `usize`), so wasm32 (32-bit
        //     usize) and native (64-bit usize) fold the SAME bytes;
        //   * `FxHasher` is deterministic and seedless;
        //   * tags are hashed at a fixed width (`u8`); the term set is folded in SORTED per-term-hash
        //     order, so the value is also independent of dict-id assignment (thread count).
        // The golden value below was produced by running this test once and pasting the output. If
        // the byte sequence ever becomes architecture-dependent again (e.g. a stray `write_usize`),
        // OR the id-order-independent fold regresses, this assertion fails loudly. dict_len /
        // triple_count are stable properties of graph A; content_hash is the FxHash of the
        // documented sorted-term-set fold.
        let f = Fingerprint::of(&graph(A));
        // Graph A: 4 IRIs (ex:alice, ex:knows, ex:bob, ex:carol), 2 triples.
        assert_eq!(f.dict_len, 4, "dict_len of graph A");
        assert_eq!(f.triple_count, 2, "triple_count of graph A");
        assert_eq!(
            f.content_hash, GOLDEN_A_CONTENT_HASH,
            "content_hash drifted — a width/endianness/fold change broke the cross-machine guarantee"
        );
    }

    /// (sq-32i5, value updated sq-xhiv) Golden content hash of graph `A`,
    /// architecture-independent AND dict-id-order-independent by construction (all hashed inputs are
    /// fixed-width; `FxHasher` is seedless & deterministic; the term set is folded in sorted order).
    /// Produced by running `golden_fingerprint_is_architecture_independent` once.
    const GOLDEN_A_CONTENT_HASH: u64 = 10_048_621_300_020_413_664;
}
