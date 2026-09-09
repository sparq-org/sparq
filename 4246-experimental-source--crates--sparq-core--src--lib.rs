#![doc = include_str!("../README.md")]
// [OPUS-4.8] MS-G2 (sq-8wbn): make `// SAFETY:` mandatory on every first-party unsafe
// block. Mechanically enforces the per-site argument the unsafe-register documents.
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod compress;
pub mod dict;
#[cfg(feature = "dict-spill")]
pub mod dictspill;
// [SONNET-4.6] sq-96hp1 (survey §A2 / research/data-structures.md §B1): the OPT-IN Elias-Fano
// / Partitioned-Elias-Fano column codecs with compressed `next_geq` seek. OFF by default — it is
// a MEASUREMENT-GATED prototype plus its A/B harness, deliberately NOT routed as a `PermData`
// variant and not called by any join, so the shipped store keeps the varint block codec
// bit-exactly and the lean default / wasm build pays nothing. Pure `std`, no new dependency.
#[cfg(feature = "elias-fano")]
pub mod eliasfano;
#[cfg(feature = "mmap")]
pub mod extsort;
// [OPUS-4.8] (sq-7d3dj.18) Prefix-memoized IRI-validation fast path in FRONT of the full
// `oxiri` RFC-3987 automaton. OPT-IN behind `iri-fast` (OFF by default) so the lean default /
// wasm build never names `oxiri` as a direct dep and pays nothing; when on, the parallel
// N-Triples / N-Quads loader validates each IRI through it (fast path, oxiri-equivalent).
#[cfg(feature = "iri-fast")]
pub mod iri;
mod nt;
// [OPUS-4.8] sq-jocpn: the opt-in native byte-level Turtle parser. OFF by default (it names
// `oxiri` as a dep for byte-identical base resolution) so the lean default / wasm build never
// links it and the incumbent oxttl Turtle path stays shipped until this drop-in is A/B-validated.
#[cfg(feature = "native-ttl")]
mod ttl;
// [OPUS-4.8] sq-yj76l (gh #1121): the opt-in `SharedGraph` server-sharing handle. OFF by
// default so the lean default / wasm build never links it (it is `std::sync` only — no new
// dep — but the surface stays out of the default API).
#[cfg(feature = "shared")]
pub mod shared;
pub mod store;
pub mod strdist;
pub mod temporal;

use dict::{Dict, Id};
use temporal::Temporal;

/// Per-chunk parse output: a partial dictionary + that chunk's local-id triples, one
/// element per parallel chunk (the parallelizable half of ingest, merged downstream).
// [OPUS-4.8] Used only by `parse_block` (`#[cfg(feature = "parallel")]`); match that gating
// so neither the default non-parallel build nor the no-default-features (wasm) build flags
// the alias as dead code under the `-D warnings` clippy gate.
#[cfg(feature = "parallel")]
type ChunkPartials = Vec<(Dict, Vec<[Id; 3]>)>;
// [OPUS-4.8] sq-dvyi: JSON-LD parser is OPT-IN behind the `jsonld` feature so the
// default (lean) wasm bundle never links `oxjsonld`.
#[cfg(feature = "jsonld")]
use oxjsonld::JsonLdParser;
// [OPUS-4.8] sq-f47w1 (survey §B1): RDF/XML parser is OPT-IN behind the `rdfxml` feature
// so the default (lean) wasm bundle never links `oxrdfxml` (which pulls `quick-xml`).
#[cfg(feature = "rdfxml")]
use oxrdfxml::RdfXmlParser;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use oxttl::{NQuadsParser, NTriplesParser, TriGParser, TurtleParser};
use store::{Pattern, TripleStore};

/// An immutable, dictionary-encoded RDF graph ready for querying.
pub struct Graph {
    pub dict: Dict,
    pub store: TripleStore,
    /// Parallel to the dictionary: the f64 value of each numeric literal (NaN for
    /// non-numeric terms). Lets the engine evaluate numeric filters / comparisons
    /// / ORDER BY without materialising the term and parsing its string each time
    /// — a lightweight, u32-id-preserving stand-in for QLever's inline ValueIds.
    numerics: NumData,
    /// Parallel to the dictionary: the precomputed comparison key of each
    /// `xsd:dateTime`/`xsd:dateTimeStamp`/`xsd:date` literal (see
    /// [`temporal::Temporal`]). The temporal twin of `numerics`: dateTime
    /// FILTER / ORDER BY / MIN/MAX read the timeline value O(1) from the cache
    /// instead of materialising the term and re-parsing its lexical per row.
    temporals: TempData,
    /// [OPUS-4.8] (sq-lr2ii) Memoised guard against the engine's f64 sargable-FILTER fast
    /// path deciding a comparison wrongly for an f64-INEXACT decimal. `0` = not yet computed,
    /// `1` = known to hold NO such decimal (fast path safe), `2` = holds at least one (the
    /// engine must decline the fast path and use the exact evaluator). Lazily filled by
    /// [`has_high_precision_decimal`](Self::has_high_precision_decimal); reset to `0` by a
    /// delta that appends terms (`apply_delta_mem`) so it is recomputed over the grown
    /// dictionary. Interior-mutable so a shared `&Graph` can populate it; never observable in
    /// results (pure correctness gate).
    high_precision_decimal: std::sync::atomic::AtomicU8,
    /// Named graphs (each a self-contained `Graph`), keyed by their name term. Empty for the
    /// usual single-default-graph load; populated by [`load_dataset`](Self::load_dataset) from
    /// N-Quads / TriG so the engine can evaluate `GRAPH <iri> { … }` / `GRAPH ?g { … }`.
    ///
    /// This Vec is POSITIONAL — the on-disk named sub-tree (`named/<i>/`) and the manifest are
    /// indexed by position, so it must NEVER be reordered. A prefix/range index over the graph
    /// IRIs is therefore a SEPARATE sorted side structure
    /// ([`for_named_graphs_with_prefix`](Self::for_named_graphs_with_prefix)), never a reordering
    /// of this Vec.
    pub named: Vec<(Term, Graph)>,
    /// [OPUS-4.8] (sq-zz8z, gh-51) Lazily-built sorted permutation of `named` indices, ordered by
    /// the UTF-8 bytes of each graph's IRI string, enabling a binary-search RANGE SCAN for a
    /// prefix-scoped `GRAPH ?g … FILTER(STRSTARTS(STR(?g), prefix))` instead of an O(graphs) full
    /// scan (PSS multi-tenant `usage(prefix)`). Cached and keyed by `named.len()`: adding or
    /// removing a named graph changes the length (the only operations that change the SET of graph
    /// IRIs — an in-place `apply_delta` to an EXISTING graph changes neither its name nor the
    /// length), so a stale cache is detected and rebuilt. Interior-mutable so a shared `&Graph`
    /// can populate it; never observable in results (pure acceleration).
    graph_prefix_index: std::sync::Mutex<Option<(usize, Vec<u32>)>>,
    /// The write-ahead log for a DIRECTORY-BACKED graph (opened via [`open`](Self::open)):
    /// every [`apply_delta`](Self::apply_delta) batch is appended + fsync'd here BEFORE it
    /// is applied, and replayed into the delta-overlay on the next `open` — durability for
    /// incremental updates. `None` for in-memory graphs (updates are overlay-only).
    #[cfg(feature = "mmap")]
    wal: Option<Wal>,
    /// [OPUS-4.8] (sq-ycle) The parent-level REDO JOURNAL (`dir/txn.log`) for a DIRECTORY-BACKED
    /// graph — the SINGLE atomic commit point for a whole multi-operation SPARQL UPDATE body. The
    /// resolved per-slot quad-delta of one `apply_effects` call is recorded here as ONE fsync'd
    /// frame BEFORE it is materialised across the per-graph WALs, so a crash mid-body can never
    /// leave a PARTIAL durable write (e.g. parent containment without the child graph). On the next
    /// `open` a committed frame is redone idempotently and the journal truncated. `None` for
    /// in-memory graphs (the per-`apply_effects` commit/clear are then no-ops). See [`TxnJournal`].
    #[cfg(feature = "mmap")]
    txn: Option<TxnJournal>,
}

/// [OPUS-4.8] (sq-5lf) An IMMUTABLE, logically-independent point-in-time view of a
/// [`Graph`], produced cheaply by [`Graph::snapshot`] (O(pending delta), never
/// O(triples)) via the structural fork — the immutable base storage is `Arc`-shared, so
/// a snapshot duplicates neither the permutation indexes nor the dictionary arena.
///
/// A `GraphSnapshot` reads exactly the triples present when it was taken and is
/// unaffected by any later mutation of the source graph; equally, the source can never
/// be reached through the snapshot. It [`Deref`](std::ops::Deref)s to `&Graph` (so every
/// graph READ method — `len`, `id_of`, `pattern`, `iter_ids`, `store`/`dict` access for
/// the query engine — is available and a `&GraphSnapshot` is usable anywhere a `&Graph`
/// is) but deliberately exposes NO `DerefMut` and no mutating method: the snapshot cannot
/// be mutated, so it cannot drift from its point-in-time state. It is [`Send`] + [`Sync`]
/// (a `Graph` is), so it can be published across threads — the production serving pattern
/// (one mutable master + a cheap immutable snapshot per commit / per closed RSP window).
///
/// If you need a snapshot you can then keep mutating, use [`Graph::fork`] (it returns a
/// `Graph`); `snapshot` is the read-only surface layered on top of it.
pub struct GraphSnapshot {
    graph: Graph,
}

impl GraphSnapshot {
    /// Consumes the snapshot, yielding the underlying point-in-time [`Graph`] as an
    /// independently mutable graph (it already carries its own `Arc`-shared base + delta —
    /// this is just dropping the read-only wrapper, O(1)). Use when a consumer that took an
    /// immutable snapshot now wants to apply further updates to that exact state, e.g.
    /// sparq-py's `Graph.copy()` returning a mutable copy.
    #[inline]
    pub fn into_graph(self) -> Graph {
        self.graph
    }

    /// Borrows the underlying [`Graph`] (also reachable via the [`Deref`](std::ops::Deref)
    /// impl; this is the explicit form for call sites that prefer it).
    #[inline]
    pub fn as_graph(&self) -> &Graph {
        &self.graph
    }
}

impl std::ops::Deref for GraphSnapshot {
    type Target = Graph;
    #[inline]
    fn deref(&self) -> &Graph {
        &self.graph
    }
}

// [OPUS-4.8] (sq-yj76l, gh #1121) GUARANTEE — for all feature states — that a `Graph` and its
// read-only `GraphSnapshot` stay `Send` + `Sync`, so they can be shared across the threads of a
// multi-threaded server (axum/actix) directly or via [`shared::SharedGraph`]. This is a
// zero-cost compile-time check: adding a future non-`Send`/non-`Sync` field (e.g. an `Rc` or a
// bare interior-mutable cell) to `Graph` would fail the build HERE rather than silently breaking
// downstream multi-threaded users. The `mmap`/`dict-spill`-gated fields (`File`, `Mmap`,
// `TxnJournal`) are themselves `Send + Sync`, so this holds with every feature on too.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Graph>();
    assert_send_sync::<GraphSnapshot>();
};

/// Backing storage for the numeric-value cache (`numerics[id-1]` = f64 value of term
/// `id`, NaN for non-numeric): owned dense in RAM, mmap'd from disk (out-of-core), or
/// SPARSE — only the numeric terms in a hash map. Most RDF terms are IRIs/strings (NaN),
/// and small integers inline (carrying their own value, never cached), so the dense
/// cache is mostly — often entirely — NaN; the sparse form stores only the few real
/// numeric literals, the right shape for the memory-bound browser store.
enum NumData {
    Owned(Vec<f64>),
    /// The mmap'd dense cache, plus a small side map for terms APPENDED after open
    /// (delta-overlay updates) — the mmap'd file cannot grow, the dictionary can.
    #[cfg(feature = "mmap")]
    Mapped(memmap2::Mmap, rustc_hash::FxHashMap<Id, f64>),
    Sparse(rustc_hash::FxHashMap<Id, f64>),
    /// A FORKED graph's cache: the base graph's cache SHARED immutably (Arc) plus a
    /// small side map for terms interned after the fork — the in-RAM twin of
    /// `Mapped`'s grow-over-immutable-base shape. Lookup: base first, side map as the
    /// fallback (the side map only ever holds ids the base does not cover).
    /// INVARIANT: the inner cache is never itself `Forked` (fork flattens one level).
    Forked { base: std::sync::Arc<NumData>, extra: rustc_hash::FxHashMap<Id, f64> },
}

impl NumData {
    /// The cached numeric value of a 1-based dictionary id, or `None` if it is not a
    /// (cached) numeric literal. The engine's O(1) numeric fast path.
    // [FABLE-5] Keep the dominant dense-cache probe small enough that LTO reliably places it
    // inside numeric gather loops; the larger storage-variant dispatch stays outlined.
    #[inline(always)]
    fn lookup(&self, id: Id) -> Option<f64> {
        if let NumData::Owned(values) = self {
            let value = *values.get((id - 1) as usize)?;
            return (!value.is_nan()).then_some(value);
        }
        self.lookup_non_owned(id)
    }

    #[inline(never)]
    fn lookup_non_owned(&self, id: Id) -> Option<f64> {
        match self {
            NumData::Sparse(m) => m.get(&id).copied(),
            #[cfg(feature = "mmap")]
            NumData::Mapped(_, extra) => match self.as_slice().get((id - 1) as usize) {
                Some(v) if !v.is_nan() => Some(*v),
                Some(_) => None,
                // Beyond the mmap'd dense cache: a term appended after open.
                None => extra.get(&id).copied(),
            },
            NumData::Owned(_) => unreachable!("Owned handled by lookup"),
            NumData::Forked { base, extra } => {
                base.lookup(id).or_else(|| extra.get(&id).copied())
            }
        }
    }

    /// Appends cache entries for freshly interned dictionary ids `old_len+1 ..= dict.len()`
    /// (delta-overlay growth): the dense backing extends in place; the sparse / mmap'd
    /// backings record only the real numeric values in their side map.
    fn extend_for(&mut self, dict: &Dict, old_len: usize) {
        for i in old_len..dict.len() {
            let id = i as Id + 1;
            let v = numeric_of(&dict.term(id));
            match self {
                NumData::Owned(vec) => vec.push(v),
                NumData::Sparse(m) => {
                    if !v.is_nan() {
                        m.insert(id, v);
                    }
                }
                #[cfg(feature = "mmap")]
                NumData::Mapped(_, extra) => {
                    if !v.is_nan() {
                        extra.insert(id, v);
                    }
                }
                NumData::Forked { extra, .. } => {
                    if !v.is_nan() {
                        extra.insert(id, v);
                    }
                }
            }
        }
    }

    /// The cache for a structurally FORKED graph: the base cache shared immutably,
    /// new terms recorded in a per-fork side map. Forking a fork shares the same
    /// base Arc and copies the (small) side map; the FIRST fork of a flat cache
    /// pays a one-time O(n) copy to freeze the shareable base (the mmap'd backing
    /// is materialised — an `Mmap` cannot be cloned; forked graphs are in-memory).
    fn fork(&self) -> NumData {
        match self {
            NumData::Forked { base, extra } => NumData::Forked {
                base: std::sync::Arc::clone(base),
                extra: extra.clone(),
            },
            NumData::Owned(v) => NumData::Forked {
                base: std::sync::Arc::new(NumData::Owned(v.clone())),
                extra: rustc_hash::FxHashMap::default(),
            },
            NumData::Sparse(m) => NumData::Forked {
                base: std::sync::Arc::new(NumData::Sparse(m.clone())),
                extra: rustc_hash::FxHashMap::default(),
            },
            #[cfg(feature = "mmap")]
            NumData::Mapped(_, extra) => NumData::Forked {
                // Materialise the mmap'd dense part (an `Mmap` cannot be cloned);
                // ids the file does not cover stay in the carried side map.
                base: std::sync::Arc::new(NumData::Owned(self.as_slice().to_vec())),
                extra: extra.clone(),
            },
        }
    }

    /// Folds a forked cache's side map into a FRESH shared base (compaction): one
    /// dense/sparse backing covering the whole dictionary, kept in the SHAREABLE
    /// `Forked` shape (Arc base + empty side map) — so the side map stops growing
    /// AND the next fork is still an Arc bump, never an O(dict) re-freeze.
    /// Non-forked backings are returned as-is.
    fn fold(self, dict_len: usize) -> NumData {
        match self {
            NumData::Forked { base, extra } => {
                let folded = match &*base {
                    NumData::Owned(v) => {
                        let mut dense = v.clone();
                        dense.resize(dict_len, f64::NAN);
                        for (&id, &val) in &extra {
                            dense[(id - 1) as usize] = val;
                        }
                        NumData::Owned(dense)
                    }
                    NumData::Sparse(m) => {
                        let mut m = m.clone();
                        m.extend(extra);
                        NumData::Sparse(m)
                    }
                    // `fork` never freezes a Forked or Mapped base (invariant above).
                    _ => unreachable!("a forked numeric cache's base is Owned or Sparse"),
                };
                NumData::Forked { base: std::sync::Arc::new(folded), extra: rustc_hash::FxHashMap::default() }
            }
            other => other,
        }
    }

    /// The dense f64 slice — valid only for the Owned/Mapped backings (the ones `save`
    /// persists); the sparse backing is never written to disk. Only the mmap (save /
    /// mapped-lookup) paths need it.
    #[cfg(feature = "mmap")]
    #[inline]
    fn as_slice(&self) -> &[f64] {
        match self {
            NumData::Owned(v) => v,
            #[cfg(feature = "mmap")]
            NumData::Mapped(m, _) => {
                let n = m.len() / std::mem::size_of::<f64>();
                // SAFETY: numerics.bin is a whole number of f64; the mmap base is
                // page-aligned (>= the 8-byte f64 alignment).
                unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<f64>(), n) }
            }
            NumData::Sparse(_) => unreachable!("as_slice on a sparse numeric cache"),
            NumData::Forked { .. } => unreachable!("as_slice on a forked numeric cache"),
        }
    }

    /// Resident heap bytes (a memory-mapped cache contributes 0 — it is page cache).
    #[inline]
    fn heap_bytes(&self) -> usize {
        match self {
            NumData::Owned(v) => v.capacity() * std::mem::size_of::<f64>(),
            #[cfg(feature = "mmap")]
            NumData::Mapped(_, extra) => extra.capacity() * 17,
            // hashbrown: ~(8-byte key + 8-byte f64 + 1 control byte) per slot.
            NumData::Sparse(m) => m.capacity() * 17,
            // Reachable footprint: the Arc-shared base is counted in full (like the
            // store's Arc-shared permutations), so a compacted graph — whose whole
            // cache lives in the folded base — still reports its memory. Forks that
            // share a base each report it (roborev 1945).
            NumData::Forked { base, extra } => base.heap_bytes() + extra.capacity() * 17,
        }
    }

    /// Converts a dense cache into the sparse form when it is mostly NaN (≥ 3/4 of terms
    /// non-numeric — almost always true), keeping only the real numeric literals. A no-op
    /// (kept dense) when numeric values are common enough that the map would not save.
    fn into_sparse_if_worthwhile(self) -> NumData {
        let dense = match &self {
            NumData::Owned(v) => v,
            _ => return self, // mmap'd/already-sparse: leave as is
        };
        let numeric = dense.iter().filter(|x| !x.is_nan()).count();
        if numeric * 4 > dense.len() {
            return self; // numeric-dense: the Vec is the better representation
        }
        let mut m: rustc_hash::FxHashMap<Id, f64> = rustc_hash::FxHashMap::default();
        m.reserve(numeric);
        for (i, &v) in dense.iter().enumerate() {
            if !v.is_nan() {
                m.insert(i as Id + 1, v);
            }
        }
        NumData::Sparse(m)
    }
}

/// Backing storage for the temporal-value cache, mirroring [`NumData`]: owned dense in
/// RAM (two parallel columns — a flag byte per term and an f64 instant per term), mmap'd
/// from `temporals.bin` (out-of-core), or SPARSE (only the temporal literals, the right
/// shape for the memory-bound browser store — most terms are not dates).
enum TempData {
    /// `cells[id-1]` — flag (see [`temp_flag`]; 0 = not temporal) + instant in ONE
    /// 16-byte cell, so a cache probe touches a single cache line (the probes are
    /// random-access from row order; split columns would double the misses).
    Owned(Vec<TempCell>),
    /// The mmap'd dense cache (`temporals.bin`: `n` little-endian f64 instants then `n`
    /// flag bytes), plus a side map for terms APPENDED after open (delta-overlay growth).
    #[cfg(feature = "mmap")]
    Mapped(memmap2::Mmap, rustc_hash::FxHashMap<Id, Temporal>),
    Sparse(rustc_hash::FxHashMap<Id, Temporal>),
    /// A FORKED graph's cache — see [`NumData::Forked`]: shared immutable base + a
    /// per-fork side map for terms interned after the fork.
    Forked { base: std::sync::Arc<TempData>, extra: rustc_hash::FxHashMap<Id, Temporal> },
}

/// Dense-cache flag byte of a temporal value: kind + timezone presence. 0 = not temporal.
fn temp_flag(t: Temporal) -> u8 {
    match (t.kind, t.has_tz) {
        (temporal::TemporalKind::DateTime, false) => 1,
        (temporal::TemporalKind::DateTime, true) => 2,
        (temporal::TemporalKind::Date, false) => 3,
        (temporal::TemporalKind::Date, true) => 4,
    }
}

/// Decodes a dense-cache cell back into the temporal value (`None` for flag 0).
fn temp_unflag(flag: u8, instant: f64) -> Option<Temporal> {
    let (kind, has_tz) = match flag {
        1 => (temporal::TemporalKind::DateTime, false),
        2 => (temporal::TemporalKind::DateTime, true),
        3 => (temporal::TemporalKind::Date, false),
        4 => (temporal::TemporalKind::Date, true),
        _ => return None,
    };
    Some(Temporal { instant, has_tz, kind })
}

impl TempData {
    /// The cached temporal value of a 1-based dictionary id, or `None` if it is not a
    /// (cached) dateTime/date literal. The engine's O(1) temporal fast path.
    #[inline]
    fn lookup(&self, id: Id) -> Option<Temporal> {
        let i = (id - 1) as usize;
        match self {
            TempData::Owned(cells) => cells.get(i).and_then(|c| temp_unflag(c.flag, c.instant)),
            TempData::Sparse(m) => m.get(&id).copied(),
            TempData::Forked { base, extra } => base.lookup(id).or_else(|| extra.get(&id).copied()),
            #[cfg(feature = "mmap")]
            TempData::Mapped(m, extra) => {
                let n = Self::mapped_len(m);
                if i < n {
                    // SAFETY: the instant section is `n` little-endian f64 at offset 0
                    // (page-aligned mmap base >= 8-byte alignment); flags follow at n*8.
                    let instant = unsafe { *m.as_ptr().cast::<f64>().add(i) };
                    temp_unflag(m[n * 8 + i], instant)
                } else {
                    extra.get(&id).copied() // appended after open
                }
            }
        }
    }

    /// Number of terms covered by a mapped `temporals.bin` (9 bytes per term).
    #[cfg(feature = "mmap")]
    #[inline]
    fn mapped_len(m: &memmap2::Mmap) -> usize {
        m.len() / 9
    }

    /// Appends cache entries for freshly interned dictionary ids `old_len+1 ..= dict.len()`
    /// (delta-overlay growth), mirroring [`NumData::extend_for`].
    fn extend_for(&mut self, dict: &Dict, old_len: usize) {
        for i in old_len..dict.len() {
            let id = i as Id + 1;
            let t = temporal_of_id(dict, id);
            match self {
                TempData::Owned(cells) => cells.push(match t {
                    Some(t) => TempCell { instant: t.instant, flag: temp_flag(t) },
                    None => TempCell { instant: f64::NAN, flag: 0 },
                }),
                TempData::Sparse(m) => {
                    if let Some(t) = t {
                        m.insert(id, t);
                    }
                }
                #[cfg(feature = "mmap")]
                TempData::Mapped(_, extra) => {
                    if let Some(t) = t {
                        extra.insert(id, t);
                    }
                }
                TempData::Forked { extra, .. } => {
                    if let Some(t) = t {
                        extra.insert(id, t);
                    }
                }
            }
        }
    }

    /// The cache for a structurally FORKED graph — mirror of [`NumData::fork`].
    fn fork(&self) -> TempData {
        match self {
            TempData::Forked { base, extra } => TempData::Forked {
                base: std::sync::Arc::clone(base),
                extra: extra.clone(),
            },
            TempData::Owned(cells) => TempData::Forked {
                base: std::sync::Arc::new(TempData::Owned(cells.clone())),
                extra: rustc_hash::FxHashMap::default(),
            },
            TempData::Sparse(m) => TempData::Forked {
                base: std::sync::Arc::new(TempData::Sparse(m.clone())),
                extra: rustc_hash::FxHashMap::default(),
            },
            #[cfg(feature = "mmap")]
            TempData::Mapped(m, extra) => {
                // Materialise the mmap'd cells (an `Mmap` cannot be cloned).
                let n = Self::mapped_len(m);
                // SAFETY: instants are `n` little-endian f64 at offset 0 (page-aligned).
                let instants = unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<f64>(), n) };
                let cells: Vec<TempCell> = (0..n)
                    .map(|i| TempCell { instant: instants[i], flag: m[n * 8 + i] })
                    .collect();
                TempData::Forked {
                    base: std::sync::Arc::new(TempData::Owned(cells)),
                    extra: extra.clone(),
                }
            }
        }
    }

    /// Folds a forked cache's side map into a fresh shared base, keeping the
    /// shareable `Forked` shape — mirror of [`NumData::fold`].
    fn fold(self, dict_len: usize) -> TempData {
        match self {
            TempData::Forked { base, extra } => {
                let folded = match &*base {
                    TempData::Owned(cells) => {
                        let mut cells = cells.clone();
                        cells.resize(dict_len, TempCell { instant: f64::NAN, flag: 0 });
                        for (&id, &t) in &extra {
                            cells[(id - 1) as usize] = TempCell { instant: t.instant, flag: temp_flag(t) };
                        }
                        TempData::Owned(cells)
                    }
                    TempData::Sparse(m) => {
                        let mut m = m.clone();
                        m.extend(extra);
                        TempData::Sparse(m)
                    }
                    _ => unreachable!("a forked temporal cache's base is Owned or Sparse"),
                };
                TempData::Forked { base: std::sync::Arc::new(folded), extra: rustc_hash::FxHashMap::default() }
            }
            other => other,
        }
    }

    /// Resident heap bytes (a memory-mapped cache contributes 0 — it is page cache).
    #[inline]
    fn heap_bytes(&self) -> usize {
        match self {
            TempData::Owned(cells) => cells.capacity() * std::mem::size_of::<TempCell>(),
            #[cfg(feature = "mmap")]
            TempData::Mapped(_, extra) => extra.capacity() * 25,
            // hashbrown: ~(4-byte key pad to 8 + 16-byte Temporal + control) per slot.
            TempData::Sparse(m) => m.capacity() * 25,
            // Reachable footprint: the Arc-shared base is counted in full — see the
            // matching NumData::Forked arm (roborev 1945).
            TempData::Forked { base, extra } => base.heap_bytes() + extra.capacity() * 25,
        }
    }

    /// Converts a dense cache into the sparse form when it is mostly empty (≥ 3/4 of
    /// terms non-temporal — almost always true), mirroring the numerics cache.
    fn into_sparse_if_worthwhile(self) -> TempData {
        let cells = match &self {
            TempData::Owned(c) => c,
            _ => return self, // mmap'd/already-sparse: leave as is
        };
        let temporal = cells.iter().filter(|c| c.flag != 0).count();
        if temporal * 4 > cells.len() {
            return self; // temporal-dense: the cells are the better representation
        }
        let mut m: rustc_hash::FxHashMap<Id, Temporal> = rustc_hash::FxHashMap::default();
        m.reserve(temporal);
        for (i, c) in cells.iter().enumerate() {
            if let Some(t) = temp_unflag(c.flag, c.instant) {
                m.insert(i as Id + 1, t);
            }
        }
        TempData::Sparse(m)
    }
}

/// The cached temporal value of dictionary term `id`, parsed from its borrowed parts
/// (no `Term` materialisation). `None` for non-temporal terms and ill-formed lexicals.
fn temporal_of_id(dict: &Dict, id: Id) -> Option<Temporal> {
    match dict.term_parts(id) {
        dict::TermParts::Lit { value, datatype, lang: None } => Temporal::of_lit(value, datatype),
        _ => None,
    }
}

/// One dense temporal-cache cell: the flag (see [`temp_flag`]; 0 = not temporal) and
/// the precomputed instant, co-located so a probe is one cache-line touch.
#[derive(Clone, Copy)]
struct TempCell {
    instant: f64,
    flag: u8,
}

/// The temporal-value cache cells for a dictionary (see [`TempData::Owned`]).
/// Parallel when the `parallel` feature is on.
fn temporals_of(dict: &Dict) -> Vec<TempCell> {
    let n = dict.len();
    let cell = |i: usize| -> TempCell {
        match temporal_of_id(dict, i as Id + 1) {
            Some(t) => TempCell { instant: t.instant, flag: temp_flag(t) },
            None => TempCell { instant: f64::NAN, flag: 0 },
        }
    };
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(cell).collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        (0..n).map(cell).collect()
    }
}

/// Writes the temporal-value cache to disk (`n` little-endian f64 instants, then `n`
/// flag bytes) so it can be memory-mapped on open instead of recomputed.
#[cfg(feature = "mmap")]
fn write_temporals(path: &std::path::Path, flags: &[u8], instants: &[f64]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    // SAFETY: reinterpret the contiguous f64 column as bytes for writing.
    let ibytes = unsafe { std::slice::from_raw_parts(instants.as_ptr().cast::<u8>(), std::mem::size_of_val(instants)) };
    f.write_all(ibytes)?;
    f.write_all(flags)?;
    f.flush()
}

/// [FABLE-5] (sq-9781x) Parse a numeric-literal lexical to its f64 image using the XSD
/// `doubleRep` lexical space — the SINGLE shared routine for the XSD f64 SPELLING rules.
///
/// This function is DATATYPE-AGNOSTIC: it decides only whether a lexical is a well-formed
/// XSD double lexical (specials + scientific form), NOT whether it is well-formed for a
/// specific integer/decimal datatype. The numeric-value CACHE
/// (`numerics_of`/`numeric_of`/`cached_numeric_f64`) layers the datatype-AWARE gate
/// `numeric_datatype_wellformed` ON TOP of this, so a cache HIT is now equivalent to
/// `sparq_substrate::numeric::Num::of_literal` acceptance (sq-6b1lj — integers scale-0,
/// decimals no-exponent, i128-fit), NOT merely to this f64-seam acceptance. The lenient
/// engine `as_num`/`as_f64` and reasoner `as_f64` seams are also datatype-aware as of
/// sq-74oy4, so `"1.5"^^xsd:integer` now uniformly type-errors on `=`/`<`/`>` (cache-miss →
/// exact evaluator) instead of the pre-fix fast-path-only over-inclusion; the substrate
/// differential test `cache_f64_seam_vs_as_numeric_differential` pins that agreement.
///
/// It is the lowest-tier home of the parser `sparq_substrate::numeric::parse_xsd_f64`
/// re-exports (the substrate depends on `sparq-core`, not the reverse, so the shared body
/// must live here). The acceptance set is XSD's, not Rust's:
///
/// - The XSD specials `NaN` / `INF` / `+INF` / `-INF` parse; Rust-`FromStr`-only spellings
///   the XSD lexical space FORBIDS (`inf` / `infinity` / `-inf` / `nan` / `Infinity` …) are
///   REJECTED, even though `str::parse::<f64>` would accept them. This is load-bearing: the
///   old raw `str::parse::<f64>` cache stored `inf` for `"inf"^^xsd:double`, which the
///   evaluator type-errors — a cache MORE lenient than the evaluator.
/// - No trimming here (byte-identical to the substrate re-export). Callers that must match
///   the evaluator's TRIMMING acceptance path (`Num::of_literal`, XSD `collapse` facet) trim
///   the lexical themselves before calling — `cached_numeric_f64` does exactly that, and
///   the lenient `as_num`/`as_f64` seams trim too as of sq-74oy4 (so a whitespace-padded
///   `" 1"^^xsd:integer` is value-1 uniformly on `=`, `<`, `>`, not a `<`/`>` type error).
///
/// `None` for an ill-formed lexical.
#[inline]
pub fn parse_xsd_f64(v: &str) -> Option<f64> {
    match v {
        "NaN" => Some(f64::NAN),
        "INF" | "+INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        // Only ASCII digits / sign / point / exponent letters reach Rust's parser, which
        // excludes every non-XSD spelling (`inf`/`infinity`/`nan`/hex/`_` separators …).
        _ if v.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E')) => v.parse::<f64>().ok(),
        _ => None,
    }
}

/// `true` iff `v` (already TRIMMED) is a well-formed lexical FOR its numeric `datatype`
/// under XSD's per-datatype lexical space — the datatype-AWARE gate that mirrors
/// `sparq_substrate::numeric::Num::of_literal`'s ACCEPTANCE set (not its typed value).
///
/// [FABLE-5] (sq-74oy4 / sq-6b1lj) This is the second half of the numeric-cache alignment:
/// sq-9781x aligned the cache's f64 SPELLINGS + trimming with the evaluator's f64 seam, but
/// left the cache datatype-AGNOSTIC — it stored an f64 for a lexical ill-formed FOR ITS
/// DATATYPE (`"1.5"^^xsd:integer`, `"1E2"^^xsd:decimal`, a >38-digit decimal) that
/// `Num::of_literal` type-errors, so the sargable `=`/`<` fast path and `JKey::Num`
/// value-join over-included such a row. This gate reproduces `of_literal`'s per-datatype
/// acceptance so a cache HIT is now equivalent to `of_literal` acceptance for the SAME f64.
///
/// `sparq-core` is the leanest tier and cannot depend on `sparq-substrate` (which owns
/// `Num`/`Dec`), so the acceptance rules are re-derived here from the SAME grammar
/// (`split_decimal`-style digit scan + i128 fit); an anti-drift differential test pins this
/// against `Num::of_literal` over a lexical×datatype matrix in `sparq-substrate`.
///
/// - **integer family** (`is_integer_datatype`): a SCALE-0 decimal lexical (NO exponent)
///   that fits `i128` — matching `Num::of_literal`, which routes an over-`i64` integer
///   through `Dec::parse` and accepts scale 0. So `"5"`, `"+3"`, `"007"`, `"5."`, `"5.0"`,
///   `"5.00"` (all value-5 integers) are well-formed; `"1.5"` (scale 1), `".5"`,
///   `"1E2"^^xsd:integer`, and a >i128 integer are ill-formed.
/// - **`xsd:decimal`**: `[+-]?digits(.digits)?` (NO exponent) with the mantissa within
///   `i128`. `"1E2"^^xsd:decimal` / a >i128-mantissa decimal are ill-formed.
/// - **`xsd:float` / `xsd:double`**: the full XSD `doubleRep` lexical space — exactly
///   [`parse_xsd_f64`] (`Some`), which already matches `of_literal`.
/// - any non-numeric datatype: `false`.
fn numeric_datatype_wellformed(v: &str, datatype: &str) -> bool {
    // Parse `[+-]?digits(.digits)?` (no exponent) into its i128-fit mantissa + written scale
    // — the shared decimal-lexical scan `Num::of_literal`'s integer/decimal paths ride on
    // (`Dec::parse` / `Dec::parse_lexical` in sparq-substrate). `None` = ill-formed (bad char,
    // empty, or mantissa beyond i128). The scale is the NUMBER OF TRAILING FRACTION DIGITS
    // as WRITTEN — for the scale-0 integer test, trailing zeros do NOT count (`Dec::parse`
    // normalises them: `"5.0"` is scale-0). So compute the NORMALISED scale (strip trailing
    // fraction zeros) exactly as `of_literal` sees it.
    fn scan_decimal(v: &str) -> Option<u32> {
        let body = v.strip_prefix(['+', '-']).unwrap_or(v);
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if (int.is_empty() && frac.is_empty())
            || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit())
        {
            return None;
        }
        // i128-fit of the concatenated mantissa (leading `+`/`-` already stripped).
        let mut mag: i128 = 0;
        for &ch in int.as_bytes().iter().chain(frac.as_bytes()) {
            mag = mag.checked_mul(10).and_then(|m| m.checked_add((ch - b'0') as i128))?;
        }
        // Normalised scale: trailing fraction zeros are insignificant (`Dec::parse` drops
        // them), so `"5.00"` is scale-0, matching `of_literal`'s scale-0 integer acceptance.
        Some(frac.trim_end_matches('0').len() as u32)
    }
    if is_integer_datatype(datatype) {
        // scale-0, i128-fit — `Num::of_literal` accepts `"5"`, `"+3"`, `"007"`, `"5."`,
        // `"5.0"` (all value-5 integers) but NOT `"5.5"` (scale 1) or a >i128 mantissa.
        return matches!(scan_decimal(v), Some(0));
    }
    if datatype == xsd::DECIMAL.as_str() {
        // any scale (no exponent), i128-fit mantissa — mirrors `Dec::parse_lexical`.
        return scan_decimal(v).is_some();
    }
    // float / double: the datatype-agnostic XSD doubleRep space is already `of_literal`'s.
    (datatype == xsd::FLOAT.as_str() || datatype == xsd::DOUBLE.as_str()) && parse_xsd_f64(v).is_some()
}

/// The DATATYPE-AWARE cached f64 of a numeric literal `(value, datatype)`, or `NaN` (the
/// cache's not-a-value sentinel) when the lexical is ill-formed FOR its datatype. Trims the
/// lexical (XSD `collapse` whitespace facet — the same trim `Num::of_literal` /
/// `Dec::parse_lexical` apply) then gates the f64 on [`numeric_datatype_wellformed`], so a
/// cache HIT is equivalent to `of_literal` acceptance for the same f64. [FABLE-5]
/// (sq-74oy4 / sq-6b1lj)
#[inline]
pub(crate) fn cached_numeric_f64(value: &str, datatype: &str) -> f64 {
    let v = value.trim();
    if numeric_datatype_wellformed(v, datatype) {
        parse_xsd_f64(v).unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// The numeric-value CACHE's acceptance of a literal `(value, datatype)`: `Some(f64)` iff the
/// lexical is well-formed FOR its datatype (equivalently `Num::of_literal` accepts it), else
/// `None`. This is exactly what `Graph::numeric_value` returns for the term (modulo the
/// genuine-`NaN`-double sentinel, which reads back as `None` in both). Public so the
/// substrate's cross-seam differential test can assert the cache seam against the
/// datatype-AWARE `Num::of_literal` WITHOUT re-implementing the cache's acceptance (which
/// would make the test circular). [FABLE-5] (sq-74oy4 / sq-6b1lj)
#[inline]
pub fn numeric_cache_value(value: &str, datatype: &str) -> Option<f64> {
    let v = cached_numeric_f64(value, datatype);
    if v.is_nan() {
        None
    } else {
        Some(v)
    }
}

/// The f64 value of a term if it is a well-formed numeric XSD literal FOR its datatype, else
/// NaN. Routes through [`cached_numeric_f64`], the DATATYPE-AWARE acceptance that mirrors
/// `Num::of_literal` (see [`numeric_datatype_wellformed`]): a lexical ill-formed for its
/// datatype (`"1.5"^^xsd:integer`) folds to the NaN cache-miss sentinel exactly as the
/// evaluator type-errors it. [FABLE-5] (sq-9781x / sq-74oy4 / sq-6b1lj)
fn numeric_of(term: &Term) -> f64 {
    match term {
        Term::Literal(l) if is_numeric_dt(l) => cached_numeric_f64(l.value(), l.datatype().as_str()),
        _ => f64::NAN,
    }
}

/// [OPUS-4.8] (sq-lr2ii) `true` iff dict id `id` is an `xsd:decimal` literal whose lexical
/// form carries MORE than 15 significant digits — i.e. its f64 image may be inexact, so the
/// engine's f64 sargable FILTER fast path is unsafe for it. Only `xsd:decimal` is checked:
/// integer exactness is handled by the engine's constant-side guard and float/double values
/// are their own f64. See [`Graph::has_high_precision_decimal`].
fn is_high_precision_decimal(dict: &Dict, id: Id) -> bool {
    matches!(dict.term_parts(id),
        dict::TermParts::Lit { value, datatype, lang: None }
            if datatype == xsd::DECIMAL.as_str() && decimal_significant_digits(value) > 15)
}

/// Significant decimal digits of a plain decimal lexical `[+-]?digits(.digits)?`: leading
/// integer zeros and leading fractional zeros (for a value `< 1`) are not significant, e.g.
/// `007.50` -> 3, `0.00123` -> 3, `1.000000000000000001` -> 19. `usize::MAX` for a lexical that
/// is not a plain decimal (treated as unsafe by the `> 15` guard). Kept LOCAL to sparq-core:
/// this crate is the leanest tier and cannot depend on `sparq-substrate`; the count mirrors the
/// engine's `sig_digits` (constant-side guard) so the two round-trip decisions stay consistent.
fn decimal_significant_digits(s: &str) -> usize {
    let s = s.trim();
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty()) || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
        return usize::MAX;
    }
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    if int.is_empty() {
        frac.trim_start_matches('0').len()
    } else {
        int.len() + frac.len()
    }
}

/// True for `xsd:integer` and its derived integer subtypes (NOT decimal/double/float) —
/// the datatypes whose values are exact integers (parseable as `i128`).
pub fn is_integer_datatype(dt: &str) -> bool {
    dt == xsd::INTEGER.as_str()
        || dt == xsd::LONG.as_str()
        || dt == xsd::INT.as_str()
        || dt == xsd::SHORT.as_str()
        || dt == xsd::BYTE.as_str()
        || dt == xsd::NON_NEGATIVE_INTEGER.as_str()
        || dt == xsd::POSITIVE_INTEGER.as_str()
        || dt == xsd::NON_POSITIVE_INTEGER.as_str()
        || dt == xsd::NEGATIVE_INTEGER.as_str()
        || dt == xsd::UNSIGNED_INT.as_str()
        || dt == xsd::UNSIGNED_LONG.as_str()
        || dt == xsd::UNSIGNED_SHORT.as_str()
        || dt == xsd::UNSIGNED_BYTE.as_str()
}

fn is_numeric_dt(l: &Literal) -> bool {
    is_numeric_datatype_str(l.datatype().as_str())
}

/// `is_numeric_dt` by datatype IRI string — shared with the spilled-dict build, which
/// computes the numeric cache from borrowed term components without an `oxrdf::Term`.
/// (A language-tagged literal's datatype is `rdf:langString`, never numeric.)
pub(crate) fn is_numeric_datatype_str(dt: &str) -> bool {
    dt == xsd::INTEGER.as_str()
        || dt == xsd::DECIMAL.as_str()
        || dt == xsd::DOUBLE.as_str()
        || dt == xsd::FLOAT.as_str()
        || dt == xsd::LONG.as_str()
        || dt == xsd::INT.as_str()
        || dt == xsd::SHORT.as_str()
        || dt == xsd::BYTE.as_str()
        || dt == xsd::NON_NEGATIVE_INTEGER.as_str()
        || dt == xsd::POSITIVE_INTEGER.as_str()
        || dt == xsd::NON_POSITIVE_INTEGER.as_str()
        || dt == xsd::NEGATIVE_INTEGER.as_str()
        || dt == xsd::UNSIGNED_INT.as_str()
        || dt == xsd::UNSIGNED_LONG.as_str()
        || dt == xsd::UNSIGNED_SHORT.as_str()
        || dt == xsd::UNSIGNED_BYTE.as_str()
}

impl Graph {
    /// Loads triples from an RDF document (default graph only for M1; named
    /// graphs from TriG/N-Quads are folded into the default graph). Returns the
    /// built graph. `format`: "turtle" | "ntriples" | "nquads" | "trig" | "jsonld"
    /// (plus the usual aliases — "ttl"/"text/turtle", "nt", "nq", "json-ld" /
    /// "application/ld+json", …). JSON-LD is recognised only when the crate is built with
    /// the OPT-IN `jsonld` feature (it links `oxjsonld`). [OPUS-4.8] (sq-m2pc) An
    /// UNRECOGNISED `format` is now an `Err` — it is NOT silently parsed as Turtle (so a
    /// "jsonld" string in a build without the feature errors rather than mis-parsing).
    /// JSON-LD `@graph` named graphs are folded into the default graph here; use
    /// [`load_dataset`](Self::load_dataset) to preserve them.
    pub fn load_str(text: &str, format: &str) -> Result<Graph, String> {
        let (dict, triples) = Self::parse_to_triples(text, format)?;
        Ok(Self::from_parts(dict, triples))
    }

    /// Parses an RDF document into its dictionary + interned triples WITHOUT building the
    /// indexes. The seam that opt-in reasoning hooks into: a caller (e.g. the CLI, which can
    /// depend on `sparq-reason`) parses, materializes the entailed triples, then calls
    /// [`from_parts`](Self::from_parts) — keeping all reasoning out of the core engine.
    pub fn parse_to_triples(text: &str, format: &str) -> Result<(Dict, Vec<[Id; 3]>), String> {
        let mut dict = Dict::new();
        let mut triples: Vec<[Id; 3]> = Vec::new();
        let bytes = text.as_bytes();

        macro_rules! push_triple {
            ($s:expr, $p:expr, $o:expr) => {{
                let s = dict.intern(&subject_term($s));
                let p = dict.intern(&Term::NamedNode($p.clone()));
                let o = dict.intern($o);
                triples.push([s, p, o]);
            }};
        }

        match format {
            // [OPUS-4.8] (sq-m2pc) `nt`/`application/n-triples` are accepted aliases (the
            // extension + media type) — previously they only "worked" by falling through the
            // catch-all and parsing N-Triples AS Turtle; now they take the fast N-Triples path.
            "ntriples" | "n-triples" | "nt" | "application/n-triples" => {
                // N-Triples is one statement per line, so the input can be split at
                // newline boundaries and parsed + interned in parallel (each thread
                // builds a partial dictionary, then the partials are merged).
                #[cfg(feature = "parallel")]
                {
                    return parse_ntriples_parallel(bytes);
                }
                #[cfg(not(feature = "parallel"))]
                {
                    let mut d = Dict::new();
                    let t = nt::parse_chunk(bytes, &mut d)?;
                    return Ok((d, t));
                }
            }
            "nquads" | "n-quads" | "nq" | "application/n-quads" => {
                for q in NQuadsParser::new().for_slice(bytes) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            "trig" | "application/trig" => {
                for q in TriGParser::new().for_slice(bytes) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            // [OPUS-4.8] sq-dvyi: JSON-LD. A whole-document JSON parse (NOT line-oriented,
            // so no parallel chunking applies) yielding quads; the quad's graph name is
            // FOLDED here (default-graph load) — `load_dataset` preserves it instead.
            // OPT-IN behind the `jsonld` feature (keeps the lean bundle free of oxjsonld).
            #[cfg(feature = "jsonld")]
            _ if is_jsonld_format(format) => {
                for q in JsonLdParser::new().for_slice(bytes) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            // [OPUS-4.8] sq-f47w1 (survey §B1): RDF/XML. A whole-document XML parse (NOT
            // line-oriented, so no parallel chunking applies) yielding TRIPLES — RDF/XML has
            // no named-graph syntax, so nothing is folded. OPT-IN behind the `rdfxml` feature
            // (keeps the lean bundle free of oxrdfxml / quick-xml). Same proven parser the
            // sparq-server GSP write-body + conformance paths already use.
            #[cfg(feature = "rdfxml")]
            _ if is_rdfxml_format(format) => {
                for t in RdfXmlParser::new().for_slice(bytes) {
                    let t = t.map_err(|e| e.to_string())?;
                    push_triple!(&t.subject, &t.predicate, &t.object);
                }
            }
            // [OPUS-4.8] (sq-m2pc) Turtle is gated on its explicit alias set — NOT a
            // catch-all — so a typo'd or unsupported `format` errors below instead of
            // silently parsing as Turtle and returning `Ok`.
            _ if is_turtle_format(format) => {
                // Turtle is not line-oriented, but it splits at top-level statement
                // terminators (with the @prefix preamble shared into each chunk), parsed in
                // parallel with a serial fallback on any mis-split — see parse_turtle_parallel.
                #[cfg(feature = "parallel")]
                {
                    return parse_turtle_parallel(bytes);
                }
                // [OPUS-4.8] (sq-jocpn) The non-parallel build (e.g. wasm) also routes through the
                // native parser under `native-ttl` — a whole-document parse with no base.
                #[cfg(all(not(feature = "parallel"), feature = "native-ttl"))]
                {
                    let ids = ttl::parse(bytes, None, &mut dict)?;
                    triples.extend_from_slice(&ids);
                }
                #[cfg(all(not(feature = "parallel"), not(feature = "native-ttl")))]
                {
                    for t in TurtleParser::new().for_slice(bytes) {
                        let t = t.map_err(|e| e.to_string())?;
                        push_triple!(&t.subject, &t.predicate, &t.object);
                    }
                }
            }
            _ => return Err(unknown_format_err(format)),
        }

        Ok((dict, triples))
    }

    /// Like [`load_str`](Self::load_str) but resolves relative IRIs in the document
    /// against `base` — the entry point for documents that carry no `@base` of their
    /// own (e.g. SHACL shapes graphs addressed by their location, W3C test-suite
    /// manifests). `format`: as `load_str`; the line-based formats ("ntriples" /
    /// "nquads") only allow absolute IRIs, so `base` has no effect on them.
    pub fn load_str_with_base(text: &str, format: &str, base: &str) -> Result<Graph, String> {
        let (dict, triples) = Self::parse_to_triples_with_base(text, format, base)?;
        Ok(Self::from_parts(dict, triples))
    }

    /// [`parse_to_triples`](Self::parse_to_triples) with a base IRI for resolving the
    /// document's relative IRIs (Turtle / TriG; the line-based formats have no
    /// relative IRIs and ignore `base`). Parses serially — base-IRI documents are
    /// typically small (shapes graphs, manifests), so the parallel chunked path is
    /// not worth its base-rewriting complexity here.
    pub fn parse_to_triples_with_base(
        text: &str,
        format: &str,
        base: &str,
    ) -> Result<(Dict, Vec<[Id; 3]>), String> {
        let mut dict = Dict::new();
        let mut triples: Vec<[Id; 3]> = Vec::new();
        let bytes = text.as_bytes();

        macro_rules! push_triple {
            ($s:expr, $p:expr, $o:expr) => {{
                let s = dict.intern(&subject_term($s));
                let p = dict.intern(&Term::NamedNode($p.clone()));
                let o = dict.intern($o);
                triples.push([s, p, o]);
            }};
        }
        match format {
            "ntriples" | "n-triples" | "nt" | "application/n-triples" | "nquads" | "n-quads"
            | "nq" | "application/n-quads" => {
                return Self::parse_to_triples(text, format);
            }
            "trig" | "application/trig" => {
                let parser = TriGParser::new()
                    .with_base_iri(base)
                    .map_err(|e| format!("invalid base IRI {base:?}: {e}"))?;
                for q in parser.for_slice(bytes) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            // [OPUS-4.8] sq-dvyi: JSON-LD with a document base — the URL load path (a
            // document fetched from a URL with relative IRIs). Graph name folded.
            // OPT-IN behind the `jsonld` feature (keeps the lean bundle free of oxjsonld).
            #[cfg(feature = "jsonld")]
            _ if is_jsonld_format(format) => {
                let parser = JsonLdParser::new()
                    .with_base_iri(base)
                    .map_err(|e| format!("invalid base IRI {base:?}: {e}"))?;
                for q in parser.for_slice(bytes) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            // [OPUS-4.8] sq-f47w1 (survey §B1): RDF/XML with a document base — `rdf:ID`
            // fragments and relative IRIs resolve against `base` (the URL load path). RDF/XML
            // has no named graphs, so nothing is folded. OPT-IN behind the `rdfxml` feature.
            #[cfg(feature = "rdfxml")]
            _ if is_rdfxml_format(format) => {
                let parser = RdfXmlParser::new()
                    .with_base_iri(base)
                    .map_err(|e| format!("invalid base IRI {base:?}: {e}"))?;
                for t in parser.for_slice(bytes) {
                    let t = t.map_err(|e| e.to_string())?;
                    push_triple!(&t.subject, &t.predicate, &t.object);
                }
            }
            // [OPUS-4.8] (sq-m2pc) Turtle is gated on its explicit alias set, mirroring
            // `parse_to_triples`; an unknown `format` errors instead of parsing as Turtle.
            _ if is_turtle_format(format) => {
                // [OPUS-4.8] (sq-jocpn) Under `native-ttl`, the WITH-BASE serial Turtle path — the
                // one the W3C TurtleTests conformance ratchet drives (`turtle_suite.rs` calls
                // `parse_to_triples_with_base`) — runs through the native parser too, so the ratchet
                // pins the native parser's accept/reject + resolved-term set against oxttl's on the
                // whole W3C suite. Base resolution is delegated to the same `oxiri` automaton oxttl
                // uses, so resolved IRIs are byte-identical.
                #[cfg(feature = "native-ttl")]
                {
                    let ids = ttl::parse(bytes, Some(base), &mut dict)?;
                    triples.extend_from_slice(&ids);
                }
                #[cfg(not(feature = "native-ttl"))]
                {
                    let parser = TurtleParser::new()
                        .with_base_iri(base)
                        .map_err(|e| format!("invalid base IRI {base:?}: {e}"))?;
                    for t in parser.for_slice(bytes) {
                        let t = t.map_err(|e| e.to_string())?;
                        push_triple!(&t.subject, &t.predicate, &t.object);
                    }
                }
            }
            _ => return Err(unknown_format_err(format)),
        }

        Ok((dict, triples))
    }

    /// Load an RDF DATASET (N-Quads / TriG) preserving NAMED GRAPHS as separate sub-graphs, so the
    /// engine can evaluate `GRAPH <iri> { … }` / `GRAPH ?g { … }`. Default-graph triples form the
    /// main graph; each named graph becomes a [`named`](Self::named) entry. Formats without named
    /// graphs defer to [`load_str`](Self::load_str). In-memory only (the mmap path is triple-only).
    pub fn load_dataset(text: &str, format: &str) -> Result<Graph, String> {
        // [OPUS-4.8] sq-dvyi: JSON-LD carries named graphs (`@graph` with an outer `@id`),
        // so it must take the dataset path too — but it is a WHOLE-DOCUMENT JSON parse
        // (not line/statement chunkable), so it always goes through the serial loader.
        // OPT-IN behind the `jsonld` feature (keeps the lean bundle free of oxjsonld).
        #[cfg(feature = "jsonld")]
        if is_jsonld_format(format) {
            return Self::load_dataset_serial(text, format);
        }
        // [OPUS-4.8] (sq-01yr) The N-Quads/TriG dataset aliases — including `nq`/
        // `application/n-quads` (mirroring sq-m2pc) — take the dataset path so their named
        // graphs are preserved; everything else defers to `load_str` (which itself rejects
        // unknown formats under sq-m2pc rather than folding them into Turtle).
        if !is_dataset_format(format) {
            return Self::load_str(text, format);
        }
        // [OPUS-4.8] (sq-25r3) N-Quads is newline-delimited, so a byte-range chunk-parallel parse
        // is correct: the quad's 4th (graph) field just routes its triple to a per-graph bucket,
        // and each graph's buckets merge through the SAME dataset-scoped sharded/serial dict merge
        // the N-Triples fast path uses (one dict PER graph, mirroring the serial loader).
        //
        // [OPUS-4.8] (sq-ev37) TriG is NOT line-oriented (it nests `GRAPH g { … }` / `g { … }` /
        // `{ … }` blocks carrying Turtle's `@prefix`/`@base` scope and document-scoped blank-node
        // labels), so byte-range newline chunking is wrong. Instead it reuses the Turtle
        // statement-terminator chunking ([`trig_chunks`]): each chunk is a directive-snapshot + a
        // run of same-graph statements re-wrapped as `label { … }`, parsed back into per-graph
        // buckets and merged per graph exactly like N-Quads. The splitter is conservative and the
        // loader redoes the document serially on any per-chunk parse error, so the result equals
        // [`load_dataset_serial`] (up to anonymous blank-node ids, which differ run-to-run even
        // between two serial parses).
        #[cfg(feature = "parallel")]
        {
            if matches!(format, "nquads" | "n-quads" | "nq" | "application/n-quads") {
                Self::load_nquads_parallel(text.as_bytes())
            } else {
                Self::load_trig_parallel(text.as_bytes())
            }
        }
        #[cfg(not(feature = "parallel"))]
        {
            Self::load_dataset_serial(text, format)
        }
    }

    /// [FABLE-5] (sq-tonhr.2) Like [`load_dataset`](Self::load_dataset) but resolves the
    /// document's relative IRIs against `base` — the DATASET companion to
    /// [`load_str_with_base`](Self::load_str_with_base), preserving named graphs instead of
    /// folding them. The entry point for base-relative dataset documents addressed by their
    /// location (e.g. the W3C rdf-trig conformance suite, whose expectation files bake in the
    /// suite's canonical base). Parses serially — base-IRI documents are typically small
    /// (manifests, test actions), so the parallel chunked path is not worth base-rewriting
    /// complexity here (mirroring `parse_to_triples_with_base`). The line-based N-Quads format
    /// has no relative IRIs, so `base` has no effect on it; non-dataset formats defer to
    /// `load_str_with_base` (named-graph-free).
    pub fn load_dataset_with_base(text: &str, format: &str, base: &str) -> Result<Graph, String> {
        if !is_dataset_format(format) {
            #[cfg(feature = "jsonld")]
            if is_jsonld_format(format) {
                return Self::load_dataset_serial_base(text, format, Some(base));
            }
            return Self::load_str_with_base(text, format, base);
        }
        Self::load_dataset_serial_base(text, format, Some(base))
    }

    /// [OPUS-4.8] (sq-25r3) Serial dataset loader (the correctness reference + the non-`parallel`
    /// and TriG fallback). Routes each quad to a per-graph bucket and builds one sub-graph per
    /// graph, with the default graph as the main graph. Named graphs are emitted in
    /// FIRST-OCCURRENCE document order (deterministic — the old `HashMap` iteration order was not),
    /// so the parallel path can be a byte-identical drop-in.
    fn load_dataset_serial(text: &str, format: &str) -> Result<Graph, String> {
        Self::load_dataset_serial_base(text, format, None)
    }

    /// [FABLE-5] (sq-tonhr.2) [`load_dataset_serial`] with an optional base IRI — the shared
    /// body behind the no-base serial loader and [`load_dataset_with_base`].
    fn load_dataset_serial_base(text: &str, format: &str, base: Option<&str>) -> Result<Graph, String> {
        use oxrdf::GraphName;
        use std::collections::HashMap;
        let bytes = text.as_bytes();
        // `groups` is in first-occurrence order of graph keys; `index` maps a key to its slot so
        // repeated references to the same graph append to one bucket without re-ordering.
        let mut groups: Vec<(Option<Term>, Vec<[Term; 3]>)> = Vec::new();
        let mut index: HashMap<Option<Term>, usize> = HashMap::new();
        macro_rules! group {
            ($parser:expr) => {
                for q in $parser.for_slice(bytes) {
                    let q = q.map_err(|e| e.to_string())?;
                    let g = match q.graph_name {
                        GraphName::DefaultGraph => None,
                        GraphName::NamedNode(n) => Some(Term::NamedNode(n)),
                        GraphName::BlankNode(b) => Some(Term::BlankNode(b)),
                    };
                    let slot = *index.entry(g.clone()).or_insert_with(|| {
                        groups.push((g.clone(), Vec::new()));
                        groups.len() - 1
                    });
                    groups[slot]
                        .1
                        .push([subject_term(&q.subject), Term::NamedNode(q.predicate), q.object]);
                }
            };
        }
        // [FABLE-5] (sq-tonhr.2) Attach the optional base IRI to a parser that supports one.
        macro_rules! with_base {
            ($parser:expr) => {
                match base {
                    Some(b) => $parser
                        .with_base_iri(b)
                        .map_err(|e| format!("invalid base IRI {b:?}: {e}"))?,
                    None => $parser,
                }
            };
        }
        match format {
            // [OPUS-4.8] (sq-01yr) The N-Quads alias set (incl. `nq`/`application/n-quads`,
            // mirroring the `parse_to_triples` set from sq-m2pc) — single authority in
            // `is_nquads_format`. N-Quads has no relative IRIs, so `base` is ignored.
            _ if is_nquads_format(format) => group!(NQuadsParser::new()),
            // [OPUS-4.8] sq-dvyi: JSON-LD yields quads with a graph name, so the same
            // per-graph bucketing applies — `@graph`/`@id` named graphs are preserved.
            // OPT-IN behind the `jsonld` feature (keeps the lean bundle free of oxjsonld).
            #[cfg(feature = "jsonld")]
            _ if is_jsonld_format(format) => group!(with_base!(JsonLdParser::new())),
            // [OPUS-4.8] (sq-01yr) TriG is gated on its explicit alias set — NOT a catch-all —
            // mirroring the `parse_to_triples` Turtle fix (sq-m2pc). A typo'd or unsupported
            // dataset `format` now ERRORS here instead of silently parsing as TriG and
            // returning `Ok`. `load_dataset` already routes non-dataset formats to `load_str`,
            // but the direct callers (the `jsonld`/non-`parallel` paths and the TriG-parallel
            // serial fallback) inherited this independent silent fallback.
            _ if is_trig_format(format) => group!(with_base!(TriGParser::new())),
            _ => return Err(unknown_format_err(format)),
        }
        let build_terms = |triples: &[[Term; 3]]| -> Graph {
            let mut dict = Dict::new();
            let ids: Vec<[Id; 3]> =
                triples.iter().map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)]).collect();
            Self::build(dict, ids)
        };
        let mut g: Option<Graph> = None;
        let mut named: Vec<(Term, Graph)> = Vec::new();
        for (name, triples) in &groups {
            match name {
                None => g = Some(build_terms(triples)),
                Some(name) => named.push((name.clone(), build_terms(triples))),
            }
        }
        let mut g = g.unwrap_or_else(|| build_terms(&[]));
        g.named = named;
        Ok(g)
    }

    /// [OPUS-4.8] (sq-25r3) Chunk-parallel N-Quads dataset loader. Splits the buffer at newline
    /// boundaries, parses each range into per-graph partial dicts + local-id triples (no shared
    /// state), then — PER GRAPH — merges the partials through the same dict-merge the N-Triples
    /// fast path uses (sharded on ≥2 threads, serial `merge_remap` otherwise / when triple terms
    /// are present). Graphs are emitted in first-occurrence document order, byte-identical to
    /// [`load_dataset_serial`]. Cross-chunk blank-node identity (S/P/O AND graph names) survives:
    /// labels are kept verbatim and unify by label in each graph's dict merge, exactly as the
    /// serial dataset-scoped parse does.
    #[cfg(feature = "parallel")]
    fn load_nquads_parallel(bytes: &[u8]) -> Result<Graph, String> {
        let threads = rayon::current_num_threads().max(1);
        let target = (threads * 4).min(bytes.len() / 4096 + 1).max(1);
        Self::load_nquads_chunked(bytes, target)
    }

    /// [`load_nquads_parallel`] with an explicit chunk-count `target` — separated so the
    /// differential tests can force small documents to fan out across many ranges.
    #[cfg(feature = "parallel")]
    fn load_nquads_chunked(bytes: &[u8], target: usize) -> Result<Graph, String> {
        use rayon::prelude::*;
        use std::collections::HashMap;
        // Split into newline-aligned ranges and parse each into per-graph buckets in parallel.
        let bounds = newline_chunk_bounds(bytes, target);
        type ChunkBuckets = Vec<(Option<nt::GraphKey>, Dict, Vec<[Id; 3]>)>;
        let per_chunk: Vec<ChunkBuckets> = bounds
            .par_iter()
            .map(|&(s, e)| nt::parse_quads_chunk(&bytes[s..e]))
            .collect::<Result<Vec<_>, String>>()?;
        // Regroup the per-chunk buckets BY GRAPH, in first-occurrence document order (chunks are in
        // document order, and within a chunk buckets are already first-occurrence). For each graph
        // we accumulate its (Dict, triples) partials across all chunks — the exact input shape the
        // N-Triples sharded/serial merge consumes.
        let mut index: HashMap<Option<nt::GraphKey>, usize> = HashMap::new();
        let mut per_graph: Vec<(Option<nt::GraphKey>, ChunkPartials)> = Vec::new();
        for chunk in per_chunk {
            for (key, dict, triples) in chunk {
                let slot = *index.entry(key.clone()).or_insert_with(|| {
                    per_graph.push((key.clone(), Vec::new()));
                    per_graph.len() - 1
                });
                per_graph[slot].1.push((dict, triples));
            }
        }
        // Merge each graph's partials into one (Dict, triples) and build its sub-graph.
        let mut main: Option<Graph> = None;
        let mut named: Vec<(Term, Graph)> = Vec::new();
        for (key, partials) in per_graph {
            let (dict, ids) = merge_partials(partials);
            let sub = Self::build(dict, ids);
            match key {
                None => main = Some(sub),
                Some(k) => named.push((k.to_term(), sub)),
            }
        }
        let mut g = main.unwrap_or_else(|| Self::build(Dict::new(), Vec::new()));
        g.named = named;
        Ok(g)
    }

    /// [OPUS-4.8] (sq-ev37) Chunk-parallel TriG dataset loader. Splits the document with the
    /// TriG-aware statement chunker ([`trig_chunks`]) — directive-snapshot + a run of same-graph
    /// statements re-wrapped as `label { … }` per chunk — parses each chunk into per-graph buckets
    /// in parallel ([`parse_trig_chunk`]), then merges PER GRAPH through the SAME
    /// [`merge_partials`] the N-Quads/N-Triples fast paths use. Falls back to the serial oxttl path
    /// ([`load_dataset_serial`]) when the document is not safely splittable OR any chunk fails to
    /// re-parse, so the result always equals the serial parse (up to anonymous blank-node ids).
    #[cfg(feature = "parallel")]
    fn load_trig_parallel(bytes: &[u8]) -> Result<Graph, String> {
        // Below ~1 statement/thread or on a single thread, chunking is pure overhead — parse
        // serially. Otherwise aim for ~4 chunks/thread (the Turtle policy), capped by document size.
        let threads = rayon::current_num_threads().max(1);
        if threads == 1 {
            return Self::load_dataset_serial(std::str::from_utf8(bytes).map_err(|e| e.to_string())?, "trig");
        }
        let target = (threads * 4).min(bytes.len() / 8192 + 1).max(1);
        Self::load_trig_chunked(bytes, target)
    }

    /// [`load_trig_parallel`] with an explicit chunk-count `target` — separated so the differential
    /// tests can force small documents to fan out across many chunks (and so the splitter's
    /// per-graph routing + `label { … }` re-wrapping is exercised across genuine chunk boundaries).
    #[cfg(feature = "parallel")]
    fn load_trig_chunked(bytes: &[u8], target: usize) -> Result<Graph, String> {
        use rayon::prelude::*;
        use std::collections::HashMap;
        let serial = || {
            Self::load_dataset_serial(
                std::str::from_utf8(bytes).map_err(|e| e.to_string())?,
                "trig",
            )
        };
        let chunks = match trig_chunks(bytes, target) {
            Some(c) if c.len() > 1 => c,
            // Not safely splittable (or only one chunk): the serial path is the reference anyway.
            _ => return serial(),
        };
        type ChunkBuckets = Vec<(Option<nt::GraphKey>, Dict, Vec<[Id; 3]>)>;
        let per_chunk: Result<Vec<ChunkBuckets>, String> =
            chunks.par_iter().map(|chunk| parse_trig_chunk(chunk)).collect();
        let per_chunk = match per_chunk {
            Ok(p) => p,
            // An over-eager split produced invalid TriG — redo the whole document serially. This is
            // the safety net that makes any clean-but-wrong split observationally impossible.
            Err(_) => return serial(),
        };
        // Regroup per-chunk buckets BY GRAPH in first-occurrence document order (chunks are in
        // document order; within a chunk buckets are already first-occurrence) — the exact input
        // shape the per-graph merge consumes. Identical to the N-Quads loader's regroup.
        let mut index: HashMap<Option<nt::GraphKey>, usize> = HashMap::new();
        let mut per_graph: Vec<(Option<nt::GraphKey>, ChunkPartials)> = Vec::new();
        for chunk in per_chunk {
            for (key, dict, triples) in chunk {
                let slot = *index.entry(key.clone()).or_insert_with(|| {
                    per_graph.push((key.clone(), Vec::new()));
                    per_graph.len() - 1
                });
                per_graph[slot].1.push((dict, triples));
            }
        }
        let mut main: Option<Graph> = None;
        let mut named: Vec<(Term, Graph)> = Vec::new();
        for (key, partials) in per_graph {
            let (dict, ids) = merge_partials(partials);
            let sub = Self::build(dict, ids);
            match key {
                None => main = Some(sub),
                Some(k) => named.push((k.to_term(), sub)),
            }
        }
        let mut g = main.unwrap_or_else(|| Self::build(Dict::new(), Vec::new()));
        g.named = named;
        Ok(g)
    }

    /// Builds a graph from an already-interned dictionary + triple set (e.g. after opt-in
    /// reasoning materialized additional triples). Public counterpart of the internal
    /// `build`.
    pub fn from_parts(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        Self::build(dict, triples)
    }

    /// [OPUS-4.8] (gh-1118) An empty, in-memory graph — the trivial, INFALLIBLE constructor.
    ///
    /// Equivalent to [`from_parts`](Self::from_parts)`(Dict::new(), Vec::new())` but spelled the
    /// obvious way, so callers no longer reach for `Graph::load_str("", "turtle").unwrap()` (which
    /// parses an empty document and forces error handling for an operation that cannot fail) or the
    /// lower-level `from_parts` plumbing just to get an empty graph. Build it up incrementally with
    /// [`insert_triple`](Self::insert_triple) / [`apply_delta`](Self::apply_delta), or load into it
    /// via the `apply_delta_nquads` / `apply_delta` paths. This is an IN-MEMORY graph (no directory
    /// association, so `apply_delta` is overlay-only — there is no write-ahead log); use
    /// [`open`](Self::open) for a durable directory-backed graph. Also reachable as
    /// [`Graph::default()`](Default::default).
    #[inline]
    pub fn new() -> Graph {
        Self::from_parts(Dict::new(), Vec::new())
    }

    /// [OPUS-4.8] (sq-zz8z, gh-51) The sort key the prefix index orders graph names by: the
    /// string returned by SPARQL `STR(?g)`. For a named-node graph (the only kind a dataset's
    /// `GRAPH ?g` ever binds) that is the IRI itself; defined for any `Term` so the index is
    /// total. Keeping this ONE function makes the index's ordering and a query's
    /// `STRSTARTS(STR(?g), …)` use the SAME notion of the graph's string — they cannot diverge.
    fn graph_name_str(t: &Term) -> &str {
        match t {
            Term::NamedNode(n) => n.as_str(),
            Term::BlankNode(b) => b.as_str(),
            Term::Literal(l) => l.value(),
            #[allow(unreachable_patterns)]
            _ => "",
        }
    }

    /// [OPUS-4.8] (sq-zz8z, gh-51) Invokes `f` once for every named graph whose `STR(name)` starts
    /// with `prefix`, IN UNSPECIFIED ORDER, via a binary-search RANGE SCAN over a cached sorted
    /// index of the graph IRIs — O(log G + matches) instead of the O(G) full scan that a
    /// `GRAPH ?g … FILTER(STRSTARTS(STR(?g), prefix))` would otherwise perform over ALL named
    /// graphs (the PSS multi-tenant `usage(prefix)` cost cliff, gh-51).
    ///
    /// The result is EXACTLY the set of graphs a full scan + `STRSTARTS(STR(?g), prefix)` filter
    /// would keep (an empty prefix matches every graph; a no-match prefix yields nothing), so the
    /// indexed path is observationally identical to the unindexed one — see the equivalence tests.
    /// The index is built lazily on first use and cached, keyed by `named.len()` (the only thing
    /// that changes when the SET of graph IRIs changes); a length change rebuilds it.
    pub fn for_named_graphs_with_prefix<F: FnMut(&Term, &Graph)>(&self, prefix: &str, mut f: F) {
        let n = self.named.len();
        if n == 0 {
            return;
        }
        // Empty prefix matches everything — no point building/consulting the index.
        if prefix.is_empty() {
            for (name, sub) in &self.named {
                f(name, sub);
            }
            return;
        }
        // Collect the matching `named` indices UNDER the lock, then drop the guard BEFORE
        // invoking the caller-provided `f`. Holding the mutex across `f` would needlessly
        // serialize concurrent readers and could deadlock if `f` (directly or indirectly)
        // re-entered a path that locks the same mutex. The snapshot of matches is taken
        // while the lock is held, so correctness is preserved.
        let matches: Vec<u32> = {
            let mut guard = self.graph_prefix_index.lock().unwrap_or_else(|e| e.into_inner());
            let order: &Vec<u32> = match guard.as_ref() {
                Some((len, idx)) if *len == n => idx,
                _ => {
                    // (Re)build: a permutation of `named` indices sorted by `STR(name)` bytes.
                    let mut idx: Vec<u32> = (0..n as u32).collect();
                    idx.sort_by(|&a, &b| {
                        Self::graph_name_str(&self.named[a as usize].0)
                            .cmp(Self::graph_name_str(&self.named[b as usize].0))
                    });
                    *guard = Some((n, idx));
                    &guard.as_ref().unwrap().1
                }
            };
            // Range scan: STRSTARTS(s, prefix) holds for a CONTIGUOUS block of the sorted order
            // (all strings >= prefix and < the prefix's successor). `partition_point` gives the
            // lower bound; we then walk while the name still starts with `prefix`.
            let key = |i: u32| Self::graph_name_str(&self.named[i as usize].0);
            let lo = order.partition_point(|&i| key(i) < prefix);
            let mut matches = Vec::new();
            for &i in &order[lo..] {
                let s = key(i);
                if !s.starts_with(prefix) {
                    break; // sorted: once a name no longer starts with prefix, none after it does
                }
                matches.push(i);
            }
            matches
            // `guard` dropped here, before `f` is invoked below.
        };
        for i in matches {
            let (name, sub) = &self.named[i as usize];
            f(name, sub);
        }
    }

    /// [OPUS-4.8] (sq-quuu) Returns the named graph whose name term is exactly `name`, or
    /// `None` if this dataset has no such named graph. Each named graph is itself a
    /// self-contained [`Graph`] (its own dictionary + permutation indexes), so the returned
    /// `&Graph` can be passed anywhere a `&Graph` is accepted — in particular to the read-only
    /// GenAI crates (`sparq-introspect`, `sparq-sim`, `sparq-vectors`), which operate over the
    /// store of whatever `Graph` they are handed. This is the per-name way to scope those crates
    /// to a single named graph instead of the (default-graph) `self`.
    ///
    /// It is the per-name companion to
    /// [`for_named_graphs_with_prefix`](Self::for_named_graphs_with_prefix) (which is
    /// prefix-scoped). It is a linear scan over the `named` Vec (a dataset's distinct graphs are
    /// typically few); for a prefix sweep use the indexed prefix method instead.
    ///
    /// The DEFAULT graph is `self` itself — it is not part of `named` and is not returned here;
    /// pass `&graph` directly to scope introspect/sim/vectors to the default graph.
    #[inline]
    pub fn named_graph(&self, name: &Term) -> Option<&Graph> {
        self.named.iter().find(|(n, _)| n == name).map(|(_, g)| g)
    }

    /// Streaming loader: parses an RDF document incrementally from a reader (so a
    /// gzip / bzip2 decompression stream can be ingested without holding the whole
    /// document in memory). Same formats as [`load_str`](Self::load_str). The
    /// dictionary and triple buffer still grow in memory, so the full store only
    /// fits for datasets that fit in RAM; the streaming ingest *throughput*,
    /// however, is measurable on arbitrarily large inputs (see `sparq-cli ingest`).
    pub fn load_reader<R: std::io::Read>(reader: R, format: &str) -> Result<Graph, String> {
        let mut dict = Dict::new();
        let mut triples: Vec<[Id; 3]> = Vec::new();

        macro_rules! push_triple {
            ($s:expr, $p:expr, $o:expr) => {{
                let s = dict.intern(&subject_term($s));
                let p = dict.intern(&Term::NamedNode($p.clone()));
                let o = dict.intern($o);
                triples.push([s, p, o]);
            }};
        }

        match format {
            "nquads" | "n-quads" => {
                for q in NQuadsParser::new().for_reader(reader) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            "trig" | "application/trig" => {
                for q in TriGParser::new().for_reader(reader) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            "turtle" | "ttl" => {
                for t in TurtleParser::new().for_reader(reader) {
                    let t = t.map_err(|e| e.to_string())?;
                    push_triple!(&t.subject, &t.predicate, &t.object);
                }
            }
            // [OPUS-4.8] sq-dvyi: JSON-LD over a streaming reader (the CLI's file path).
            // Graph name folded — `load_reader` is the triple-only entry point.
            // OPT-IN behind the `jsonld` feature (keeps the lean bundle free of oxjsonld).
            #[cfg(feature = "jsonld")]
            _ if is_jsonld_format(format) => {
                for q in JsonLdParser::new().for_reader(reader) {
                    let q = q.map_err(|e| e.to_string())?;
                    push_triple!(&q.subject, &q.predicate, &q.object);
                }
            }
            _ => {
                for t in NTriplesParser::new().for_reader(reader) {
                    let t = t.map_err(|e| e.to_string())?;
                    push_triple!(&t.subject, &t.predicate, &t.object);
                }
            }
        }

        Ok(Self::build(dict, triples))
    }

    /// Streaming PARALLEL load: reads the (already-decompressed) `reader` in newline-aligned
    /// ~32 MiB blocks and parses each block in parallel, so the full decompressed document is
    /// NEVER materialised in memory — only a few blocks in flight plus the growing
    /// dictionary/triples. (The store itself must fit in RAM; this removes the redundant
    /// full-text copy a read-to-string load would hold alongside it.) For N-Triples; other
    /// formats defer to the serial streaming `load_reader`.
    ///
    /// The read/decompress runs PIPELINED on its own thread (same 3-stage design as the
    /// external build's `build_external_ntriples_parallel`): stage 1 fills full 32 MiB
    /// blocks from the reader (looping over short `read()`s — a gzip/zstd decompressor
    /// returns 0.4–1.6 MB per call, and flushing a parse+merge round per `read()` was the
    /// measured 3.5–5× streaming-ingest slowdown), stage 2 parses each block on the rayon
    /// pool, stage 3 (the caller's thread) merges dictionaries — so decode overlaps
    /// parse+merge and streaming ingest approaches max(decode, parse).
    #[cfg(feature = "parallel")]
    pub fn load_reader_parallel<R: std::io::Read + Send>(reader: R, format: &str) -> Result<Graph, String> {
        if !matches!(format, "ntriples" | "n-triples") {
            return Self::load_reader(reader, format);
        }
        Self::load_ntriples_pipelined(reader, 32 << 20)
    }

    /// The pipelined N-Triples loader behind [`load_reader_parallel`], with the block size
    /// as a parameter so tests can exercise the multi-block / boundary-straddling paths
    /// cheaply (production always passes 32 MiB).
    #[cfg(feature = "parallel")]
    fn load_ntriples_pipelined<R: std::io::Read + Send>(reader: R, block_size: usize) -> Result<Graph, String> {
        use std::sync::mpsc::sync_channel;
        // ≥2 rayon threads: ONE sharded dict spans all blocks, so the per-block dict merge
        // — the measured serial `merge_remap` that capped load scaling at ~1.8× on 4
        // identical cores — runs parallel across hash-shards; triples carry temporary
        // sharded ids until one parallel final remap. On one thread the proven serial
        // merge is kept (no sharding overhead).
        let sharded = rayon::current_num_threads() > 1;
        let mut sd = dict::ShardedDict::new(if sharded { default_shards() } else { 1 });
        let mut global = Dict::new();
        let mut all: Vec<[Id; 3]> = Vec::new();
        // Raw blocks flow read-thread -> parse; parsed partials flow parse-thread -> merge
        // (this thread). Bounds of 1 keep at most ~3 blocks (+1 block's partials) in
        // flight while letting decode, parse and merge overlap.
        let (tx, rx) = sync_channel::<Vec<u8>>(1);
        type Partials = Vec<(Dict, Vec<[Id; 3]>)>;
        let (ptx, prx) = sync_channel::<Partials>(1);
        std::thread::scope(|scope| -> Result<(), String> {
            // Stage 1 — read (the caller's decompressor) on its own thread, emitting
            // newline-aligned FULL blocks: loop `read()` until the block is full or EOF.
            let producer = scope.spawn(move || -> Result<(), String> {
                let mut reader = reader;
                let mut readbuf = vec![0u8; block_size];
                let mut carry: Vec<u8> = Vec::new();
                loop {
                    let mut filled = 0;
                    while filled < block_size {
                        let n = reader.read(&mut readbuf[filled..]).map_err(|e| e.to_string())?;
                        if n == 0 {
                            break;
                        }
                        filled += n;
                    }
                    if filled == 0 {
                        // EOF: a final line without a trailing newline lives in `carry`.
                        if !carry.is_empty() {
                            let _ = tx.send(std::mem::take(&mut carry));
                        }
                        return Ok(());
                    }
                    // Emit `carry + readbuf[..filled]` up to the last newline; carry the
                    // remainder (a partial line split across the block boundary) forward.
                    let mut block = std::mem::take(&mut carry);
                    block.extend_from_slice(&readbuf[..filled]);
                    let cut = block.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
                    carry = block[cut..].to_vec();
                    block.truncate(cut);
                    if !block.is_empty() && tx.send(block).is_err() {
                        return Ok(()); // a downstream stage errored and dropped the receiver
                    }
                }
            });
            // Stage 2 — parse+intern each block in parallel (per-chunk local dicts, no
            // shared state), forwarding the partials to the merge stage.
            let parser = scope.spawn(move || -> Result<(), String> {
                for block in rx {
                    let partials = parse_block(&block)?;
                    if ptx.send(partials).is_err() {
                        return Ok(()); // the merge stage errored and dropped the receiver
                    }
                }
                Ok(())
            });
            // Stage 3 (this thread) — dict merge. Blocks arrive in document order, so id
            // assignment matches the non-pipelined load (deterministic).
            for partials in prx {
                if sharded {
                    // [OPUS-4.8] Propagate a malformed-triple-term error (the streaming sharded
                    // path is not triple-term-guarded) instead of panicking.
                    sharded_extend(&mut sd, &partials, &mut all)?;
                } else {
                    for (pd, pt) in partials {
                        let remap = global.merge_remap(&pd);
                        remap_extend(&mut all, pt, &remap);
                    }
                }
            }
            // Join parse first (it feeds stage 3 — surface a parse error), then the producer.
            parser.join().map_err(|_| "parse thread panicked".to_string())??;
            producer.join().map_err(|_| "read thread panicked".to_string())?
        })?;
        if sharded {
            let (dict, ids) = finish_sharded(sd, all);
            return Ok(Self::build(dict, ids));
        }
        Ok(Self::build(global, all))
    }

    /// Builds the store + numeric cache from interned triples (shared by the
    /// string and streaming loaders).
    fn build(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        let store = TripleStore::from_triples(triples);
        let numerics = NumData::Owned(numerics_of(&dict));
        // Unlike the numerics cache (dense f64 = 8 B/term), the dense temporal cells
        // are 16 B/term — go sparse straight away when temporals are rare (usually:
        // none at all -> an empty map, zero memory), keeping the load-time memory
        // metric flat for non-temporal datasets. Temporal-heavy data stays dense.
        let temporals = TempData::Owned(temporals_of(&dict)).into_sparse_if_worthwhile();
        Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: Vec::new(),
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) In-memory graph: no parent-level redo journal.
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }

    /// Like [`load_str`](Self::load_str) but stores the permutation indexes
    /// BLOCK-COMPRESSED (~4-6 B/triple vs 12) — the memory-bound build for the browser,
    /// trading a bounded per-scan decode for ~2.5x more triples per byte of RAM. Query
    /// results are identical to the raw build.
    pub fn load_str_compressed(text: &str, format: &str) -> Result<Graph, String> {
        let g = Self::load_str(text, format)?;
        Ok(g.into_compressed())
    }

    /// Re-encodes into the memory-bound storage mode: the permutations BLOCK-COMPRESSED and
    /// the dictionary's id→term storage compacted to a single BLOB (no per-term `Box<str>`).
    /// Keeps the numeric cache and term ids. The browser/RAM-constrained path; identical
    /// query results, a small per-scan decode.
    pub fn into_compressed(self) -> Graph {
        let triples: Vec<[Id; 3]> = {
            let scan = self.store.scan(&[None, None, None]);
            scan.rows.iter().map(|r| scan.to_spo(r)).collect()
        };
        Graph {
            store: TripleStore::from_triples_compressed(triples),
            dict: self.dict.into_blob(),
            // The numeric cache is mostly (often entirely) NaN — keep only the real
            // numeric literals when sparse, freeing the dense f64-per-term Vec.
            numerics: self.numerics.into_sparse_if_worthwhile(),
            temporals: self.temporals.into_sparse_if_worthwhile(),
            // sq-lr2ii: re-encoding keeps the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named,
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: self.wal,
            // [OPUS-4.8] (sq-ycle) Re-encoding keeps the directory association — carry the redo
            // journal handle with the WAL, so a compressed graph stays atomically durable.
            #[cfg(feature = "mmap")]
            txn: self.txn,
        }
    }

    /// Persists the graph to `dir` (the permutation indexes + the dictionary) so it can
    /// later be QUERIED with the indexes MEMORY-MAPPED via [`open`](Self::open) — the
    /// out-of-core path for datasets larger than RAM.
    ///
    /// [OPUS-4.8] (sq-3ui0, gh-45) NAMED GRAPHS are persisted too: each named graph is
    /// saved (recursively, identical format) under `dir/named/<i>/`, and a manifest
    /// `dir/named.bin` records every graph's name term in index order. `open` replays the
    /// manifest so a save→open round-trip is LOSSLESS for the whole dataset (every named
    /// graph's triples + its IRI). A default-graph-only graph writes NO `named/` subtree
    /// and NO `named.bin`, so its on-disk layout is byte-identical to before this change.
    #[cfg(feature = "mmap")]
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.store.save(dir)?; // folds any delta-overlay into the persisted permutations
        self.dict.save_mmap(dir)?; // includes appended (delta-overlay) terms
        // [OPUS-4.8] (sq-7ph8) STREAM the numeric/temporal caches block-by-block straight from
        // the in-RAM cache instead of materialising a whole-dictionary dense intermediate
        // (`dense_numerics`/`dense_temporals`) first — bounding the finalize RSS peak for a
        // SPARSE/FORKED cache (the common non-numeric/non-temporal case).
        let n = self.dict.len();
        stream_write_numerics(&dir.join("numerics.bin"), n, &self.numerics)?;
        stream_write_temporals(&dir.join("temporals.bin"), n, &self.temporals)?;
        self.save_named(dir, false)
    }

    /// Like [`save`](Self::save) but persists the permutation indexes BLOCK-COMPRESSED
    /// (~3-5x smaller on disk; the dictionary/numerics files are unchanged). `open`
    /// auto-detects the format per file, serving compressed perms by lazy block-wise
    /// decode off the mapped file; call [`decompress_indexes`](Self::decompress_indexes)
    /// after open to trade RAM for exactly-raw query speed instead.
    #[cfg(feature = "mmap")]
    pub fn save_compressed(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.store.save_compressed(dir)?; // folds any delta-overlay, like `save`
        self.dict.save_mmap(dir)?;
        // [OPUS-4.8] (sq-7ph8) Stream the caches block-by-block — see `save` for the rationale.
        let n = self.dict.len();
        stream_write_numerics(&dir.join("numerics.bin"), n, &self.numerics)?;
        stream_write_temporals(&dir.join("temporals.bin"), n, &self.temporals)?;
        // [OPUS-4.8] (sq-3ui0) Named graphs are persisted block-compressed too.
        self.save_named(dir, true)
    }

    /// [OPUS-4.8] (sq-3ui0, gh-45) Persists every named graph under `dir/named/<i>/` and
    /// writes the manifest `dir/named.bin` (the ordered list of graph-name terms). Each
    /// sub-graph goes through the SAME [`save`](Self::save) / [`save_compressed`] machinery
    /// (so it round-trips losslessly, recursively, and carries its own per-graph WAL on
    /// `open`). No `named/` subtree and no `named.bin` are written when there are no named
    /// graphs, so the default-graph-only directory layout is unchanged (byte-identical).
    ///
    /// STALE-STATE HYGIENE: when there ARE named graphs we first remove any pre-existing
    /// `named/` subtree so a re-save into the same directory cannot leave orphaned old
    /// sub-graphs behind; when there are none we remove both the subtree and the manifest
    /// so a graph that lost all its named graphs persists as a clean default-only directory.
    #[cfg(feature = "mmap")]
    fn save_named(&self, dir: &std::path::Path, compressed: bool) -> std::io::Result<()> {
        let named_dir = dir.join(NAMED_SUBDIR);
        let manifest = dir.join(NAMED_MANIFEST);
        if self.named.is_empty() {
            // Default-graph-only: ensure no stale named state lingers, then write nothing.
            std::fs::remove_dir_all(&named_dir).ok();
            std::fs::remove_file(&manifest).ok();
            return Ok(());
        }
        // Rewrite the subtree from scratch so removed/renamed graphs cannot survive a re-save.
        std::fs::remove_dir_all(&named_dir).ok();
        std::fs::create_dir_all(&named_dir)?;
        for (i, (_, sub)) in self.named.iter().enumerate() {
            let sub_dir = named_dir.join(i.to_string());
            if compressed {
                sub.save_compressed(&sub_dir)?;
            } else {
                sub.save(&sub_dir)?;
            }
        }
        // Write the manifest LAST (after every sub-graph is durable) so a crash mid-save
        // never leaves a manifest pointing at a missing/partial sub-graph. The presence of
        // a complete `named.bin` is the commit point for the named-graph set.
        let names: Vec<Term> = self.named.iter().map(|(n, _)| n.clone()).collect();
        write_named_manifest(dir, &names)
    }

    /// [OPUS-4.8] (sq-3ui0, gh-45) Loads the named graphs persisted by
    /// [`save_named`](Self::save_named) back into [`named`](Self::named), each opened via
    /// [`open`](Self::open) (so every sub-graph is memory-mapped and carries its OWN
    /// per-graph WAL). Returns an empty vec — backward compatible — when `dir` carries no
    /// `named.bin` (a default-graph-only directory, or one saved before this format). A
    /// present-but-malformed manifest (wrong magic / unknown version) is a hard error
    /// rather than a silent misread, so an incompatible on-disk format is never opened as
    /// if it were valid.
    #[cfg(feature = "mmap")]
    fn open_named(dir: &std::path::Path) -> std::io::Result<Vec<(Term, Graph)>> {
        let manifest = dir.join(NAMED_MANIFEST);
        let bytes = match std::fs::read(&manifest) {
            Ok(b) => b,
            // No manifest: default-graph-only or a pre-named-graph directory.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let bad = |msg: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("named.bin: {msg}"));
        if bytes.len() < 12 {
            return Err(bad("manifest too short for header"));
        }
        let magic = u32::from_le_bytes(bytes[0..4].try_into().expect("4 bytes"));
        if magic != NAMED_MANIFEST_MAGIC {
            return Err(bad("bad magic (not a named-graph manifest)"));
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
        if version != NAMED_FORMAT_VERSION {
            return Err(bad(&format!("unsupported format version {version} (this build writes/reads {NAMED_FORMAT_VERSION})")));
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes")) as usize;
        // [OPUS-4.8] (Copilot review #69, finding 3) `count` is attacker-controlled: a corrupt
        // or hostile manifest can declare a huge count behind a tiny file, and the old
        // `Vec::with_capacity(count)` would attempt a multi-GB reservation (OOM/abort) BEFORE
        // any per-entry bounds check ran. Each entry costs >= MIN_MANIFEST_ENTRY_BYTES on disk
        // (kind byte + u32 length prefix, zero-length value), so the post-header byte count is
        // a hard upper bound on how many entries can possibly follow — clamp the reservation to
        // it. The decode loop below still errors cleanly via `decode_graph_name`'s bounds checks
        // if the file actually ends early, so a count the file can't back is a clean
        // `InvalidData` (truncated manifest), never an OOM. Same defensive idiom as
        // `Dict::open_mmap` (dict.rs) and the rest of the corruption-hardened `open` path.
        let remaining = bytes.len() - 12;
        let cap = count.min(remaining / MIN_MANIFEST_ENTRY_BYTES);
        let named_dir = dir.join(NAMED_SUBDIR);
        let mut out = Vec::with_capacity(cap);
        let mut pos = 12usize;
        for i in 0..count {
            let (name, next) = decode_graph_name(&bytes, pos).map_err(|m| bad(&m))?;
            pos = next;
            let sub = Graph::open(&named_dir.join(i.to_string()))?;
            out.push((name, sub));
        }
        Ok(out)
    }

    /// Decodes any block-compressed permutation indexes into raw RAM (the load-time
    /// decompression mode for a compressed dir: one up-front decode, then scans are
    /// zero-cost slice borrows, identical to a raw store). No-op on raw/mapped indexes.
    pub fn decompress_indexes(&mut self) {
        self.store.decompress_to_ram();
    }

    /// Opens a graph saved by [`save`](Self::save) with its permutation indexes AND
    /// numeric-value cache MEMORY-MAPPED (paged in on demand) — so a large out-of-core
    /// dataset opens near-instantly without re-parsing every term, and the cache stays
    /// off the heap. The dictionary is loaded into RAM; the big indexes stay on disk. If
    /// `numerics.bin` is absent or stale (a graph saved before this cache existed), the
    /// cache is recomputed, preserving backward compatibility.
    #[cfg(feature = "mmap")]
    pub fn open(dir: &std::path::Path) -> std::io::Result<Graph> {
        // [OPUS-4.8] (review 1593) Finish or roll back any compaction directory swap that a
        // crash interrupted, so `dir` is always present (never lost in the rename window).
        recover_compaction(dir)?;
        // [OPUS-4.8] (sq-glw2) Finish or roll back any named-graph DROP sub-tree swap a crash
        // interrupted, so `named/` is always consistent with the manifest before `open_named`.
        recover_named_drop(dir)?;
        let store = TripleStore::open(dir)?;
        // [OPUS-4.8] (review 1325, sub-finding 2) Prefer the mmap dictionary format
        // (`dict-meta.bin`, written by `save_mmap`); fall back to the LEGACY single-file
        // `dict.bin` (written by the older `Dict::save`) so graph directories saved before
        // the mmap dictionary format still open. The legacy dict loads fully into RAM (no
        // mmap), which is the only difference — every term still round-trips.
        let dict = if dir.join("dict-meta.bin").exists() {
            Dict::open_mmap(dir)?
        } else {
            let legacy = dir.join("dict.bin");
            if legacy.exists() {
                Dict::open(&legacy)?
            } else {
                // Neither format present — surface the original mmap error (NotFound on
                // dict-meta.bin), matching the previous behaviour for a corrupt directory.
                Dict::open_mmap(dir)?
            }
        };
        let np = dir.join("numerics.bin");
        let numerics = match std::fs::File::open(&np) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * std::mem::size_of::<f64>() => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                NumData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            _ => NumData::Owned(numerics_of(&dict)),
        };
        let tp = dir.join("temporals.bin");
        let temporals = match std::fs::File::open(&tp) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * 9 => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                TempData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            // Absent or stale (a graph saved before this cache existed): recompute —
            // backward compatible, like the numerics cache.
            _ => TempData::Owned(temporals_of(&dict)),
        };
        // [OPUS-4.8] (sq-3ui0, gh-45) Restore the named graphs (each opened memory-mapped
        // with its own per-graph WAL) so a save→open round-trip is lossless for the whole
        // dataset. Empty for a default-graph-only / pre-named-graph directory.
        let named = Self::open_named(dir)?;
        let mut g = Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named,
            graph_prefix_index: std::sync::Mutex::new(None),
            wal: None,
            txn: None,
        };
        // Replay any write-ahead log into the delta-overlay (recovery after a crash or a
        // plain not-yet-compacted close), stopping cleanly at the first torn record and
        // truncating the log there — then keep the log open for further appends.
        for (insert, t) in Wal::replay(dir)? {
            if insert {
                g.apply_delta_mem(&[t], &[]);
            } else {
                g.apply_delta_mem(&[], &[t]);
            }
        }
        g.wal = Some(Wal::open(dir)?);
        // [OPUS-4.8] (sq-ycle) AFTER the per-graph WALs are replayed (above + per named graph in
        // `open_named`), redo any committed parent-level transaction frame, idempotently. A
        // multi-op UPDATE body's resolved per-slot quad-delta is one atomic `txn.log` frame; if a
        // crash interrupted materialisation after that single fsync, the per-graph WALs hold only a
        // PREFIX of the body — replaying the frame heals the desync. Re-applying records the WALs
        // already materialised is a no-op: `apply_delta` replays with set semantics (re-inserting a
        // present triple / deleting an absent one changes nothing) and `ensure_named` is
        // find-or-create.
        //
        // [OPUS-4.8] (Copilot #135) DURABILITY ORDER (load-bearing): the records MUST be
        // MATERIALISED INTO THE DURABLE PER-GRAPH WAL (`redo_txn_record` -> `apply_delta`, which
        // appends + fsyncs) BEFORE the `txn.log` is truncated. The earlier code applied them with
        // `apply_delta_mem` (IN-MEMORY only) and then truncated the journal, so a record that lived
        // ONLY in `txn.log` — the precise crash window the journal exists for — was applied to this
        // process's memory but written nowhere durable, then ERASED by the truncation, so it did NOT
        // survive the NEXT restart. Re-logging into the WAL first (then truncate) makes the recovered
        // state durable across any number of restarts; a re-crash between WAL-redo and truncation
        // simply redoes the (idempotent) frame again on the next open.
        let txn_records = TxnJournal::replay(dir)?;
        for (insert, slot, t) in txn_records {
            g.redo_txn_record(insert, slot, t).map_err(std::io::Error::other)?;
        }
        // Every committed record is now durably in a per-graph WAL (fsync'd by `apply_delta`); only
        // NOW is it safe to truncate the journal that was the sole durable home of those records.
        let mut journal = TxnJournal::open(dir)?;
        journal.truncate()?;
        g.txn = Some(journal);
        Ok(g)
    }

    /// [OPUS-4.8] (sq-ycle) Idempotently re-apply one redo-journal record on `open`. A default-slot
    /// record goes to this graph's default store; a named-slot record is routed to the matching
    /// named sub-graph (find-or-create via [`ensure_named`](Self::ensure_named), so a frame whose
    /// child-graph creation crashed before materialisation still rebuilds the graph). Set-semantic:
    /// re-applying a change the per-graph WAL already materialised is a no-op.
    ///
    /// [OPUS-4.8] (Copilot #135) DURABILITY: the redo goes through the WAL-logged + fsync'd
    /// [`apply_delta`](Self::apply_delta) (NOT the in-memory-only `apply_delta_mem`), so a record
    /// that lived ONLY in `txn.log` — the exact crash window the journal exists for (crash AFTER
    /// `commit_txn` fsync, BEFORE per-graph WAL materialisation) — is MATERIALISED into the durable
    /// per-graph WAL on recovery. The caller truncates `txn.log` only AFTER every record has been
    /// re-logged this way (see [`open`](Self::open)), so the recovered state survives ANY number of
    /// subsequent restarts. Re-logging a record the per-graph WAL already holds is harmless: the WAL
    /// replays with set semantics (re-inserting a present triple / deleting an absent one is a
    /// no-op), so a re-crash mid-redo simply redoes the (idempotent) frame again.
    #[cfg(feature = "mmap")]
    fn redo_txn_record(&mut self, insert: bool, slot: Option<Term>, t: [Term; 3]) -> Result<(), String> {
        match slot {
            None => {
                if insert {
                    self.apply_delta(&[t], &[])?;
                } else {
                    self.apply_delta(&[], &[t])?;
                }
            }
            Some(name) => {
                // An insert into an absent named graph must materialise the graph (find-or-create);
                // a delete from an absent one is a no-op (nothing to retract).
                let idx = if insert {
                    Some(self.ensure_named(&name)?)
                } else {
                    self.named.iter().position(|(n, _)| *n == name)
                };
                if let Some(i) = idx {
                    if insert {
                        self.named[i].1.apply_delta(&[t], &[])?;
                    } else {
                        self.named[i].1.apply_delta(&[], &[t])?;
                    }
                }
            }
        }
        Ok(())
    }

    /// EXTERNAL-MEMORY build: streams an RDF document and writes the on-disk permutation
    /// indexes + dictionary directly, sorting the triples through disk-backed runs so the
    /// dataset's indexes can be CONSTRUCTED without ever holding them all in RAM. Only one
    /// `chunk`-sized buffer of triples (plus the growing dictionary) is resident at a time;
    /// the rest lives in sorted run files that are k-way merged. The result is identical to
    /// `save()` of an in-memory `load`, but bounded-memory — the billion-scale ingest path.
    /// Open it with [`open`](Self::open).
    ///
    /// `chunk` is the number of triples per in-memory run (e.g. 8_000_000 ≈ 96 MB of ids).
    #[cfg(feature = "mmap")]
    pub fn build_external<R: std::io::Read + Send>(
        reader: R,
        format: &str,
        dir: &std::path::Path,
        chunk: usize,
    ) -> Result<(), String> {
        // The SPILLED dictionary (`dict-spill` feature, env-gated via SPARQ_DICT_SPILL):
        // bounded-RSS builds for dictionaries larger than RAM. Output is byte-identical
        // to the (default) sharded path; N-Triples only, like the sharded path.
        #[cfg(feature = "dict-spill")]
        if matches!(format, "ntriples" | "n-triples") {
            if let Some(cfg) = dictspill::SpillConfig::from_env() {
                return Self::build_external_spill(reader, format, dir, chunk, &cfg);
            }
        }
        // The SHARDED-dict N-Triples ingest (parallel dict consolidation) is the DEFAULT
        // when it can run (parallel build, >1 rayon thread); `SPARQ_SHARDED_DICT=0` (or
        // `off`) opts out to the serial-merge path, `=1` keeps forcing it on (its id
        // assignment needs ≥2 threads' shard count to be meaningfully parallel, but it is
        // correct on any). The on-disk FORMAT is identical either way (same writers, same
        // record layouts — only term-id ASSIGNMENT differs), so no format-version bump:
        // stores built by either path open interchangeably.
        #[cfg(feature = "parallel")]
        let sharded = match std::env::var("SPARQ_SHARDED_DICT").as_deref() {
            Ok("0") | Ok("off") => false,
            Ok(_) => true,
            Err(_) => rayon::current_num_threads() > 1,
        };
        #[cfg(not(feature = "parallel"))]
        let sharded = false;
        Self::build_external_opts(reader, format, dir, chunk, sharded)
    }

    /// [`build_external`](Self::build_external) with the sharded-dict choice EXPLICIT
    /// (no env lookup) — for tests and embedders that need a specific path.
    #[cfg(feature = "mmap")]
    pub fn build_external_opts<R: std::io::Read + Send>(
        reader: R,
        format: &str,
        dir: &std::path::Path,
        chunk: usize,
        sharded: bool,
    ) -> Result<(), String> {
        use store::{Perm, BUILT};
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join("tmp");
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

        let mut dict = Dict::new();
        let mut buf: Vec<[Id; 3]> = Vec::with_capacity(chunk);
        let mut runs: Vec<std::path::PathBuf> = Vec::new();
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        let _t_build = std::time::Instant::now();
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        build_timing::reset();

        // Stream-parse + intern, spilling SPO-sorted runs to disk whenever the buffer fills.
        macro_rules! push_triple {
            ($s:expr, $p:expr, $o:expr) => {{
                let s = dict.intern(&subject_term($s));
                let p = dict.intern(&Term::NamedNode($p.clone()));
                let o = dict.intern($o);
                buf.push([s, p, o]);
                if buf.len() >= chunk {
                    extsort::spill_run(&mut buf, &mut runs, &tmp).map_err(|e| e.to_string())?;
                }
            }};
        }

        // PARALLEL sharded-dict ingest for N-Triples (the default — see `build_external`):
        // interns through N hash-shards (no serial `merge_remap`), spilling temporary
        // sharded ids that an order-preserving remap turns into final dense ids after the
        // SPO sort. When opted out (or for other formats / non-parallel builds), the
        // serial-merge path runs.
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        let sharded = matches!(format, "ntriples" | "n-triples") && sharded;
        #[cfg(not(all(feature = "mmap", feature = "parallel")))]
        let sharded = {
            let _ = sharded;
            false
        };
        let mut sharded_remap: Option<(Vec<u64>, u32)> = None;
        // [OPUS-4.8 sq-3l43] Serial dict-consolidation seconds (`into_merged`) on the sharded
        // default path, folded into the phase report so the FULL dict-consolidation bucket
        // (the one this rung-5 spike re-measures) appears on one honestly-labelled line.
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        let mut cons_secs: Option<f64> = None;

        if sharded {
            #[cfg(all(feature = "mmap", feature = "parallel"))]
            {
                let mut sd = dict::ShardedDict::new(default_shards());
                build_external_ntriples_sharded(reader, &mut sd, &mut buf, &mut runs, &tmp, chunk)?;
                let t_cons = std::time::Instant::now();
                let (merged, base, stride) = sd.into_merged();
                let elapsed = t_cons.elapsed().as_secs_f64();
                if build_timing::enabled() {
                    eprintln!("[build-timing] dict consolidate (into_merged): {elapsed:.2}s");
                    cons_secs = Some(elapsed);
                }
                dict = merged;
                sharded_remap = Some((base, stride));
            }
        } else {
            match format {
                "nquads" | "n-quads" => {
                    for q in NQuadsParser::new().for_reader(reader) {
                        let q = q.map_err(|e| e.to_string())?;
                        push_triple!(&q.subject, &q.predicate, &q.object);
                    }
                }
                "trig" | "application/trig" => {
                    for q in TriGParser::new().for_reader(reader) {
                        let q = q.map_err(|e| e.to_string())?;
                        push_triple!(&q.subject, &q.predicate, &q.object);
                    }
                }
                "turtle" | "ttl" => {
                    for t in TurtleParser::new().for_reader(reader) {
                        let t = t.map_err(|e| e.to_string())?;
                        push_triple!(&t.subject, &t.predicate, &t.object);
                    }
                }
                _ => {
                    // N-Triples is the billion-scale bulk format: parse it with the custom
                    // byte parser over PARALLEL buffers (per-buffer partial dicts merged into
                    // the running dict), the user-requested "parallelise parsing of the file".
                    // Still bounded-memory: one ~64 MiB buffer + its partials at a time.
                    #[cfg(feature = "parallel")]
                    build_external_ntriples_parallel(reader, &mut dict, &mut buf, &mut runs, &tmp, chunk)?;
                    #[cfg(not(feature = "parallel"))]
                    for t in NTriplesParser::new().for_reader(reader) {
                        let t = t.map_err(|e| e.to_string())?;
                        push_triple!(&t.subject, &t.predicate, &t.object);
                    }
                }
            }
        }
        extsort::spill_run(&mut buf, &mut runs, &tmp).map_err(|e| e.to_string())?;
        buf.shrink_to_fit();
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        if build_timing::enabled() {
            // [OPUS-4.8 sq-3l43] The sharded default path's merge/remap buckets are PIPELINED
            // (parallel intern + overlapped remap), NOT the additive-serial `merge_remap`/
            // `triple-remap` of the non-sharded parallel build — label them accordingly so the
            // dict-consolidation bucket re-measurement reads correctly.
            let (kind, cons) = if sharded {
                (build_timing::PathKind::Pipelined, cons_secs)
            } else {
                (build_timing::PathKind::Serial, None)
            };
            build_timing::report("parse+intern+spill done", kind, cons, _t_build.elapsed().as_secs_f64());
        }

        // Merge the SPO runs into the SPO permutation file (deduplicating).
        let spo_path = dir.join(format!("perm{}.bin", Perm::Spo as usize));
        extsort::kway_merge(&runs, &spo_path).map_err(|e| e.to_string())?;
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        if build_timing::enabled() {
            eprintln!("[build-timing] kway_merge SPO done | {:.2}s wall to here", _t_build.elapsed().as_secs_f64());
        }
        for r in &runs {
            std::fs::remove_file(r).ok();
        }
        // Sharded build: the SPO perm holds TEMPORARY sharded ids — remap them to final dense
        // ids in place (order-preserving, so it stays sorted+deduped) BEFORE the sibling sorts
        // read it, so the permutations and the merged dictionary agree on ids.
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        if let Some((base, stride)) = &sharded_remap {
            remap_perm_file(&spo_path, base, *stride)?;
        }

        // Build the other BUILT permutations by external-sorting the SPO file into each
        // order (re-reading it memory-mapped, so this stays bounded-memory too).
        let (map, n) = extsort::map_perm(&spo_path).map_err(|e| e.to_string())?;
        // SAFETY: perm0 is a whole number of [u32;3] rows written above; map outlives the loop.
        let spo: &[[Id; 3]] =
            unsafe { std::slice::from_raw_parts(map.as_ptr().cast::<[Id; 3]>(), n) };
        let siblings: Vec<Perm> = BUILT.iter().copied().filter(|&p| p != Perm::Spo).collect();
        // [OPUS-4.8] sq-vkz7: opt-in compressed build — the sibling sorts write `SPQCPRM1`
        // straight from their merge tail, and SPO is re-encoded below. SPO itself must stay
        // RAW until the siblings finish, because they re-sort by mmapping it as `[[Id;3]]`.
        let compressed = build_compressed_perms();
        let sib_sort = |perm: Perm, sub: &std::path::Path, per: usize| -> Result<(), String> {
            std::fs::create_dir_all(sub).map_err(|e| e.to_string())?;
            let out = dir.join(format!("perm{}.bin", perm as usize));
            if compressed {
                extsort::external_sort_compressed(spo.iter().copied(), perm.order(), &out, sub, per)
                    .map_err(|e| e.to_string())
            } else {
                extsort::external_sort(spo.iter().copied(), perm.order(), &out, sub, per).map_err(|e| e.to_string())
            }
        };
        // The sibling sorts are independent — run them CONCURRENTLY (each in its own tmp
        // subdir, so run files don't collide), sharing the chunk budget so total resident
        // memory stays ~`chunk`. The shared SPO mmap is read-only (paged, no extra RAM).
        // Persisting the DICTIONARY (save_mmap: the term blob + sorted-hash index) and the
        // numeric cache only needs `dict` — it is INDEPENDENT of the permutation sorts. Run
        // it CONCURRENTLY with the sibling sorts (on its own thread) so the multi-hundred-MB
        // dict write is hidden under the sort time instead of being a serial tail. Output is
        // byte-identical (same files); only the wall-clock ordering overlaps.
        std::thread::scope(|scope| -> Result<(), String> {
            let dict_ref = &dict;
            let finalize = scope.spawn(move || -> Result<(), String> {
                dict_ref.save_mmap(dir).map_err(|e| e.to_string())?;
                write_numerics(&dir.join("numerics.bin"), &numerics_of(dict_ref)).map_err(|e| e.to_string())?;
                let (tf, ti) = {
                    let cells = temporals_of(dict_ref);
                    (cells.iter().map(|c| c.flag).collect::<Vec<u8>>(), cells.iter().map(|c| c.instant).collect::<Vec<f64>>())
                };
                write_temporals(&dir.join("temporals.bin"), &tf, &ti).map_err(|e| e.to_string())?;
                Ok(())
            });
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;
                let per = (chunk / siblings.len().max(1)).max(1 << 16);
                siblings
                    .par_iter()
                    .try_for_each(|&perm| sib_sort(perm, &tmp.join(format!("p{}", perm as usize)), per))?;
            }
            #[cfg(not(feature = "parallel"))]
            for &perm in &siblings {
                sib_sort(perm, &tmp, chunk)?;
            }
            finalize.join().map_err(|_| "dict-finalize thread panicked".to_string())??;
            Ok(())
        })?;
        drop(map);
        #[cfg(all(feature = "mmap", feature = "parallel"))]
        if build_timing::enabled() {
            eprintln!("[build-timing] sibling sorts ∥ dict-save done | {:.2}s wall to here", _t_build.elapsed().as_secs_f64());
        }

        // [OPUS-4.8] sq-vkz7: the siblings are already `SPQCPRM1`; compress SPO LAST, now
        // that no sibling sort needs it raw. One streaming pass over the raw SPO perm (the
        // 1/6 we had to keep raw) — never the `open` + `decode_all` of ALL SIX perms the
        // separate `recompress` would do.
        if compressed {
            compress_perm_file_in_place(&spo_path)?;
        }

        // Empty files for the unbuilt permutations so `open` finds all six slots.
        for i in 0..6 {
            let p = dir.join(format!("perm{i}.bin"));
            if !p.exists() {
                std::fs::File::create(&p).map_err(|e| e.to_string())?;
            }
        }
        // Compute per-predicate stats once (a one-time POS/PSO scan) and persist them so
        // query-open never re-scans those indexes — keeping out-of-core open fast + small.
        let store = TripleStore::open(dir).map_err(|e| e.to_string())?;
        store.save_pred_stats(dir).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// [OPUS-4.8] (sq-5atq) EXTERNAL-MEMORY build for a DATASET WITH NAMED GRAPHS — the
    /// out-of-core twin of an in-RAM dataset `load_dataset` + [`save`](Self::save). Streams an
    /// N-Quads (`"nquads"`/`"n-quads"`) or TriG (`"trig"`) document and writes the SAME
    /// on-disk layout `save_named` emits — the default graph in `dir`
    /// itself plus each named graph under `dir/named/<i>/`, committed by the `dir/named.bin`
    /// manifest — so [`open`](Self::open) reads the whole dataset back LOSSLESSLY (mmap).
    ///
    /// BOUNDED MEMORY THROUGHOUT. We do NOT build the dataset in RAM. Instead the quad
    /// stream is read ONCE and PARTITIONED BY GRAPH NAME into per-graph on-disk N-Triples
    /// spill files (only the parser's buffer + small per-graph write buffers are resident);
    /// then each graph's spill is built INDEPENDENTLY through the existing single-graph
    /// external pipeline ([`build_external`](Self::build_external) → external SPO sort,
    /// disk-backed runs, k-way merge), each into its own directory with its own dictionary —
    /// exactly the per-sub-graph `save()` that `save_named` produces. Peak RAM is therefore
    /// bounded by the per-graph external build's `chunk`, not by the dataset size.
    ///
    /// Re-serialising each quad's triple to canonical N-Triples for the spill is LOSSLESS:
    /// the same escaping the parser accepts, blank-node labels preserved verbatim (and
    /// blank-node scope is per-graph in N-Quads/TriG, so a relabel-free round-trip through a
    /// single per-graph file preserves identity exactly as the in-RAM dataset loader does).
    ///
    /// Graph names are kept in FIRST-OCCURRENCE document order — the same order the in-RAM
    /// dataset loaders populate [`named`](Self::named) and that `save_named` then writes — so
    /// the on-disk `named/<i>/` indices + manifest match the in-RAM `save_named` path. The
    /// default-graph-only case writes no `named/` subtree and no manifest (byte-identical to
    /// [`build_external`](Self::build_external)), and an empty default graph still produces a
    /// valid (empty) store in `dir`.
    ///
    /// `chunk` is the per-graph external-build run size (see `build_external`).
    #[cfg(feature = "mmap")]
    pub fn build_external_quads<R: std::io::Read>(
        reader: R,
        format: &str,
        dir: &std::path::Path,
        chunk: usize,
    ) -> Result<(), String> {
        use oxrdf::{GraphName, TripleRef};
        use std::io::Write;

        // VALIDATE CHEAP PRECONDITIONS BEFORE ANY DESTRUCTIVE FILESYSTEM MUTATION.
        // The stale-state hygiene below removes a prior build's named subtree/manifest; if we
        // did that first and only THEN rejected an unsupported `format`, we would have already
        // destroyed an existing dataset at `dir` (data loss) while returning `Err`. So the
        // format check happens up-front, before `create_dir_all`/`remove_dir_all`.
        if !matches!(
            format,
            "nquads" | "n-quads" | "trig" | "application/trig"
        ) {
            return Err(format!(
                "build_external_quads: unsupported quad format {format:?} (expected nquads or trig)"
            ));
        }

        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

        // Per-graph N-Triples spill directory. A scope guard removes it on EVERY exit path
        // (parse error, IO error, per-graph build error, or success) so a huge spill is never
        // leaked when the build aborts mid-way. `disarm()` is called only after the manifest
        // is committed, at which point the explicit removal below is the normal cleanup.
        let spill_dir = dir.join("quads-spill");
        std::fs::remove_dir_all(&spill_dir).ok();
        std::fs::create_dir_all(&spill_dir).map_err(|e| e.to_string())?;
        struct SpillGuard {
            dir: std::path::PathBuf,
            armed: bool,
        }
        impl SpillGuard {
            fn disarm(&mut self) {
                self.armed = false;
            }
        }
        impl Drop for SpillGuard {
            fn drop(&mut self) {
                if self.armed {
                    std::fs::remove_dir_all(&self.dir).ok();
                }
            }
        }
        let mut spill_guard = SpillGuard { dir: spill_dir.clone(), armed: true };

        // Stale-state hygiene mirroring `save_named`: a re-build into the same directory must
        // not inherit a previous build's named subtree/manifest. This is destructive, so it
        // runs AFTER the format validation above.
        std::fs::remove_dir_all(dir.join(NAMED_SUBDIR)).ok();
        std::fs::remove_file(dir.join(NAMED_MANIFEST)).ok();

        // First-occurrence order of named-graph names, and a BOUNDED pool of open writers.
        // The default graph is `default.nt`; named graph `i` (in first-occurrence order) is
        // `g<i>.nt`.
        //
        // FD BOUNDING: the out-of-core use case is "many named graphs", and holding one
        // `BufWriter<File>` open per graph would exhaust the process file-descriptor limit
        // (`ulimit -n`) on a large dataset. Instead `WriterPool` keeps at most `cap` files
        // open at once: routing to an evicted graph flushes+closes the least-recently-used
        // writer and REOPENS the target in APPEND mode, so every quad is still routed to the
        // right per-graph spill (correctness preserved) with O(cap) FDs regardless of graph
        // count. The default graph shares the same pool under slot `usize::MAX`.
        const DEFAULT_SLOT: usize = usize::MAX;
        let mut names: Vec<Term> = Vec::new();
        let mut index: std::collections::HashMap<Term, usize> = std::collections::HashMap::new();
        // The N-Triples low-level serializer is STATELESS (it just writes one canonical line
        // per triple to the writer it is handed), so a single instance serves every graph.
        let mut ser = oxttl::NTriplesSerializer::new().low_level();

        // A bounded LRU pool of open per-graph spill writers. `slot_path` maps a slot id to
        // its spill file; `open` lazily (re)opens it in append mode, evicting the LRU writer
        // when at capacity. A slot's file is `create`d on first open (truncating any stale
        // file) and `append`ed on every subsequent reopen.
        struct WriterPool {
            spill_dir: std::path::PathBuf,
            cap: usize,
            // (slot, writer) in LRU order: front = least-recently-used, back = most-recent.
            open: std::collections::VecDeque<(usize, std::io::BufWriter<std::fs::File>)>,
            // Slots that have been created at least once (so reopen uses append, not create).
            seen: std::collections::HashSet<usize>,
        }
        impl WriterPool {
            fn new(spill_dir: std::path::PathBuf, cap: usize) -> Self {
                WriterPool {
                    spill_dir,
                    cap: cap.max(1),
                    open: std::collections::VecDeque::new(),
                    seen: std::collections::HashSet::new(),
                }
            }
            fn slot_path(&self, slot: usize) -> std::path::PathBuf {
                if slot == DEFAULT_SLOT {
                    self.spill_dir.join("default.nt")
                } else {
                    self.spill_dir.join(format!("g{slot}.nt"))
                }
            }
            /// Make `slot`'s writer the most-recently-used open writer and return a mutable
            /// reference to it, evicting (flush+close) the LRU writer if at capacity.
            fn writer(
                &mut self,
                slot: usize,
            ) -> Result<&mut std::io::BufWriter<std::fs::File>, String> {
                // Already open: move to the back (most-recently-used).
                if let Some(pos) = self.open.iter().position(|(s, _)| *s == slot) {
                    let entry = self.open.remove(pos).expect("position just found");
                    self.open.push_back(entry);
                    return Ok(&mut self.open.back_mut().expect("just pushed").1);
                }
                // Not open: evict LRU if full.
                if self.open.len() >= self.cap {
                    if let Some((_, mut w)) = self.open.pop_front() {
                        w.flush().map_err(|e| e.to_string())?;
                        // `w` (the File) is dropped here, releasing its FD.
                    }
                }
                let path = self.slot_path(slot);
                let f = if self.seen.insert(slot) {
                    // First time we touch this slot: truncate/create.
                    std::fs::File::create(&path).map_err(|e| e.to_string())?
                } else {
                    // Reopen a previously-evicted slot: append so prior lines survive.
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .map_err(|e| e.to_string())?
                };
                self.open.push_back((slot, std::io::BufWriter::new(f)));
                Ok(&mut self.open.back_mut().expect("just pushed").1)
            }
            /// Flush+close every open writer (final flush before the per-graph builds).
            fn flush_all(&mut self) -> Result<(), String> {
                while let Some((_, mut w)) = self.open.pop_front() {
                    w.flush().map_err(|e| e.to_string())?;
                }
                Ok(())
            }
        }
        // Default open-writer budget. Generous enough to avoid thrashing on typical datasets
        // yet a small constant, so FD usage is O(1) in the graph count. Tests override this
        // via the env hook below to force eviction with only a few graphs.
        let pool_cap: usize = std::env::var("SPARQ_QUADS_SPILL_MAX_OPEN")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&c| c > 0)
            .unwrap_or(256);
        let mut pool = WriterPool::new(spill_dir.clone(), pool_cap);
        // Whether the default graph ever received a quad (decides whether `default.nt` exists).
        let mut have_default = false;

        macro_rules! route {
            ($q:expr) => {{
                let q = $q.map_err(|e| e.to_string())?;
                let triple =
                    TripleRef::new(&q.subject, q.predicate.as_ref(), q.object.as_ref());
                let slot = match q.graph_name {
                    GraphName::DefaultGraph => {
                        have_default = true;
                        DEFAULT_SLOT
                    }
                    other => {
                        let name = match other {
                            GraphName::NamedNode(n) => Term::NamedNode(n),
                            GraphName::BlankNode(b) => Term::BlankNode(b),
                            GraphName::DefaultGraph => unreachable!("matched above"),
                        };
                        match index.get(&name) {
                            Some(&i) => i,
                            None => {
                                let i = names.len();
                                index.insert(name.clone(), i);
                                names.push(name);
                                i
                            }
                        }
                    }
                };
                let w = pool.writer(slot)?;
                ser.serialize_triple(triple, &mut *w).map_err(|e| e.to_string())?;
            }};
        }

        match format {
            "nquads" | "n-quads" => {
                for q in NQuadsParser::new().for_reader(reader) {
                    route!(q);
                }
            }
            "trig" | "application/trig" => {
                for q in TriGParser::new().for_reader(reader) {
                    route!(q);
                }
            }
            // Unsupported formats are rejected up-front (before any mutation); unreachable here.
            _ => unreachable!("format validated above"),
        }

        // Flush every spill writer so the per-graph files are complete before we build them.
        pool.flush_all()?;

        // Build the DEFAULT graph in `dir` itself (even when empty — an empty `default.nt`
        // streams to a valid empty store, matching a default-graph-only `save`).
        let default_path = spill_dir.join("default.nt");
        if !have_default {
            // No default-graph quad was seen, so the file was never created — make an empty
            // one so the default store is a valid (empty) store.
            std::fs::File::create(&default_path).map_err(|e| e.to_string())?;
        }
        let default_file = std::fs::File::open(&default_path).map_err(|e| e.to_string())?;
        Self::build_external(default_file, "ntriples", dir, chunk)?;

        // Build each NAMED graph into `dir/named/<i>/` (first-occurrence order = manifest
        // order), then commit the manifest LAST — the same ordering + commit point as
        // `save_named`, so `open`/`open_named` read the dataset back identically.
        if !names.is_empty() {
            let named_dir = dir.join(NAMED_SUBDIR);
            std::fs::create_dir_all(&named_dir).map_err(|e| e.to_string())?;
            for i in 0..names.len() {
                let f = std::fs::File::open(spill_dir.join(format!("g{i}.nt"))).map_err(|e| e.to_string())?;
                Self::build_external(f, "ntriples", &named_dir.join(i.to_string()), chunk)?;
            }
            write_named_manifest(dir, &names).map_err(|e| e.to_string())?;
        }

        // Success: disarm the guard and remove the spill dir explicitly.
        spill_guard.disarm();
        std::fs::remove_dir_all(&spill_dir).ok();
        Ok(())
    }

    /// External-memory build with the SPILLED term dictionary (`dict-spill` feature):
    /// peak RSS is bounded by `cfg.mem_budget` (dictionary included) instead of growing
    /// with the distinct-term count — terms spill to disk and are externally
    /// deduplicated/ranked into EXACTLY the ids the (default) sharded in-RAM
    /// consolidation assigns, so every output file is byte-identical to
    /// [`build_external`](Self::build_external)'s. N-Triples only (the same RDF 1.2 triple-term
    /// restriction as the sharded path). Design: research/external-dictionary.md.
    #[cfg(feature = "dict-spill")]
    pub fn build_external_spill<R: std::io::Read + Send>(
        reader: R,
        format: &str,
        dir: &std::path::Path,
        chunk: usize,
        cfg: &dictspill::SpillConfig,
    ) -> Result<(), String> {
        use store::{Perm, BUILT};
        if !matches!(format, "ntriples" | "n-triples") {
            return Err("the dict-spill external build supports N-Triples only".to_string());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join("tmp");
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        dictspill::ensure_disk(&tmp, cfg.disk_floor)?;
        let _t_build = std::time::Instant::now();
        build_timing::reset();

        // Stages 1-3: decompress + parallel parse + bounded-cache resolve + stage.
        let mut interner =
            dictspill::SpillInterner::new(default_shards(), &tmp, cfg).map_err(|e| e.to_string())?;
        build_external_ntriples_dictspill(reader, &mut interner)?;
        if build_timing::enabled() {
            // [OPUS-4.8 sq-3l43] The spill path's "merge" bucket is the bounded PARALLEL
            // `intern_batch` occupancy (Pipelined), not a serial `merge_remap`. Consolidation
            // (`consolidate`) is a LATER phase, reported separately below, so `None` here.
            build_timing::report(
                "parse+route+stage done",
                build_timing::PathKind::Pipelined,
                None,
                _t_build.elapsed().as_secs_f64(),
            );
        }

        // Phases 2-4: external dedup/rank. The dictionary files and the numeric/temporal
        // caches are STREAM-written in final-id order here, so no dict-finalize thread
        // (and no resident dictionary) exists later.
        let t_cons = std::time::Instant::now();
        let plan = dictspill::consolidate(interner, dir, &tmp, cfg)?;
        if build_timing::enabled() {
            eprintln!("[build-timing] dict consolidate (spilled): {:.2}s", t_cons.elapsed().as_secs_f64());
        }

        // Phase 5: remap the staged triples to final dense ids, spilling SPO-sorted runs.
        let mut buf: Vec<[Id; 3]> = Vec::with_capacity(chunk.min(1 << 24));
        let mut runs: Vec<std::path::PathBuf> = Vec::new();
        dictspill::remap_staged(plan, &mut buf, &mut runs, &tmp, chunk)?;
        extsort::spill_run(&mut buf, &mut runs, &tmp).map_err(|e| e.to_string())?;
        drop(buf); // free the chunk buffer before the k-way merge + sibling sorts

        // Merge the SPO runs (ids are FINAL already — no `remap_perm_file` pass needed).
        let spo_path = dir.join(format!("perm{}.bin", Perm::Spo as usize));
        extsort::kway_merge(&runs, &spo_path).map_err(|e| e.to_string())?;
        if build_timing::enabled() {
            eprintln!("[build-timing] kway_merge SPO done | {:.2}s wall to here", _t_build.elapsed().as_secs_f64());
        }
        for r in &runs {
            std::fs::remove_file(r).ok();
        }

        // Sibling permutations: the same concurrent external sorts as
        // `build_external_opts` (minus its dict-finalize thread).
        let (map, n) = extsort::map_perm(&spo_path).map_err(|e| e.to_string())?;
        // SAFETY: perm0 is a whole number of [u32;3] rows written above; map outlives the loop.
        let spo: &[[Id; 3]] =
            unsafe { std::slice::from_raw_parts(map.as_ptr().cast::<[Id; 3]>(), n) };
        let siblings: Vec<Perm> = BUILT.iter().copied().filter(|&p| p != Perm::Spo).collect();
        // [OPUS-4.8] sq-vkz7: opt-in compressed build, same as `build_external_opts` —
        // siblings write `SPQCPRM1` directly; SPO is re-encoded after they finish.
        let compressed = build_compressed_perms();
        let sib_sort = |perm: Perm, sub: &std::path::Path, per: usize| -> Result<(), String> {
            std::fs::create_dir_all(sub).map_err(|e| e.to_string())?;
            let out = dir.join(format!("perm{}.bin", perm as usize));
            if compressed {
                extsort::external_sort_compressed(spo.iter().copied(), perm.order(), &out, sub, per)
                    .map_err(|e| e.to_string())
            } else {
                extsort::external_sort(spo.iter().copied(), perm.order(), &out, sub, per).map_err(|e| e.to_string())
            }
        };
        {
            use rayon::prelude::*;
            let per = (chunk / siblings.len().max(1)).max(1 << 16);
            siblings
                .par_iter()
                .try_for_each(|&perm| sib_sort(perm, &tmp.join(format!("p{}", perm as usize)), per))?;
        }
        drop(map);
        if build_timing::enabled() {
            eprintln!("[build-timing] sibling sorts done | {:.2}s wall to here", _t_build.elapsed().as_secs_f64());
        }
        // SPO compressed last, now that no sibling sort needs it raw (see `build_external_opts`).
        if compressed {
            compress_perm_file_in_place(&spo_path)?;
        }

        // Empty files for the unbuilt permutations so `open` finds all six slots.
        for i in 0..6 {
            let p = dir.join(format!("perm{i}.bin"));
            if !p.exists() {
                std::fs::File::create(&p).map_err(|e| e.to_string())?;
            }
        }
        let store = TripleStore::open(dir).map_err(|e| e.to_string())?;
        store.save_pred_stats(dir).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// The numeric value of a term id, or `None` if it is not a numeric literal.
    /// O(1), no allocation — the engine's fast path for numeric filters. An inline
    /// integer id carries its value directly (no lookup); other ids use the cache.
    #[inline]
    pub fn numeric_value(&self, id: Id) -> Option<f64> {
        if dict::is_inline(id) {
            return Some((id - dict::INLINE_BASE) as f64);
        }
        self.numerics.lookup(id)
    }

    /// The temporal (xsd:dateTime / xsd:dateTimeStamp / xsd:date) value of a term id,
    /// or `None` if it is not a well-formed temporal literal. O(1), no allocation, no
    /// lexical re-parse — the engine's fast path for dateTime FILTER / ORDER BY /
    /// MIN/MAX, the temporal twin of [`numeric_value`](Self::numeric_value).
    #[inline]
    pub fn temporal_value(&self, id: Id) -> Option<Temporal> {
        if dict::is_inline(id) {
            return None; // inline ids are integers, never temporal
        }
        self.temporals.lookup(id)
    }

    /// The lexical form of a term id IF it is an exact-valued numeric literal (an
    /// `xsd:integer` subtype or `xsd:decimal` — NOT float/double, whose value IS its f64).
    /// Used to disambiguate comparisons that the f64 fast path collapses (integers > 2^53,
    /// high-precision decimals); only reached when the f64 values compared equal, so the
    /// allocation is rare. Inline-integer ids format their value directly.
    pub fn exact_numeric_lexical(&self, id: Id) -> Option<String> {
        if dict::is_inline(id) {
            return Some((id - dict::INLINE_BASE).to_string());
        }
        match self.dict.term_parts(id) {
            dict::TermParts::Lit { value, datatype, lang: None }
                if is_integer_datatype(datatype) || datatype == xsd::DECIMAL.as_str() =>
            {
                Some(value.to_string())
            }
            _ => None,
        }
    }

    /// [OPUS-4.8] (sq-lr2ii) `true` if the graph holds any `xsd:decimal` literal with MORE
    /// than 15 significant digits — a value the f64 `numerics` cache CANNOT represent exactly,
    /// so the engine's f64-based sargable FILTER fast path could decide `=`/`<`/`>`/`<=`/`>=`
    /// WRONGLY for it (e.g. `"1.000000000000000001"^^xsd:decimal` shares the f64 `1.0` with the
    /// constant `1`, yet is not equal to it). The engine consults this at the sargable-decision
    /// point to DECLINE that fast path and fall back to the exact general evaluator; a graph
    /// without any such decimal keeps the fast path (a decimal of `<= 15` significant digits
    /// round-trips through f64 unambiguously, and a large-integer collision needs a `> 15`-digit
    /// CONSTANT, which the engine already declines). Integers/float/double are exempt: an
    /// integer's exactness is the constant-side guard's job and a float/double's value IS its f64.
    ///
    /// Memoised (see `high_precision_decimal`): the first call scans the graph's numeric terms
    /// once (short-circuiting on the first offender); later calls are an atomic load. A delta
    /// that appends terms resets the memo so it is recomputed over the grown dictionary. This is
    /// a CONSERVATIVE, graph-wide gate — one offending decimal declines the numeric fast path for
    /// every comparison on the graph — chosen for safety (correctness is never at risk; only the
    /// pushdown optimisation is skipped for affected graphs).
    pub fn has_high_precision_decimal(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        match self.high_precision_decimal.load(Relaxed) {
            2 => true,
            1 => false,
            _ => {
                let found = (1..=self.dict.len() as Id)
                    .any(|id| self.numerics.lookup(id).is_some() && is_high_precision_decimal(&self.dict, id));
                self.high_precision_decimal.store(if found { 2 } else { 1 }, Relaxed);
                found
            }
        }
    }

    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// A rough estimate of the graph's in-memory footprint in bytes (dictionary +
    /// the six permutation indexes), for benchmarking.
    pub fn heap_bytes(&self) -> usize {
        self.dict.heap_bytes() + self.store.heap_bytes() + self.numerics.heap_bytes() + self.temporals.heap_bytes()
    }

    /// Resolves a term to its id, or `None` if the term is absent (so a pattern
    /// bound to it cannot match).
    pub fn id_of(&self, term: &Term) -> Option<Id> {
        let id = self.dict.lookup(term);
        if id == dict::NO_ID {
            None
        } else {
            Some(id)
        }
    }

    /// [FABLE-5] (sq-1ivw7) Whether ANY triple with predicate `predicate` in the CURRENT store
    /// snapshot (base index MERGED with the pending update overlay) has a LITERAL object.
    ///
    /// Answers, against the live snapshot, the term-kind question the engine's predicate-range
    /// non-literal inference asks: a variable bound only as the OBJECT of patterns whose predicate
    /// is this constant IRI can be treated as non-literal (id-comparable for `=`/`!=`) for a query
    /// execution iff this returns `false`. It scans the predicate's object column with an EARLY
    /// EXIT on the first literal found — cheap when a literal is near the front, O(matches) when the
    /// predicate is genuinely literal-free (the case that fires the fast path). The result is only
    /// valid for THIS snapshot: an UPDATE inserting a literal object publishes a NEW generation, so
    /// a fresh evaluation re-checks against the new snapshot (there is no cross-snapshot memo).
    ///
    /// `predicate` is a raw dictionary id. An absent predicate (no triples) returns `false`
    /// (vacuously literal-free), which is correct: the object variable is then unbound anyway.
    ///
    /// SCOPE (load-bearing): this answers for the RECEIVER graph ONLY — it scans `self.store`,
    /// the receiver's own triples (a named graph is a self-contained sub-`Graph` with its own
    /// `store`/`dict`), and never `self.named`. A caller must invoke it on the SAME graph the
    /// object variable is bound from: an object under a `GRAPH <g>`/`GRAPH ?g` block is bound
    /// from the named sub-graph, so classifying it off the enclosing (default) graph's column is
    /// UNSOUND. The engine enforces this by staying conservative for object slots under any GRAPH
    /// block at an outer dispatch (see `collect_kind_positions`' `G::Graph` arm).
    pub fn predicate_has_literal_object(&self, predicate: Id) -> bool {
        if predicate == dict::NO_ID {
            return false;
        }
        let scan = self.store.scan(&[None, Some(predicate), None]);
        for row in scan.rows.iter() {
            let o = scan.to_spo(row)[2];
            if dict::is_literal_id(&self.dict, o) {
                return true;
            }
        }
        false
    }

    /// Builds an id pattern from optional terms; returns `None` if any bound
    /// term is absent from the dictionary.
    pub fn pattern(
        &self,
        s: Option<&Term>,
        p: Option<&NamedNode>,
        o: Option<&Term>,
    ) -> Option<Pattern> {
        let resolve = |t: Option<&Term>| -> Option<Option<Id>> {
            match t {
                None => Some(None),
                Some(t) => self.id_of(t).map(Some),
            }
        };
        let s = resolve(s)?;
        let p = match p {
            None => None,
            Some(n) => Some(self.id_of(&Term::NamedNode(n.clone()))?),
        };
        let o = resolve(o)?;
        Some([s, p, o])
    }

    /// Iterates every triple of the DEFAULT graph as canonical `[subject, predicate,
    /// object]` dictionary ids, in sorted (S, P, O) order — the borrowing triple
    /// iterator for downstream crates (validation, similarity, export) that today
    /// re-derive it from a raw `store.scan`. Zero allocation per row; resolve ids
    /// lazily with [`Dict::term`](dict::Dict::term) (always) or
    /// [`Dict::term_parts`](dict::Dict::term_parts) (real dictionary ids — see
    /// [`dict::is_inline`]). Because the order is subject-sorted, distinct subjects
    /// fall out of run boundaries; for distinct objects use
    /// [`iter_ids_sorted`](Self::iter_ids_sorted)`(2)`.
    pub fn iter_ids(&self) -> TripleIdIter<'_> {
        TripleIdIter { scan: self.store.scan(&[None, None, None]), i: 0 }
    }

    /// [`iter_ids`](Self::iter_ids) sorted by the canonical column `col`
    /// (0 = subject, 1 = predicate, 2 = object) — e.g. `iter_ids_sorted(2)` yields
    /// object-sorted triples so distinct objects are adjacent.
    pub fn iter_ids_sorted(&self, col: usize) -> TripleIdIter<'_> {
        TripleIdIter { scan: self.store.scan_sorted(&[None, None, None], col), i: 0 }
    }

    // ---- Structural fork (snapshot generations) --------------------------------------

    /// A STRUCTURAL FORK of this graph: a new, independently mutable `Graph` that
    /// SHARES the base's immutable storage — the six permutation indexes, the planner
    /// stats, the dictionary's frozen base and the numeric/temporal caches are all
    /// `Arc`-shared — and carries only the small per-generation delta by value (the
    /// pending store overlay, the dictionary extension, the cache side maps). Cost:
    /// O(pending delta), NOT O(graph) — except the FIRST fork of a flat (never-forked)
    /// graph, which pays a one-time O(n) dictionary/cache freeze to mint the shared
    /// base (the indexes are born shareable and never need freezing).
    ///
    /// The fork and the base then evolve independently through
    /// [`apply_delta`](Self::apply_delta): neither ever mutates shared storage, so
    /// concurrent readers of the base are unaffected (the snapshot-generation pattern:
    /// fork → apply a batch → publish, with [`compact`](Self::compact) folding the
    /// accumulated delta back into flat storage when it grows past a threshold —
    /// which also re-freezes the dictionary so later forks stay cheap).
    ///
    /// New terms interned in a fork get ids above the shared base's high-water mark
    /// (still below the inline-integer range), so a term in the shared base resolves
    /// to the SAME id in every generation. Named graphs are forked recursively. The
    /// fork carries NO write-ahead log (`apply_delta` on it is overlay-only): the
    /// generation pattern is an in-memory serving construct — durability stays with
    /// whatever owns the base.
    pub fn fork(&self) -> Graph {
        Graph {
            dict: self.dict.fork(),
            store: self.store.fork(),
            numerics: self.numerics.fork(),
            temporals: self.temporals.fork(),
            // sq-lr2ii: the fork shares the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named.iter().map(|(name, g)| (name.clone(), g.fork())).collect(),
            // A fork is a fresh logical copy; rebuild the prefix index lazily on first use.
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) A fork/snapshot is a logically-independent in-memory copy with
            // NO directory association — no WAL and no redo journal (like `wal: None`).
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }

    /// [OPUS-4.8] (sq-5lf) A cheap, logically-INDEPENDENT, IMMUTABLE point-in-time copy
    /// of this graph — O(pending delta), never O(triples). This is the public snapshot
    /// surface beads `sq-3p1` ("Core: cheap O(overlay) Graph::snapshot API") and
    /// `sq-5lf` specify, and what the blocked downstream consumers (sparq-rsp's
    /// true-overlay window eval, sparq-py's `Graph.copy()`, the RDF/JS incremental-update
    /// story) want: hand out an O(1)-ish snapshot that reads exactly the triples present
    /// NOW and is unaffected by any later mutation of `self` (or of the snapshot — it is
    /// immutable). Built on the [structural fork](Self::fork): the six permutation
    /// indexes, planner stats, frozen dictionary base and numeric/temporal caches are
    /// `Arc`-shared; only the small per-generation delta is copied by value. (As with
    /// `fork`, the FIRST snapshot of a flat, never-forked graph pays a one-time O(n)
    /// dictionary/cache freeze to mint the shareable base; subsequent snapshots of the
    /// frozen lineage are O(overlay). Call [`compact`](Self::compact) to refreeze.)
    ///
    /// The returned [`GraphSnapshot`] is [`Send`] + [`Sync`] and derefs to `&Graph`, so it
    /// is queryable exactly like a graph (it can be passed anywhere a `&Graph` is — the
    /// engine reads `store`/`dict` through the deref) but exposes NO mutating method, so a
    /// snapshot can never diverge from its point-in-time state. To obtain a snapshot you
    /// can then keep mutating, use [`fork`](Self::fork) instead (which yields a `Graph`).
    pub fn snapshot(&self) -> GraphSnapshot {
        GraphSnapshot { graph: self.fork() }
    }

    /// Total pending-delta size carried by this graph (and its named graphs): store
    /// overlay entries + dictionary extension terms. This is what a [`fork`](Self::fork)
    /// copies by value — the input to a compaction threshold policy.
    pub fn pending_delta_len(&self) -> usize {
        self.store.overlay_len()
            + self.dict.appended_len()
            + self.named.iter().map(|(_, g)| g.pending_delta_len()).sum::<usize>()
    }

    // ---- Incremental updates (T17): delta-overlay + WAL durability ------------------

    /// Applies an incremental update batch — `deletes` first, then `inserts` (SPARQL's
    /// DELETE/INSERT application order) — through the store's DELTA-OVERLAY: O(batch)
    /// work instead of the O(n) full rebuild. New terms are interned APPEND-ONLY (the
    /// dictionary grows; existing ids never change), so readers of existing ids are
    /// unaffected. For a directory-backed graph (opened via [`open`](Self::open)) the
    /// batch is appended to the write-ahead log and fsync'd BEFORE it is applied, so a
    /// crash replays it on the next open. Fold the overlay back into the immutable base
    /// periodically with [`compact`](Self::compact).
    pub fn apply_delta(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) -> Result<(), String> {
        if inserts.is_empty() && deletes.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(w) = &mut self.wal {
            w.append_batch(inserts, deletes).map_err(|e| format!("WAL append failed: {e}"))?;
        }
        self.apply_delta_mem(inserts, deletes);
        Ok(())
    }

    /// [OPUS-4.8] (gh-1122) Insert a SINGLE triple from `oxrdf` terms — the ergonomic
    /// convenience over [`apply_delta`](Self::apply_delta) for the one-triple case.
    ///
    /// Each position accepts anything that converts into an `oxrdf::Term` (`NamedNode`,
    /// `Literal`, `BlankNode`, an [RDF 1.2] triple term, or a `Term` itself), so callers avoid both
    /// building a dictionary-encoded `[[Term; 3]]` batch by hand AND assembling/escaping an
    /// `INSERT DATA { … }` SPARQL string for what is conceptually one append. The term is interned
    /// APPEND-ONLY and applied through the same delta-overlay path as `apply_delta`, so it inherits
    /// the identical semantics: set-valued (re-inserting an existing triple is a no-op), O(1) work,
    /// and — for a directory-backed graph (opened via [`open`](Self::open)) — WAL-logged + fsync'd
    /// before it is applied. To add several triples at once, prefer one
    /// [`apply_delta`](Self::apply_delta) batch over a loop of single inserts (one WAL append).
    ///
    /// [RDF 1.2]: https://www.w3.org/TR/rdf12-concepts/
    pub fn insert_triple(
        &mut self,
        subject: impl Into<Term>,
        predicate: impl Into<Term>,
        object: impl Into<Term>,
    ) -> Result<(), String> {
        let triple = [subject.into(), predicate.into(), object.into()];
        self.apply_delta(&[triple], &[])
    }

    /// [OPUS-4.8] (gh-1122) Remove a SINGLE triple from `oxrdf` terms — the retraction twin of
    /// [`insert_triple`](Self::insert_triple), delegating to [`apply_delta`](Self::apply_delta).
    ///
    /// Removing a triple the graph does not contain is a NO-OP (set semantics, inherited from
    /// `apply_delta`), never an error. As with `insert_triple`, for a directory-backed graph the
    /// deletion is WAL-logged + fsync'd before it is applied; to retract several triples at once,
    /// prefer one [`apply_delta`](Self::apply_delta) batch.
    pub fn remove_triple(
        &mut self,
        subject: impl Into<Term>,
        predicate: impl Into<Term>,
        object: impl Into<Term>,
    ) -> Result<(), String> {
        let triple = [subject.into(), predicate.into(), object.into()];
        self.apply_delta(&[], &[triple])
    }

    /// [OPUS-4.8] (sq-ycle) Commit a whole multi-operation UPDATE body's resolved per-slot
    /// quad-delta as ONE atomic, all-or-nothing durable frame — the SINGLE commit point for
    /// `sparq_engine::apply_effects`. `records` is `(is_insert, slot, triple)` quads computed
    /// against the CURRENT graph state (so CLEAR/DROP have already been expanded to concrete
    /// retraction records by the caller). One `write_all` + one `sync_data()` makes the body
    /// durable as a unit BEFORE it is materialised across the per-graph WALs; if a crash interrupts
    /// materialisation, [`open`](Self::open) redoes this frame idempotently. A NO-OP (returns `Ok`)
    /// for an IN-MEMORY graph (no journal) and for an empty record set, so the in-memory live
    /// update path is byte-for-byte unchanged.
    pub fn commit_txn(&mut self, records: &[(bool, Option<Term>, [Term; 3])]) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        if let Some(j) = &mut self.txn {
            return j.append(records).map_err(|e| format!("txn journal commit failed: {e}"));
        }
        #[cfg(not(feature = "mmap"))]
        let _ = records;
        Ok(())
    }

    /// [OPUS-4.8] (sq-ycle) Clear the redo journal after its frame has been fully materialised into
    /// the per-graph state (the second half of `sparq_engine::apply_effects`'s journal-first
    /// protocol), so the next body starts from an empty journal and a clean reopen is a no-op. A
    /// NO-OP for an in-memory graph (no journal).
    pub fn clear_txn(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        if let Some(j) = &mut self.txn {
            return j.truncate().map_err(|e| format!("txn journal clear failed: {e}"));
        }
        Ok(())
    }

    /// [OPUS-4.8] (review 1593) DURABLY empties this graph's default-graph content while
    /// PRESERVING its directory / WAL association — the durable replacement for
    /// `*graph = empty_graph()` on a directory-backed graph (where the latter dropped the WAL
    /// and a reopen restored the old on-disk base). For a directory-backed graph the current
    /// triples are retracted through [`apply_delta`](Self::apply_delta), so the deletions are
    /// WAL-logged + fsync'd and survive a crash/reopen; for an in-memory graph it clears the
    /// store overlay in place (no WAL to keep). Named graphs are untouched.
    pub fn clear_default_durable(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        if self.wal.is_some() {
            // Retract every current default-graph triple as a WAL-logged delete batch.
            let triples: Vec<[Term; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows
                    .iter()
                    .map(|r| {
                        let spo = scan.to_spo(r);
                        [self.dict.term(spo[0]), self.dict.term(spo[1]), self.dict.term(spo[2])]
                    })
                    .collect()
            };
            if !triples.is_empty() {
                self.apply_delta(&[], &triples)?;
            }
            return Ok(());
        }
        // In-memory graph: no WAL/dir to preserve — clear the store content in place. The
        // dictionary is intentionally kept (ids stay stable; the empty store references none).
        self.store = TripleStore::from_triples(Vec::new());
        Ok(())
    }

    /// [OPUS-4.8] (sq-glw2) DURABLY empties an EXISTING named sub-graph (CLEAR GRAPH `<g>` /
    /// CLEAR NAMED of `name`) while PRESERVING the sub-graph's slot, directory and per-graph
    /// WAL association — the durable replacement for `self.named[i].1 = empty_graph()`, which
    /// (on a directory-backed parent) dropped the sub-graph's WAL/dir so the clear only became
    /// durable at the next compaction. The retraction routes through the sub-graph's own
    /// [`clear_default_durable`](Self::clear_default_durable), so for a directory-backed
    /// sub-graph the deletions are WAL-logged + fsync'd (durable before this returns) and
    /// survive a crash/reopen; the manifest is untouched (the slot stays). Returns `true` if a
    /// matching named graph existed (and was emptied), `false` if it was absent — the caller
    /// keeps the SPARQL CLEAR no-op-on-absent semantics. An in-memory parent's sub-graph just
    /// clears its store in place (no WAL to keep), identical to the old behaviour. Available in
    /// every build — the durability only kicks in for a directory-backed sub-graph (the in-memory
    /// path of [`clear_default_durable`](Self::clear_default_durable) is a plain store clear).
    pub fn clear_named_durable(&mut self, name: &Term) -> Result<bool, String> {
        match self.named.iter().position(|(n, _)| n == name) {
            Some(i) => {
                self.named[i].1.clear_default_durable()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// [OPUS-4.8] (sq-glw2) DURABLY removes an EXISTING named sub-graph (DROP GRAPH `<g>`): the
    /// sub-graph ceases to exist — its on-disk directory AND its manifest entry are removed so
    /// the removal survives a reopen IMMEDIATELY, not just at the next compaction. The old
    /// `self.named.retain(...)` dropped the entry in memory only, leaving the on-disk
    /// `named/<i>/` + manifest entry behind, so a reopen RESTORED the dropped graph.
    ///
    /// The named sub-tree is positional (`named/<i>/` ↔ `self.named[i]`) and the manifest's
    /// presence is the commit point, so a drop must RENUMBER the surviving sub-graphs to stay
    /// contiguous. To make that crash-safe we mirror [`compact`](Self::compact)'s rollback-safe
    /// directory swap, scoped to the `named/` sub-tree: the renumbered survivors are built in a
    /// staging `named.drop-new/`, fsync'd, then swapped in (`named` → `named.drop-old`,
    /// `named.drop-new` → `named`), and only THEN is the shrunk manifest written (the manifest
    /// rewrite is itself atomic+dir-fsync'd via `write_named_manifest`). An interrupted swap
    /// is completed/rolled back deterministically by `recover_named_drop` on the next
    /// [`open`](Self::open). Surviving sub-graphs are re-opened from their new directories so
    /// each re-acquires a correctly-indexed per-graph WAL.
    ///
    /// Returns `true` if a matching named graph existed (and was removed), `false` if absent —
    /// the caller keeps the SPARQL DROP no-op-on-absent semantics. An in-memory parent (or any
    /// build without the `mmap` feature) simply drops the entry — there is no directory/manifest
    /// to update, matching the old `self.named.retain(...)` behaviour.
    pub fn drop_named_durable(&mut self, name: &Term) -> Result<bool, String> {
        let Some(idx) = self.named.iter().position(|(n, _)| n == name) else {
            return Ok(false);
        };
        // Directory-backed parent: durably remove the sub-dir + manifest entry. For an in-memory
        // parent (or a non-mmap build) fall through to the plain in-memory entry removal below.
        #[cfg(feature = "mmap")]
        if let Some(dir) = self.wal.as_ref().map(|w| w.dir.clone()) {
            return self.drop_named_durable_dir(idx, &dir);
        }
        self.named.remove(idx);
        Ok(true)
    }

    /// [OPUS-4.8] (sq-glw2) The directory-backed half of [`drop_named_durable`](Self::drop_named_durable):
    /// remove the in-memory entry then durably renumber/persist the surviving named sub-tree
    /// (see that method's docs for the crash-safe swap protocol). Split out so the public method
    /// stays free of `mmap`-only fields.
    #[cfg(feature = "mmap")]
    fn drop_named_durable_dir(&mut self, idx: usize, dir: &std::path::Path) -> Result<bool, String> {
        // [OPUS-4.8] (Copilot #123, Issue 1) Drop the entry in memory first; `self.named` is now
        // the SURVIVING set in final order. RELEASE the removed sub-graph's mmap/WAL handles
        // IMMEDIATELY (it is gone) so no live map points into `named/<idx>/` before the directory
        // is removed/renamed below — required on Windows (a mapped file blocks dir removal), cheap
        // and correct everywhere else.
        let removed = self.named.remove(idx);
        drop(removed);
        // Re-persist the renumbered survivor set in ONE staging swap (or tear the whole sub-tree
        // down when none survive). Shared with the batch drop-all path.
        self.persist_named_after_drop(dir)?;
        Ok(true)
    }

    /// [OPUS-4.8] (sq-glw2, Copilot #123) DURABLY removes EVERY named graph in ONE manifest
    /// rewrite + one staging swap — the batch DROP NAMED / named-part-of DROP ALL. Replaces the
    /// engine's old loop that called [`drop_named_durable`](Self::drop_named_durable) once per
    /// graph: each of those rebuilt the ENTIRE surviving named set (renumber + manifest rewrite),
    /// making DROP ALL O(n²) in the number of named graphs. Clearing the whole set is O(n) — one
    /// pass to release every sub-graph's handles, one manifest removal, one sub-tree removal — so
    /// this routes straight through the no-survivors arm of `persist_named_after_drop`.
    ///
    /// In-memory parent (or a non-`mmap` build): just clears the entries, matching the old
    /// `self.named.clear()`. Idempotent — a no-op when there are no named graphs.
    pub fn drop_all_named_durable(&mut self) -> Result<(), String> {
        if self.named.is_empty() {
            return Ok(());
        }
        // Release every sub-graph's mmap/WAL handles before any directory is touched (Issue 1/2:
        // no live map may point into `named/<i>/` across the removal — correct on every platform,
        // required on Windows where a mapped file blocks `remove_dir_all`/`rename`).
        self.named.clear();
        #[cfg(feature = "mmap")]
        if let Some(dir) = self.wal.as_ref().map(|w| w.dir.clone()) {
            return self.persist_named_after_drop(&dir);
        }
        Ok(())
    }

    /// [OPUS-4.8] (sq-glw2, Copilot #123) Crash-safe persistence of `self.named` AFTER a drop has
    /// shrunk it (one entry, or all of them). `self.named` is already the SURVIVING set in final
    /// order; the dropped sub-graphs' handles must already be released by the caller so no live
    /// mmap/WAL points into the `named/` sub-tree being removed/renamed (cross-platform-safe;
    /// required on Windows). Two cases:
    ///
    /// - NO survivors: the whole sub-tree + manifest go away (a default-only dir). The manifest is
    ///   removed FIRST as the commit point, then the sub-tree — a crash after it leaves an orphan
    ///   `named/` that `open_named` ignores (no manifest ⇒ no named graphs); `recover_named_drop`
    ///   cleans up any staging siblings on the next open.
    /// - SOME survivors: the renumbered survivors are written into a staging `named.drop-new/`,
    ///   fsync'd, then swapped in (`named` → `drop-old`, `drop-new` → `named`) and the shrunk
    ///   manifest rewritten — the same rollback-safe protocol as [`compact`](Self::compact),
    ///   recovered by [`recover_named_drop`]. The survivors' OWN handles are released before the
    ///   rename and re-acquired by re-`open`ing each from its new directory afterwards, so a live
    ///   map never points into the directory being renamed (Issue 2; Windows-safe).
    #[cfg(feature = "mmap")]
    fn persist_named_after_drop(&mut self, dir: &std::path::Path) -> Result<(), String> {
        let named_dir = dir.join(NAMED_SUBDIR);
        let new_dir = named_dir.with_extension("drop-new");
        let old_dir = named_dir.with_extension("drop-old");

        if self.named.is_empty() {
            remove_named_manifest(dir).map_err(|e| e.to_string())?;
            std::fs::remove_dir_all(&named_dir).ok();
            std::fs::remove_dir_all(&new_dir).ok();
            std::fs::remove_dir_all(&old_dir).ok();
            return Ok(());
        }

        // Build the renumbered survivor sub-tree in a fresh staging dir, fsync it, then swap.
        std::fs::remove_dir_all(&new_dir).ok();
        std::fs::remove_dir_all(&old_dir).ok();
        std::fs::create_dir_all(&new_dir).map_err(|e| e.to_string())?;
        // Close every survivor's WAL before copying its directory so no open append handle is
        // mid-flight across the swap. Then persist each survivor — folding any overlay — into its
        // NEW positional slot, capturing its name for the rebuilt manifest.
        let mut names: Vec<Term> = Vec::with_capacity(self.named.len());
        for (i, (name, sub)) in self.named.iter_mut().enumerate() {
            sub.wal = None;
            sub.save(&new_dir.join(i.to_string())).map_err(|e| e.to_string())?;
            names.push(name.clone());
        }
        fsync_dir(&new_dir).map_err(|e| e.to_string())?;
        // [OPUS-4.8] (Copilot #123, Issue 2) RELEASE every survivor's live mmap/WAL handles
        // BEFORE the rename: each is now fully persisted under `new_dir`, so drop the in-memory
        // `Graph`s (mmaps into the OLD `named/<i>/` included) and re-open from the new directories
        // after the swap. Renaming/removing a directory with mapped files into it fails on
        // Windows; releasing first makes the swap safe on every platform (mirrors `compact`, which
        // likewise drops its handles via `*self = Graph::open(..)` after its swap).
        self.named.clear();
        // Rollback-safe swap (mirrors `compact`): old aside, new in, parent fsync each rename.
        std::fs::rename(&named_dir, &old_dir).map_err(|e| e.to_string())?;
        fsync_dir(dir).map_err(|e| e.to_string())?;
        std::fs::rename(&new_dir, &named_dir).map_err(|e| e.to_string())?;
        fsync_dir(dir).map_err(|e| e.to_string())?;
        // The sub-tree is now the renumbered survivors. Commit the new set by rewriting the
        // manifest (atomic temp+rename, fsyncs the parent dir) — the commit point for the set.
        write_named_manifest(dir, &names).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(&old_dir).ok();
        // Re-open survivors from their NEW directories so each carries a correctly-indexed WAL
        // (and a fresh, correctly-targeted mmap into the swapped-in `named/`).
        for (i, name) in names.into_iter().enumerate() {
            let sub = Graph::open(&named_dir.join(i.to_string())).map_err(|e| e.to_string())?;
            self.named.push((name, sub));
        }
        Ok(())
    }

    /// [`apply_delta`](Self::apply_delta) over the whole DATASET, parsing the batch
    /// from two N-Quads documents (N-Triples is the default-graph subset): `deletes`
    /// first, then `inserts`, grouped and applied PER GRAPH — default-graph statements
    /// to this graph, named-graph statements to the matching [`named`](Self::named)
    /// sub-graph (auto-created on first insert; deleting from an absent graph is a
    /// no-op). O(batch) through each graph's delta overlay, no index rebuild. Blank
    /// nodes denote concrete nodes BY LABEL (unlike SPARQL `DELETE DATA`, which cannot
    /// name a bnode), which is what makes quad-level retraction of bnode triples
    /// possible. The string-parsing entry point for bindings (wasm/python) callers.
    pub fn apply_delta_nquads(&mut self, inserts: &str, deletes: &str) -> Result<(), String> {
        use oxrdf::GraphName;
        type Slot = Option<Term>;
        // Per-graph delta: `(slot, inserts, deletes)` — one entry per distinct graph.
        type SlotDelta = (Slot, Vec<[Term; 3]>, Vec<[Term; 3]>);
        fn parse(text: &str) -> Result<Vec<(Slot, [Term; 3])>, String> {
            let mut out = Vec::new();
            for q in NQuadsParser::new().for_slice(text.as_bytes()) {
                let q = q.map_err(|e| e.to_string())?;
                let slot = match q.graph_name {
                    GraphName::DefaultGraph => None,
                    GraphName::NamedNode(n) => Some(Term::NamedNode(n)),
                    GraphName::BlankNode(b) => Some(Term::BlankNode(b)),
                };
                out.push((slot, [subject_term(&q.subject), Term::NamedNode(q.predicate), q.object]));
            }
            Ok(out)
        }
        // Group per slot preserving first-seen order (datasets hold few graphs, so a
        // linear scan beats hashing Terms), keeping each slot's deletes and inserts
        // together so they go through ONE apply_delta call (deletes applied first).
        let mut slots: Vec<SlotDelta> = Vec::new();
        for (is_insert, items) in [(false, parse(deletes)?), (true, parse(inserts)?)] {
            for (slot, t) in items {
                let entry = match slots.iter_mut().find(|(s, _, _)| *s == slot) {
                    Some(e) => e,
                    None => {
                        slots.push((slot, Vec::new(), Vec::new()));
                        slots.last_mut().expect("just pushed")
                    }
                };
                if is_insert {
                    entry.1.push(t);
                } else {
                    entry.2.push(t);
                }
            }
        }
        // [OPUS-4.8] (sq-3ui0, gh-45) For a DIRECTORY-BACKED parent, a NEW named graph is
        // created with its own on-disk directory + per-graph WAL under `dir/named/<i>/` so
        // its very first mutation is WAL-logged + fsync'd (durable before this call
        // returns) and the manifest is kept in sync — not deferred to the next save. An
        // in-memory parent keeps the overlay-only path (no directory, no WAL).
        #[cfg(feature = "mmap")]
        let parent_dir = self.wal.as_ref().map(|w| w.dir.clone());
        for (slot, ins, del) in slots {
            match slot {
                None => self.apply_delta(&ins, &del)?,
                Some(name) => {
                    if let Some(i) = self.named.iter().position(|(n, _)| *n == name) {
                        self.named[i].1.apply_delta(&ins, &del)?;
                    } else if !ins.is_empty() {
                        #[cfg(feature = "mmap")]
                        if let Some(dir) = &parent_dir {
                            let mut g = self.create_durable_named(dir, &name)?;
                            g.apply_delta(&ins, &[])?;
                            self.named.push((name, g));
                            continue;
                        }
                        let mut g = Graph::from_parts(Dict::new(), Vec::new());
                        g.apply_delta(&ins, &[])?;
                        self.named.push((name, g));
                    } // deletes against an absent graph: no-op
                }
            }
        }
        Ok(())
    }

    /// [OPUS-4.8] (sq-3ui0, gh-45) Creates a fresh, EMPTY named sub-graph that is
    /// directory-backed (its own `dir/named/<i>/` + per-graph WAL) and appends/updates the
    /// parent's `named.bin` manifest so the new graph is recoverable on the next
    /// [`open`](Self::open) even before a full [`save`](Self::save)/[`compact`]. The index
    /// `i` is the parent's current `named.len()` (the slot this graph will occupy). Saving
    /// an empty graph then re-opening it yields a graph with its own WAL, so the caller can
    /// immediately [`apply_delta`](Self::apply_delta) durably.
    #[cfg(feature = "mmap")]
    fn create_durable_named(&self, dir: &std::path::Path, name: &Term) -> Result<Graph, String> {
        let i = self.named.len();
        let named_dir = dir.join(NAMED_SUBDIR);
        let sub_dir = named_dir.join(i.to_string());
        std::fs::create_dir_all(&named_dir).map_err(|e| e.to_string())?;
        // Persist an empty sub-graph, then open it so it carries its own WAL.
        let empty = Graph::from_parts(Dict::new(), Vec::new());
        empty.save(&sub_dir).map_err(|e| e.to_string())?;
        // fsync `dir/named/` so the new sub-graph DIRECTORY's existence is durable before the
        // manifest names it (otherwise a crash could leave the manifest pointing at a dir whose
        // creation never reached disk).
        fsync_dir(&named_dir).map_err(|e| e.to_string())?;
        // Rewrite the manifest with the new entry appended (index order == named order).
        // [OPUS-4.8] (Copilot review #69, finding 2) `named.bin` lives in the PARENT `dir`, not
        // in `dir/named/`. The old code only fsync'd `dir/named/`, so a crash could lose the
        // manifest entry even though the sub-graph was on disk. `write_named_manifest` now does
        // an atomic temp+rename AND fsyncs the parent `dir` (the dir that holds `named.bin`), so
        // the manifest entry — the recovery commit point — is itself durable.
        let mut names: Vec<Term> = self.named.iter().map(|(n, _)| n.clone()).collect();
        names.push(name.clone());
        write_named_manifest(dir, &names).map_err(|e| e.to_string())?;
        Graph::open(&sub_dir).map_err(|e| e.to_string())
    }

    /// [OPUS-4.8] (sq-7cxr, gh-44) Returns the index of the named sub-graph called `name`,
    /// CREATING it if absent. The created sub-graph is DURABLE (its own `dir/named/<i>/` +
    /// per-graph WAL + manifest entry) whenever this parent graph is itself directory-backed
    /// (opened via [`open`](Self::open)); for an in-memory parent it is a plain in-memory
    /// sub-graph (`wal: None`), byte-identical to the previous `named.push((name, empty()))`
    /// behaviour. This is the durability seam the SPARQL-Update path needs: a `GRAPH <g> { … }`
    /// INSERT that first touches a brand-new named graph on a persisted server must give that
    /// graph its own WAL so its triples survive a restart, exactly as a default-graph or
    /// already-existing-named-graph mutation does. An in-memory parent is unaffected.
    ///
    /// Without the `mmap` feature there is no directory/WAL machinery at all, so this always
    /// creates an in-memory sub-graph (the parent can never be directory-backed).
    pub fn ensure_named(&mut self, name: &Term) -> Result<usize, String> {
        if let Some(i) = self.named.iter().position(|(n, _)| n == name) {
            return Ok(i);
        }
        #[cfg(feature = "mmap")]
        if let Some(dir) = self.wal.as_ref().map(|w| w.dir.clone()) {
            let g = self.create_durable_named(&dir, name)?;
            self.named.push((name.clone(), g));
            return Ok(self.named.len() - 1);
        }
        self.named.push((name.clone(), Graph::from_parts(Dict::new(), Vec::new())));
        Ok(self.named.len() - 1)
    }

    /// The in-memory half of [`apply_delta`](Self::apply_delta) (no WAL append) — also
    /// the target the WAL replays into on [`open`](Self::open).
    fn apply_delta_mem(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
        // A delete only matters if every term resolves — otherwise the triple cannot be
        // present, and deleting must NOT intern the (absent) terms.
        let del_ids: Vec<[Id; 3]> = deletes
            .iter()
            .filter_map(|[s, p, o]| Some([self.id_of(s)?, self.id_of(p)?, self.id_of(o)?]))
            .collect();
        let old_len = self.dict.len();
        let ins_ids: Vec<[Id; 3]> = inserts
            .iter()
            .map(|[s, p, o]| [self.dict.intern(s), self.dict.intern(p), self.dict.intern(o)])
            .collect();
        // Keep the numeric- and temporal-filter caches covering the grown dictionary.
        self.numerics.extend_for(&self.dict, old_len);
        self.temporals.extend_for(&self.dict, old_len);
        // sq-lr2ii: an inserted term may be an f64-inexact decimal. If the sargable-safety
        // guard was memoised as "no such decimal" (1), reset it to recompute over the grown
        // dictionary; a "found" (2) verdict is monotonic (terms are never removed) and stays.
        let _ = self.high_precision_decimal.compare_exchange(
            1,
            0,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.store.apply_delta(&ins_ids, &del_ids);
    }

    /// Folds the delta-overlay into a REBUILT immutable base (the periodic compaction
    /// that keeps scans overlay-free). The dictionary is kept as-is — ids are stable;
    /// terms only referenced by deleted triples linger until a full reload (cheap, and
    /// it keeps compaction O(triples) with no re-interning). For a directory-backed
    /// graph the new base is persisted ATOMICALLY (written to a fresh sibling directory,
    /// then swapped in via rename) and the write-ahead log truncated; the graph re-opens
    /// memory-mapped from the new base.
    pub fn compact(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Fold the structural-fork layers first (ids unchanged throughout): named
        // graphs recursively, then this graph's dictionary extension into a fresh
        // frozen base (so the NEXT fork is O(1) again) and the forked caches flat.
        // No-ops on a never-forked graph.
        for (_, g) in &mut self.named {
            g.compact()?;
        }
        if self.dict.is_forked() {
            self.dict = self.dict.compacted();
            let n = self.dict.len();
            let numerics = std::mem::replace(&mut self.numerics, NumData::Sparse(rustc_hash::FxHashMap::default()));
            self.numerics = numerics.fold(n);
            let temporals = std::mem::replace(&mut self.temporals, TempData::Sparse(rustc_hash::FxHashMap::default()));
            self.temporals = temporals.fold(n);
        }
        if self.store.has_overlay() {
            let triples: Vec<[Id; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows.iter().map(|r| scan.to_spo(r)).collect()
            };
            self.store = TripleStore::from_triples(triples);
        } else {
            // Nothing pending: for a directory-backed graph just discard the (no-op) log.
            #[cfg(feature = "mmap")]
            if let Some(w) = &mut self.wal {
                w.truncate().map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            self.persist_swap(&dir)?;
        }
        Ok(())
    }

    /// [OPUS-4.8] (sq-x32t) ROLLBACK-SAFE on-disk swap shared by [`compact`](Self::compact) and
    /// [`vacuum`](Self::vacuum): persist `self`'s CURRENT in-memory image as the new durable base
    /// at `dir`, atomically, truncating the old WAL. Factored out of `compact` (review 1593) so
    /// the erasure-grade vacuum reuses the exact same crash-safe machinery.
    ///
    /// The two-rename swap has a window where the canonical `dir` does not exist (between the two
    /// renames); a crash there must NOT lose the dataset. We:
    ///
    ///   1. write the new base to `<dir>.compact-new` and fsync the DIRECTORY (so its existence +
    ///      contents are durable before it can become canonical);
    ///   2. rename `dir` -> `<dir>.compact-old`, fsync the parent (the rename is durable);
    ///   3. rename `<dir>.compact-new` -> `dir`, fsync the parent.
    ///
    /// If a crash interrupts the swap, [`recover_compaction`] (run on every [`open`](Self::open))
    /// deterministically completes or rolls it back from the surviving sibling. The graph is then
    /// re-opened memory-mapped from the new base with a fresh, empty WAL.
    #[cfg(feature = "mmap")]
    fn persist_swap(&mut self, dir: &std::path::Path) -> Result<(), String> {
        // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be renamed
        // away with `dir`), then run the shared crash-safe swap, writing `self`'s CURRENT image as
        // the new base. Adopt the re-opened directory-backed graph in place. [OPUS-4.8] (sq-ft7u)
        self.wal = None;
        let reopened = swap_dir_to_new_base(dir, |new_dir| self.save(new_dir))
            .map_err(|e| e.to_string())?;
        *self = reopened;
        Ok(())
    }

    /// [OPUS-4.8] (sq-ft7u) PUBLIC crash-safe RESTORE-INTO-DURABLE seam. Atomically REPLACE the
    /// durable contents of the directory-backed store at `dir` with `fresh`'s image, reusing the
    /// EXACT same rollback-safe two-rename protocol as the internal compaction swap (the helper
    /// `persist_swap` also calls — there is one crash-safe swap implementation, not two). Returns
    /// the directory-backed [`Graph`] re-opened memory-mapped from the new base (a fresh, empty
    /// WAL), so the caller's subsequent updates WAL-append to the restored store.
    ///
    /// This is the durable counterpart to the server's online in-memory restore: the caller imports
    /// (and thus FULLY validates) the backup artifact into `fresh` BEFORE calling this, so the swap
    /// only ever runs over a known-good image.
    ///
    /// CRASH-SAFETY + FAIL-CLOSED. The protocol is: write `fresh`'s image to `<dir>.compact-new`
    /// and fsync that directory (durable before it can become canonical); rename `dir` ->
    /// `<dir>.compact-old` (parent fsync); rename `<dir>.compact-new` -> `dir` (parent fsync);
    /// re-open memory-mapped from the new base; drop `<dir>.compact-old`. There is a window where
    /// the canonical `dir` does not exist (between the two renames); a crash there is HEALED
    /// deterministically on the next `open` (which runs the compaction recovery from the surviving
    /// `.compact-new` / `.compact-old` sibling), so `dir` ends up old-or-new — NEVER corrupt or
    /// partially written. If anything fails BEFORE the first rename (e.g. writing the new base),
    /// `dir` is untouched: the old durable store survives intact.
    ///
    /// PRECONDITION (caller's responsibility): NO other handle may be mutating `dir` (WAL-appending)
    /// concurrently — the caller must quiesce the durable writer that owns `dir` before invoking
    /// this, exactly as the in-process compaction swap runs only on the single writer thread.
    #[cfg(feature = "mmap")]
    pub fn restore_into_durable(
        dir: &std::path::Path,
        fresh: Graph,
    ) -> std::io::Result<Graph> {
        swap_dir_to_new_base(dir, |new_dir| fresh.save(new_dir))
    }

    /// [OPUS-4.8] (sq-ft7u) The directory this graph is durably backed by (its `--persist` dir),
    /// or `None` for an in-memory graph. The companion to [`restore_into_durable`](Self::restore_into_durable):
    /// a caller doing an in-place durable restore reads the backing dir, then closes the WAL
    /// (see [`close_wal`](Self::close_wal)) before swapping the directory over to the new base.
    #[cfg(feature = "mmap")]
    pub fn persist_dir(&self) -> Option<std::path::PathBuf> {
        self.wal.as_ref().map(|w| w.dir.clone())
    }

    /// [OPUS-4.8] (sq-ft7u) Close (drop) this graph's write-ahead-log handle, if any. Used right
    /// before a directory swap that renames the backing dir away (the WAL's `wal.log` lives under
    /// it), exactly as the internal compaction swap does — the swap then re-opens a fresh handle.
    /// A no-op for an in-memory graph. Safe to call on a graph whose dir is about to be replaced
    /// by [`restore_into_durable`](Self::restore_into_durable); the graph keeps serving reads from
    /// its in-memory image, it simply stops appending to the (about-to-be-renamed) log.
    #[cfg(feature = "mmap")]
    pub fn close_wal(&mut self) {
        self.wal = None;
    }

    /// [OPUS-4.8] (sq-x32t, epic sq-toze.33) ERASURE-GRADE VACUUM. Like [`compact`](Self::compact)
    /// it physically rewrites a directory-backed store to contain only the current LIVE triples —
    /// but it additionally PURGES THE DICTIONARY: it re-interns every live term into a FRESH
    /// dictionary, so a term string (an IRI or a literal VALUE — e.g. personal data) that is no
    /// longer referenced by any live triple after a `DELETE` / `DROP GRAPH` is physically gone
    /// from the on-disk dictionary blob too, not just from the triple indexes.
    ///
    /// Why this exists separately from `compact`: `compact` keeps the dictionary as-is (ids stay
    /// stable, which makes the serving-path fold O(triples) with no re-interning); orphaned term
    /// strings linger in the dict until a full reload. That is the right trade for the periodic
    /// serving compaction, but NOT for erasure — a deleted literal's bytes would survive. `vacuum`
    /// pays the O(terms) re-intern so erasure is complete down to the value bytes.
    ///
    /// The live dataset (default graph + every named graph, recursively) is dumped to `[Term; 3]`
    /// and re-interned into a brand-new [`Graph`] with an empty [`Dict`]; that fresh image is then
    /// swapped in via the SAME rollback-safe `persist_swap` the compaction
    /// uses (atomic, crash-safe, WAL-truncated). The live triple set is preserved EXACTLY
    /// (round-trip). For an in-memory graph it just replaces the dictionary/store in place.
    ///
    /// PHYSICAL-ERASURE SCOPE (honest): this scrubs the engine's own on-disk segments + dictionary;
    /// it cannot reach bytes already copied off-box (filesystem snapshots, block-level COW history,
    /// external backups), which the storage/backup tier must handle per the retention-erasure
    /// runbook.
    pub fn vacuum(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Re-intern the whole live dataset into a fresh graph with an EMPTY dictionary, so any
        // term orphaned by a prior delete/drop is dropped (it is never re-interned).
        let mut fresh = self.reintern_live()?;
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be
            // renamed away). `persist_swap` writes `fresh`'s image to `dir` and re-opens
            // `fresh` memory-mapped from the new base with a fresh WAL; adopt that re-opened,
            // directory-backed graph back into `self` so subsequent updates WAL-append to it.
            self.wal = None;
            fresh.persist_swap(&dir)?;
        }
        // Adopt the freshly re-interned (and, when persisted, re-opened) image in place.
        std::mem::swap(self, &mut fresh);
        Ok(())
    }

    /// [OPUS-4.8] (sq-x32t) Builds a fresh in-memory [`Graph`] holding exactly this graph's LIVE
    /// triples (default + named, recursively), re-interned into a brand-new [`Dict`] — so terms
    /// orphaned by a delete/drop are absent from the result. Carries NO directory/WAL association
    /// (the caller's [`persist_swap`](Self::persist_swap) gives the swapped-in graph its own).
    fn reintern_live(&self) -> Result<Graph, String> {
        // Dump the live default-graph triples as Terms (overlay already merged by `scan`).
        let live: Vec<[Term; 3]> = {
            let scan = self.store.scan(&[None, None, None]);
            scan.rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    [self.dict.term(spo[0]), self.dict.term(spo[1]), self.dict.term(spo[2])]
                })
                .collect()
        };
        let mut fresh = Graph::from_parts(Dict::new(), Vec::new());
        if !live.is_empty() {
            fresh.apply_delta(&live, &[])?;
        }
        // Named graphs: re-intern each recursively and attach it as an IN-MEMORY sub-graph (no
        // directory/WAL — the parent's `save` writes the whole `named/` sub-tree afresh).
        for (name, sub) in &self.named {
            let sub_fresh = sub.reintern_live()?;
            fresh.named.push((name.clone(), sub_fresh));
        }
        Ok(fresh)
    }
}

/// [OPUS-4.8] (gh-1118) An empty, in-memory [`Graph`] — the trivial INFALLIBLE default. Delegates
/// to [`Graph::new`], so `Graph::default()` and `Graph::new()` are interchangeable.
impl Default for Graph {
    #[inline]
    fn default() -> Self {
        Graph::new()
    }
}

/// [OPUS-4.8] (review 1593) fsync a DIRECTORY so a rename of (or into) it is durable. On
/// failure (e.g. a platform/filesystem that rejects opening a dir) it is a best-effort no-op,
/// matching the durability the rest of the persistence path assumes.
#[cfg(feature = "mmap")]
fn fsync_dir(dir: &std::path::Path) -> std::io::Result<()> {
    match std::fs::File::open(dir) {
        Ok(f) => match f.sync_all() {
            Ok(()) => Ok(()),
            // Some filesystems return EINVAL/unsupported for fsync on a directory handle;
            // that is not a data-loss condition, so treat it as success.
            Err(e) if matches!(e.kind(), std::io::ErrorKind::InvalidInput) => Ok(()),
            Err(e) => Err(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// [OPUS-4.8] (sq-ft7u) The ONE crash-safe directory-base swap, factored out of
/// [`persist_swap`](Graph::persist_swap) so the public [`restore_into_durable`](Graph::restore_into_durable)
/// reuses the IDENTICAL rollback-safe protocol rather than copy-pasting it. `write_new_base` is
/// handed the (cleared) `<dir>.compact-new` sibling and must write the desired new durable image
/// there (e.g. `graph.save(new_dir)`). On success the new base is canonical at `dir` and a graph
/// re-opened memory-mapped from it (fresh, empty WAL) is returned.
///
/// The protocol (matching the design recorded on `persist_swap`):
///   1. clear any stale siblings, then write the new base to `<dir>.compact-new` and fsync that
///      DIRECTORY (so it is durable before it can become canonical);
///   2. rename `dir` -> `<dir>.compact-old`, fsync the parent (the rename is durable);
///   3. rename `<dir>.compact-new` -> `dir`, fsync the parent;
///   4. re-open from the new base, THEN drop `<dir>.compact-old` (open mmaps keep unlinked files
///      alive on unix, so the re-open must precede the cleanup).
///
/// FAIL-CLOSED: if `write_new_base` (or its fsync) fails — i.e. anything BEFORE the first rename —
/// `dir` is never touched, so the old durable store survives intact. A crash DURING the two
/// renames is healed by [`recover_compaction`] on the next [`open`](Graph::open), which promotes
/// `compact-new` or rolls back to `compact-old` — `dir` is never lost.
#[cfg(feature = "mmap")]
fn swap_dir_to_new_base<F>(dir: &std::path::Path, write_new_base: F) -> std::io::Result<Graph>
where
    F: FnOnce(&std::path::Path) -> std::io::Result<()>,
{
    let new_dir = dir.with_extension("compact-new");
    let old_dir = dir.with_extension("compact-old");
    std::fs::remove_dir_all(&new_dir).ok();
    std::fs::remove_dir_all(&old_dir).ok();
    // Build + sync the new base FIRST. A failure here leaves `dir` untouched (fail-closed) —
    // the two renames below have not started, so the old durable store is intact.
    write_new_base(&new_dir)?;
    fsync_dir(&new_dir)?;
    let parent = dir.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::rename(dir, &old_dir)?;
    fsync_dir(parent)?;
    std::fs::rename(&new_dir, dir)?;
    fsync_dir(parent)?;
    // Re-open memory-mapped from the new base (fresh, empty WAL); only then drop the old files.
    let reopened = Graph::open(dir)?;
    std::fs::remove_dir_all(&old_dir).ok();
    Ok(reopened)
}

/// [OPUS-4.8] (review 1593) Recover an INTERRUPTED [`compact`](Graph::compact) directory swap
/// so the canonical `dir` always ends up present after any crash. Deterministic from the
/// surviving siblings (`<dir>.compact-old` / `<dir>.compact-new`):
///
/// - `dir` present: the swap completed (or never started) — just remove any stale siblings.
/// - `dir` MISSING, `compact-new` present: a crash between the two renames — the new base was
///   already fully written + dir-fsynced, so COMPLETE the swap by promoting `compact-new`.
/// - `dir` MISSING, only `compact-old` present: a crash before the new base was ready — ROLL
///   BACK by restoring `compact-old`.
#[cfg(feature = "mmap")]
fn recover_compaction(dir: &std::path::Path) -> std::io::Result<()> {
    let new_dir = dir.with_extension("compact-new");
    let old_dir = dir.with_extension("compact-old");
    let parent = dir.parent().unwrap_or_else(|| std::path::Path::new("."));
    if dir.exists() {
        // Canonical dir is intact; drop any leftover siblings from a crash after the swap.
        std::fs::remove_dir_all(&new_dir).ok();
        std::fs::remove_dir_all(&old_dir).ok();
        return Ok(());
    }
    if new_dir.exists() {
        // Complete the swap: the new base was fully written + synced before the swap began.
        std::fs::rename(&new_dir, dir)?;
        fsync_dir(parent)?;
        std::fs::remove_dir_all(&old_dir).ok();
    } else if old_dir.exists() {
        // Roll back: restore the previous base (the new one never reached durability).
        std::fs::rename(&old_dir, dir)?;
        fsync_dir(parent)?;
    }
    Ok(())
}

/// [OPUS-4.8] (sq-glw2) Recover an INTERRUPTED [`drop_named_durable`](Graph::drop_named_durable)
/// named-sub-tree swap so `named/` always ends up consistent with the manifest after any crash.
/// Mirrors [`recover_compaction`] but scoped to the `named/` sub-tree (`named.drop-old` /
/// `named.drop-new` siblings):
///
/// - `named/` present: the swap completed (or never started) — drop any stale siblings. The
///   manifest is then reconciled by [`open_named`](Graph::open_named) (it trusts the manifest's
///   count; extra leftover sub-dirs beyond it are ignored, a missing one surfaces as a clean
///   error).
/// - `named/` MISSING, `named.drop-new` present: a crash between the two renames — the new
///   (renumbered survivor) sub-tree was fully written + fsync'd, so COMPLETE the swap by
///   promoting it.
/// - `named/` MISSING, only `named.drop-old` present: a crash before the new sub-tree was
///   durable — ROLL BACK by restoring the old sub-tree (the manifest still matches it).
///
/// Run BEFORE [`open_named`](Graph::open_named) on every open. Best-effort and idempotent.
#[cfg(feature = "mmap")]
fn recover_named_drop(dir: &std::path::Path) -> std::io::Result<()> {
    let named_dir = dir.join(NAMED_SUBDIR);
    let new_dir = named_dir.with_extension("drop-new");
    let old_dir = named_dir.with_extension("drop-old");
    if named_dir.exists() {
        std::fs::remove_dir_all(&new_dir).ok();
        std::fs::remove_dir_all(&old_dir).ok();
        return Ok(());
    }
    if new_dir.exists() {
        // The new survivor sub-tree was fully written + fsync'd before the swap began.
        std::fs::rename(&new_dir, &named_dir)?;
        fsync_dir(dir)?;
        std::fs::remove_dir_all(&old_dir).ok();
    } else if old_dir.exists() {
        // The new sub-tree never reached durability: restore the previous one.
        std::fs::rename(&old_dir, &named_dir)?;
        fsync_dir(dir)?;
    }
    Ok(())
}

/// [OPUS-4.8] (sq-3ui0, gh-45) The subdirectory holding the persisted named graphs (one
/// numbered sub-directory per named graph), and the manifest file naming them in order.
#[cfg(feature = "mmap")]
const NAMED_SUBDIR: &str = "named";
#[cfg(feature = "mmap")]
const NAMED_MANIFEST: &str = "named.bin";
/// [OPUS-4.8] (Copilot review #69, finding 1) Temp filename for the ATOMIC manifest write: the
/// full image is written here, fsync'd, then renamed over [`NAMED_MANIFEST`] so a crash
/// mid-write never leaves a partial/corrupt `named.bin`. See [`write_named_manifest`].
#[cfg(feature = "mmap")]
const NAMED_MANIFEST_TMP: &str = "named.bin.tmp";
/// Manifest magic ("NMG1" little-endian) — identifies a named-graph manifest and lets a
/// stray/legacy file be rejected rather than misread.
#[cfg(feature = "mmap")]
const NAMED_MANIFEST_MAGIC: u32 = 0x31_47_4D_4E;
/// On-disk named-graph format version. Bumped if the named-graph LAYOUT changes
/// incompatibly; `open_named` refuses an unknown version rather than silently misreading.
#[cfg(feature = "mmap")]
const NAMED_FORMAT_VERSION: u32 = 1;
/// [OPUS-4.8] (Copilot review #69, finding 3) Smallest possible on-disk size of one manifest
/// entry: `[kind u8][len u32]` (1 + 4) with a zero-length value. The post-header byte count
/// divided by this is a hard upper bound on the entry count, so `open_named` clamps its
/// pre-allocation to it — a hostile `count` field can never drive an unbounded reservation.
#[cfg(feature = "mmap")]
const MIN_MANIFEST_ENTRY_BYTES: usize = 5;

/// [OPUS-4.8] (sq-3ui0) Writes the named-graph manifest `dir/named.bin`: the magic +
/// format version + count, then each graph name in order (see [`encode_graph_name`]). The
/// sub-graph directories `dir/named/<i>/` must already be durable before this is called —
/// the manifest's presence is the commit point for the named-graph set.
///
/// [OPUS-4.8] (Copilot review #69, findings 1+2) The write is ATOMIC and DURABLE: a plain
/// `std::fs::write` truncates `named.bin` IN PLACE, so a crash mid-write leaves a partial
/// (corrupt) manifest that `open_named` then rejects as `InvalidData` — bricking the dataset.
/// Instead we write the full image to a sibling `named.bin.tmp`, fsync IT (its bytes are
/// durable), `rename` it over `named.bin` (the rename is atomic — a reader sees either the old
/// complete manifest or the new complete manifest, never a torn one), then fsync the PARENT
/// `dir` (which is what actually contains `named.bin`) so the rename — i.e. the commit point of
/// the whole named-graph set — survives a crash. This is the same temp-file + fsync + rename +
/// dir-fsync discipline the compaction directory-swap uses (`Graph::compact`).
#[cfg(feature = "mmap")]
fn write_named_manifest(dir: &std::path::Path, names: &[Term]) -> std::io::Result<()> {
    use std::io::Write;
    let mut buf = Vec::with_capacity(12 + names.len() * 32);
    buf.extend_from_slice(&NAMED_MANIFEST_MAGIC.to_le_bytes());
    buf.extend_from_slice(&NAMED_FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&(names.len() as u32).to_le_bytes());
    for name in names {
        encode_graph_name(&mut buf, name);
    }
    let final_path = dir.join(NAMED_MANIFEST);
    let tmp_path = dir.join(NAMED_MANIFEST_TMP);
    // Write the full image to a sibling temp file and fsync its CONTENTS before the rename, so
    // the bytes the rename publishes are already durable (a crash can't surface a
    // zero-length/partial file under the canonical name).
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(&buf)?;
        f.sync_all()?;
    }
    // Atomically publish: rename can't tear, so `named.bin` is always a COMPLETE manifest.
    std::fs::rename(&tmp_path, &final_path)?;
    // fsync the dir holding `named.bin` so the rename (the commit) is itself durable.
    fsync_dir(dir)
}

/// [OPUS-4.8] (sq-glw2) Durably REMOVES the named-graph manifest `dir/named.bin` (the commit
/// point when a [`drop_named_durable`](Graph::drop_named_durable) leaves NO named graphs): once
/// the manifest is gone, [`open_named`](Graph::open_named) reports zero named graphs regardless
/// of any leftover `named/` sub-tree (a crash after this leaves only an orphan sub-tree, cleaned
/// up by [`recover_named_drop`]). The parent `dir` is fsync'd so the removal survives a crash.
/// A no-op (success) when there is no manifest.
#[cfg(feature = "mmap")]
fn remove_named_manifest(dir: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(NAMED_MANIFEST)) {
        Ok(()) => fsync_dir(dir),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// [OPUS-4.8] (sq-3ui0) Serialise a graph-name term into the manifest buffer as
/// `[kind u8][len u32][raw value bytes]`, where kind is 0 for a NamedNode (value = the
/// raw IRI) and 1 for a BlankNode (value = the raw label). Storing the RAW string with a
/// length prefix avoids any N-Triples escaping ambiguity on the round-trip. Graph names
/// are only ever NamedNode or BlankNode (never a literal/triple — see
/// [`load_dataset`](Graph::load_dataset) / [`apply_delta_nquads`](Graph::apply_delta_nquads)).
#[cfg(feature = "mmap")]
fn encode_graph_name(buf: &mut Vec<u8>, name: &Term) {
    // Graph names are constrained to NamedNode | BlankNode by every public loader; encode
    // each with its raw value. A literal/triple graph name cannot occur, so map it to its
    // lexical form as a NamedNode rather than panicking (keeps the encoding total).
    let (kind, val): (u8, String) = match name {
        Term::NamedNode(n) => (0, n.as_str().to_string()),
        Term::BlankNode(b) => (1, b.as_str().to_string()),
        other => (0, other.to_string()),
    };
    buf.push(kind);
    let bytes = val.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// [OPUS-4.8] (sq-3ui0) Inverse of [`encode_graph_name`]: decode the term starting at
/// `pos`, returning it plus the offset just past it. Bounds-checked — a truncated or
/// non-UTF-8 record is a clean `Err`, never a panic / OOB read (the manifest is a trusted
/// asset, but the loader stays memory-safe under corruption like the rest of `open`).
#[cfg(feature = "mmap")]
fn decode_graph_name(bytes: &[u8], pos: usize) -> Result<(Term, usize), String> {
    let kind = *bytes.get(pos).ok_or("truncated manifest (missing kind byte)")?;
    let lo = pos + 1;
    let len_end = lo + 4;
    let len_bytes = bytes.get(lo..len_end).ok_or("truncated manifest (missing length)")?;
    let len = u32::from_le_bytes(len_bytes.try_into().expect("4 bytes")) as usize;
    let val_end = len_end + len;
    let val_bytes = bytes.get(len_end..val_end).ok_or("truncated manifest (length exceeds buffer)")?;
    let val = std::str::from_utf8(val_bytes).map_err(|_| "non-UTF-8 graph name in manifest".to_string())?;
    let term = match kind {
        0 => Term::NamedNode(oxrdf::NamedNode::new(val).map_err(|e| format!("invalid IRI graph name: {e}"))?),
        1 => Term::BlankNode(oxrdf::BlankNode::new(val).map_err(|e| format!("invalid blank-node graph name: {e}"))?),
        k => return Err(format!("unknown graph-name kind byte {k}")),
    };
    Ok((term, val_end))
}

/// The append-only write-ahead log of update operations for a directory-backed graph.
///
/// [OPUS-4.8] (review 1593) Entries are framed as ATOMIC BATCHES, not independent per-triple
/// records, so a crash mid-append can never leave a PARTIALLY-applied batch on replay. Each
/// batch is laid out as:
///
/// ```text
/// [BATCH_MAGIC u32][n_records u32][body_len u32]   <- header
/// <body_len bytes of records>                       <- each: [op u8][len u32][N-Triples bytes]
/// [checksum u64][COMMIT_MARKER u32]                 <- commit trailer (written last)
/// ```
///
/// The checksum (FNV-1a over the header + body) plus the trailing commit marker form the
/// COMMIT POINT: `replay` only applies a batch whose trailer is present AND whose checksum
/// matches, and TRUNCATES the log at the start of the first batch that fails either check
/// (the torn tail of an interrupted append). So replay applies exactly the batches that were
/// fully durable — never a prefix of one. The whole batch is `write_all`'d then fsync'd once.
#[cfg(feature = "mmap")]
struct Wal {
    file: std::fs::File,
    dir: std::path::PathBuf,
}

#[cfg(feature = "mmap")]
impl Wal {
    const INSERT: u8 = 0;
    const DELETE: u8 = 1;
    /// Batch frame magic ("WBA1" little-endian) — starts every batch header.
    const BATCH_MAGIC: u32 = 0x31_41_42_57;
    /// Commit marker ("DONE" little-endian) — written last, after the checksum.
    const COMMIT_MARKER: u32 = 0x45_4E_4F_44;

    fn path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("wal.log")
    }

    fn open(dir: &std::path::Path) -> std::io::Result<Wal> {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(Self::path(dir))?;
        Ok(Wal { file, dir: dir.to_path_buf() })
    }

    /// FNV-1a 64-bit checksum (no extra deps; deterministic across runs/platforms).
    fn checksum(bytes: &[u8]) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        h
    }

    /// Appends one ATOMIC batch — delete records first, then inserts (the order
    /// `apply_delta` applies them) — framed with a header + checksum + commit marker, and
    /// fsyncs, so the WHOLE batch is durable as a unit BEFORE it is applied in memory. A
    /// crash mid-append leaves an uncommitted tail that `replay` discards entirely.
    fn append_batch(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) -> std::io::Result<()> {
        use std::io::Write;
        let mut body = Vec::new();
        let mut n_records = 0u32;
        for t in deletes {
            Self::push_record(&mut body, Self::DELETE, t);
            n_records += 1;
        }
        for t in inserts {
            Self::push_record(&mut body, Self::INSERT, t);
            n_records += 1;
        }
        let mut buf = Vec::with_capacity(body.len() + 24);
        buf.extend_from_slice(&Self::BATCH_MAGIC.to_le_bytes());
        buf.extend_from_slice(&n_records.to_le_bytes());
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);
        // Checksum covers everything written so far (header + body); the commit marker is
        // appended AFTER the checksum so a torn write can't produce a valid-looking trailer.
        let crc = Self::checksum(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&Self::COMMIT_MARKER.to_le_bytes());
        self.file.write_all(&buf)?;
        self.file.sync_data()
    }

    fn push_record(buf: &mut Vec<u8>, op: u8, t: &[Term; 3]) {
        // oxrdf's Display is the canonical N-Triples serialisation of each term.
        let line = format!("{} {} {} .\n", t[0], t[1], t[2]);
        buf.push(op);
        buf.extend_from_slice(&(line.len() as u32).to_le_bytes());
        buf.extend_from_slice(line.as_bytes());
    }

    /// Empties the log (after a compaction persisted the folded base).
    fn truncate(&mut self) -> std::io::Result<()> {
        self.file.set_len(0)?;
        self.file.sync_data()
    }

    /// Reads `dir`'s log into `(is_insert, triple)` ops in append order, batch by batch.
    /// Only fully-committed batches (intact header + body + matching checksum + commit
    /// marker) are applied; the file is TRUNCATED at the start of the first incomplete or
    /// corrupt batch — the torn tail of an interrupted append — so the partial batch is
    /// NEVER partially applied and subsequent appends start at a clean batch boundary.
    fn replay(dir: &std::path::Path) -> std::io::Result<Vec<(bool, [Term; 3])>> {
        let path = Self::path(dir);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut ops = Vec::new();
        let mut good = 0usize; // end of the last fully-committed batch
        'batches: while good < bytes.len() {
            let bstart = good;
            // Header: magic + n_records + body_len (12 bytes).
            if bstart + 12 > bytes.len() {
                break; // torn header
            }
            let rd = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
            if rd(bstart) != Self::BATCH_MAGIC {
                break; // not a batch frame (corruption / torn tail)
            }
            let n_records = rd(bstart + 4) as usize;
            let body_len = rd(bstart + 8) as usize;
            let body_start = bstart + 12;
            let Some(body_end) = body_start.checked_add(body_len).filter(|&e| e <= bytes.len()) else {
                break; // torn body
            };
            // Commit trailer: checksum (8) + marker (4).
            let Some(trailer_end) = body_end.checked_add(12).filter(|&e| e <= bytes.len()) else {
                break; // trailer not yet written — uncommitted batch
            };
            let stored_crc = u64::from_le_bytes(bytes[body_end..body_end + 8].try_into().expect("8 bytes"));
            let marker = rd(body_end + 8);
            if marker != Self::COMMIT_MARKER || Self::checksum(&bytes[bstart..body_end]) != stored_crc {
                break; // uncommitted / corrupt batch — discard the tail
            }
            // The batch is committed: parse its records. (A parse failure here would mean a
            // committed-but-corrupt body — treat it as a torn tail and stop, conservatively.)
            let mut pos = body_start;
            let mut batch_ops = Vec::with_capacity(n_records);
            for _ in 0..n_records {
                if pos + 5 > body_end {
                    break 'batches;
                }
                let op = bytes[pos];
                let len = rd(pos + 1) as usize;
                let start = pos + 5;
                let Some(end) = start.checked_add(len).filter(|&e| e <= body_end) else {
                    break 'batches;
                };
                let Some(t) = parse_nt_record(&bytes[start..end]) else {
                    break 'batches;
                };
                batch_ops.push((op == Self::INSERT, t));
                pos = end;
            }
            if batch_ops.len() != n_records {
                break; // body record count disagreed with the header — corrupt
            }
            ops.extend(batch_ops);
            good = trailer_end;
        }
        if good < bytes.len() {
            let f = std::fs::OpenOptions::new().write(true).open(&path)?;
            f.set_len(good as u64)?;
            f.sync_data()?;
        }
        Ok(ops)
    }
}

/// [OPUS-4.8] (sq-ycle) One resolved redo-journal record: `(is_insert, slot, triple)` — an
/// insert/delete flag, the graph slot (`None` = default graph, `Some` = that named graph) and the
/// term-triple. The unit the parent-level [`TxnJournal`] commits and [`Graph::commit_txn`] takes.
#[cfg(feature = "mmap")]
type TxnRecord = (bool, Option<Term>, [Term; 3]);

/// [OPUS-4.8] (sq-ycle) Serialise a journal SLOT — the graph a record targets — into `buf`.
/// A named slot reuses the manifest's [`encode_graph_name`] (kind 0 NamedNode / 1 BlankNode);
/// the DEFAULT graph (`None`) gets a distinct marker byte (kind 2, no payload) so [`decode_slot`]
/// can tell "default graph" apart from "named graph" unambiguously on replay.
#[cfg(feature = "mmap")]
fn encode_slot(buf: &mut Vec<u8>, slot: &Option<Term>) {
    match slot {
        Some(name) => encode_graph_name(buf, name),
        // Default-graph marker: kind 2, no length/value (kinds 0/1 are NamedNode/BlankNode).
        None => buf.push(2),
    }
}

/// [OPUS-4.8] (sq-ycle) Inverse of [`encode_slot`]: decode a slot at `pos`, returning it plus the
/// offset just past it. Kind 2 (no payload) is the default graph; 0/1 defer to [`decode_graph_name`]
/// for a NamedNode/BlankNode. Bounds-checked — a truncated record is a clean `Err`, never a panic.
#[cfg(feature = "mmap")]
fn decode_slot(bytes: &[u8], pos: usize) -> Result<(Option<Term>, usize), String> {
    match bytes.get(pos) {
        Some(2) => Ok((None, pos + 1)),
        Some(0) | Some(1) => {
            let (term, end) = decode_graph_name(bytes, pos)?;
            Ok((Some(term), end))
        }
        Some(k) => Err(format!("unknown txn slot kind byte {k}")),
        None => Err("truncated txn record (missing slot kind byte)".to_string()),
    }
}

/// [OPUS-4.8] (sq-ycle) A parent-level REDO JOURNAL — the single durable COMMIT POINT for a
/// whole multi-operation SPARQL UPDATE body — at `dir/txn.log`, a sibling of the per-graph [`Wal`].
///
/// The problem it fixes: a multi-op body like `DROP SILENT GRAPH <r> ; INSERT DATA { GRAPH <r> ... ;
/// GRAPH <parent> ldp:contains <r> }` materialises through [`apply_effects`] as N independent
/// `apply_delta` calls across DIFFERENT files (the `<r>` per-graph `wal.log`, the `<parent>`
/// per-graph `wal.log`, the `named.bin` manifest), each fsync'd on its own. A crash between them
/// leaves a PARTIAL durable write (parent containment present but child graph missing, or vice
/// versa). This journal records the WHOLE resolved per-slot quad-delta of one `apply_effects` call
/// as ONE framed, checksummed, single-`sync_data()` frame. That single fsync is THE commit point;
/// the per-graph WAL appends still happen but are now materialisation, not the commit point.
///
/// On [`open`](Graph::open), AFTER the per-graph WALs are replayed, any committed `txn.log` frame
/// is replayed idempotently and then the journal is truncated. [OPUS-4.8] (Copilot #135) The redo
/// goes through the DURABLE [`apply_delta`](Graph::apply_delta) (WAL-logged + fsync'd), NOT the
/// in-memory-only `apply_delta_mem`, and the truncation happens only AFTER every record is durably
/// in a per-graph WAL — so a record that lived ONLY in `txn.log` (the crash-after-commit /
/// before-materialisation window) is moved into durable storage on recovery and survives the NEXT
/// restart, instead of being applied in-memory once and then erased by the truncation. Idempotency
/// holds because [`apply_delta`](Graph::apply_delta) replays with set semantics (re-inserting a
/// present triple / deleting an absent one is a no-op) and [`ensure_named`](Graph::ensure_named) is
/// find-or-create — so re-applying records the per-graph WALs already materialised changes nothing.
///
/// Frame layout (byte-identical write+fsync discipline to [`Wal::append_batch`]):
///
/// ```text
/// [TXN_MAGIC u32][n_records u32][body_len u32]   <- header
/// <body_len bytes of records>                     <- each: [op u8][slot ...][nt_len u32][N-Triples]
/// [checksum u64][COMMIT_MARKER u32]               <- commit trailer (written last)
/// ```
///
/// where a record's SLOT is [`encode_slot`]'s `[kind u8]( [len u32][value] )?` and the triple is
/// the N-Triples line + `u32` length, exactly as [`Wal::push_record`] frames it (so the same
/// [`parse_nt_record`] decodes it). The FNV-1a [`Wal::checksum`] over header+body plus the trailing
/// [`Wal::COMMIT_MARKER`] form the commit point: [`replay`](TxnJournal::replay) applies only frames
/// whose trailer is present AND whose checksum matches, and truncates at the first torn/corrupt
/// frame — so a torn frame is discarded WHOLE, never partially applied.
#[cfg(feature = "mmap")]
struct TxnJournal {
    file: std::fs::File,
}

#[cfg(feature = "mmap")]
impl TxnJournal {
    const INSERT: u8 = 0;
    const DELETE: u8 = 1;
    /// Frame magic ("WTX1" little-endian) — distinct from `Wal::BATCH_MAGIC` so a `txn.log` frame
    /// can never be mistaken for a `wal.log` batch (and vice versa) on a misdirected read.
    const TXN_MAGIC: u32 = 0x31_58_54_57;

    fn path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("txn.log")
    }

    fn open(dir: &std::path::Path) -> std::io::Result<TxnJournal> {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(Self::path(dir))?;
        Ok(TxnJournal { file })
    }

    /// Frames `records` (each `(is_insert, slot, triple)`) into ONE checksummed frame and fsyncs —
    /// THE single commit point for one `apply_effects` body. A crash mid-append leaves an
    /// uncommitted tail that [`replay`](Self::replay) discards entirely. An empty record set is a
    /// no-op (an UPDATE that resolves to no data change carries no atomicity obligation).
    fn append(&mut self, records: &[TxnRecord]) -> std::io::Result<()> {
        use std::io::Write;
        if records.is_empty() {
            return Ok(());
        }
        let mut body = Vec::new();
        for (insert, slot, t) in records {
            let op = if *insert { Self::INSERT } else { Self::DELETE };
            body.push(op);
            encode_slot(&mut body, slot);
            // The triple as N-Triples + u32 length — byte-identical to `Wal::push_record`'s line
            // form (minus the op byte, which we wrote before the slot above), so the same
            // `parse_nt_record` decodes it on replay.
            let line = format!("{} {} {} .\n", t[0], t[1], t[2]);
            body.extend_from_slice(&(line.len() as u32).to_le_bytes());
            body.extend_from_slice(line.as_bytes());
        }
        let mut buf = Vec::with_capacity(body.len() + 24);
        buf.extend_from_slice(&Self::TXN_MAGIC.to_le_bytes());
        buf.extend_from_slice(&(records.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);
        // Checksum covers header + body; the commit marker is appended AFTER it so a torn write
        // can't produce a valid-looking trailer (identical discipline to `Wal::append_batch`).
        let crc = Wal::checksum(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&Wal::COMMIT_MARKER.to_le_bytes());
        self.file.write_all(&buf)?;
        self.file.sync_data()
    }

    /// Empties the journal (after its frame has been materialised into the per-graph state).
    fn truncate(&mut self) -> std::io::Result<()> {
        self.file.set_len(0)?;
        self.file.sync_data()
    }

    /// Reads `dir`'s journal into `(is_insert, slot, triple)` records in append order, frame by
    /// frame. Only fully-COMMITTED frames (intact header + body + matching checksum + commit
    /// marker) are returned; the file is TRUNCATED at the start of the first incomplete/corrupt
    /// frame — the torn tail of an interrupted append — so a partial frame is NEVER partially
    /// applied. EXACT clone of [`Wal::replay`]'s torn-tail logic, extended with the per-record slot.
    fn replay(dir: &std::path::Path) -> std::io::Result<Vec<TxnRecord>> {
        let path = Self::path(dir);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut ops = Vec::new();
        let mut good = 0usize; // end of the last fully-committed frame
        'frames: while good < bytes.len() {
            let bstart = good;
            // Header: magic + n_records + body_len (12 bytes).
            if bstart + 12 > bytes.len() {
                break; // torn header
            }
            let rd = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
            if rd(bstart) != Self::TXN_MAGIC {
                break; // not a txn frame (corruption / torn tail)
            }
            let n_records = rd(bstart + 4) as usize;
            let body_len = rd(bstart + 8) as usize;
            let body_start = bstart + 12;
            let Some(body_end) = body_start.checked_add(body_len).filter(|&e| e <= bytes.len()) else {
                break; // torn body
            };
            // Commit trailer: checksum (8) + marker (4).
            let Some(trailer_end) = body_end.checked_add(12).filter(|&e| e <= bytes.len()) else {
                break; // trailer not yet written — uncommitted frame
            };
            let stored_crc = u64::from_le_bytes(bytes[body_end..body_end + 8].try_into().expect("8 bytes"));
            let marker = rd(body_end + 8);
            if marker != Wal::COMMIT_MARKER || Wal::checksum(&bytes[bstart..body_end]) != stored_crc {
                break; // uncommitted / corrupt frame — discard the tail
            }
            // The frame is committed: parse its records. A parse failure here means a
            // committed-but-corrupt body — treat it as a torn tail and stop, conservatively.
            let mut pos = body_start;
            let mut frame_ops = Vec::with_capacity(n_records);
            for _ in 0..n_records {
                if pos >= body_end {
                    break 'frames;
                }
                let op = bytes[pos];
                pos += 1;
                let Ok((slot, after_slot)) = decode_slot(&bytes[..body_end], pos) else {
                    break 'frames;
                };
                pos = after_slot;
                if pos + 4 > body_end {
                    break 'frames;
                }
                let len = rd(pos) as usize;
                let start = pos + 4;
                let Some(end) = start.checked_add(len).filter(|&e| e <= body_end) else {
                    break 'frames;
                };
                let Some(t) = parse_nt_record(&bytes[start..end]) else {
                    break 'frames;
                };
                frame_ops.push((op == Self::INSERT, slot, t));
                pos = end;
            }
            if frame_ops.len() != n_records {
                break; // body record count disagreed with the header — corrupt
            }
            ops.extend(frame_ops);
            good = trailer_end;
        }
        if good < bytes.len() {
            let f = std::fs::OpenOptions::new().write(true).open(&path)?;
            f.set_len(good as u64)?;
            f.sync_data()?;
        }
        Ok(ops)
    }
}

/// Parses one WAL record body (a single N-Triples statement) into its term triple.
#[cfg(feature = "mmap")]
fn parse_nt_record(bytes: &[u8]) -> Option<[Term; 3]> {
    let mut it = NTriplesParser::new().for_slice(bytes);
    let t = it.next()?.ok()?;
    Some([subject_term(&t.subject), Term::NamedNode(t.predicate), t.object])
}

/// The numeric-value cache for a dictionary: `numerics[id-1]` is the f64 value of term
/// `id` (NaN for non-numeric). Parallel when the `parallel` feature is on.
///
/// [SONNET-4.6] (sq-7d3dj.6) Rebuilt from borrowed `TermParts` — no per-id owned `Term`
/// allocation. Uses the `is_numeric_datatype_str` borrowed-component path already proven
/// by the spill build (`dictspill.rs`), removing O(distinct-terms) allocations from
/// `Graph::build`.
fn numerics_of(dict: &Dict) -> Vec<f64> {
    let n = dict.len();
    let numeric_of_parts = |i: usize| -> f64 {
        match dict.term_parts(i as Id + 1) {
            // [FABLE-5] (sq-9781x / sq-74oy4 / sq-6b1lj) Route through the DATATYPE-AWARE
            // acceptance (`cached_numeric_f64` → `numeric_datatype_wellformed`) on the TRIMMED
            // value, so a cache HIT is equivalent to `Num::of_literal` acceptance for exactly
            // the same f64. This admits the whitespace-padded lexical (` 1`^^xsd:integer) AND
            // rejects both the Rust-only `inf`/`infinity`/`nan` spellings (already so since
            // sq-9781x) AND the per-datatype-ill-formed lexicals `of_literal` type-errors
            // (`"1.5"^^xsd:integer`, `"1E2"^^xsd:decimal`, an i128-overflow decimal — sq-6b1lj):
            // they now fold to the NaN cache-miss sentinel, so the sargable `=`/`<` fast path
            // and `JKey::Num` value-join defer to the exact evaluator and agree with it.
            dict::TermParts::Lit { value, datatype, lang: None } if is_numeric_datatype_str(datatype) => {
                cached_numeric_f64(value, datatype)
            }
            _ => f64::NAN,
        }
    };
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(numeric_of_parts).collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        (0..n).map(numeric_of_parts).collect()
    }
}

/// Writes the numeric-value cache to disk (raw little-endian f64) so it can be
/// memory-mapped on open instead of recomputed.
#[cfg(feature = "mmap")]
fn write_numerics(path: &std::path::Path, nums: &[f64]) -> std::io::Result<()> {
    // SAFETY: reinterpret the contiguous f64 cache as bytes for writing.
    let bytes = unsafe { std::slice::from_raw_parts(nums.as_ptr().cast::<u8>(), std::mem::size_of_val(nums)) };
    std::fs::write(path, bytes)
}

/// [OPUS-4.8] (sq-7ph8) STREAM-writes the dense `numerics.bin` (`n` little-endian f64, the
/// same layout [`write_numerics`] emits and [`Graph::open`] mmaps) DIRECTLY from the cache,
/// in fixed-size blocks, without first materialising a whole-dictionary dense `Vec<f64>`.
///
/// The in-RAM save path used to call `numerics_of`/`dense_numerics`, which for a SPARSE or
/// FORKED cache rebuilds an O(distinct-terms) dense `Vec` (8 B/term) purely to write it — an
/// RSS spike on top of the resident dict. Probing the cache per id (`lookup`, O(1)) and
/// flushing a small reusable block buffer keeps peak resident memory at the block size, the
/// same bounded-write discipline the dict-spill path (`dictspill.rs`) already uses. The bytes
/// written are identical: NaN for non-numeric ids, the cached f64 otherwise.
#[cfg(feature = "mmap")]
fn stream_write_numerics(path: &std::path::Path, n: usize, num: &NumData) -> std::io::Result<()> {
    use std::io::Write;
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    const BLOCK: usize = 1 << 16; // ids per flush (512 KiB of f64)
    let mut buf: Vec<f64> = Vec::with_capacity(BLOCK.min(n));
    let flush = |w: &mut std::io::BufWriter<std::fs::File>, buf: &mut Vec<f64>| -> std::io::Result<()> {
        // SAFETY: reinterpret the contiguous f64 block as bytes for writing. (sq-7ph8)
        // - `buf` is a live `Vec<f64>` of `buf.len()` initialised elements; `size_of_val(&buf[..])`
        //   = `len * 8` covers exactly that contiguous, fully-initialised region (no over-read).
        // - target `u8` has alignment 1; the f64 source is over-aligned, so the cast never
        //   produces a misaligned access.
        // - the bytes are only READ (passed to `write_all`), never written through the alias.
        // - the `&[u8]` is consumed within this closure before `buf.clear()`; it never escapes
        //   the source borrow, so no dangling/provenance issue.
        // - native-endian reinterpret, identical to `write_numerics` above and symmetric with the
        //   native-endian read in `NumData::as_slice`: this cache is rebuilt locally, never shipped
        //   cross-arch, so write-native + read-native round-trips. Byte-identical to the old dense
        //   write (asserted by `streamed_caches_byte_identical_to_dense`).
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), std::mem::size_of_val(&buf[..])) };
        w.write_all(bytes)?;
        buf.clear();
        Ok(())
    };
    for id in 1..=n as Id {
        buf.push(num.lookup(id).unwrap_or(f64::NAN));
        if buf.len() == BLOCK {
            flush(&mut w, &mut buf)?;
        }
    }
    if !buf.is_empty() {
        flush(&mut w, &mut buf)?;
    }
    w.flush()
}

/// [OPUS-4.8] (sq-7ph8) STREAM-writes the dense `temporals.bin` (`n` little-endian f64
/// instants then `n` flag bytes — the layout [`write_temporals`] emits and
/// [`TempData::lookup`]/[`Graph::open`] read) DIRECTLY from the cache, without first
/// materialising the two whole-dictionary dense columns `dense_temporals` builds.
///
/// `dense_temporals` always allocated a fresh `(Vec<u8>, Vec<f64>)` (9 B/term) even when the
/// in-RAM cache is SPARSE (the usual non-temporal case: a tiny map), a transient
/// O(distinct-terms) RSS spike purely to feed the writer. Here the instants are streamed in
/// one id pass (probing `lookup`, O(1)) through a small reusable f64 block; the flags follow
/// in a second pass through a small `u8` block. Peak resident memory is the block size, not
/// the dictionary — matching the streamed dict-spill write. Output bytes are identical.
#[cfg(feature = "mmap")]
fn stream_write_temporals(path: &std::path::Path, n: usize, temp: &TempData) -> std::io::Result<()> {
    use std::io::Write;
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    const BLOCK: usize = 1 << 16;
    // Pass 1: the f64 instant column (NaN for non-temporal ids — temp_flag 0 carries the
    // "not temporal" decode, so the instant value of a flag-0 cell is irrelevant on read).
    let mut fbuf: Vec<f64> = Vec::with_capacity(BLOCK.min(n));
    let flush_f = |w: &mut std::io::BufWriter<std::fs::File>, buf: &mut Vec<f64>| -> std::io::Result<()> {
        // SAFETY: reinterpret the contiguous f64 instant block as bytes for writing. (sq-7ph8)
        // Same invariants as `stream_write_numerics::flush`: `size_of_val(&buf[..]) = len*8` views
        // exactly the live, initialised `Vec<f64>` region; `u8` has align 1 so no misalignment;
        // the bytes are read-only (fed to `write_all`); the `&[u8]` never escapes this closure
        // (consumed before `buf.clear()`); native-endian, symmetric with the native-endian temporal
        // read path and byte-identical to `write_temporals`. The trailing flag column below is a
        // plain `Vec<u8>` write (no unsafe).
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), std::mem::size_of_val(&buf[..])) };
        w.write_all(bytes)?;
        buf.clear();
        Ok(())
    };
    for id in 1..=n as Id {
        fbuf.push(temp.lookup(id).map(|t| t.instant).unwrap_or(f64::NAN));
        if fbuf.len() == BLOCK {
            flush_f(&mut w, &mut fbuf)?;
        }
    }
    if !fbuf.is_empty() {
        flush_f(&mut w, &mut fbuf)?;
    }
    // Pass 2: the flag byte column.
    let mut gbuf: Vec<u8> = Vec::with_capacity(BLOCK.min(n));
    for id in 1..=n as Id {
        gbuf.push(temp.lookup(id).map(temp_flag).unwrap_or(0));
        if gbuf.len() == BLOCK {
            w.write_all(&gbuf)?;
            gbuf.clear();
        }
    }
    if !gbuf.is_empty() {
        w.write_all(&gbuf)?;
    }
    w.flush()
}

fn subject_term(s: &oxrdf::NamedOrBlankNode) -> Term {
    match s {
        oxrdf::NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        oxrdf::NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

/// [OPUS-4.8] sq-dvyi: the `format` strings the loaders accept for JSON-LD — the short
/// key the JS surface passes (`"jsonld"`), the hyphenated spelling, and the IANA media
/// type (`application/ld+json`, what an `Accept`-negotiated URL load reports). Kept in
/// one place so every loader arm (`parse_to_triples` / `…_with_base` / `load_reader` /
/// `load_dataset`) recognises the same set. OPT-IN behind the `jsonld` feature; every
/// call site is gated on the same feature, so the helper is only compiled when JSON-LD is.
#[cfg(feature = "jsonld")]
fn is_jsonld_format(format: &str) -> bool {
    matches!(format, "jsonld" | "json-ld" | "application/ld+json")
}

/// [OPUS-4.8] sq-f47w1 (survey §B1) Whether `format` names the RDF/XML serialization: the
/// short key, the hyphenated spelling, and the IANA media type (`application/rdf+xml`, what
/// an `Accept`-negotiated URL load reports). Kept in one place so every RDF/XML loader arm
/// (`parse_to_triples` / `…_with_base`) recognises the same set. OPT-IN behind the `rdfxml`
/// feature; every call site is gated on the same feature, so the helper is only compiled
/// when RDF/XML is. When the feature is OFF these strings fall through to `unknown_format_err`
/// (NOT mis-parsed as Turtle), exactly as a `"jsonld"` string does in a non-`jsonld` build.
#[cfg(feature = "rdfxml")]
fn is_rdfxml_format(format: &str) -> bool {
    matches!(format, "rdfxml" | "rdf-xml" | "application/rdf+xml")
}

/// [OPUS-4.8] (sq-m2pc) Whether `format` names the Turtle serialization. The single
/// authority for the Turtle alias set, kept in lock-step with the CLI's `is_known_format`.
/// `parse_to_triples`/`parse_to_triples_with_base` previously fell back to Turtle for ANY
/// unrecognised string (so a typo'd or unsupported format silently parsed as Turtle and
/// returned `Ok`); they now gate on this set and reject the unknown rest.
fn is_turtle_format(format: &str) -> bool {
    matches!(
        format,
        "turtle" | "ttl" | "text/turtle" | "application/turtle"
    )
}

/// [OPUS-4.8] (sq-01yr) Whether `format` names the TriG serialization. The single authority
/// for the TriG alias set, mirroring [`is_turtle_format`]. `load_dataset_serial` previously
/// fell back to TriG for ANY string that was not N-Quads/JSON-LD (so a typo'd or unsupported
/// dataset format silently parsed as TriG and returned `Ok`); it now gates on this set and
/// rejects the unknown rest via [`unknown_format_err`].
fn is_trig_format(format: &str) -> bool {
    matches!(format, "trig" | "application/trig")
}

/// [OPUS-4.8] (sq-01yr) Whether `format` names a quad-bearing dataset serialization (N-Quads
/// or TriG, plus their aliases) that `load_dataset` routes through the per-graph dataset path
/// to PRESERVE named graphs. Any other format defers to `load_str` (which folds named graphs
/// into the default graph, and rejects genuinely-unknown strings under sq-m2pc). JSON-LD is
/// handled by its own `is_jsonld_format` gate ahead of this check, so it is not listed here.
fn is_nquads_format(format: &str) -> bool {
    matches!(format, "nquads" | "n-quads" | "nq" | "application/n-quads")
}

/// [OPUS-4.8] (sq-01yr) The set of dataset formats `load_dataset` takes through the per-graph
/// path — the union of [`is_nquads_format`] and [`is_trig_format`].
fn is_dataset_format(format: &str) -> bool {
    is_nquads_format(format) || is_trig_format(format)
}

/// [OPUS-4.8] (sq-m2pc) The error returned by `parse_to_triples`/`…_with_base` for a
/// format string they do not recognise — the replacement for the old silent
/// "unknown ⇒ Turtle" catch-all. The `jsonld` entry only appears when that opt-in
/// feature is compiled in (the only build in which a "jsonld"/"json-ld" string parses).
fn unknown_format_err(format: &str) -> String {
    // [OPUS-4.8] sq-f47w1: the `jsonld` / `rdfxml` entries only appear when their opt-in
    // feature is compiled in (the only build in which those strings parse rather than error).
    let known = match (cfg!(feature = "jsonld"), cfg!(feature = "rdfxml")) {
        (true, true) => "turtle | ntriples | nquads | trig | jsonld | rdfxml",
        (true, false) => "turtle | ntriples | nquads | trig | jsonld",
        (false, true) => "turtle | ntriples | nquads | trig | rdfxml",
        (false, false) => "turtle | ntriples | nquads | trig",
    };
    format!("unknown RDF format {format:?} (known: {known})")
}

/// Splits a byte buffer into ~`target` ranges, each ending on a newline so no
/// (single-line) N-Triples statement is split across a boundary.
#[cfg(feature = "parallel")]
fn newline_chunk_bounds(bytes: &[u8], target: usize) -> Vec<(usize, usize)> {
    let mut bounds = Vec::with_capacity(target);
    let chunk = (bytes.len() / target.max(1)).max(1);
    let mut start = 0;
    while start < bytes.len() {
        let mut end = (start + chunk).min(bytes.len());
        if end < bytes.len() {
            match bytes[end..].iter().position(|&b| b == b'\n') {
                Some(p) => end += p + 1,
                None => end = bytes.len(),
            }
        }
        bounds.push((start, end));
        start = end;
    }
    bounds
}

/// Per-ISA software prefetch-for-read hint (x86 `prefetcht0`, aarch64 `prfm pldl1keep`, a no-op
/// elsewhere). Correctness-neutral — a prefetch never faults and never changes architectural
/// state; it only asks the CPU to pull a cache line in early.
#[cfg(feature = "parallel")]
#[inline(always)]
fn prefetch_read<T>(p: *const T) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: _mm_prefetch is defined for any address (the hint is dropped on a bad one).
    unsafe {
        core::arch::x86_64::_mm_prefetch(p as *const i8, core::arch::x86_64::_MM_HINT_T0);
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: prfm is a hint — it cannot fault or write memory/registers.
    unsafe {
        core::arch::asm!("prfm pldl1keep, [{0}]", in(reg) p, options(nostack, preserves_flags));
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let _ = p;
}

/// Compile-time default for the dict-remap software prefetch, chosen per hardware from measured
/// A/B results (the `bench-remap` micro-benchmark, prefetch on vs off; the hint helps or *hurts*
/// depending on the core's hardware prefetcher):
///   x86_64 (Intel/AMD; incl. AWS c7i Sapphire Rapids):   +7.5%  -> ON
///   aarch64 + macOS (Apple M-series):                    +22%   -> ON
///   aarch64 + Linux (AWS Graviton3 / Neoverse-V1, etc.): -10%   -> OFF — the HW prefetcher
///       already saturates the gather, so explicit `prfm` hints only add instruction overhead.
/// Overridable at runtime for re-tuning on new silicon: `SPARQ_PREFETCH=1` forces on,
/// `SPARQ_NO_PREFETCH=1` forces off.
#[cfg(feature = "parallel")]
const PREFETCH_DEFAULT: bool = cfg!(any(
    target_arch = "x86_64",
    all(target_arch = "aarch64", target_os = "macos"),
));

/// Append `triples`, remapped from a partial dict's ids to the merged global ids
/// (`remap[id-1]`; inline-integer ids pass through). The `remap` gather is latency-bound on a
/// large global dictionary (the build-path bottleneck — triple-remap measured at ~3s/50M), so
/// each iteration software-prefetches the gather targets of a triple `DIST` ahead. Hardware-
/// specific (per-ISA prefetch) but correctness-neutral; the prefetch is just a hint.
#[cfg(feature = "parallel")]
fn remap_extend(out: &mut Vec<[Id; 3]>, triples: Vec<[Id; 3]>, remap: &[Id]) {
    const DIST: usize = 32;
    let n = triples.len();
    let base = remap.as_ptr();
    // Per-hardware default (PREFETCH_DEFAULT, measured), with a runtime override. The getenv runs
    // once per merged-dict load — which then processes millions of triples — so it is free.
    let do_prefetch = match (
        std::env::var("SPARQ_PREFETCH").as_deref(),
        std::env::var("SPARQ_NO_PREFETCH").as_deref(),
    ) {
        (Ok("1"), _) => true,
        (_, Ok("1")) => false,
        _ => PREFETCH_DEFAULT,
    };
    let lut = |id: Id| -> Id {
        if id >= dict::INLINE_BASE {
            id
        } else {
            remap[(id - 1) as usize]
        }
    };
    out.reserve(n);
    for i in 0..n {
        if do_prefetch && i + DIST < n {
            for &id in &triples[i + DIST] {
                if id < dict::INLINE_BASE {
                    // SAFETY: id-1 < remap.len() for every dictionary id; prefetch is hint-only.
                    prefetch_read(unsafe { base.add((id - 1) as usize) });
                }
            }
        }
        let [s, p, o] = triples[i];
        out.push([lut(s), lut(p), lut(o)]);
    }
}

/// Borrowing iterator over a graph's default-graph triples as canonical
/// `[subject, predicate, object]` ids — see [`Graph::iter_ids`] /
/// [`Graph::iter_ids_sorted`]. Holds the underlying index scan (a zero-copy
/// borrow on an overlay-free store), so it borrows the graph for its lifetime.
pub struct TripleIdIter<'g> {
    scan: store::Scan<'g>,
    i: usize,
}

impl Iterator for TripleIdIter<'_> {
    type Item = [Id; 3];

    #[inline]
    fn next(&mut self) -> Option<[Id; 3]> {
        let row = self.scan.rows.get(self.i)?;
        self.i += 1;
        Some(self.scan.to_spo(row))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.scan.rows.len() - self.i;
        (n, Some(n))
    }
}

impl ExactSizeIterator for TripleIdIter<'_> {}

/// Isolated micro-benchmark of the latency-bound `remap_extend` gather (the build-path
/// bottleneck the per-ISA prefetch targets). Builds `n` synthetic triples whose ids scatter
/// randomly across a `dict_size`-entry remap table (so the gather misses cache like a real large
/// global dictionary), then times `remap_extend` over `iters` runs and returns the best (ms).
/// Honours `SPARQ_NO_PREFETCH=1`. Used to measure the prefetch's effect per hardware in isolation,
/// undiluted by parsing. Not part of the query/build path.
#[cfg(feature = "parallel")]
pub fn bench_remap(n: usize, dict_size: usize, iters: usize) -> f64 {
    // Cheap deterministic LCG scatter (no rand dep, no Date/Random harness restrictions).
    let mut x: u64 = 0x9E3779B97F4A7C15;
    let mut next = || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
    let ds = dict_size.max(1) as u64;
    let mut triples: Vec<[Id; 3]> = Vec::with_capacity(n);
    for _ in 0..n {
        // ids in [1, dict_size], scattered, so the gather misses cache.
        let s = (1 + (next() % ds)) as Id;
        let p = (1 + (next() % ds)) as Id;
        let o = (1 + (next() % ds)) as Id;
        triples.push([s, p, o]);
    }
    // Identity remap of the right size (values irrelevant to the gather's latency).
    let remap: Vec<Id> = (1..=dict_size as Id).collect();
    let mut best = f64::INFINITY;
    for _ in 0..iters.max(1) {
        let mut out: Vec<[Id; 3]> = Vec::new();
        let t = std::time::Instant::now();
        remap_extend(&mut out, triples.clone(), &remap);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        std::hint::black_box(&out);
        if ms < best { best = ms; }
    }
    best
}

/// Parses + interns N-Triples in parallel: each chunk builds a partial dictionary +
/// local-id triples, then the partials are merged into one global dictionary with the
/// local ids remapped. Interning is per-thread (no shared lock); the merge is linear.
#[cfg(feature = "parallel")]
fn parse_ntriples_parallel(bytes: &[u8]) -> Result<(Dict, Vec<[Id; 3]>), String> {
    Ok(merge_partials(parse_block(bytes)?))
}

/// [OPUS-4.8] (sq-25r3) Consolidates a set of per-chunk partial dicts + local-id triples into one
/// global `Dict` + remapped triples — the merge stage shared by the in-memory N-Triples loader
/// ([`parse_ntriples_parallel`]) and the per-graph N-Quads loader ([`Graph::load_nquads_parallel`],
/// which calls this once per graph). Sharded on ≥2 threads (the parallel dict consolidation that
/// breaks the measured serial-`merge_remap` ceiling); serial `merge_remap` on one thread.
#[cfg(feature = "parallel")]
fn merge_partials(partials: ChunkPartials) -> (Dict, Vec<[Id; 3]>) {
    let total: usize = partials.iter().map(|(_, t)| t.len()).sum();
    // [OPUS-4.8] (sq-87bq) RDF 1.2 triple terms are now consolidated by the SHARDED merge
    // too: `ShardedDict::intern_partials` interns them structurally into a dedicated triple
    // shard (a serial second pass keyed on their components' already-routed temp ids — see
    // `intern_triple_terms`), preserving the cross-shard term↔id bijection. This is the SAME
    // machinery the sharded external builder (sq-t3rt, #91) and the pipelined in-RAM loader
    // (`load_ntriples_pipelined`) already use successfully with triple terms; the earlier
    // `has_triple_terms ⇒ serial` guard here (added by sq-hxgb before that path was wired in)
    // is therefore stale and removed, so triple-term bulk loads keep full parse parallelism.
    // One rayon thread: the sharded merge would only add routing/consolidation overhead —
    // keep the proven serial merge (also the byte-reference the differential tests pin).
    if rayon::current_num_threads() <= 1 {
        let cap = partials.iter().map(|(d, _)| d.len()).max().unwrap_or(0);
        let mut global = Dict::with_capacity(cap);
        let mut all = Vec::with_capacity(total);
        for (pd, ptriples) in partials {
            let remap = global.merge_remap(&pd);
            // Dictionary ids are 1-based and below INLINE_BASE; inline-integer ids carry
            // their value and pass through unchanged. Prefetches the gather (remap_extend).
            remap_extend(&mut all, ptriples, &remap);
        }
        return (global, all);
    }
    // ≥2 threads: SHARDED parallel dict consolidation (the measured serial `merge_remap`
    // ceiling — load plateaued at ~1.8× on 4 identical cores — was exactly this stage).
    let mut sd = dict::ShardedDict::new(default_shards());
    let mut all: Vec<[Id; 3]> = Vec::with_capacity(total);
    // [OPUS-4.8] (sq-87bq) `sharded_extend` (well-formed input, incl. RDF 1.2 triple terms)
    // only `Err`s on a MALFORMED triple term — a component id out of the partial's range or
    // referencing an unpopulated slot. `parse_block` produces well-formed partials (children
    // precede their parent triple in arena order), so an error here is an internal invariant
    // breach, not recoverable input. Surface it loudly.
    sharded_extend(&mut sd, &partials, &mut all).expect("sharded merge: well-formed partials must not error");
    finish_sharded(sd, all)
}

/// The shard count for the parallel in-memory/streaming dict consolidation (2 shards per
/// rayon thread for load balance; same policy as the external sharded build).
#[cfg(feature = "parallel")]
fn default_shards() -> usize {
    (rayon::current_num_threads() * 2).clamp(4, 64)
}

#[cfg(feature = "mmap")]
thread_local! {
    /// [OPUS-4.8] sq-vkz7 — per-thread override of the [`build_compressed_perms`] gate, so
    /// tests/benchmarks can request the compressed build WITHOUT mutating the process-global
    /// environment (a `set_var`/`getenv` data race under parallel `cargo test`, exactly the
    /// hazard the dict-spill `from_lookup` note documents). `None` ⇒ fall back to the env var.
    /// Read once on the build's orchestrating thread before any rayon fan-out, so a value set
    /// on the calling thread is observed by the whole build.
    static BUILD_COMPRESSED_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// [OPUS-4.8] sq-vkz7 — TEST/BENCH hook: run `f` with the [`build_compressed_perms`] gate
/// forced to `on`, on THIS thread only, restoring the previous override afterwards. Used by
/// the differential build tests so they never touch the global environment.
#[cfg(feature = "mmap")]
#[doc(hidden)]
pub fn with_build_compressed<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let prev = BUILD_COMPRESSED_OVERRIDE.with(|c| c.replace(Some(on)));
    let r = f();
    BUILD_COMPRESSED_OVERRIDE.with(|c| c.set(prev));
    r
}

/// [OPUS-4.8] sq-vkz7 — gate for the external build to emit BLOCK-COMPRESSED (`SPQCPRM1`)
/// permutation files DIRECTLY from the sort/merge tail, skipping the separate
/// `open` + `decode_all` + re-encode recompress pass. Off by default (RAW stays the build
/// default per `research/compressed-perms-verdict.md`); set
/// `SPARQ_BUILD_COMPRESSED=1|on|true` (or [`with_build_compressed`] in tests) to opt in. The
/// output is byte-identical to running the raw build then `recompress`.
#[cfg(feature = "mmap")]
fn build_compressed_perms() -> bool {
    if let Some(forced) = BUILD_COMPRESSED_OVERRIDE.with(|c| c.get()) {
        return forced;
    }
    matches!(
        std::env::var("SPARQ_BUILD_COMPRESSED").as_deref(),
        Ok("1") | Ok("on") | Ok("true")
    )
}

/// [OPUS-4.8] sq-vkz7 — converts a RAW `[u32;3]` permutation file at `path` to the
/// BLOCK-COMPRESSED `SPQCPRM1` format IN PLACE, by streaming its rows through
/// [`compress::CompressedPermWriter`] into a temp file and renaming it over `path`. One
/// sequential pass over the raw rows (already sorted+deduped on disk) — no `Graph::open`,
/// no `decode_all`. Used only for the SPO perm, which the build had to keep raw while the
/// sibling sorts re-read it. An empty raw file (unbuilt perm) is left untouched.
#[cfg(feature = "mmap")]
fn compress_perm_file_in_place(path: &std::path::Path) -> Result<(), String> {
    let (map, n) = extsort::map_perm(path).map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(()); // empty perm stays raw-empty, exactly like the non-compressed build
    }
    // SAFETY: the perm file is a whole number of [u32;3] rows written by this build.
    let rows: &[[Id; 3]] = unsafe { std::slice::from_raw_parts(map.as_ptr().cast::<[Id; 3]>(), n) };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".cmp");
    let tmp = std::path::PathBuf::from(tmp);
    let mut w = compress::CompressedPermWriter::create(&tmp).map_err(|e| e.to_string())?;
    for &row in rows {
        w.push(row).map_err(|e| e.to_string())?;
    }
    w.finish(&tmp).map_err(|e| e.to_string())?;
    drop(map); // release the raw mapping before replacing the file underneath it
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Interns a parsed block's partial dicts into the sharded dict and appends the block's
/// triples (remapped to TEMPORARY sharded ids, inline ids passing through) to `all` — the
/// parallel-merge step shared by the in-memory N-Triples loaders. The remap gather runs
/// in parallel (indexed `par_extend`, deterministic order).
// [OPUS-4.8] Returns `Err` (propagated, not panicking) when a partial carries a malformed
// triple term — see `ShardedDict::intern_partials`.
#[cfg(feature = "parallel")]
fn sharded_extend(
    sd: &mut dict::ShardedDict,
    partials: &[(Dict, Vec<[Id; 3]>)],
    all: &mut Vec<[Id; 3]>,
) -> Result<(), String> {
    use rayon::prelude::*;
    let remaps = sd.intern_partials(partials)?;
    for (pidx, (_, ptriples)) in partials.iter().enumerate() {
        let rm = &remaps[pidx];
        let map = |id: Id| if id >= dict::INLINE_BASE { id } else { rm[id as usize] };
        all.par_extend(ptriples.par_iter().map(|&[s, p, o]| [map(s), map(p), map(o)]));
    }
    Ok(())
}

/// Consolidates the sharded dict into one `Dict` (parallel arena move + parallel-hash
/// lookup-table rebuild, so the result serves `lookup`/`intern` like a serially-built
/// dict) and remaps `all` from temporary sharded ids to the final dense ids in parallel.
#[cfg(feature = "parallel")]
fn finish_sharded(sd: dict::ShardedDict, mut all: Vec<[Id; 3]>) -> (Dict, Vec<[Id; 3]>) {
    use rayon::prelude::*;
    let (mut dict, base, stride) = sd.into_merged();
    dict.build_table();
    all.par_iter_mut().for_each(|t| {
        for c in t.iter_mut() {
            *c = dict::remap_sharded(*c, &base, stride);
        }
    });
    (dict, all)
}

/// [OPUS-4.8] (T1) Intern an oxttl-parsed subject (`NamedNode`/`BlankNode`) from its BORROWED
/// `&str`, without materializing an owned `oxrdf::Term`. `subject_term` + `dict.intern(&Term)`
/// built a heap `Term` per S — bumping the `NamedNode`/`BlankNode` ref-count (an `Arc`/`Rc` clone
/// in oxrdf's interned-IRI representation) and constructing the `Term` enum — only for `intern`
/// to immediately re-borrow `.as_str()` and dispatch right back to `intern_iri`/`intern_blank`.
/// Here we dispatch on the borrowed component directly: the only allocation left is the one
/// `intern_*` makes on a genuinely new term (copy into `Box<str>`), which is unavoidable.
#[cfg(feature = "parallel")]
#[inline]
fn intern_subject_ref(dict: &mut Dict, s: &oxrdf::NamedOrBlankNode) -> Id {
    match s {
        oxrdf::NamedOrBlankNode::NamedNode(n) => dict.intern_iri(n.as_str()),
        oxrdf::NamedOrBlankNode::BlankNode(b) => dict.intern_blank(b.as_str()),
    }
}

/// [OPUS-4.8] (T1) Intern an oxttl-parsed object `Term` from its BORROWED components — the object
/// slot is the one that can be a literal or (RDF 1.2) a triple term. IRIs/blank nodes/literals
/// dispatch straight to the component interners with `&str` views; a nested triple term recurses
/// (its s/p/o are interned first, then the triple is stored by component ids, matching
/// `Dict::intern(&Term::Triple(_))`). No owned `Term` is built for the common non-triple-term case.
#[cfg(feature = "parallel")]
#[inline]
fn intern_object_ref(dict: &mut Dict, o: &Term) -> Id {
    match o {
        Term::NamedNode(n) => dict.intern_iri(n.as_str()),
        Term::BlankNode(b) => dict.intern_blank(b.as_str()),
        Term::Literal(l) => {
            dict.intern_lit(l.value(), l.datatype().as_str(), crate::dict::lang_with_dir(l).as_deref())
        }
        // RDF 1.2 triple term: rare; fall back to the owned-Term path (handles nesting +
        // content-addressed triple ids identically to the serial parser).
        Term::Triple(_) => dict.intern(o),
    }
}

/// Serial Turtle parse of `bytes` into `dict` — the fallback and the per-chunk worker.
///
/// [OPUS-4.8] (T1) Interns each S/P/O directly from oxttl's already-parsed BORROWED term views
/// (`intern_subject_ref` / `intern_iri` / `intern_object_ref`) instead of first materializing an
/// owned `oxrdf::Term` per component and re-borrowing it in `dict.intern`. oxttl still does all
/// tokenization, grammar and prefixed-name expansion — this only removes the per-triple `Term`
/// heap churn between oxttl's output and the dict.
#[cfg(feature = "parallel")]
fn parse_turtle_chunk(bytes: &[u8], dict: &mut Dict) -> Result<Vec<[Id; 3]>, String> {
    // [OPUS-4.8] (sq-jocpn) When the opt-in `native-ttl` feature is on, parse this chunk with the
    // native byte-level Turtle parser instead of oxttl. It is a byte-identical drop-in (same
    // resolved terms — IRI resolution is delegated to the same `oxiri` automaton oxttl uses — same
    // triple set, same accept/reject), pinned by the `native_ttl_matches_oxttl` differential and
    // the W3C TurtleTests ratchet. Anonymous blank-node LABELS differ (as they already do between
    // two oxttl runs), which the blank-node-isomorphic differential/merge accommodates. No base:
    // the parallel loader only routes here for base-less documents (the with-base entry point is
    // its own serial path below), and each chunk carries its own directive snapshot.
    #[cfg(feature = "native-ttl")]
    {
        ttl::parse(bytes, None, dict)
    }
    // [SONNET-4.6] (sq-wrn61) Pre-size the output triple Vec to a capacity HINT derived
    // from the input size, so the append loop does not repeatedly `grow_amortized`
    // (each growth is a realloc + memcpy of the whole Vec). Profiling the single-thread
    // incumbent path with `perf` (60 MB / 2.56 M-triple corpus, work box, NON-CANONICAL)
    // attributes ~53% to oxttl (tokenizer + prefixed-name expansion), ~30% to the dict
    // intern (`find_iri` hashbrown lookup + key memcmp), and the rest to allocation churn,
    // of which the un-pre-sized triple Vec's realloc/memcpy is a measurable slice. This is
    // a PURE capacity hint: it never bounds the parsed count — the Vec still grows if the
    // estimate is low — so the parse result is byte-for-byte identical.
    #[cfg(not(feature = "native-ttl"))]
    {
        let mut triples = Vec::with_capacity(estimate_turtle_triples(bytes.len()));
        for t in TurtleParser::new().for_slice(bytes) {
            let t = t.map_err(|e| e.to_string())?;
            let s = intern_subject_ref(dict, &t.subject);
            let p = dict.intern_iri(t.predicate.as_str());
            let o = intern_object_ref(dict, &t.object);
            triples.push([s, p, o]);
        }
        Ok(triples)
    }
}

/// [SONNET-4.6] (sq-wrn61) Estimate the triple count of a Turtle document from its byte
/// length, for pre-sizing the output `Vec<[Id; 3]>` (and, at the top of the load, the
/// `Dict`). This is a HINT ONLY — a low estimate just means the Vec grows a little, a high
/// estimate over-reserves harmlessly; it NEVER changes the parsed result.
///
/// The divisor is a deliberately CONSERVATIVE bytes-per-triple: predicate-grouped,
/// prefixed Turtle (the realistic human-authored shape) is compact (~20-25 bytes/triple on
/// the bench corpus), but real-world Turtle with long absolute IRIs is far more verbose. A
/// conservative (large) divisor UNDER-estimates on compact input — still removing most of
/// the early doublings — while never grossly over-allocating on verbose input. The `+ 1`
/// keeps the hint non-zero for a pathologically small chunk.
#[cfg(feature = "parallel")]
#[inline]
fn estimate_turtle_triples(byte_len: usize) -> usize {
    // Conservative: assume ~40 bytes per triple. On the compact bench corpus (~23 B/triple)
    // this reserves ~57% of the final count up front — enough to skip the costly early
    // grow_amortized doublings — without over-reserving on verbose real-world Turtle.
    const AVG_TTL_BYTES_PER_TRIPLE: usize = 40;
    byte_len / AVG_TTL_BYTES_PER_TRIPLE + 1
}

/// Skip whitespace and `#`-comments from `i`, returning the next significant byte offset.
#[cfg(feature = "parallel")]
fn skip_ws_comments(bytes: &[u8], mut i: usize) -> usize {
    let n = bytes.len();
    loop {
        while i < n && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        if i < n && bytes[i] == b'#' {
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
        } else {
            return i;
        }
    }
}

/// Is the token at `k` a SPARQL-style `PREFIX`/`BASE` directive (keyword + whitespace)?
#[cfg(feature = "parallel")]
fn is_sparql_directive_start(bytes: &[u8], k: usize) -> bool {
    let m = |kw: &[u8]| {
        bytes.len() > k + kw.len()
            && bytes[k..k + kw.len()].eq_ignore_ascii_case(kw)
            && matches!(bytes.get(k + kw.len()), Some(b' ' | b'\t' | b'\n' | b'\r'))
    };
    m(b"prefix") || m(b"base")
}

/// [OPUS-4.8] (T3) Delimit a SPARQL-style `PREFIX`/`BASE` directive starting at `start` (which
/// [`is_sparql_directive_start`] has already confirmed). Unlike a statement or an `@`-directive,
/// the SPARQL form has **no `.` terminator** — it ends right after the closing `>` of its single
/// `IRIREF` (the last token of both `BASE <iri>` and `PREFIX pname: <iri>`). Returns the offset
/// just past that `>`, or `None` (→ caller falls back to serial) if the directive is malformed or
/// truncated, in which case the serial oxttl parser produces the rejection.
///
/// Correctness of treating this as a snapshot span: the byte run `[start, end)` is exactly the
/// directive text oxttl would consume, so replaying it verbatim into a later chunk's preamble
/// reproduces the prefix/base binding identically (confirmed: oxttl accepts mixed `@`/SPARQL
/// forms, SPARQL redefinitions, and relative SPARQL `BASE`). A trailing `.` after the `>` (which
/// is INVALID Turtle for the SPARQL form) is deliberately NOT consumed: it remains in the stream
/// as the start of the next unit, where the per-chunk oxttl parse rejects it — preserving the
/// rejection the W3C `turtle-syntax-bad-base-03` case requires.
///
/// The scan skips inter-token whitespace and `#`-comments but otherwise only has to locate the
/// IRIREF: anything between the keyword and the `<` that is NOT whitespace/comment/`PNAME_NS`
/// makes the directive malformed, so we bail to serial rather than guess.
#[cfg(feature = "parallel")]
fn next_sparql_directive_end(bytes: &[u8], start: usize) -> Option<usize> {
    let n = bytes.len();
    // Past the keyword: `PREFIX` is 6 bytes, `BASE` is 4. `is_sparql_directive_start` guarantees
    // one of them matches at `start` followed by whitespace.
    let kw_len = if bytes[start..].len() >= 6 && bytes[start..start + 6].eq_ignore_ascii_case(b"prefix") {
        6
    } else {
        4 // base
    };
    // Skip whitespace/comments up to the IRIREF (for PREFIX the PNAME_NS sits in between; we
    // don't need to validate it — the only `<` before the directive's own IRIREF is that IRIREF,
    // and oxttl validates the PNAME_NS when it re-parses the snapshot).
    let mut i = skip_ws_comments(bytes, start + kw_len);
    // Find the IRIREF's opening `<`, allowing only a PNAME_NS token (PREFIX form) before it. If
    // we hit a statement-relevant byte (`.`, a quote, EOF) before `<`, the directive is malformed.
    while i < n && bytes[i] != b'<' {
        match bytes[i] {
            b'.' | b'"' | b'\'' | b';' | b',' | b'[' | b']' | b'(' | b')' | b'@' => return None,
            b'#' => i = skip_ws_comments(bytes, i),
            _ => i += 1,
        }
    }
    if i >= n {
        return None; // no IRIREF before EOF
    }
    // Consume the IRIREF `<...>`; `>` cannot appear unescaped inside (it is `>`), so the
    // first `>` closes it. A newline inside an IRIREF is also illegal, but oxttl re-validates the
    // snapshot, so we only need a cheap span here.
    i += 1;
    memchr::memchr(b'>', &bytes[i..]).map(|off| i + off + 1)
}

/// Scan from `start` (in the top-level/normal Turtle state) to the next statement-terminating
/// `.` (one followed by whitespace/EOF/comment, i.e. not a decimal point or PN_LOCAL dot),
/// skipping over IRIs `<...>`, string literals (all four quote forms, with `\` escapes), and
/// `#` comments. Returns the offset just past the `.`, or `None` if EOF is reached first
/// (malformed/incomplete → caller falls back to the serial parser).
///
/// Blank-node syntax (`[...]` property lists, `(...)` collections, `_:` labels) needs no
/// special handling here: in VALID Turtle a `.` followed by whitespace/EOF/`#` cannot occur
/// inside them outside of the strings/IRIs/comments already skipped — DECIMAL/DOUBLE require
/// a digit or exponent immediately after the dot, and PN_LOCAL / BLANK_NODE_LABEL dots must
/// be followed by a continuation character — so every offset returned is a true top-level
/// end-of-statement. (On INVALID input a mis-split makes some chunk fail to parse, and
/// [`parse_turtle_chunked`] redoes the document serially.) See [`turtle_chunks`] for why
/// chunk-independent parsing of the blank nodes themselves is sound.
#[cfg(feature = "parallel")]
fn next_terminator(bytes: &[u8], start: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = start;
    while i < n {
        // [OPUS-4.8] (T2) SIMD-skip the uninteresting run to the next byte that can change the
        // scan state. At top level only `. < # " ' \` matter — every other byte is copied
        // through. The body-of-the-whole-document terminator pre-scan walks each byte exactly
        // once, so this inner advance is pure critical-path latency; `memchr` runs it at
        // ~tens-of-GB/s instead of one byte/iteration. The structured handlers below
        // (string / IRI / comment skipping, the `.`-terminator and `\`-escape tests) are
        // UNCHANGED, so behaviour — including the review-1398 PN_LOCAL_ESC `\#` fix — is
        // identical; this only fast-forwards over the runs the old `_ => i += 1` arm crawled.
        //
        // [OPUS-4.8] (sq-98w7z.1 hang fix) The scan-byte set has SIX members but `memchr` tops
        // out at three needles, so this needs two passes. The earlier form ran BOTH `memchr3`
        // calls over the SAME full tail `bytes[i..]` and took `x.min(y)` — which is O(tail) work
        // per state-change even when one class of byte is entirely absent. On a document with no
        // string literals or PN_LOCAL escapes (e.g. an all-IRI N-Triples-shaped body), the
        // quote/backslash `memchr3` found NOTHING and re-walked the whole remaining buffer once
        // per `<`/`.`, making `next_terminator` O(statement · tail) ⇒ the whole parse O(n²) and a
        // 55k-triple load hang for minutes. Fix: find the FIRST `. < #` (the class that ends a
        // scan region), then look for a quote/backslash ONLY in the window BEFORE it. Each byte
        // is now examined by the bounded second pass at most once, restoring O(n).
        let a = memchr::memchr3(b'.', b'<', b'#', &bytes[i..]);
        // Second pass is bounded: if `a` found a byte at offset `x`, a quote/backslash can only
        // matter if it occurs strictly before `x` (otherwise `a`'s byte wins the `min`); if `a`
        // found nothing, the region to the end is the whole tail (unchanged behaviour, but now it
        // is the ONLY full-tail scan and it terminates the statement search via `None`).
        let bound = a.map_or(bytes.len() - i, |x| x);
        let b = memchr::memchr3(b'"', b'\'', b'\\', &bytes[i..i + bound]);
        i += match (a, b) {
            (None, None) => return None, // no interesting byte before EOF → incomplete statement
            (Some(x), None) => x,
            (None, Some(y)) => y,
            (Some(x), Some(y)) => x.min(y),
        };
        match bytes[i] {
            b'#' => {
                match memchr::memchr(b'\n', &bytes[i..]) {
                    Some(off) => i += off + 1,
                    None => i = n,
                }
            }
            b'<' => {
                i += 1;
                // clippy::question_mark (rust-clippy 1.97): an unterminated `<` (no closing
                // `>`) means the input is malformed → bail out with None via `?`. [FABLE-5]
                let off = memchr::memchr(b'>', &bytes[i..])?;
                i += off + 1;
            }
            q @ (b'"' | b'\'') => {
                let triple = i + 2 < n && bytes[i + 1] == q && bytes[i + 2] == q;
                if triple {
                    i += 3;
                    loop {
                        if i >= n {
                            return None;
                        }
                        if bytes[i] == b'\\' {
                            i += 2;
                        } else if bytes[i] == q && i + 2 < n && bytes[i + 1] == q && bytes[i + 2] == q {
                            i += 3;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                } else {
                    i += 1;
                    while i < n && bytes[i] != q {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                    if i >= n {
                        return None;
                    }
                    i += 1;
                }
            }
            b'.' => {
                match bytes.get(i + 1) {
                    None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'#') => return Some(i + 1),
                    _ => i += 1,
                }
            }
            // [OPUS-4.8] PN_LOCAL_ESC (review 1398): a prefixed-name local can contain an
            // escaped reserved char, e.g. `ex:foo\#bar` or `ex:a\.b`. Skip the backslash AND
            // the escaped byte so an escaped `#` is NOT read as a comment start (which would
            // swallow a real `.` terminator and could scan past an interspersed
            // `@base`/`@prefix`, leaving a later chunk to parse under a stale preamble). Bail
            // to serial if the `\` is the last byte (malformed).
            b'\\' => {
                if i + 1 >= n {
                    return None;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// Split Turtle `bytes` into independently-parseable chunks for parallel parsing, or `None` to
/// fall back to serial. Each chunk is `directive-snapshot + a run of statements`, so every
/// prefix/base in scope at that chunk's start is re-declared in the chunk.
///
/// # Directives anywhere (T3 — no longer a serial cliff), BOTH the `@`- and the SPARQL-style form
/// [OPUS-4.8] Directives are tracked as ordered byte-spans as the body is scanned, and each chunk
/// is prefixed with the **verbatim, in-order concatenation of every directive that precedes the
/// chunk's first statement** (joined by `\n`). This replays the document's directive prelude
/// exactly, so it is correct even for redefinitions and relative `@base <rel>` / `BASE <rel>`
/// (which resolve against the running base): oxttl re-processes the snapshot in the same order the
/// serial parse does, so each chunk sees the identical prefix table and base. A leading run of
/// directives (the common serializer shape) is just the degenerate case where the snapshot is the
/// same for every chunk. Previously ANY directive interspersed among the triples — and ANY
/// SPARQL-style `PREFIX`/`BASE` anywhere — dropped the WHOLE file to serial; that 0× cliff is gone
/// for both forms:
/// - Turtle `@prefix`/`@base`: `.`-terminated, delimited by [`next_terminator`].
/// - SPARQL-style `PREFIX`/`BASE`: NO `.` terminator, delimited at its IRIREF by
///   [`next_sparql_directive_end`]. (An invalid trailing `.` is left in the stream so the per-chunk
///   oxttl parse still rejects it — see that fn.)
///
/// Still bails (→ serial): anything the scanner cannot cleanly delimit (a truncated/malformed
/// directive or statement, which the serial oxttl parser then rejects).
///
/// BLANK NODES do not force a serial fallback. (They used to bail this function out — and 42
/// `_:` labels among the 1.5 M statements of the real Wikidata slice silently forfeited the
/// entire ~2.9× chunk-parallel speedup.) Document-scoped blank-node identity survives
/// chunk-independent parsing because the per-chunk partial dicts are merged into the global
/// [`Dict`], which interns blank nodes BY LABEL (`merge_remap` → `intern_blank`):
///
/// - LABELED `_:x`: oxttl preserves the written label verbatim
///   (`BlankNode::new_unchecked(label)`), so occurrences of `_:x` in *different* chunks intern
///   to the SAME global id — exactly the document-scoped unification the serial parser
///   produces, even for labels reused in arbitrarily distant statements. The dict merge IS the
///   shared label-intern map; no per-chunk label namespacing, boundary scanning, or post-pass
///   unification is needed.
/// - ANONYMOUS `[...]` / `(...)`: a nest is confined to one statement, hence to one chunk, so
///   anonymous nodes never need cross-chunk unification — only cross-chunk *distinctness*.
///   oxttl mints them via `BlankNode::default()`, a fresh random 128-bit id (thread-safe), so
///   distinctness holds with the same probabilistic guarantee the serial parser already relies
///   on WITHIN a single document (a fresh random id colliding with another anonymous id, or
///   with a user-written label, is equally improbable either way — parallelism adds no new
///   collision class, only more draws from the same 2^128 space).
///
/// The parallel result therefore equals the serial parse up to anonymous blank-node ids,
/// which differ run-to-run even between two serial parses.
#[cfg(feature = "parallel")]
fn turtle_chunks(bytes: &[u8], target: usize) -> Option<Vec<Vec<u8>>> {
    let n = bytes.len();

    // A parsed statement: its byte span `[start, end)` and how many `@`-directive spans precede
    // it (so its chunk's synthetic preamble = the in-order concatenation of `dirs[..dirs_before]`).
    struct Stmt {
        start: usize,
        end: usize,
        dirs_before: usize,
    }

    // [OPUS-4.8] (T3) Single pass over the top level: classify each unit as a directive (recorded
    // as an ordered byte-span) or a statement (recorded with the directive count in scope).
    // Directives — BOTH the Turtle `@prefix`/`@base` form (`.`-terminated) AND the SPARQL-style
    // `PREFIX`/`BASE` form (no `.`, delimited at its IRIREF by `next_sparql_directive_end`) — and
    // statements may interleave freely. Each chunk's synthetic preamble replays every directive in
    // scope verbatim, so a mid-body redefinition (either form) is correct.
    let mut dirs: Vec<(usize, usize)> = Vec::new();
    let mut stmts: Vec<Stmt> = Vec::new();
    let mut j = skip_ws_comments(bytes, 0);
    while j < n {
        if is_sparql_directive_start(bytes, j) {
            // SPARQL-style `PREFIX`/`BASE`: a directive span with no `.` terminator.
            let end = next_sparql_directive_end(bytes, j)?;
            dirs.push((j, end));
            j = skip_ws_comments(bytes, end);
            continue;
        }
        let is_directive = bytes[j] == b'@';
        let end = next_terminator(bytes, j)?;
        if is_directive {
            dirs.push((j, end));
        } else {
            stmts.push(Stmt { start: j, end, dirs_before: dirs.len() });
        }
        j = skip_ws_comments(bytes, end);
    }
    if stmts.len() < 2 {
        return None;
    }

    // Build each chunk's synthetic preamble lazily: it depends only on `dirs_before`, which is
    // monotonically non-decreasing in document order, so consecutive statements usually share it.
    // The preamble is the verbatim, in-order bytes of `dirs[..dirs_before]`, joined by `\n` so
    // adjacent directives never run together — an exact replay of the document's directive
    // prelude as seen at that point (correct for redefinitions and relative `@base`).
    let snapshot = |dirs_before: usize| -> Vec<u8> {
        let mut pre = Vec::new();
        for &(s, e) in &dirs[..dirs_before] {
            pre.extend_from_slice(&bytes[s..e]);
            pre.push(b'\n');
        }
        pre
    };

    // Partition the statements into ~target contiguous groups. A group must not straddle a
    // change in `dirs_before` (a directive appearing mid-group), or the statements after the
    // directive would parse under a stale snapshot — so split a group at any such change too.
    let per = (stmts.len() / target.max(1)).max(1);
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut idx = 0;
    while idx < stmts.len() {
        let dirs_before = stmts[idx].dirs_before;
        let group_start = stmts[idx].start;
        let mut end_i = (idx + per).min(stmts.len());
        // Shrink the group so every statement in it shares the same directive snapshot.
        // (Range bounds are snapshotted at loop entry, so the in-loop `end_i` write only
        // takes effect via the immediate `break`.)
        for (k, stmt) in stmts.iter().enumerate().take(end_i).skip(idx + 1) {
            if stmt.dirs_before != dirs_before {
                end_i = k;
                break;
            }
        }
        let body_end = stmts[end_i - 1].end;
        let mut chunk = snapshot(dirs_before);
        chunk.extend_from_slice(&bytes[group_start..body_end]);
        chunks.push(chunk);
        idx = end_i;
    }
    Some(chunks)
}

/// Parse Turtle in parallel by statement-boundary chunking (see [`turtle_chunks`]). If the input
/// is not safely splittable, OR any chunk fails to parse (an over-eager split), it falls back to
/// the serial parser — so the result always equals a plain serial Turtle parse, up to anonymous
/// blank-node ids (which differ run-to-run even between two serial parses; labeled blank nodes
/// and all ground terms are byte-identical).
#[cfg(feature = "parallel")]
fn parse_turtle_parallel(bytes: &[u8]) -> Result<(Dict, Vec<[Id; 3]>), String> {
    let threads = rayon::current_num_threads().max(1);
    if threads == 1 {
        // No parallelism available: chunking is pure overhead (measured ~16% at 1T on the
        // wikidata slice — 1.300 s chunked vs 1.12 s serial) — parse directly.
        // [SONNET-4.6] (sq-wrn61) Pre-size the dict table so the intern loop skips the early
        // `reserve_rehash` growths (perf attributed a small but real slice to hashbrown
        // resize on this single-thread path). Distinct terms are a fraction of triples;
        // half the triple estimate is a safe under-reservation (a low guess just rehashes a
        // little, never wrong). Chunked callers merge into a ShardedDict, so this only
        // applies to the serial branch.
        let mut dict = Dict::with_capacity(estimate_turtle_triples(bytes.len()) / 2);
        return parse_turtle_chunk(bytes, &mut dict).map(|t| (dict, t));
    }
    let target = (threads * 4).min(bytes.len() / 8192 + 1).max(1);
    parse_turtle_chunked(bytes, target)
}

/// [`parse_turtle_parallel`] with an explicit chunk-count target — separated so the
/// differential tests can force small documents to fan out.
#[cfg(feature = "parallel")]
fn parse_turtle_chunked(bytes: &[u8], target: usize) -> Result<(Dict, Vec<[Id; 3]>), String> {
    use rayon::prelude::*;
    let serial = || {
        let mut dict = Dict::new();
        parse_turtle_chunk(bytes, &mut dict).map(|t| (dict, t))
    };
    let chunks = match turtle_chunks(bytes, target) {
        Some(c) if c.len() > 1 => c,
        _ => return serial(),
    };
    type Partials = Vec<(Dict, Vec<[Id; 3]>)>;
    let partials: Result<Partials, String> = chunks
        .par_iter()
        .map(|chunk| {
            let mut dict = Dict::new();
            parse_turtle_chunk(chunk, &mut dict).map(|t| (dict, t))
        })
        .collect();
    let partials = match partials {
        Ok(p) => p,
        Err(_) => return serial(), // an over-eager split produced invalid Turtle — redo serially
    };
    // [OPUS-4.8] (sq-eq26, T4) Consolidate the per-chunk partial dicts through the SHARED
    // `merge_partials`: serial `merge_remap` on one thread (proven byte-reference the
    // differential oracles pin), but on ≥2 threads the SHARDED `ShardedDict` merge the
    // N-Triples loaders already use — the parallel dict consolidation that breaks the
    // serial-`merge_remap` ceiling. Turtle's per-chunk parser ([`parse_turtle_chunk`])
    // resets prefix/base/blank-node scope at each chunk boundary and emits FULLY-RESOLVED
    // terms + per-chunk-unique blank-node labels into each partial `Dict` (turtle_chunks
    // only splits where directive snapshots are shared and blank labels can't collide), so
    // a partial here is structurally identical to an N-Triples block's partial: the merge
    // sees only ground terms and labelled blank nodes and unifies them by term equality,
    // exactly as the serial `merge_remap` loop did. RDF-1.2 triple terms are consolidated by
    // the sharded merge too (sq-87bq), so triple-term Turtle stays eligible. The output
    // (term + triple set) is identical to the serial merge — pinned by the
    // `parallel_turtle_*_match_serial` differential oracles below.
    Ok(merge_partials(partials))
}

/// [OPUS-4.8] (sq-ev37) Delimit a TriG graph LABEL token starting at `start` — the `labelOrSubject`
/// that precedes a `{ … }` block (`label { … }` or `GRAPH label { … }`). A label is one of an
/// `IRIREF` (`<…>`), a `BLANK_NODE_LABEL` (`_:name`), or a `PrefixedName` (`pfx:local` / `:local`).
/// Returns the offset just past the token, or `None` (→ caller falls back to serial) when the token
/// is anything we do not delimit cleanly (an anonymous `[]` graph label, a collection, EOF, …).
///
/// The returned span is REPLAYED VERBATIM as the chunk's graph header, so its exact bytes — and in
/// particular a blank-node label `_:g` — are preserved; the per-graph dataset merge then unifies
/// that label across chunks exactly as the serial parse / the N-Quads fast path do (sq-25r3). We
/// only need a span here: oxttl re-validates the token when it re-parses the chunk.
#[cfg(feature = "parallel")]
fn scan_graph_label(bytes: &[u8], start: usize) -> Option<usize> {
    let n = bytes.len();
    if start >= n {
        return None;
    }
    match bytes[start] {
        // IRIREF: `<` … `>` (no unescaped `>` inside; oxttl re-validates the snapshot).
        b'<' => memchr::memchr(b'>', &bytes[start + 1..]).map(|off| start + 1 + off + 1),
        // BLANK_NODE_LABEL `_:name` or a PrefixedName `pfx:local` / `:local`: a run of name
        // characters terminated by whitespace, a comment, or the opening `{`. PN_CHARS / `.` / `:`
        // are all consumed; the `{` (or ws/comment) that follows ends the token. (oxttl validates
        // the precise PN grammar on re-parse — a malformed label makes that chunk fail → serial.)
        b'_' | b':' | b'A'..=b'Z' | b'a'..=b'z' | 0x80..=0xff => {
            let mut i = start;
            while i < n
                && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'#' | b'{' | b'}')
            {
                i += 1;
            }
            // A label must be non-empty; the caller checks that a `{` follows (after ws/comments).
            (i > start).then_some(i)
        }
        _ => None,
    }
}

/// [OPUS-4.8] (sq-ev37) A scanned TriG top-level unit: one statement's byte span `[start, end)`,
/// the graph it belongs to (`None` = default graph; `Some((s, e))` = the verbatim label byte-span),
/// and how many directive spans precede it (its chunk's preamble = `dirs[..dirs_before]`).
#[cfg(feature = "parallel")]
struct TrigStmt {
    start: usize,
    end: usize,
    graph: Option<(usize, usize)>,
    dirs_before: usize,
}

/// [OPUS-4.8] (sq-ev37) Split a TriG dataset into independently-parseable chunks for the
/// chunk-parallel in-memory loader, or `None` to fall back to the serial oxttl path. This is the
/// TriG twin of [`turtle_chunks`]: it reuses the SAME statement-terminator scanning
/// ([`next_terminator`]) and directive-snapshot machinery (Turtle `@prefix`/`@base` and SPARQL
/// `PREFIX`/`BASE` spans, replayed verbatim into each chunk), and adds the TriG-specific structure:
///
/// 1. **Graph routing context per statement.** A top-level unit is a directive, a DEFAULT-graph
///    triple (`subj … .`), or a graph BLOCK — `{ … }` (default graph), `label { … }`, or
///    `GRAPH label { … }`. While inside a block every statement is routed to that block's graph;
///    the closing `}` returns to the top level. Each statement records its graph as the verbatim
///    label byte-span (or `None`).
/// 2. **Block-open/close + headers as units.** The `label {` / `GRAPH label {` header and the
///    closing `}` are consumed as structural tokens (not statements). A chunk that carries a run of
///    statements from graph `G` is re-wrapped as `label { … }` (the verbatim label), so oxttl
///    re-parses it back into `G`. This lets a single large block split across many chunks while
///    each chunk stays a self-contained, re-parseable TriG fragment.
///
/// Then [`parse_trig_chunk`] parses each chunk into per-graph buckets and the loader merges them
/// with the SAME per-graph [`merge_partials`] the N-Quads fast path uses — so blank-node + prefix
/// scope across blocks and chunk boundaries is resolved by the dataset-scoped dict merge (by label,
/// exactly as the serial parse), not by anything fragile in the splitter.
///
/// Correctness is paramount: anything the scanner cannot cleanly delimit (a `triplesOrGraph`
/// subject that is not a simple label, an anonymous-blank-node graph label, a truncated block, a
/// `}`-before-`{` imbalance, …) returns `None` and the loader runs the proven serial path — and
/// even a clean split is only ACCEPTED if every chunk re-parses (the loader redoes serially on any
/// per-chunk parse error), so an over-eager split can never silently change the result.
///
/// Two-level only: TriG graph blocks do not nest, so the scanner needs no stack — just a current
/// graph context (`None` at the top level, `Some(label)` inside one block).
#[cfg(feature = "parallel")]
fn trig_chunks(bytes: &[u8], target: usize) -> Option<Vec<Vec<u8>>> {
    let n = bytes.len();
    let mut dirs: Vec<(usize, usize)> = Vec::new();
    let mut stmts: Vec<TrigStmt> = Vec::new();
    // `None` at depth 0 (top level); `Some(label_span)` inside a block (label_span `None` ⇒ the
    // anonymous `{ … }` default-graph block).
    let mut cur_graph: Option<Option<(usize, usize)>> = None;
    let mut j = skip_ws_comments(bytes, 0);
    while j < n {
        match cur_graph {
            // ── Inside a block: statements terminated by `.`, or the closing `}`. ──────────────
            Some(label) => {
                if bytes[j] == b'}' {
                    cur_graph = None;
                    j = skip_ws_comments(bytes, j + 1);
                    continue;
                }
                let end = next_terminator(bytes, j)?;
                stmts.push(TrigStmt { start: j, end, graph: label, dirs_before: dirs.len() });
                j = skip_ws_comments(bytes, end);
            }
            // ── Top level: directive, graph block, or a default-graph triple. ─────────────────
            None => {
                if is_sparql_directive_start(bytes, j) {
                    let end = next_sparql_directive_end(bytes, j)?;
                    dirs.push((j, end));
                    j = skip_ws_comments(bytes, end);
                    continue;
                }
                match bytes[j] {
                    // `@prefix` / `@base`: a `.`-terminated directive span.
                    b'@' => {
                        let end = next_terminator(bytes, j)?;
                        dirs.push((j, end));
                        j = skip_ws_comments(bytes, end);
                    }
                    // Anonymous default-graph block `{ … }`.
                    b'{' => {
                        cur_graph = Some(None);
                        j = skip_ws_comments(bytes, j + 1);
                    }
                    // A stray `}` at top level is malformed → serial.
                    b'}' => return None,
                    // `GRAPH label { … }` (case-insensitive keyword + ws). A subject that merely
                    // starts with `g`/`G` (e.g. `graph:x` / `<…graph> …`) does NOT match because
                    // `matches_kw_ws` requires whitespace/comment immediately after `graph`.
                    _ if matches_kw_ws(bytes, j, b"graph") => {
                        let after_kw = skip_ws_comments(bytes, j + 5);
                        let label_end = scan_graph_label(bytes, after_kw)?;
                        let at_brace = skip_ws_comments(bytes, label_end);
                        if bytes.get(at_brace) != Some(&b'{') {
                            return None; // `GRAPH` not followed by `label {` — malformed.
                        }
                        cur_graph = Some(Some((after_kw, label_end)));
                        j = skip_ws_comments(bytes, at_brace + 1);
                    }
                    // `triplesOrGraph`: a leading SIMPLE label token. If the next significant byte
                    // is `{`, it is a `label { … }` block; otherwise it is a default-graph triple
                    // whose subject is that token — delimited by `next_terminator` from `j` (the
                    // label scan is only a lookahead; the statement still starts at `j`).
                    b'<' | b'_' | b':' | b'A'..=b'Z' | b'a'..=b'z' | 0x80..=0xff => {
                        if let Some(label_end) = scan_graph_label(bytes, j) {
                            let at = skip_ws_comments(bytes, label_end);
                            if bytes.get(at) == Some(&b'{') {
                                cur_graph = Some(Some((j, label_end)));
                                j = skip_ws_comments(bytes, at + 1);
                                continue;
                            }
                        }
                        let end = next_terminator(bytes, j)?;
                        stmts.push(TrigStmt { start: j, end, graph: None, dirs_before: dirs.len() });
                        j = skip_ws_comments(bytes, end);
                    }
                    // Any other top-level statement start (a `[` blank-property-list subject, a `(`
                    // collection subject, a literal — i.e. NOT a graph label). It is a default-graph
                    // triple; delimit it. (A `[ … ] { … }` graph block is not valid TriG, so a `{`
                    // can never legally follow such a subject — if one did, the chunk would fail to
                    // re-parse and the loader would fall back to serial.)
                    _ => {
                        let end = next_terminator(bytes, j)?;
                        stmts.push(TrigStmt { start: j, end, graph: None, dirs_before: dirs.len() });
                        j = skip_ws_comments(bytes, end);
                    }
                }
            }
        }
    }
    // An unclosed block (EOF while inside one) is malformed → serial.
    if cur_graph.is_some() {
        return None;
    }
    if stmts.len() < 2 {
        return None;
    }

    // Directive snapshot: the verbatim, in-order bytes of `dirs[..dirs_before]`, joined by `\n` —
    // the exact replay [`turtle_chunks`] uses (correct for redefinitions and relative `@base`).
    let snapshot = |dirs_before: usize| -> Vec<u8> {
        let mut pre = Vec::new();
        for &(s, e) in &dirs[..dirs_before] {
            pre.extend_from_slice(&bytes[s..e]);
            pre.push(b'\n');
        }
        pre
    };

    // Partition statements into ~target contiguous groups. A group must share BOTH the directive
    // snapshot AND the graph context (so the chunk's preamble and `label { … }` wrapper are well
    // defined) — split a group at any change of either.
    let per = (stmts.len() / target.max(1)).max(1);
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut idx = 0;
    while idx < stmts.len() {
        let dirs_before = stmts[idx].dirs_before;
        let graph = stmts[idx].graph;
        let group_start = stmts[idx].start;
        let mut end_i = (idx + per).min(stmts.len());
        for (k, stmt) in stmts.iter().enumerate().take(end_i).skip(idx + 1) {
            if stmt.dirs_before != dirs_before || stmt.graph != graph {
                end_i = k;
                break;
            }
        }
        let body_end = stmts[end_i - 1].end;
        let mut chunk = snapshot(dirs_before);
        match graph {
            // Default-graph statements: emit them bare (a valid TriG document body).
            None => chunk.extend_from_slice(&bytes[group_start..body_end]),
            // Block statements: re-wrap in `label { … }` so oxttl re-parses them back into the
            // right graph. The label bytes are replayed VERBATIM (blank-node labels and prefixed
            // names included), so graph identity is preserved across chunks.
            Some((ls, le)) => {
                chunk.extend_from_slice(&bytes[ls..le]);
                chunk.extend_from_slice(b" {\n");
                chunk.extend_from_slice(&bytes[group_start..body_end]);
                chunk.extend_from_slice(b"\n}\n");
            }
        }
        chunks.push(chunk);
        idx = end_i;
    }
    Some(chunks)
}

/// [OPUS-4.8] (sq-ev37) Is the case-insensitive keyword `kw` at `k`, followed by whitespace or a
/// comment (so `GRAPH ` matches but a `graph:` prefixed name or a `<…graph>` IRI does not)?
#[cfg(feature = "parallel")]
fn matches_kw_ws(bytes: &[u8], k: usize, kw: &[u8]) -> bool {
    bytes.len() > k + kw.len()
        && bytes[k..k + kw.len()].eq_ignore_ascii_case(kw)
        && matches!(bytes.get(k + kw.len()), Some(b' ' | b'\t' | b'\n' | b'\r' | b'#'))
}

/// [OPUS-4.8] (sq-ev37) Parse one self-contained TriG chunk into per-graph buckets — the per-chunk
/// worker for [`Graph::load_trig_chunked`], the oxttl analogue of [`nt::parse_quads_chunk`]. Routes
/// each quad's triple to a bucket keyed by its graph name (`None` = default graph), interning S/P/O
/// into that bucket's OWN partial dict (the graph NAME is the routing key, never interned), in
/// first-occurrence order within the chunk — exactly the input shape the per-graph
/// [`merge_partials`] consumes. Errors (an over-eager split that yields invalid TriG) propagate so
/// the loader can redo the document serially.
#[cfg(feature = "parallel")]
#[allow(clippy::type_complexity)]
fn parse_trig_chunk(bytes: &[u8]) -> Result<Vec<(Option<nt::GraphKey>, Dict, Vec<[Id; 3]>)>, String> {
    use oxrdf::GraphName;
    use std::collections::HashMap;
    let mut buckets: Vec<(Option<nt::GraphKey>, Dict, Vec<[Id; 3]>)> = Vec::new();
    let mut index: HashMap<Option<nt::GraphKey>, usize> = HashMap::new();
    for q in TriGParser::new().for_slice(bytes) {
        let q = q.map_err(|e| e.to_string())?;
        let key = match q.graph_name {
            GraphName::DefaultGraph => None,
            GraphName::NamedNode(nn) => Some(nt::GraphKey::Iri(nn.into_string())),
            GraphName::BlankNode(b) => Some(nt::GraphKey::Blank(b.into_string())),
        };
        let slot = *index.entry(key.clone()).or_insert_with(|| {
            buckets.push((key.clone(), Dict::new(), Vec::new()));
            buckets.len() - 1
        });
        let (_, dict, triples) = &mut buckets[slot];
        let s = intern_subject_ref(dict, &q.subject);
        let p = dict.intern_iri(q.predicate.as_str());
        let o = intern_object_ref(dict, &q.object);
        triples.push([s, p, o]);
    }
    Ok(buckets)
}

/// Streams N-Triples from `reader` in newline-aligned ~64 MiB blocks, parsing+interning
/// each block IN PARALLEL (the custom byte parser, per-block partial dicts merged into
/// the running `dict`), and spilling SPO runs — the parallel-parse path for the external
/// (billion-scale, bounded-memory) build.
///
/// DECOMPRESSION is PIPELINED onto its own thread feeding a bounded channel, so it OVERLAPS
/// parsing+spilling instead of running additively. For a `.bz2` ingest — where the (slow,
/// single-stream) decompress dominates wall-time — this hides the parse cost under the
/// decompress, the largest measured ingest win. At most a few 64 MiB blocks are in flight,
/// so memory stays bounded.
#[cfg(all(feature = "mmap", feature = "parallel"))]
fn build_external_ntriples_parallel<R: std::io::Read + Send>(
    reader: R,
    dict: &mut Dict,
    buf: &mut Vec<[Id; 3]>,
    runs: &mut Vec<std::path::PathBuf>,
    tmp: &std::path::Path,
    chunk: usize,
) -> Result<(), String> {
    use std::sync::mpsc::sync_channel;
    const BLOCK: usize = 64 << 20; // 64 MiB
    // [OPUS-4.8] (review 1357) Cache timing-enabled ONCE; the hot per-block merge/remap loop
    // then takes no `Instant::now()` and no atomic update when SPARQ_BUILD_TIMING is unset.
    let timing = build_timing::enabled();
    let (tx, rx) = sync_channel::<Vec<u8>>(3);
    // Parsed partials flow parse-thread -> merge (this thread). A small bound keeps memory
    // bounded (a couple of blocks' partials in flight) while letting the rayon PARSE of the
    // next block overlap the SERIAL dict-merge of the current one. Profiling showed the
    // merge (merge_remap + triple-remap, ~10.5s/50M) dominates the parallel parse (~5.2s),
    // and they previously ran sequentially per block; this 3-stage pipeline hides the parse.
    type Partials = Vec<(Dict, Vec<[Id; 3]>)>;
    let (ptx, prx) = sync_channel::<Partials>(2);

    std::thread::scope(|scope| -> Result<(), String> {
        // Stage 1 — decompress + read on its own thread, emitting newline-aligned blocks.
        let producer = scope.spawn(move || -> Result<(), String> {
            let mut reader = reader;
            let mut readbuf = vec![0u8; BLOCK];
            let mut carry: Vec<u8> = Vec::new();
            loop {
                // Fill the read buffer (a single read may return less than requested).
                let mut filled = 0;
                while filled < BLOCK {
                    let n = reader.read(&mut readbuf[filled..]).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    filled += n;
                }
                if filled == 0 {
                    // EOF: a final line without a trailing newline lives in `carry`.
                    if !carry.is_empty() {
                        let _ = tx.send(std::mem::take(&mut carry));
                    }
                    return Ok(());
                }
                // Emit `carry + readbuf[..filled]` up to the last newline; carry the
                // remainder (a partial line split across the read boundary) to the next.
                let mut block = std::mem::take(&mut carry);
                block.extend_from_slice(&readbuf[..filled]);
                let cut = block.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
                carry = block[cut..].to_vec();
                block.truncate(cut);
                if tx.send(block).is_err() {
                    return Ok(()); // a downstream stage errored and dropped the receiver
                }
            }
        });

        // Stage 2 — parse+intern each block in parallel (per-chunk local dicts, no shared
        // state), forwarding the partials to the merge stage. Concurrent with stage 3.
        let parser = scope.spawn(move || -> Result<(), String> {
            for block in rx {
                if block.is_empty() {
                    continue;
                }
                let partials = parse_block(&block)?;
                if ptx.send(partials).is_err() {
                    return Ok(()); // the merge stage errored and dropped the receiver
                }
            }
            Ok(())
        });

        // Stage 3 (this thread) — SERIAL dict-merge + triple-remap + spill; owns dict/buf/
        // runs. The id-assignment order is identical to the old sequential path (blocks
        // arrive in order; partials are in chunk order), so the output is byte-identical.
        for partials in prx {
            for (pd, ptriples) in partials {
                let t_merge = build_timing::start(timing);
                let remap = dict.merge_remap(&pd);
                build_timing::record(t_merge, &build_timing::MERGE_NS);
                let t_remap = build_timing::start(timing);
                let map = |id: Id| if id >= dict::INLINE_BASE { id } else { remap[(id - 1) as usize] };
                // Prefetch the remap gather DIST triples ahead — the large-global-dict gather
                // is the build-path bottleneck (per-ISA hint, correctness-neutral).
                let base = remap.as_ptr();
                for i in 0..ptriples.len() {
                    if i + 32 < ptriples.len() {
                        for &id in &ptriples[i + 32] {
                            if id < dict::INLINE_BASE {
                                // SAFETY: id-1 < remap.len(); prefetch is hint-only.
                                prefetch_read(unsafe { base.add((id - 1) as usize) });
                            }
                        }
                    }
                    let [s, p, o] = ptriples[i];
                    buf.push([map(s), map(p), map(o)]);
                    if buf.len() >= chunk {
                        extsort::spill_run(buf, runs, tmp).map_err(|e| e.to_string())?;
                    }
                }
                build_timing::record(t_remap, &build_timing::REMAP_NS);
            }
        }
        // Join parse first (it feeds stage 3 — surface a parse error), then the producer.
        parser.join().map_err(|_| "parse thread panicked".to_string())??;
        producer.join().map_err(|_| "decompression thread panicked".to_string())?
    })
}

/// SHARDED variant of the parallel ingest: same decompress→parse stages, but the merge stage
/// interns into a hash-sharded dictionary (`ShardedDict`) so the dominant dict work runs in
/// parallel across shards instead of through one serial `merge_remap`. Triples are spilled
/// with TEMPORARY sharded ids (`shard*STRIDE+local`); `ShardedDict::into_merged` + an
/// order-preserving `remap_perm_file` pass turn them into final dense ids after the sort.
#[cfg(all(feature = "mmap", feature = "parallel"))]
fn build_external_ntriples_sharded<R: std::io::Read + Send>(
    reader: R,
    sharded: &mut dict::ShardedDict,
    buf: &mut Vec<[Id; 3]>,
    runs: &mut Vec<std::path::PathBuf>,
    tmp: &std::path::Path,
    chunk: usize,
) -> Result<(), String> {
    use std::sync::mpsc::sync_channel;
    const BLOCK: usize = 64 << 20;
    // [OPUS-4.8] (review 1357) Cache timing-enabled once; copied into the merge/remap stages.
    let timing = build_timing::enabled();
    let (tx, rx) = sync_channel::<Vec<u8>>(3);
    type Partials = Vec<(Dict, Vec<[Id; 3]>)>;
    let (ptx, prx) = sync_channel::<Partials>(2);

    std::thread::scope(|scope| -> Result<(), String> {
        // Stage 1 — decompress (identical to the non-sharded pipeline).
        let producer = scope.spawn(move || -> Result<(), String> {
            let mut reader = reader;
            let mut readbuf = vec![0u8; BLOCK];
            let mut carry: Vec<u8> = Vec::new();
            loop {
                let mut filled = 0;
                while filled < BLOCK {
                    let n = reader.read(&mut readbuf[filled..]).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    filled += n;
                }
                if filled == 0 {
                    if !carry.is_empty() {
                        let _ = tx.send(std::mem::take(&mut carry));
                    }
                    return Ok(());
                }
                let mut block = std::mem::take(&mut carry);
                block.extend_from_slice(&readbuf[..filled]);
                let cut = block.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
                carry = block[cut..].to_vec();
                block.truncate(cut);
                if tx.send(block).is_err() {
                    return Ok(());
                }
            }
        });
        // Stage 2 — parse (identical).
        // [OPUS-4.8] (sq-t3rt) The sharded consolidation now handles RDF 1.2 triple terms
        // (`ShardedDict::intern_partials` interns them structurally into a dedicated triple
        // shard), so no rejection here — unlike the dict-spill path (sq-jvbr), which still
        // cannot and keeps `reject_triple_terms_in_external_build`.
        let parser = scope.spawn(move || -> Result<(), String> {
            for block in rx {
                if block.is_empty() {
                    continue;
                }
                let partials = parse_block(&block)?;
                if ptx.send(partials).is_err() {
                    return Ok(());
                }
            }
            Ok(())
        });
        // Stage 4 — triple REMAP + spill on its own thread, so it runs CONCURRENTLY with
        // stage 3's interning of the next batch (they were one serial stage — the measured
        // ~200 s/1 B "dict bucket"; now the critical path is max(intern, remap), not the
        // sum). The remap gather itself is rayon-parallel (indexed map into a scratch that
        // is appended at exact `chunk` boundaries, so the run files — and every downstream
        // byte — stay identical to the old serial loop's).
        type Batch = (Vec<(Dict, Vec<[Id; 3]>)>, Vec<Vec<Id>>);
        let (rtx, rrx) = sync_channel::<Batch>(1);
        let remapper = scope.spawn(move || -> Result<(), String> {
            use rayon::prelude::*;
            let mut scratch: Vec<[Id; 3]> = Vec::new();
            for (partials, remaps) in rrx {
                let t_remap = build_timing::start(timing);
                for (pidx, (_, ptriples)) in partials.iter().enumerate() {
                    let rm = &remaps[pidx];
                    let map = |id: Id| if id >= dict::INLINE_BASE { id } else { rm[id as usize] };
                    ptriples.par_iter().map(|&[s, p, o]| [map(s), map(p), map(o)]).collect_into_vec(&mut scratch);
                    let mut rest: &[[Id; 3]] = &scratch;
                    while !rest.is_empty() {
                        let take = (chunk - buf.len()).min(rest.len());
                        buf.extend_from_slice(&rest[..take]);
                        rest = &rest[take..];
                        if buf.len() >= chunk {
                            extsort::spill_run(buf, runs, tmp).map_err(|e| e.to_string())?;
                        }
                    }
                }
                build_timing::record(t_remap, &build_timing::REMAP_NS);
            }
            Ok(())
        });
        // Stage 3 (this thread) — SHARDED merge: route each partial's (non-inline) terms to
        // shards and intern in parallel (component-based, no Term alloc; the routing and the
        // remap-table scatter are parallel too — see `ShardedDict::intern_partials`), then
        // hand the batch to stage 4 for the triple remap. Batches flow IN ORDER, so id
        // assignment is deterministic and identical to the previous serial-stage version.
        let feed = || -> Result<(), String> {
            for partials in prx {
                let t_merge = build_timing::start(timing);
                // [OPUS-4.8] Propagate a malformed-triple-term error instead of panicking.
                let remaps = sharded.intern_partials(&partials)?;
                build_timing::record(t_merge, &build_timing::MERGE_NS);
                if rtx.send((partials, remaps)).is_err() {
                    return Ok(()); // the remap stage errored and dropped the receiver
                }
            }
            Ok(())
        };
        let fed = feed();
        drop(rtx); // close the channel so stage 4 drains and exits
        remapper.join().map_err(|_| "remap thread panicked".to_string())??;
        fed?;
        parser.join().map_err(|_| "parse thread panicked".to_string())??;
        producer.join().map_err(|_| "decompression thread panicked".to_string())?
    })
}

/// Rewrite a permutation file in place, remapping every temporary sharded id to its final
/// dense id (`dict::remap_sharded`). Order-preserving (temp ids sort like final ids), so the
/// already-sorted, deduplicated file stays sorted — no re-sort needed.
#[cfg(all(feature = "mmap", feature = "parallel"))]
fn remap_perm_file(path: &std::path::Path, base: &[u64], stride: u32) -> Result<(), String> {
    let f = std::fs::OpenOptions::new().read(true).write(true).open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len() as usize;
    if len == 0 {
        return Ok(());
    }
    // SAFETY: read-write mapping of a freshly-written perm file of whole [u32;3] rows.
    let mut mmap = unsafe { memmap2::MmapMut::map_mut(&f) }.map_err(|e| e.to_string())?;
    // SAFETY: the mmap base is page-aligned (≥ 4-byte u32 align) and the file is a whole
    // number of [u32;3] rows, so `len/4` u32 cells are in-bounds; the `MmapMut` is
    // exclusively owned here and the rayon writes below are disjoint by index. [OPUS-4.8 sq-8wbn]
    let ids: &mut [u32] = unsafe { std::slice::from_raw_parts_mut(mmap.as_mut_ptr().cast::<u32>(), len / 4) };
    use rayon::prelude::*;
    ids.par_iter_mut().for_each(|id| *id = dict::remap_sharded(*id, base, stride));
    mmap.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// SPILLED-dict variant of the parallel N-Triples ingest: the same decompress→parse
/// stages as the sharded pipeline, but stage 3 routes terms through the BOUNDED spill
/// interner (`dictspill::SpillInterner::intern_batch`) instead of in-RAM shard dicts,
/// and triples are STAGED to disk unsorted (their final ids are unknown until the
/// external dedup completes) rather than spilled as sorted runs. Batches flow IN ORDER,
/// so per-shard first-occurrence order — and the final id assignment — is identical to
/// the sharded in-RAM path.
#[cfg(feature = "dict-spill")]
fn build_external_ntriples_dictspill<R: std::io::Read + Send>(
    reader: R,
    interner: &mut dictspill::SpillInterner,
) -> Result<(), String> {
    use std::sync::mpsc::sync_channel;
    // 16 MiB blocks (vs the sharded path's 64 MiB): the in-flight partial dicts (up to
    // ~3 batches across the two channels) dominate the spill path's RSS FLOOR in the
    // unique-literal regime, and the id assignment is CHUNKING-INVARIANT — per-shard
    // first-occurrence order depends only on the (line, position) order of first
    // occurrences, not on where block/partial boundaries fall — so smaller blocks trade
    // nothing but per-batch overhead. (The differential test builds the reference with
    // the sharded path's 64 MiB blocks, so it verifies this invariance empirically.)
    const BLOCK: usize = 16 << 20;
    // [OPUS-4.8] (review 1357) Cache timing-enabled once for the merge stage below.
    let timing = build_timing::enabled();
    let (tx, rx) = sync_channel::<Vec<u8>>(3);
    type Partials = Vec<(Dict, Vec<[Id; 3]>)>;
    let (ptx, prx) = sync_channel::<Partials>(2);

    std::thread::scope(|scope| -> Result<(), String> {
        // Stage 1 — decompress (identical to the sharded pipeline).
        let producer = scope.spawn(move || -> Result<(), String> {
            let mut reader = reader;
            let mut readbuf = vec![0u8; BLOCK];
            let mut carry: Vec<u8> = Vec::new();
            loop {
                let mut filled = 0;
                while filled < BLOCK {
                    let n = reader.read(&mut readbuf[filled..]).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    filled += n;
                }
                if filled == 0 {
                    if !carry.is_empty() {
                        let _ = tx.send(std::mem::take(&mut carry));
                    }
                    return Ok(());
                }
                let mut block = std::mem::take(&mut carry);
                block.extend_from_slice(&readbuf[..filled]);
                let cut = block.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
                carry = block[cut..].to_vec();
                block.truncate(cut);
                if tx.send(block).is_err() {
                    return Ok(());
                }
            }
        });
        // Stage 2 — parse (identical).
        let parser = scope.spawn(move || -> Result<(), String> {
            for block in rx {
                if block.is_empty() {
                    continue;
                }
                let partials = parse_block(&block)?;
                // [OPUS-4.8] (sq-jvbr) The dict-spill consolidation now interns RDF 1.2 triple
                // terms structurally (`SpillInterner::intern_batch` → `finalize_triple_terms`),
                // so no rejection here — like the sharded path (sq-t3rt).
                if ptx.send(partials).is_err() {
                    return Ok(());
                }
            }
            Ok(())
        });
        // Stage 3 (this thread) — bounded-cache resolve + triple staging.
        for partials in prx {
            let t_merge = build_timing::start(timing);
            interner.intern_batch(&partials)?;
            build_timing::record(t_merge, &build_timing::MERGE_NS);
        }
        parser.join().map_err(|_| "parse thread panicked".to_string())??;
        producer.join().map_err(|_| "decompression thread panicked".to_string())?
    })
}

/// Parses one (complete-line) N-Triples byte block in parallel into per-chunk partial
/// dictionaries + local-id triples (no shared state) — the parallelizable half of ingest.
#[cfg(feature = "parallel")]
fn parse_block(bytes: &[u8]) -> Result<ChunkPartials, String> {
    use rayon::prelude::*;
    let target = (rayon::current_num_threads().max(1) * 4).min(bytes.len() / 4096 + 1);
    let bounds = newline_chunk_bounds(bytes, target);
    // [OPUS-4.8] (review 1357) Only timestamp this phase when timing is enabled — one env read
    // per ~16-64 MiB block (not per triple) and no clock/atomic on the common unset path.
    let t_parse = build_timing::start(build_timing::enabled());
    let partials: Vec<(Dict, Vec<[Id; 3]>)> = bounds
        .par_iter()
        .map(|&(s, e)| {
            // [OPUS-4.8] (sq-7d3dj.2) Pre-size the per-chunk partial dict from the chunk's byte
            // length (`bytes / AVG_NT_LINE_BYTES` ≈ triple/term count) so its term arena + lookup
            // table are reserved up front instead of rehashing/re-growing while we intern — the
            // ~5% `reserve_rehash` cost the ingest profile flagged. Capacity-only: the interned
            // term set, the term↔id bijection, and the id-assignment order are byte-identical to a
            // `Dict::new()` start (pinned by `presizing_is_pure_capacity_hint`), and this partial
            // is consumed then dropped by `merge_partials` — which builds the FINAL dict
            // independently — so the reservation never reaches `dict_bytes_per_term` /
            // `store_bytes_per_triple`. The estimate is self-bounded by the real chunk length, so
            // (unlike HDT's untrusted `num_strings`) no clamp is needed.
            let mut d = Dict::with_capacity((e - s) / nt::AVG_NT_LINE_BYTES);
            let t = nt::parse_chunk(&bytes[s..e], &mut d)?;
            Ok::<_, String>((d, t))
        })
        .collect::<Result<Vec<_>, _>>()?;
    build_timing::record(t_parse, &build_timing::PARSE_NS);
    Ok(partials)
}

/// Env-gated (`SPARQ_BUILD_TIMING`) phase-time accumulators for the parallel ingest path,
/// to attribute wall-time across parallel-parse vs dict-merge vs triple-remap.
#[cfg(feature = "parallel")]
mod build_timing {
    // clippy/dead_code: this build-phase timing helper is consumed only by the
    // `mmap`/`dict-spill` external-build paths; in the default feature set those callers
    // are cfg'd out, leaving some members unused. They are not dead under those features.
    #![allow(dead_code)]
    use std::sync::atomic::AtomicU64;
    pub static PARSE_NS: AtomicU64 = AtomicU64::new(0);
    pub static MERGE_NS: AtomicU64 = AtomicU64::new(0);
    pub static REMAP_NS: AtomicU64 = AtomicU64::new(0);
    pub fn reset() {
        use std::sync::atomic::Ordering::Relaxed;
        PARSE_NS.store(0, Relaxed);
        MERGE_NS.store(0, Relaxed);
        REMAP_NS.store(0, Relaxed);
    }
    pub fn enabled() -> bool {
        std::env::var("SPARQ_BUILD_TIMING").is_ok()
    }
    /// [OPUS-4.8] (review 1357) Take a phase start timestamp only when timing is enabled, so
    /// the hot ingest path performs NO `Instant::now()` clock read when `SPARQ_BUILD_TIMING`
    /// is unset (the common case). Callers cache `enabled()` once per build and pass the flag.
    #[inline]
    pub fn start(timing: bool) -> Option<std::time::Instant> {
        timing.then(std::time::Instant::now)
    }
    /// Accumulate the elapsed nanos for a phase iff its start was taken (timing enabled),
    /// skipping the atomic `fetch_add` entirely otherwise.
    #[inline]
    pub fn record(start: Option<std::time::Instant>, counter: &AtomicU64) {
        if let Some(t) = start {
            counter.fetch_add(t.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        }
    }
    /// Which interning topology produced the `MERGE_NS`/`REMAP_NS` buckets — they are NOT
    /// comparable across paths, so the report MUST label them honestly:
    ///
    /// * [`Serial`](PathKind::Serial) — the non-sharded parallel external build
    ///   (`build_external_ntriples_parallel`): "merge" is the one serial `Dict::merge_remap`
    ///   global-id re-intern and "remap" is a serial loop. These two buckets ADD into the
    ///   serial dict-consolidation wall (the measured ~200 s/1 B at engine `ef86e66`).
    /// * [`Pipelined`](PathKind::Pipelined) — the sharded default
    ///   (`build_external_ntriples_sharded`) and the dict-spill path: the "merge" bucket is
    ///   the PARALLEL `intern_partials`/`intern_batch` occupancy and the "remap" bucket runs
    ///   on its OWN pipeline stage CONCURRENTLY with the next batch's intern. They are
    ///   per-stage thread-occupancy (inflated by contention), NOT additive serial wall — so
    ///   labelling them `(serial)` (as this report did before [OPUS-4.8] sq-3l43) made a
    ///   rung-5 re-measurement of the dict-consolidation bucket UNINTERPRETABLE: a reader
    ///   would compare pipelined occupancy against the old additive-serial ~200 s/1 B and
    ///   draw the wrong conclusion. The interpretable dict-consolidation bucket on this path
    ///   is the `consolidate` term (`into_merged`/spilled, serial) PLUS whatever intern
    ///   occupancy exceeds the overlapped parse — which is what sq-3l43 confirms at 1 B.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    // [OPUS-4.8 sq-3l43] `build_timing` is a PRIVATE module, so these items are only
    // crate-reachable regardless of the `pub` keyword. Declare that reachability accurately
    // with `pub(crate)`: it both reflects the real (crate-internal) surface and keeps the
    // textual public-API diff gate (G2 / scripts/pub_api_diff.py, a `pub\s+(fn|enum|…)`
    // regex that does NOT resolve enclosing-module visibility) from tripping on a `pub`
    // that exports nothing.
    pub(crate) enum PathKind {
        Serial,
        Pipelined,
    }

    /// Format the per-phase build-timing line for `stage`. Pure (no I/O, no clock) so it is
    /// unit-testable; `report` prints what this returns. `consolidate_secs` is the serial
    /// dict-consolidation step (`into_merged` for sharded, `consolidate` for spilled), folded
    /// in so the FULL dict-consolidation bucket is attributed on one line; pass `None` for the
    /// serial path (whose consolidation is the inline `merge_remap` already in "merge").
    pub(crate) fn format_report(
        stage: &str,
        kind: PathKind,
        consolidate_secs: Option<f64>,
        secs: f64,
    ) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        let (p, m, r) = (
            PARSE_NS.load(Relaxed) as f64 / 1e9,
            MERGE_NS.load(Relaxed) as f64 / 1e9,
            REMAP_NS.load(Relaxed) as f64 / 1e9,
        );
        // Honest bucket labels: serial buckets ADD to the consolidation wall; pipelined
        // buckets are overlapped per-stage occupancy (see `PathKind`).
        let (merge_lbl, remap_lbl) = match kind {
            PathKind::Serial => ("merge_remap(serial)", "triple-remap(serial)"),
            PathKind::Pipelined => {
                ("intern(parallel-occupancy)", "triple-remap(pipelined-occupancy)")
            }
        };
        let cons = match consolidate_secs {
            Some(c) => format!(" | dict-consolidate(serial) {c:.2}s"),
            None => String::new(),
        };
        format!(
            "[build-timing] {stage}: parse(parallel) {p:.2}s | {merge_lbl} {m:.2}s | {remap_lbl} {r:.2}s{cons} | {secs:.2}s wall to here"
        )
    }

    pub(crate) fn report(stage: &str, kind: PathKind, consolidate_secs: Option<f64>, secs: f64) {
        eprintln!("{}", format_report(stage, kind, consolidate_secs, secs));
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::Ordering::Relaxed;

        // The phase counters are process-global statics; serialize the few tests that set
        // them so they don't race each other's `format_report` reads.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        // [OPUS-4.8 sq-3l43] The non-sharded parallel build's buckets are the genuine
        // additive-serial `merge_remap` + `triple-remap` (the measured ~200 s/1 B); they
        // MUST keep the `(serial)` label and carry NO separate consolidate term.
        #[test]
        fn serial_path_labels_buckets_serial_and_has_no_consolidate_term() {
            let _g = LOCK.lock().unwrap();
            reset();
            MERGE_NS.store(137_900_000_000, Relaxed); // 137.9 s
            REMAP_NS.store(62_200_000_000, Relaxed); //  62.2 s
            PARSE_NS.store(138_100_000_000, Relaxed); // 138.1 s
            let line = format_report("parse+intern+spill done", PathKind::Serial, None, 221.2);
            assert!(line.contains("merge_remap(serial) 137.90s"), "{line}");
            assert!(line.contains("triple-remap(serial) 62.20s"), "{line}");
            assert!(!line.contains("dict-consolidate"), "serial path folds consolidation into merge_remap: {line}");
            assert!(!line.contains("parallel-occupancy"), "{line}");
        }

        // The sharded DEFAULT (what sq-3l43 re-measures) overlaps a PARALLEL intern with a
        // pipelined remap — those buckets are per-stage occupancy, NOT additive-serial. They
        // MUST NOT be labelled `(serial)` (which would make the bucket re-measurement read as
        // a regression vs the old additive 200 s/1 B), and the serial `into_merged`
        // consolidation MUST appear as its own honestly-labelled term on the same line.
        #[test]
        fn pipelined_path_labels_occupancy_and_surfaces_consolidate_bucket() {
            let _g = LOCK.lock().unwrap();
            reset();
            MERGE_NS.store(40_000_000_000, Relaxed); // 40 s intern occupancy
            REMAP_NS.store(60_000_000_000, Relaxed); // 60 s remap occupancy
            PARSE_NS.store(138_000_000_000, Relaxed);
            let line = format_report("parse+intern+spill done", PathKind::Pipelined, Some(3.5), 150.0);
            // The whole point of the bead: the dict-consolidation bucket is the small serial
            // `into_merged` term, distinct from the (overlapped) intern/remap occupancy.
            assert!(line.contains("dict-consolidate(serial) 3.50s"), "{line}");
            assert!(line.contains("intern(parallel-occupancy) 40.00s"), "{line}");
            assert!(line.contains("triple-remap(pipelined-occupancy) 60.00s"), "{line}");
            // Must NOT mislabel the pipelined buckets as additive-serial.
            assert!(!line.contains("merge_remap(serial)"), "{line}");
            assert!(!line.contains("triple-remap(serial)"), "{line}");
        }

        // Pipelined paths whose consolidation is reported on a SEPARATE later line (dict-spill)
        // pass `None` and so carry no inline consolidate term, but still drop the serial labels.
        #[test]
        fn pipelined_path_without_inline_consolidate_omits_the_term() {
            let _g = LOCK.lock().unwrap();
            reset();
            MERGE_NS.store(10_000_000_000, Relaxed);
            let line = format_report("parse+route+stage done", PathKind::Pipelined, None, 99.0);
            assert!(!line.contains("dict-consolidate"), "{line}");
            assert!(line.contains("intern(parallel-occupancy) 10.00s"), "{line}");
            assert!(!line.contains("(serial)"), "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- sq-1ivw7: predicate-range object term-kind (predicate_has_literal_object) ----
    //
    // Direct unit tests for the two new public fns the engine's predicate-range non-literal
    // inference is built on: `dict::is_literal_id` (pure id classification) and
    // `Graph::predicate_has_literal_object` (live-snapshot object-column scan, overlay-aware).

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }
    fn lit_int(v: &str) -> Term {
        Term::Literal(oxrdf::Literal::new_typed_literal(v, oxrdf::vocab::xsd::INTEGER))
    }
    fn lit_str(v: &str) -> Term {
        Term::Literal(oxrdf::Literal::new_simple_literal(v))
    }
    fn bnode(l: &str) -> Term {
        Term::BlankNode(oxrdf::BlankNode::new(l).unwrap())
    }

    #[test]
    fn is_literal_id_classifies_each_kind() {
        let mut g = Graph::from_parts(Dict::new(), Vec::new());
        g.apply_delta(
            &[
                [iri("http://ex/s"), iri("http://ex/p"), iri("http://ex/o")],
                [iri("http://ex/s"), iri("http://ex/plain"), lit_str("hi")],
                [iri("http://ex/s"), iri("http://ex/n"), lit_int("7")], // NON-inline? 7 inlines.
                [iri("http://ex/s"), iri("http://ex/big"), lit_int("99999999999")], // dict literal
                [iri("http://ex/s"), iri("http://ex/b"), bnode("b0")],
            ],
            &[],
        )
        .unwrap();
        // IRI object -> not a literal.
        let iri_id = g.id_of(&iri("http://ex/o")).unwrap();
        assert!(!dict::is_literal_id(&g.dict, iri_id), "IRI id is not a literal");
        // bnode -> not a literal.
        let bn_id = g.id_of(&bnode("b0")).unwrap();
        assert!(!dict::is_literal_id(&g.dict, bn_id), "bnode id is not a literal");
        // plain string literal -> literal (dictionary record).
        let s_id = g.id_of(&lit_str("hi")).unwrap();
        assert!(dict::is_literal_id(&g.dict, s_id), "string literal id is a literal");
        // inline integer -> literal (tagged-id path).
        let inline_id = g.id_of(&lit_int("7")).unwrap();
        assert!(dict::is_inline(inline_id), "small integer should inline");
        assert!(dict::is_literal_id(&g.dict, inline_id), "inline integer is a literal");
        // large integer -> dictionary literal.
        let big_id = g.id_of(&lit_int("99999999999")).unwrap();
        assert!(!dict::is_inline(big_id), "out-of-range integer stays in dict");
        assert!(dict::is_literal_id(&g.dict, big_id), "dict integer literal is a literal");
        // NO_ID -> not a literal.
        assert!(!dict::is_literal_id(&g.dict, dict::NO_ID), "NO_ID is not a literal");
    }

    #[test]
    fn predicate_has_literal_object_iri_only_vs_literal_vs_bnode() {
        let mut g = Graph::from_parts(Dict::new(), Vec::new());
        g.apply_delta(
            &[
                // creator: IRI-only objects (the SP2Bench dc:creator shape).
                [iri("http://ex/paper1"), iri("http://ex/creator"), iri("http://ex/alice")],
                [iri("http://ex/paper2"), iri("http://ex/creator"), iri("http://ex/bob")],
                // creator also with a bnode object -> still literal-free.
                [iri("http://ex/paper3"), iri("http://ex/creator"), bnode("anon")],
                // title: literal objects.
                [iri("http://ex/paper1"), iri("http://ex/title"), lit_str("A Paper")],
                // year: inline-integer literal object.
                [iri("http://ex/paper1"), iri("http://ex/year"), lit_int("2020")],
            ],
            &[],
        )
        .unwrap();
        let creator = g.id_of(&iri("http://ex/creator")).unwrap();
        let title = g.id_of(&iri("http://ex/title")).unwrap();
        let year = g.id_of(&iri("http://ex/year")).unwrap();
        assert!(
            !g.predicate_has_literal_object(creator),
            "creator has only IRI/bnode objects -> literal-free"
        );
        assert!(g.predicate_has_literal_object(title), "title has a string literal object");
        assert!(g.predicate_has_literal_object(year), "year has an inline-integer literal object");
        // An absent predicate id is vacuously literal-free.
        assert!(!g.predicate_has_literal_object(dict::NO_ID), "NO_ID predicate is literal-free");
        let absent = g.id_of(&iri("http://ex/creator")).unwrap() + 1_000_000;
        assert!(!g.predicate_has_literal_object(absent), "unknown predicate has no objects");
    }

    #[test]
    fn predicate_has_literal_object_reflects_overlay_update() {
        // The SNAPSHOT-lifecycle invariant: a predicate literal-free at load becomes literal-HAVING
        // the instant an UPDATE inserts a literal object for it (through the overlay), so a fresh
        // check against the mutated snapshot must flip. This is why the engine re-checks the LIVE
        // graph every eval rather than memoising across snapshots.
        let mut g = Graph::from_parts(Dict::new(), Vec::new());
        g.apply_delta(
            &[[iri("http://ex/paper1"), iri("http://ex/creator"), iri("http://ex/alice")]],
            &[],
        )
        .unwrap();
        let creator = g.id_of(&iri("http://ex/creator")).unwrap();
        assert!(!g.predicate_has_literal_object(creator), "literal-free before the update");
        // UPDATE: insert a literal object for the same predicate.
        g.apply_delta(
            &[[iri("http://ex/paper2"), iri("http://ex/creator"), lit_str("Anon Author")]],
            &[],
        )
        .unwrap();
        assert!(
            g.predicate_has_literal_object(creator),
            "the overlay-inserted literal object must be seen by the live-snapshot check"
        );
    }

    // ---- sq-dvyi: engine-side JSON-LD ingest (the lean WASM build's `load`) ----
    //
    // The site REPL's upload/URL path loads through `Graph::load_str` / `load_dataset`
    // in the wasm bundle; JSON-LD is the one widely-served RDF syntax those entries did
    // not accept. These cover the triple-folding path, the base-IRI path (relative IRIs
    // in a fetched document), and the named-graph-preserving dataset path (`@graph`).
    // `dump_terms` returns full lexical term forms (`<iri>`, `"lit"`, `"lit"^^<dt>`), so
    // the assertions match against those, not bare strings.

    /// A JSON-LD object document parses through `load_str` to the expected triples:
    /// a plain string literal, a typed (xsd:integer) literal, and an IRI object; and
    /// every `jsonld` alias is accepted.
    #[cfg(feature = "jsonld")]
    #[test]
    fn jsonld_load_str_object() {
        let doc = r#"{
            "@context": { "ex": "http://ex/", "name": "ex:name", "age": "ex:age", "knows": { "@id": "ex:knows", "@type": "@id" } },
            "@id": "ex:alice",
            "name": "Alice",
            "age": { "@value": "30", "@type": "http://www.w3.org/2001/XMLSchema#integer" },
            "knows": "ex:bob"
        }"#;
        for fmt in ["jsonld", "json-ld", "application/ld+json"] {
            let g = Graph::load_str(doc, fmt).unwrap_or_else(|e| panic!("{fmt}: {e}"));
            assert_eq!(g.len(), 3, "{fmt}: subject has three predicates");
            let terms = dump_terms(&g);
            let has = |s: &str, p: &str, o: &str| {
                terms.iter().any(|(ts, tp, to)| ts == s && tp == p && to == o)
            };
            assert!(
                has("<http://ex/alice>", "<http://ex/name>", "\"Alice\""),
                "{fmt}: name triple missing: {terms:?}"
            );
            assert!(
                has(
                    "<http://ex/alice>",
                    "<http://ex/age>",
                    "\"30\"^^<http://www.w3.org/2001/XMLSchema#integer>"
                ),
                "{fmt}: typed age triple missing: {terms:?}"
            );
            assert!(
                has("<http://ex/alice>", "<http://ex/knows>", "<http://ex/bob>"),
                "{fmt}: knows IRI triple missing: {terms:?}"
            );
        }
    }

    /// A JSON-LD array of node objects (the common "list of records" shape) loads every
    /// node — JSON-LD is whole-document, not line-oriented, so this is one parse.
    #[cfg(feature = "jsonld")]
    #[test]
    fn jsonld_load_str_array() {
        let doc = r#"[
            { "@id": "http://ex/a", "http://ex/p": [ { "@id": "http://ex/b" } ] },
            { "@id": "http://ex/b", "http://ex/p": [ { "@id": "http://ex/c" } ] }
        ]"#;
        let g = Graph::load_str(doc, "jsonld").unwrap();
        assert_eq!(g.len(), 2);
    }

    /// Relative IRIs in a JSON-LD document resolve against the supplied base — the URL
    /// load path (a document fetched from `https://host/dir/data.jsonld`) relies on this.
    #[cfg(feature = "jsonld")]
    #[test]
    fn jsonld_load_str_with_base_resolves_relative() {
        let doc = r#"{ "@id": "alice", "http://ex/knows": { "@id": "bob" } }"#;
        let g = Graph::load_str_with_base(doc, "jsonld", "http://base.example/dir/").unwrap();
        let terms = dump_terms(&g);
        assert!(
            terms.iter().any(|(s, p, o)| s == "<http://base.example/dir/alice>"
                && p == "<http://ex/knows>"
                && o == "<http://base.example/dir/bob>"),
            "relative IRIs must resolve against the base: {terms:?}"
        );
    }

    /// `load_dataset` preserves the named graph a JSON-LD `@graph` with an outer `@id`
    /// expresses (so a `GRAPH ?g { … }` query against an uploaded JSON-LD dataset works),
    /// while `load_str` folds it into the default graph.
    #[cfg(feature = "jsonld")]
    #[test]
    fn jsonld_load_dataset_preserves_named_graph() {
        let doc = r#"{
            "@id": "http://ex/g1",
            "@graph": [ { "@id": "http://ex/s", "http://ex/p": { "@id": "http://ex/o" } } ]
        }"#;
        let ds = Graph::load_dataset(doc, "jsonld").unwrap();
        // The triple lives in the named graph, not the default graph.
        assert_eq!(ds.len(), 0, "default graph empty");
        let name = Term::NamedNode(NamedNode::new("http://ex/g1").unwrap());
        let sub = ds
            .named
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, g)| g.len());
        assert_eq!(sub, Some(1), "named graph ex:g1 holds the triple");

        // Folding (load_str) keeps the same triple but in the default graph.
        let folded = Graph::load_str(doc, "jsonld").unwrap();
        assert_eq!(folded.len(), 1);
        assert!(folded.named.is_empty());
    }

    /// Malformed JSON-LD surfaces a parse error (not a silent empty graph), so the REPL
    /// can report the failure inline.
    #[cfg(feature = "jsonld")]
    #[test]
    fn jsonld_malformed_errors() {
        assert!(Graph::load_str("{ not json", "jsonld").is_err());
    }

    // [OPUS-4.8] sq-f47w1 (survey §B1) — RDF/XML ingest. These tests exercise the REAL
    // `parse_to_triples` RDF/XML arm (not a mock), and are gated on the OPT-IN `rdfxml`
    // feature so the lean default build (where `oxrdfxml` is not linked) does not see them.

    /// A small RDF/XML document parses through `load_str` for EVERY accepted alias
    /// (`rdfxml` / `rdf-xml` / `application/rdf+xml`) to the same triple set: an IRI object,
    /// a plain literal, and a typed (`xsd:integer`) literal. Directly line-covers the new
    /// dispatch arm + `is_rdfxml_format` matcher for the per-crate coverage ratchet.
    #[cfg(feature = "rdfxml")]
    #[test]
    fn rdfxml_load_str_object() {
        let doc = r#"<?xml version="1.0"?>
            <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
                     xmlns:ex="http://ex/">
              <rdf:Description rdf:about="http://ex/alice">
                <ex:name>Alice</ex:name>
                <ex:age rdf:datatype="http://www.w3.org/2001/XMLSchema#integer">30</ex:age>
                <ex:knows rdf:resource="http://ex/bob"/>
              </rdf:Description>
            </rdf:RDF>"#;
        for fmt in ["rdfxml", "rdf-xml", "application/rdf+xml"] {
            let g = Graph::load_str(doc, fmt).unwrap_or_else(|e| panic!("{fmt}: {e}"));
            assert_eq!(g.len(), 3, "{fmt}: subject has three predicates");
            let terms = dump_terms(&g);
            let has = |s: &str, p: &str, o: &str| {
                terms.iter().any(|(ts, tp, to)| ts == s && tp == p && to == o)
            };
            assert!(
                has("<http://ex/alice>", "<http://ex/name>", "\"Alice\""),
                "{fmt}: name triple missing: {terms:?}"
            );
            assert!(
                has(
                    "<http://ex/alice>",
                    "<http://ex/age>",
                    "\"30\"^^<http://www.w3.org/2001/XMLSchema#integer>"
                ),
                "{fmt}: typed age triple missing: {terms:?}"
            );
            assert!(
                has("<http://ex/alice>", "<http://ex/knows>", "<http://ex/bob>"),
                "{fmt}: knows IRI triple missing: {terms:?}"
            );
        }
    }

    /// Relative IRIs / `rdf:ID` in an RDF/XML document resolve against the supplied base — the
    /// URL load path (a document fetched from `https://host/dir/data.rdf`) relies on this.
    /// Line-covers the `parse_to_triples_with_base` RDF/XML arm.
    #[cfg(feature = "rdfxml")]
    #[test]
    fn rdfxml_load_str_with_base_resolves_relative() {
        let doc = r#"<?xml version="1.0"?>
            <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
                     xmlns:ex="http://ex/knows#">
              <rdf:Description rdf:about="alice">
                <ex:knows rdf:resource="bob"/>
              </rdf:Description>
            </rdf:RDF>"#;
        let g = Graph::load_str_with_base(doc, "rdfxml", "http://base.example/dir/").unwrap();
        let terms = dump_terms(&g);
        assert!(
            terms.iter().any(|(s, p, o)| s == "<http://base.example/dir/alice>"
                && p == "<http://ex/knows#knows>"
                && o == "<http://base.example/dir/bob>"),
            "relative IRIs must resolve against the base: {terms:?}"
        );
    }

    /// Malformed RDF/XML surfaces a parse error (not a silent empty graph), so a loader can
    /// report the failure inline rather than swallowing it.
    #[cfg(feature = "rdfxml")]
    #[test]
    fn rdfxml_malformed_errors() {
        // Not well-formed XML (unclosed element).
        assert!(Graph::load_str("<rdf:RDF><not closed", "rdfxml").is_err());
    }

    /// ROUND-TRIP: a known graph serialised to RDF/XML (via `oxrdfxml::RdfXmlSerializer`, the
    /// same writer the sparq-server GSP/conformance paths use) and parsed back through
    /// `parse_to_triples` recovers the SAME triple set. This is the load-bearing invariant for
    /// the arm — serialise→parse equivalence — so a future regression in the dispatch wiring or
    /// term interning is caught. Covers an IRI object, a plain literal, a language-tagged
    /// literal, and a typed literal (the term shapes RDF/XML must preserve).
    #[cfg(feature = "rdfxml")]
    #[test]
    fn rdfxml_load_str_round_trip() {
        use oxrdf::{Literal, NamedNode, Triple};
        use oxrdfxml::RdfXmlSerializer;

        // `Triple::new` takes `impl Into<Subject>` / `impl Into<Term>`, so a `NamedNode`
        // converts directly — no need to name the deprecated `oxrdf::Subject` alias.
        let s = NamedNode::new("http://ex/alice").unwrap();
        let triples = vec![
            Triple::new(
                s.clone(),
                NamedNode::new("http://ex/knows").unwrap(),
                NamedNode::new("http://ex/bob").unwrap(),
            ),
            Triple::new(
                s.clone(),
                NamedNode::new("http://ex/name").unwrap(),
                Literal::new_simple_literal("Alice"),
            ),
            Triple::new(
                s.clone(),
                NamedNode::new("http://ex/greeting").unwrap(),
                Literal::new_language_tagged_literal("hi", "en").unwrap(),
            ),
            Triple::new(
                s,
                NamedNode::new("http://ex/age").unwrap(),
                Literal::new_typed_literal(
                    "30",
                    NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
                ),
            ),
        ];

        // Serialise to RDF/XML. An in-memory writer cannot fail, so the `expect`s are inert.
        let mut ser = RdfXmlSerializer::new().for_writer(Vec::new());
        for t in &triples {
            ser.serialize_triple(t.as_ref()).expect("serialise RDF/XML triple");
        }
        let bytes = ser.finish().expect("finish RDF/XML serialisation");
        let xml = String::from_utf8(bytes).expect("RDF/XML serialiser emits UTF-8");

        // Parse it back through the real dispatch and compare the recovered triple SET.
        let g = Graph::load_str(&xml, "rdfxml")
            .unwrap_or_else(|e| panic!("round-trip RDF/XML must re-parse, got {e}\n{xml}"));
        let got: std::collections::BTreeSet<_> = dump_terms(&g).into_iter().collect();

        let want: std::collections::BTreeSet<(String, String, String)> = [
            ("<http://ex/alice>", "<http://ex/knows>", "<http://ex/bob>"),
            ("<http://ex/alice>", "<http://ex/name>", "\"Alice\""),
            ("<http://ex/alice>", "<http://ex/greeting>", "\"hi\"@en"),
            (
                "<http://ex/alice>",
                "<http://ex/age>",
                "\"30\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            ),
        ]
        .into_iter()
        .map(|(s, p, o)| (s.to_string(), p.to_string(), o.to_string()))
        .collect();

        assert_eq!(got, want, "RDF/XML serialise→parse must round-trip the triple set");
    }

    /// [OPUS-4.8] (sq-zz8z, gh-51) The graph-IRI prefix RANGE SCAN returns EXACTLY the named graphs
    /// whose `STR(name)` starts with the prefix, and its lazily-built cache stays coherent across
    /// add/remove (which change `named.len()`, the cache key) on the SAME graph object. Adjacent
    /// IRIs where one is a prefix of another exercise the `partition_point` lower bound + the
    /// `starts_with` upper bound.
    #[test]
    fn graph_prefix_range_scan_and_cache_coherence() {
        let nq = "<http://ex/s0> <http://ex/p> <http://ex/o> <http://ex/a/1> .\n\
                  <http://ex/s1> <http://ex/p> <http://ex/o> <http://ex/a/10> .\n\
                  <http://ex/s2> <http://ex/p> <http://ex/o> <http://ex/a/2> .\n\
                  <http://ex/s3> <http://ex/p> <http://ex/o> <http://ex/b/1> .\n";
        let mut g = Graph::load_dataset(nq, "nquads").unwrap();
        let collect = |g: &Graph, prefix: &str| -> Vec<String> {
            let mut v = Vec::new();
            g.for_named_graphs_with_prefix(prefix, |name, _| {
                if let Term::NamedNode(n) = name {
                    v.push(n.as_str().to_string());
                }
            });
            v.sort();
            v
        };
        // a/* = a/1, a/10, a/2 (note a/1 is a prefix of a/10 — both kept).
        assert_eq!(collect(&g, "http://ex/a/"), ["http://ex/a/1", "http://ex/a/10", "http://ex/a/2"]);
        // exact-prefix that is a substring of another IRI: "a/1" matches a/1 AND a/10.
        assert_eq!(collect(&g, "http://ex/a/1"), ["http://ex/a/1", "http://ex/a/10"]);
        // empty prefix matches every graph.
        assert_eq!(collect(&g, "").len(), 4);
        // no match.
        assert!(collect(&g, "http://ex/zzz").is_empty());
        // single match.
        assert_eq!(collect(&g, "http://ex/b/"), ["http://ex/b/1"]);

        // The cache is now populated (built during the queries above). MUTATE the graph set on the
        // SAME object: add a/3 (len changes -> cache must rebuild) and the new graph must appear.
        g.ensure_named(&Term::NamedNode(NamedNode::new("http://ex/a/3").unwrap())).unwrap();
        assert_eq!(
            collect(&g, "http://ex/a/"),
            ["http://ex/a/1", "http://ex/a/10", "http://ex/a/2", "http://ex/a/3"]
        );
    }

    /// [OPUS-4.8] (sq-quuu) `named_graph(name)` returns the per-name named-graph `&Graph` so the
    /// read-only GenAI crates can be scoped to one graph of a quad dataset. The returned graph
    /// holds EXACTLY that graph's triples (not the default graph, not a mixture across graphs),
    /// and an unknown name yields `None`.
    #[test]
    fn named_graph_by_name_scopes_to_one_graph() {
        let nq = "<http://ex/a> <http://ex/p> <http://ex/x> <http://ex/g1> .\n\
                  <http://ex/b> <http://ex/p> <http://ex/y> <http://ex/g1> .\n\
                  <http://ex/c> <http://ex/p> <http://ex/z> <http://ex/g2> .\n\
                  <http://ex/d> <http://ex/p> <http://ex/w> .\n"; // default graph
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        // The default graph (`g` itself) holds only the one default-graph triple.
        assert_eq!(g.len(), 1);

        let g1 = Term::NamedNode(NamedNode::new("http://ex/g1").unwrap());
        let g2 = Term::NamedNode(NamedNode::new("http://ex/g2").unwrap());
        let missing = Term::NamedNode(NamedNode::new("http://ex/none").unwrap());

        let sub1 = g.named_graph(&g1).expect("ex:g1 exists");
        assert_eq!(sub1.len(), 2, "ex:g1 holds exactly its two triples");
        let sub2 = g.named_graph(&g2).expect("ex:g2 exists");
        assert_eq!(sub2.len(), 1, "ex:g2 holds exactly its one triple");
        assert!(g.named_graph(&missing).is_none(), "unknown name -> None");

        // The scoped sub-graph really is a usable `&Graph`: its own dictionary resolves the
        // members it contains and NOT a member of a sibling graph.
        let iri = |s: &str| Term::NamedNode(NamedNode::new(s).unwrap());
        assert!(sub1.id_of(&iri("http://ex/a")).is_some()); // ex:a is in ex:g1
        assert!(sub1.id_of(&iri("http://ex/c")).is_none()); // ex:c is NOT in ex:g1
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_matches_serial() {
        let decoded = |d: &Dict, t: &[[Id; 3]]| -> Vec<String> {
            let mut v: Vec<String> =
                t.iter().map(|&[s, p, o]| format!("{}|{}|{}", d.term(s), d.term(p), d.term(o))).collect();
            v.sort();
            v
        };
        // Blank-node-FREE doc (exercises the parallel statement-split path), >8 KiB so it fans
        // out, with edge cases that stress the terminator scan: decimals, dots inside strings &
        // IRIs, escaped quotes, multi-line ;/, statements, triple-quoted strings with `.`+
        // newlines, and trailing comments.
        let mut ttl = String::from(
            "@prefix : <http://ex/> .\n@prefix ex: <http://example.org/foo.bar#> .\n# header . comment\n",
        );
        for i in 0..500 {
            ttl.push_str(&format!(
                ":s{i} :dec {i}.5 ; :s \"a.b.c\" , \"x\\\"y.z\" ; :iri ex:rel{i} .\n\
                 :s{i} ex:m \"\"\"l1 . still\nl2.\"\"\" ; :p <http://x.y/a.b.{i}> . # trailing . c\n",
            ));
        }
        assert!(ttl.len() > 8192);
        assert!(turtle_chunks(ttl.as_bytes(), 32).is_some(), "blank-node-free doc should fan out");
        let (pd, pt) = parse_turtle_parallel(ttl.as_bytes()).unwrap();
        let mut sd = Dict::new();
        let st = parse_turtle_chunk(ttl.as_bytes(), &mut sd).unwrap();
        assert_eq!(decoded(&pd, &pt), decoded(&sd, &st), "parallel split must equal serial");
        assert!(pt.len() >= 1500);

        // Blank-node docs fan out too (the dict merge unifies labels by term equality — see
        // turtle_chunks). The differential coverage lives in
        // parallel_turtle_bnodes_match_serial; here just pin that the splitter no longer bails.
        let bn = format!(
            "@prefix : <http://ex/> .\n{}",
            ":a :p [ :q :r ] .\n:x :y ( :i1 :i2 ) .\n_:b :z :w .\n".repeat(300)
        );
        assert!(turtle_chunks(bn.as_bytes(), 32).is_some(), "blank nodes must no longer bail to serial");
    }

    /// [OPUS-4.8] (sq-98w7z.1) Regression: the parallel Turtle terminator scan must be LINEAR in
    /// document size, not quadratic. `next_terminator` used to run a full-tail `memchr3` for
    /// `" ' \` on EVERY state-change byte; on a body with NO string literals or PN_LOCAL escapes
    /// (an all-IRI N-Triples-shaped document — exactly what a wide synthetic graph produces) that
    /// scan found nothing and re-walked the whole remaining buffer once per statement, making the
    /// splitter O(n²): a 55k-triple load hung for ~19 minutes. The fix bounds the second pass to
    /// the window before the next `. < #`.
    ///
    /// NON-VACUOUS: this is a hard wall-clock ceiling, not a ratio. On the pre-fix code a 20k-row
    /// quote-free parse takes ~600 s (measured: 10k rows = 150 s, scaling ~4× per doubling); the
    /// fixed code does it in well under a second. A 20 s ceiling would be blown by >30× on the
    /// broken code yet leaves ~100× slack for the fixed code on the slowest CI runner, so it
    /// cannot flake on a slow-but-linear machine and cannot pass on a quadratic one. Work-box
    /// timings are non-canonical — only the coarse linear/quadratic distinction is asserted.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_terminator_scan_is_linear_not_quadratic() {
        // A quote/backslash-free body forces the bounded second-pass arm (`b == None`) — the exact
        // shape that triggered the quadratic full-tail re-scan. 20k statements ⇒ ~1 MB, comfortably
        // over the fan-out threshold, and large enough that O(n²) is minutes while O(n) is < 1 s.
        let rows = 20_000usize;
        let ttl: String =
            (0..rows).map(|i| format!("<http://ex/s{}> <http://ex/p> <http://ex/o> .\n", i)).collect();
        assert!(turtle_chunks(ttl.as_bytes(), 32).is_some(), "quote-free body should fan out");
        let t = std::time::Instant::now();
        let (_d, triples) = parse_turtle_parallel(ttl.as_bytes()).unwrap();
        let elapsed = t.elapsed();
        assert_eq!(triples.len(), rows, "every quote-free statement must parse");
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "quote-free parse took {:?} — the terminator scan has regressed to quadratic",
            elapsed
        );
    }

    // [SONNET-4.6] (sq-wrn61) The Turtle triple-Vec / Dict pre-sizing (`estimate_turtle_triples`)
    // is a PURE capacity hint. These pin its two load-bearing invariants: (a) it never returns
    // zero (a zero capacity would defeat the hint but not corrupt output) and never SHRINKS with
    // input size — monotonic; (b) a hint that UNDER-estimates the real count does NOT truncate the
    // parse — the Vec still grows and every triple is emitted, byte-for-byte identical to a hint-
    // free oxttl parse.

    #[cfg(feature = "parallel")]
    #[test]
    fn estimate_turtle_triples_never_zero_and_monotonic() {
        // Never zero, even for an empty or 1-byte document (a zero-capacity Vec is legal but the
        // `+ 1` guarantees the hint is always usable / non-degenerate).
        assert!(estimate_turtle_triples(0) >= 1);
        assert!(estimate_turtle_triples(1) >= 1);
        // Monotonic non-decreasing in byte length: a bigger document never hints a smaller Vec.
        let mut prev = 0usize;
        for &n in &[0usize, 1, 39, 40, 41, 4_000, 60_000_000] {
            let e = estimate_turtle_triples(n);
            assert!(e >= prev, "estimate must be monotonic: {} then {}", prev, e);
            prev = e;
        }
        // The divisor is conservative (>= 1 byte/triple), so the hint never EXCEEDS the byte
        // count — it can only ever UNDER-reserve, never grossly over-allocate.
        assert!(estimate_turtle_triples(1_000_000) <= 1_000_001);
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn presized_turtle_parse_is_hint_only_matches_oxttl_direct() {
        // A document whose bytes-per-triple is FAR below the estimate's conservative divisor:
        // very short IRIs / no prefixes means many triples per byte, so `estimate_turtle_triples`
        // UNDER-estimates the true count by a wide margin. If the estimate ever bounded the parse
        // (it must not), triples past the hinted capacity would be dropped.
        let mut ttl = String::from("@prefix : <http://e/> .\n");
        for i in 0..2000 {
            // Three compact triples per subject; predicate-grouped.
            ttl.push_str(&format!(":s{i} :a :b ; :c :d ; :e {i} .\n"));
        }
        let bytes = ttl.as_bytes();
        // Sanity: the hint really does under-estimate here (exercises the grow path).
        assert!(
            estimate_turtle_triples(bytes.len()) < 6000,
            "test must exercise the under-estimate case"
        );

        // Pre-sized incumbent path.
        let mut dict = Dict::new();
        let presized = parse_turtle_chunk(bytes, &mut dict).unwrap();

        // Independent reference: oxttl directly, no capacity hint, interned the same way.
        let mut ref_dict = Dict::new();
        let mut reference: Vec<[Id; 3]> = Vec::new();
        for t in TurtleParser::new().for_slice(bytes) {
            let t = t.unwrap();
            let s = intern_subject_ref(&mut ref_dict, &t.subject);
            let p = ref_dict.intern_iri(t.predicate.as_str());
            let o = intern_object_ref(&mut ref_dict, &t.object);
            reference.push([s, p, o]);
        }

        // Byte-for-byte identical result: same count and same decoded S/P/O for every triple.
        assert_eq!(presized.len(), reference.len(), "pre-sizing must not change the triple count");
        assert_eq!(presized.len(), 6000, "all 3*2000 triples must be emitted");
        let decode = |d: &Dict, t: &[[Id; 3]]| -> Vec<String> {
            let mut v: Vec<String> =
                t.iter().map(|&[s, p, o]| format!("{}|{}|{}", d.term(s), d.term(p), d.term(o))).collect();
            v.sort();
            v
        };
        assert_eq!(
            decode(&dict, &presized),
            decode(&ref_dict, &reference),
            "pre-sized parse must be identical to the hint-free oxttl parse"
        );
    }

    /// [OPUS-4.8] sq-t267: RDF 1.2 triple terms must parse IDENTICALLY through the
    /// chunk-parallel Turtle path and the serial path — including a triple term that lands at a
    /// CHUNK BOUNDARY. The terminator pre-scan ([`next_terminator`]) treats the `<` of `<<( … )>>`
    /// as an IRI start and scans to the first `>`, so it skips the whole triple term as one opaque
    /// span; the load-bearing risk is a DECIMAL or a `.`-bearing IRI *inside* the term being misread
    /// as a top-level statement terminator (which would split a chunk mid-triple-term and corrupt
    /// the parse). This pins chunked == serial for: (a) plain triple-term objects, (b) a decimal
    /// INSIDE the triple term, (c) the `{| … |}` annotation form (asserts base triple + reifies +
    /// annotation), and (d) a single LARGE triple-term statement whose body straddles the split
    /// point — the chunk boundary falls right after its terminator, not inside it.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_quoted_triples_match_serial() {
        let differential = |ttl: &str, target: usize, want_fanout: bool| {
            let chunks = turtle_chunks(ttl.as_bytes(), target);
            if want_fanout {
                let c = chunks.expect("quoted-triple doc must fan out, not bail to serial");
                assert!(c.len() > 1, "doc must split into multiple chunks (boundaries between triple-term statements)");
            }
            let (pd, pt) = parse_turtle_chunked(ttl.as_bytes(), target).unwrap();
            let mut sd = Dict::new();
            let st = parse_turtle_chunk(ttl.as_bytes(), &mut sd).unwrap();
            assert_eq!(
                canon_bnodes(&pd, &pt),
                canon_bnodes(&sd, &st),
                "quoted-triple chunked parse must equal serial"
            );
            (pt.len(), st.len())
        };

        // (a) Plain triple-term objects, one statement each, many statements so the splitter
        //     puts chunk boundaries BETWEEN triple-term statements. The `>` inside `>>` and the
        //     `<` of `<<(` must not desync the terminator scan.
        let mut plain = String::from("@prefix : <http://ex/> .\n");
        for i in 0..500 {
            plain.push_str(&format!(":s{i} :annotates <<( :a{i} :age {i} )>> .\n"));
        }
        assert!(plain.len() > 8192);
        let (p, s) = differential(&plain, 32, true);
        assert_eq!(p, s);
        assert_eq!(p, 500, "every quoted-triple statement must parse");

        // (b) A DECIMAL inside the triple term (`3.5`) — the `.` is inside the `<…>`-skipped span,
        //     so it must NOT be read as a statement terminator. Plus a `.`-bearing IRI inside.
        let mut decimals = String::from("@prefix : <http://ex/> .\n@prefix ex: <http://e.x/foo.bar#> .\n");
        for i in 0..400 {
            decimals.push_str(&format!(
                ":m{i} :stmt <<( ex:r{i} :weight {i}.5 )>> ; :note <<( :a :seeAlso <http://x.y/p.{i}> )>> .\n"
            ));
        }
        assert!(decimals.len() > 8192);
        differential(&decimals, 32, true);

        // (c) The `{| … |}` annotation form: each statement expands to the asserted base triple,
        //     a fresh `rdf:reifies <<( … )>>`, and the annotation triple — all of which must
        //     survive chunking identically (anonymous reifier ids canonicalised by position).
        let mut annot = String::from("@prefix : <http://ex/> .\n");
        for i in 0..400 {
            annot.push_str(&format!(":a{i} :age {i} {{| :certainty {i}.5 ; :by :src{i} |}} .\n"));
        }
        assert!(annot.len() > 8192);
        differential(&annot, 32, true);

        // (d) A SINGLE large triple-term statement (long IRIs, internal newlines/decimals)
        //     padded with ground statements either side, so a chunk boundary falls right AFTER
        //     the big statement's terminator — exercising the boundary-adjacent case without
        //     splitting the term itself. chunked == serial is the witness it stayed intact.
        let mut boundary = String::from("@prefix : <http://ex/> .\n");
        for i in 0..400 {
            boundary.push_str(&format!(":prefix{i} :predicate :object{i} .\n"));
        }
        boundary.push_str(
            ":big :annotates\n  <<( <http://very.long/iri.with.dots/subject>\n      :measuredAt\n      3.14159 )>> .\n",
        );
        for i in 0..400 {
            boundary.push_str(&format!(":postfix{i} :predicate :object{i} .\n"));
        }
        assert!(boundary.len() > 8192, "len={}", boundary.len());
        let (p, s) = differential(&boundary, 32, true);
        assert_eq!(p, s);
        assert_eq!(p, 801, "400 pre + 1 quoted + 400 post");
    }

    /// [OPUS-4.8] (sq-87bq) END-TO-END semantics of the RDF 1.2 Turtle reification surface
    /// (the SPARQL-1.2 annotation sugar) through `Graph::load_str`: the reifying-triple
    /// `<< s p o >>` (subject AND object position) and the annotation block `{| … |}` must
    /// load to the standard desugaring — an `rdf:reifies <<( s p o )>>` reifier statement plus
    /// any asserted base / annotation triples — with the triple TERM `<<( s p o )>>` a
    /// first-class `Term::Triple` object. Pins the supported surface (oxttl-desugared), not
    /// just chunked==serial parse equality.
    #[test]
    fn turtle_rdf12_reification_surface_loads_desugared() {
        const REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
        // The triple TERM both forms desugar their reifier onto.
        let tt = Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::NamedNode::new("http://ex/s").unwrap(),
            oxrdf::NamedNode::new("http://ex/p").unwrap(),
            Term::NamedNode(oxrdf::NamedNode::new("http://ex/o").unwrap()),
        )));
        let dump = |g: &Graph| {
            let scan = g.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (g.dict.term(spo[0]).to_string(), g.dict.term(spo[1]).to_string(), g.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };

        // (1) NAMED reifier `<< s p o ~ ex:r >>`: deterministic reifier id, no anon bnode.
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . << ex:s ex:p ex:o ~ ex:r >> ex:certainty 0.9 .",
            "turtle",
        )
        .unwrap();
        // The reifier `ex:r` carries `rdf:reifies <<( ex:s ex:p ex:o )>>` and the annotation.
        let r = g.dict.lookup(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/r").unwrap()));
        let reifies = g.dict.lookup(&Term::NamedNode(oxrdf::NamedNode::new(REIFIES).unwrap()));
        let tt_id = g.dict.lookup(&tt);
        assert!(tt_id != 0, "the triple term `<<( ex:s ex:p ex:o )>>` must be a first-class dict term");
        let reifies_rows = g.store.scan(&[Some(r), Some(reifies), Some(tt_id)]).rows.len();
        assert_eq!(reifies_rows, 1, "named reifier must carry exactly one `rdf:reifies <<( … )>>`");
        // The annotation triple `ex:r ex:certainty 0.9` is present.
        let cert = g.dict.lookup(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/certainty").unwrap()));
        assert_eq!(g.store.scan(&[Some(r), Some(cert), None]).rows.len(), 1, "annotation triple must load");

        // (2) Annotation block `{| … |}` desugars to: the ASSERTED base triple + a reifier
        //     (anon) `rdf:reifies <<( … )>>` + the annotation triple. The base triple is
        //     asserted (unlike the bare `<< … >>` reifier, which does NOT assert it).
        let g2 =
            Graph::load_str("@prefix ex: <http://ex/> . ex:s ex:p ex:o {| ex:certainty 0.9 |} .", "turtle").unwrap();
        let rows2 = dump(&g2);
        assert!(
            rows2.iter().any(|(s, p, o)| s == "<http://ex/s>" && p == "<http://ex/p>" && o == "<http://ex/o>"),
            "annotation block must ASSERT the base triple, got {rows2:?}"
        );
        assert!(
            rows2.iter().any(|(_, p, _)| p == &format!("<{REIFIES}>")),
            "annotation block must emit an `rdf:reifies` reifier, got {rows2:?}"
        );
        assert!(g2.dict.lookup(&tt) != 0, "the annotated base triple must appear as a triple TERM object");

        // (3) reifiedTriple in SUBJECT position `<< s p o >> p2 o2` (a Turtle construct the
        //     legacy custom NT parser cannot express, here handled by the Turtle loader): the
        //     reifier is the SUBJECT of the annotation, the base triple is NOT asserted.
        let g3 = Graph::load_str("@prefix ex: <http://ex/> . << ex:s ex:p ex:o >> ex:src ex:doc1 .", "turtle").unwrap();
        let rows3 = dump(&g3);
        assert!(
            !rows3.iter().any(|(s, p, o)| s == "<http://ex/s>" && p == "<http://ex/p>" && o == "<http://ex/o>"),
            "a bare `<< … >>` reifier must NOT assert the base triple, got {rows3:?}"
        );
        assert!(
            rows3.iter().any(|(_, p, o)| p == &format!("<{REIFIES}>") && o.starts_with("<<(")),
            "subject-position reifier must carry `rdf:reifies <<( … )>>`, got {rows3:?}"
        );
    }

    /// Decodes a parsed triple sequence to strings with blank nodes renumbered by FIRST
    /// OCCURRENCE in document order. This makes serial-vs-parallel comparison EXACT (plain
    /// `assert_eq!`) rather than requiring graph-isomorphism search: the chunked parse
    /// preserves document order (chunks merge in order), labeled blank nodes keep their
    /// written labels 1:1 in both parses, and anonymous blank nodes are minted at the same
    /// document positions — so the two parses are equivalent under a 1:1 blank-node
    /// relabeling IFF these canonical sequences are equal (the first-occurrence renumbering
    /// is itself the witness mapping).
    #[cfg(feature = "parallel")]
    fn canon_bnodes(d: &Dict, t: &[[Id; 3]]) -> Vec<[String; 3]> {
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut out = Vec::with_capacity(t.len());
        for &[s, p, o] in t {
            let mut conv = |id: Id| match d.term(id) {
                Term::BlankNode(b) => {
                    let next = seen.len();
                    format!("_:c{}", *seen.entry(b.as_str().to_owned()).or_insert(next))
                }
                other => other.to_string(),
            };
            out.push([conv(s), conv(p), conv(o)]);
        }
        out
    }

    /// Differential tests for the parallel Turtle path on BLANK-NODE documents — the inputs
    /// that previously forced a serial fallback. Each case asserts the doc actually splits
    /// into >1 chunk and that the chunked parse equals the serial parse after canonical
    /// blank-node renumbering (see [`canon_bnodes`]).
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_bnodes_match_serial() {
        let differential = |ttl: &str, target: usize| {
            let chunks = turtle_chunks(ttl.as_bytes(), target).expect("doc must fan out");
            assert!(chunks.len() > 1, "doc must split into multiple chunks");
            let (pd, pt) = parse_turtle_chunked(ttl.as_bytes(), target).unwrap();
            let mut sd = Dict::new();
            let st = parse_turtle_chunk(ttl.as_bytes(), &mut sd).unwrap();
            assert_eq!(
                canon_bnodes(&pd, &pt),
                canon_bnodes(&sd, &st),
                "chunked parse must equal serial up to anonymous bnode ids"
            );
        };

        // 1. Same LABEL in distant statements (first and last, crossing every chunk
        //    boundary) plus labels shared between adjacent statements: the merge must
        //    unify each label to ONE node, exactly like the serial document-scoped parse.
        let mut ttl = String::from("@prefix : <http://ex/> .\n_:shared :starts :here .\n");
        for i in 0..400 {
            ttl.push_str(&format!(":s{i} :p :o{i} .\n_:b{} :links _:b{} .\n", i / 3, i / 3 + 1));
        }
        ttl.push_str("_:shared :ends :here .\n");
        differential(&ttl, 16);

        // 2. Anonymous nests + collections: fresh ids must stay distinct across chunks, and
        //    each nest's internal structure must survive chunking intact.
        let mut anon = String::from("@prefix : <http://ex/> .\n");
        for i in 0..300 {
            anon.push_str(&format!(
                ":r{i} :has [ :p{i} [ :q \"v{i}.w\" ] ; :list ( 1 2.5 \"three\" ) ] .\n"
            ));
        }
        differential(&anon, 16);

        // 3. Pathological ALL-bnode doc: every subject/object a labeled bnode, chained
        //    across consecutive statements so every chunk boundary cuts a shared label.
        let mut chain = String::from("@prefix : <http://ex/> .\n");
        for i in 0..500 {
            chain.push_str(&format!("_:n{i} :next _:n{} .\n", i + 1));
        }
        differential(&chain, 16);

        // 4. The Wikidata shape that motivated the fix: a handful of labeled bnodes sprinkled
        //    through a large ground-statement body (formerly: any one of them forfeited the
        //    whole parallel parse).
        let mut sparse = String::from("@prefix : <http://ex/> .\n");
        for i in 0..600 {
            if i % 97 == 0 {
                sparse.push_str(&format!(":s{i} :somevalue _:sv{} .\n", i / 97));
            } else {
                sparse.push_str(&format!(":s{i} :p :o{i} .\n"));
            }
        }
        differential(&sparse, 16);

        // 5. Bnode-free control through the same harness (the pre-existing fan-out case).
        let mut ground = String::from("@prefix : <http://ex/> .\n");
        for i in 0..400 {
            ground.push_str(&format!(":s{i} :p \"lit {i}\" ; :q {i}.5 .\n"));
        }
        differential(&ground, 16);
    }

    /// [OPUS-4.8] (sq-eq26, T4) The chunked Turtle merge now routes through the SHARDED
    /// `ShardedDict` consolidation on ≥2 threads (the same path the N-Triples loaders use),
    /// replacing the serial `merge_remap`-in-a-loop. This pins that the sharded merge yields
    /// the IDENTICAL term + triple set as the single-thread serial merge on a multi-chunk
    /// fixture that combines all the Turtle-specific state the merge must consolidate:
    /// prefixes/base resolution, labelled + anonymous blank nodes shared across chunk
    /// boundaries, AND RDF-1.2 triple terms (the construct sq-87bq enabled for the sharded
    /// path). We run the parse under an EXPLICIT 4-thread rayon pool so the sharded path is
    /// exercised regardless of the host's ambient thread count, and compare against the
    /// serial parser under canonical blank-node renumbering.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_sharded_merge_matches_serial() {
        // A multi-chunk document: prefixes + base, ground statements, labelled blank nodes
        // shared across distant statements, anonymous nests/collections, and RDF-1.2 quoted
        // triples (object position + the `{| … |}` annotation form) interleaved so chunk
        // boundaries fall between every flavour of statement.
        let mut ttl = String::from(
            "@prefix : <http://ex/> .\n@prefix ex: <http://example.org/v#> .\n@base <http://base/> .\n",
        );
        ttl.push_str("_:shared :starts :here .\n");
        for i in 0..500 {
            ttl.push_str(&format!(
                ":s{i} :p :o{i} ; :rel ex:r{i} ; :iri <doc/{i}> .\n"
            ));
            ttl.push_str(&format!("_:b{} :links _:b{} .\n", i / 4, i / 4 + 1));
            ttl.push_str(&format!(
                ":r{i} :has [ :q \"v{i}.w\" ; :list ( 1 2.5 \"three\" ) ] .\n"
            ));
            ttl.push_str(&format!(":m{i} :annotates <<( ex:a{i} :age {i} )>> .\n"));
            ttl.push_str(&format!(
                ":a{i} :age {i} {{| :certainty {i}.5 ; :by :src{i} |}} .\n"
            ));
        }
        ttl.push_str("_:shared :ends :here .\n");
        assert!(ttl.len() > 8192);

        let target = 32;
        let chunks = turtle_chunks(ttl.as_bytes(), target).expect("doc must fan out");
        assert!(
            chunks.len() > 1,
            "doc must split into multiple chunks to exercise the merge"
        );

        // Force the SHARDED path: a 4-thread pool makes `merge_partials` route through the
        // `ShardedDict` consolidation (default_shards() >= 4) inside `parse_turtle_chunked`.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let (pd, pt) = pool.install(|| {
            assert!(
                rayon::current_num_threads() > 1,
                "pool must expose the sharded path"
            );
            parse_turtle_chunked(ttl.as_bytes(), target).unwrap()
        });

        // Serial reference: the single-chunk parser, no sharding.
        let mut sd = Dict::new();
        let st = parse_turtle_chunk(ttl.as_bytes(), &mut sd).unwrap();

        assert_eq!(
            canon_bnodes(&pd, &pt),
            canon_bnodes(&sd, &st),
            "sharded chunked merge must equal serial up to anonymous bnode ids"
        );
        assert!(pt.len() >= 2000, "expected the full triple set, got {}", pt.len());
    }

    /// [OPUS-4.8] Regression for review 1398: a PN_LOCAL_ESC `\#` in a prefixed-name local
    /// must not be read as a comment start by `next_terminator`. Before the fix, the escaped
    /// `#` swallowed the rest of its line (including the real `.` terminator), so the scanner
    /// could run through an interspersed `@base`/`@prefix` undetected; a later chunk would then
    /// parse under the stale preamble and resolve relative IRIs wrongly. The fix makes the
    /// scanner skip the escaped byte so an interspersed `@base` is seen at its true position —
    /// pre-T3 that meant a serial fallback; post-T3 (directive-snapshot per chunk) it means the
    /// `@base` is correctly attributed to the statements that follow it.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_escaped_hash_in_local() {
        // (a) Escaped `#` in a local does not eat the terminator: the splitter still scans the
        //     statements cleanly and the chunked parse equals the serial parse.
        let mut clean = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..400 {
            // `ex:foo\#bar{i}` — a valid PN_LOCAL with an escaped `#`. The `\#` must NOT start
            // a comment; the `.` after it is the real top-level terminator.
            clean.push_str(&format!("ex:s{i} ex:p ex:foo\\#bar{i} .\n"));
        }
        assert!(clean.len() > 8192);
        // The escaped-`#` doc must still split (no spurious comment-eating of terminators)…
        assert!(turtle_chunks(clean.as_bytes(), 32).is_some(), "escaped # must not break the split");
        // …and parse identically to the serial parser.
        let (pd, pt) = parse_turtle_chunked(clean.as_bytes(), 32).unwrap();
        let mut sd = Dict::new();
        let st = parse_turtle_chunk(clean.as_bytes(), &mut sd).unwrap();
        assert_eq!(canon_bnodes(&pd, &pt), canon_bnodes(&sd, &st), "escaped-# chunked parse must equal serial");
        assert_eq!(pt.len(), 400);

        // (b) [OPUS-4.8] (T3) An interspersed `@base` AFTER an escaped `#` (with enough
        //     statements to force multiple chunks) must be attributed to its FOLLOWING
        //     statements, which use a RELATIVE IRI that resolves against the new base. The
        //     escaped `#` must not let the scanner run through the `@base` (review 1398) — if it
        //     did, the relative IRIs would resolve against the stale `@base <http://first/>`.
        //     The check is the load-bearing one: chunked == serial. The base is also redefined a
        //     second time so the snapshot must carry BOTH `@base`s in order.
        let mut interspersed = String::from("@prefix ex: <http://ex/> .\n@base <http://first/> .\n");
        for i in 0..150 {
            interspersed.push_str(&format!("ex:s{i} ex:p <rel{i}> ; ex:e ex:foo\\#bar{i} .\n"));
        }
        interspersed.push_str("@base <http://second/> .\n");
        for i in 150..300 {
            interspersed.push_str(&format!("<sub{i}> ex:p <rel{i}> .\n"));
        }
        interspersed.push_str("@base <http://third/> .\n");
        for i in 300..450 {
            interspersed.push_str(&format!("<sub{i}> ex:p <rel{i}> .\n"));
        }
        let chunks = turtle_chunks(interspersed.as_bytes(), 32)
            .expect("T3: interspersed @base no longer forces serial");
        assert!(chunks.len() > 1, "must fan out across the interspersed @base");
        let (pd, pt) = parse_turtle_chunked(interspersed.as_bytes(), 32).unwrap();
        let mut sd = Dict::new();
        let st = parse_turtle_chunk(interspersed.as_bytes(), &mut sd).unwrap();
        assert_eq!(
            canon_bnodes(&pd, &pt),
            canon_bnodes(&sd, &st),
            "interspersed-@base chunked parse must equal serial (relative IRIs resolve against the right base)"
        );
    }

    /// [OPUS-4.8] (T3) Interspersed `@prefix`/`@base` directives among the body statements must
    /// no longer drop the whole document to serial — instead each chunk carries a verbatim
    /// in-order snapshot of the directives in scope at its start, so the chunked parse equals
    /// serial even for prefix REDEFINITIONS. SPARQL-style `PREFIX`/`BASE` (the no-`.` keyword
    /// form) is handled the same way (delimited at its IRIREF) — see case 4.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_interspersed_directives_match_serial() {
        let differential = |ttl: &str, target: usize, expect_fanout: bool| {
            let chunks = turtle_chunks(ttl.as_bytes(), target);
            if expect_fanout {
                let c = chunks.expect("T3: interspersed @-directives must fan out, not bail");
                assert!(c.len() > 1, "must split into multiple chunks");
            }
            let (pd, pt) = parse_turtle_chunked(ttl.as_bytes(), target).unwrap();
            let mut sd = Dict::new();
            let st = parse_turtle_chunk(ttl.as_bytes(), &mut sd).unwrap();
            assert_eq!(
                canon_bnodes(&pd, &pt),
                canon_bnodes(&sd, &st),
                "interspersed-directive chunked parse must equal serial"
            );
        };

        // 1. Prefix REDEFINITION mid-body: the SAME prefix `p:` is rebound several times, so each
        //    block of statements must see the binding in scope at its position. A stale snapshot
        //    would expand `p:x` to the wrong IRI — caught because chunked must equal serial.
        let mut redef = String::from("@prefix p: <http://v0/> .\n");
        for round in 0..6 {
            redef.push_str(&format!("@prefix p: <http://v{round}/> .\n"));
            for i in 0..80 {
                redef.push_str(&format!("p:s{round}_{i} p:p p:o{i} .\n"));
            }
        }
        assert!(redef.len() > 8192);
        differential(&redef, 32, true);

        // 2. Relative `@base` redefinition mid-body with relative-IRI subjects/objects that
        //    resolve against the running base; new prefixes appear partway through too.
        let mut mixed = String::from("@base <http://b0/> .\n@prefix a: <http://a/> .\n");
        for i in 0..100 {
            mixed.push_str(&format!("<s{i}> a:p <o{i}> .\n"));
        }
        mixed.push_str("@base <http://b1/> .\n@prefix b: <http://bb/> .\n");
        for i in 100..200 {
            mixed.push_str(&format!("<s{i}> a:p b:o{i} .\n"));
        }
        differential(&mixed, 16, true);

        // 3. A directive immediately adjacent to statements with no blank lines, and a directive
        //    as the very last top-level unit before EOF (no following statement) — the trailing
        //    directive simply contributes to no later chunk.
        let mut adjacent = String::from("@prefix x: <http://x/> .\n");
        for i in 0..60 {
            adjacent.push_str(&format!("x:s{i} x:p x:o{i} .\n"));
        }
        adjacent.push_str("@prefix y: <http://y/> .\nx:last y:p x:o .\n@prefix z: <http://z/> .\n");
        differential(&adjacent, 8, false);

        // 4. SPARQL-style `PREFIX`/`BASE` (no-`.` keyword form) interspersed and REDEFINED
        //    mid-body — must now FAN OUT (no longer the serial cliff) and match serial. The
        //    snapshot replays each SPARQL directive verbatim, so the binding in scope at each
        //    chunk's start is the right one. Includes a relative SPARQL `BASE <rel>` redefinition
        //    resolved against the running base, and prefixes introduced partway through.
        let mut sparql = String::from("PREFIX s: <http://s0/>\nBASE <http://base0/>\n");
        for round in 0..8 {
            sparql.push_str(&format!("PREFIX s: <http://s{round}/>\nBASE <http://base{round}/>\n"));
            for i in 0..120 {
                sparql.push_str(&format!("s:longkey{round}_{i} s:longpred <relative-iri-{i}> .\n"));
            }
        }
        assert!(sparql.len() > 8192);
        differential(&sparql, 32, true);

        // 5. MIXED `@`-form and SPARQL-style directives in the SAME document, interleaved with
        //    statements — both forms must be tracked in the same ordered snapshot.
        let mut mixedforms = String::from("@prefix a: <http://a/> .\nPREFIX b: <http://b/>\n");
        for i in 0..80 {
            mixedforms.push_str(&format!("a:s{i} b:p a:o{i} .\n"));
        }
        mixedforms.push_str("@base <http://base/> .\nPREFIX c: <http://c/>\n");
        for i in 80..160 {
            mixedforms.push_str(&format!("<s{i}> b:p c:o{i} .\n"));
        }
        differential(&mixedforms, 16, true);

        // 6. A `#`-comment containing `<` and `.` sitting BETWEEN the SPARQL keyword and its
        //    IRIREF — the directive delimiter must skip the whole comment line (so the in-comment
        //    `<`/`.` is never mistaken for the IRIREF / a terminator) and find the real IRIREF.
        let mut commented = String::from("PREFIX p: # a comment with < and . inside\n  <http://c0/>\n");
        for round in 0..6 {
            commented.push_str(&format!(
                "PREFIX p: #redef <bogus> .\n <http://c{round}/>\nBASE # base <x> .\n <http://b{round}/>\n"
            ));
            for i in 0..50 {
                commented.push_str(&format!("p:s{round}_{i} p:p <rel{i}> .\n"));
            }
        }
        differential(&commented, 16, true);
    }

    /// [OPUS-4.8] (sq-25r3) Canonical, comparable dump of a whole DATASET (default graph +
    /// every named graph) for the chunk-parallel-vs-serial N-Quads differential. Per graph the
    /// triples are decoded to VERBATIM term strings and SORTED. N-Quads has no anonymous-bnode
    /// syntax (`[...]`/`(...)`) — every blank node is a WRITTEN label that both the serial and the
    /// chunked parse preserve byte-for-byte — so a verbatim-label comparison is EXACT (no
    /// first-occurrence renumbering is needed, and using one would be wrong here: it would depend
    /// on the store's id-sorted iteration order, which differs between the two intern orders). The
    /// returned map is `graph-name → sorted triples`, default graph under the empty key, so a
    /// re-ordering of the `named` vec is also caught.
    #[cfg(feature = "parallel")]
    fn canon_dataset(g: &Graph) -> std::collections::BTreeMap<String, Vec<[String; 3]>> {
        fn canon_graph(g: &Graph) -> Vec<[String; 3]> {
            let mut out: Vec<[String; 3]> = g
                .iter_ids()
                .map(|[s, p, o]| [g.dict.term(s).to_string(), g.dict.term(p).to_string(), g.dict.term(o).to_string()])
                .collect();
            out.sort();
            out
        }
        let mut map = std::collections::BTreeMap::new();
        map.insert(String::new(), canon_graph(g));
        for (name, sub) in &g.named {
            map.insert(name.to_string(), canon_graph(sub));
        }
        map
    }

    /// [OPUS-4.8] (sq-25r3) DIFFERENTIAL: the chunk-parallel N-Quads loader
    /// ([`Graph::load_nquads_chunked`]) must produce a dataset IDENTICAL (default graph, the set
    /// AND order of named graphs, and every graph's triples + dict) to the serial reference
    /// ([`Graph::load_dataset_serial`]) — the risk areas being per-graph routing of the 4th field
    /// and blank-node label scoping across chunk boundaries. Each case forces a fan-out across many
    /// ranges (small `target`) so chunk boundaries fall between quads of different graphs and split
    /// runs of a shared bnode label.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_nquads_matches_serial() {
        let differential = |nq: &str, target: usize| {
            // Force >1 newline-aligned range so the per-graph routing is genuinely cross-chunk.
            let bounds = newline_chunk_bounds(nq.as_bytes(), target);
            assert!(bounds.len() > 1, "doc must split into multiple ranges (len {})", nq.len());
            let par = Graph::load_nquads_chunked(nq.as_bytes(), target).unwrap();
            let ser = Graph::load_dataset_serial(nq, "nquads").unwrap();
            // Same default-graph length + same named-graph names IN ORDER (the deterministic
            // first-occurrence order both paths now use).
            assert_eq!(par.len(), ser.len(), "default-graph triple count differs");
            let par_names: Vec<String> = par.named.iter().map(|(n, _)| n.to_string()).collect();
            let ser_names: Vec<String> = ser.named.iter().map(|(n, _)| n.to_string()).collect();
            assert_eq!(par_names, ser_names, "named-graph set/order differs");
            assert_eq!(canon_dataset(&par), canon_dataset(&ser), "chunked dataset must equal serial");
        };

        // 1. Multiple named graphs interleaved with the default graph, ground terms only — the
        //    core per-graph routing across chunk boundaries. The same predicate/object recur in
        //    different graphs (each graph has its OWN dict, exactly as the serial path builds).
        let mut multi = String::new();
        for i in 0..600 {
            let g = i % 4; // 0 -> default, 1..3 -> named graphs g1..g3
            if g == 0 {
                multi.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"));
            } else {
                multi.push_str(&format!(
                    "<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> <http://ex/g{g}> .\n"
                ));
            }
        }
        differential(&multi, 16);

        // 2. Blank nodes SHARED across chunk boundaries within a graph: the same label `_:b{k}`
        //    appears in the default graph at the very start and the very end, and labels are
        //    chained between adjacent quads, so the per-graph dict merge must unify each label to
        //    one node — the cross-chunk bnode-scope risk.
        let mut bn = String::from("_:shared <http://ex/starts> <http://ex/here> .\n");
        for i in 0..500 {
            bn.push_str(&format!(
                "_:n{} <http://ex/next> _:n{} .\n<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> <http://ex/g1> .\n",
                i / 3,
                i / 3 + 1
            ));
        }
        bn.push_str("_:shared <http://ex/ends> <http://ex/here> .\n");
        differential(&bn, 16);

        // 3. A BLANK-NODE-named graph plus bnode subjects/objects routed into it AND into the
        //    default graph: the graph name `_:g` is a routing key (never interned into a graph's
        //    dict), while a SAME-labelled `_:g` used as a subject in the default graph is a normal
        //    bnode there — the two must not be conflated.
        let mut bgraph = String::new();
        for i in 0..400 {
            if i % 2 == 0 {
                bgraph.push_str(&format!("_:x{i} <http://ex/p> \"v{i}\" _:g .\n"));
            } else {
                bgraph.push_str(&format!("<http://ex/s{i}> <http://ex/q> _:x{i} .\n"));
            }
        }
        differential(&bgraph, 16);

        // 4. Literals with datatypes, language tags (incl. `--dir`), escapes, and a `.`-bearing
        //    IRI/literal — the byte parser's literal/IRI grammar must agree with oxttl's per graph.
        //    [OPUS-4.8] (sq-langcase / #1119) The MIXED-CASE tags (`en-US`, `en-GB--ltr`) pin the
        //    casing-normalisation parity: the byte parser must lowercase the tag to the SAME slot
        //    oxttl produces, or this differential check fails on the language column.
        let mut lits = String::new();
        for i in 0..400 {
            let g = if i % 3 == 0 { String::new() } else { format!(" <http://g.x/{}.n>", i % 3) };
            lits.push_str(&format!(
                "<http://ex/s{i}> <http://ex/p> \"val.{i} \\\"q\\\" x\"@en-us{g} .\n\
                 <http://ex/s{i}> <http://ex/m> \"mixed.{i}\"@en-US{g} .\n\
                 <http://ex/s{i}> <http://ex/d> \"dir.{i}\"@en-GB--ltr{g} .\n\
                 <http://ex/s{i}> <http://ex/n> \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer>{g} .\n"
            ));
        }
        differential(&lits, 16);

        // 5. RDF 1.2 triple-term objects in a named graph (forces the serial `merge_remap` branch
        //    of the per-graph merge, since the sharded merge cannot represent triple terms).
        let mut tt = String::new();
        for i in 0..300 {
            tt.push_str(&format!(
                "<http://ex/r{i}> <http://ex/reifies> <<( <http://ex/a{i}> <http://ex/age> \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> <http://ex/meta> .\n\
                 <http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"
            ));
        }
        differential(&tt, 16);

        // 6. Empty + whitespace + comment-only lines interleaved (the parser must skip them
        //    identically to oxttl, and a chunk boundary may land on a blank line).
        let mut sparse = String::new();
        for i in 0..400 {
            sparse.push_str("# a comment . with a dot\n\n");
            let g = if i % 2 == 0 { " <http://ex/g7>" } else { "" };
            sparse.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}>{g} .\n"));
        }
        differential(&sparse, 16);

        // 7. [OPUS-4.8] (sq-25r3) Blank-node labels with INTERIOR dots in EVERY position —
        //    subject, object, AND the graph name — which the byte parser must scan exactly as
        //    oxttl (a `.` is part of the label only when not the final char). The SAME dotted
        //    label `_:n.{k}` is shared across chunk boundaries (chained between adjacent quads
        //    and reused as a graph name), so the per-graph merge must unify the dotted labels too;
        //    a dotted graph name `_:g.{k}` must not be conflated with a same-spelled S/O bnode.
        let mut dotted = String::from("_:sh.ared <http://ex/starts> _:o.0 .\n");
        for i in 0..500 {
            dotted.push_str(&format!(
                "_:n.{} <http://ex/next> _:n.{} _:g.{} .\n\
                 <http://ex/s{i}> <http://ex/p> _:n.{} .\n",
                i / 3,
                i / 3 + 1,
                i % 2,
                i / 3
            ));
        }
        dotted.push_str("_:sh.ared <http://ex/ends> _:o.0 _:g.0 .\n");
        differential(&dotted, 16);
    }

    /// [OPUS-4.8] (sq-ev37) DIFFERENTIAL: the chunk-parallel TriG loader
    /// ([`Graph::load_trig_chunked`]) must produce a dataset IDENTICAL (default graph, the set AND
    /// order of named graphs, and every graph's triples + dict) to the serial oxttl reference
    /// ([`Graph::load_dataset_serial`]) — the TriG twin of [`parallel_nquads_matches_serial`]. Each
    /// case forces a fan-out across many chunks (small `target`) so chunk boundaries fall inside
    /// blocks, between blocks of different graphs, and across runs of a shared blank-node label.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_trig_matches_serial() {
        let differential = |trig: &str, target: usize| {
            let chunks = trig_chunks(trig.as_bytes(), target)
                .unwrap_or_else(|| panic!("doc must be splittable (len {})", trig.len()));
            assert!(chunks.len() > 1, "doc must split into multiple chunks (len {})", trig.len());
            let par = Graph::load_trig_chunked(trig.as_bytes(), target).unwrap();
            let ser = Graph::load_dataset_serial(trig, "trig").unwrap();
            assert_eq!(par.len(), ser.len(), "default-graph triple count differs");
            let par_names: Vec<String> = par.named.iter().map(|(n, _)| n.to_string()).collect();
            let ser_names: Vec<String> = ser.named.iter().map(|(n, _)| n.to_string()).collect();
            assert_eq!(par_names, ser_names, "named-graph set/order differs");
            assert_eq!(canon_dataset(&par), canon_dataset(&ser), "chunked TriG must equal serial");
        };

        // 1. Many `GRAPH g { … }` blocks interleaved with top-level default-graph triples and an
        //    anonymous `{ … }` default block — the core per-graph routing across chunk boundaries.
        //    A leading `@prefix` is in scope for every chunk (the directive-snapshot replay).
        let mut multi = String::from("@prefix : <http://ex/> .\n");
        for i in 0..400 {
            match i % 4 {
                0 => multi.push_str(&format!(":s{i} :p :o{i} .\n")), // default (top level)
                1 => multi.push_str(&format!("GRAPH :g1 {{ :s{i} :p :o{i} . :s{i} :q :r{i} . }}\n")),
                2 => multi.push_str(&format!(":g2 {{ :s{i} :p :o{i} . }}\n")), // label { … }
                _ => multi.push_str(&format!("{{ :d{i} :p :o{i} . }}\n")),     // anon default block
            }
        }
        differential(&multi, 24);

        // 2. ONE large `GRAPH g { … }` block whose interior statements split across many chunks —
        //    the case the `label { … }` re-wrap exists for (a whole block bigger than a chunk). The
        //    same blank-node label `_:b{k}` recurs at the block's start and end and is chained
        //    between adjacent statements, so the per-graph dict merge must unify it across chunks.
        let mut big = String::from("@prefix : <http://ex/> .\n:top :p :level .\nGRAPH :big {\n_:shared :starts :here .\n");
        for i in 0..500 {
            big.push_str(&format!(":s{i} :p :o{i} .\n_:n{} :next _:n{} .\n", i / 3, i / 3 + 1));
        }
        big.push_str("_:shared :ends :here .\n}\n");
        differential(&big, 24);

        // 3. A BLANK-NODE-named graph `_:g { … }` (label replayed verbatim across chunks) plus
        //    same-spelled `_:g` used as a normal subject/object in the default graph — the routing
        //    key must NOT be conflated with the in-graph bnode (mirrors the N-Quads case 3).
        let mut bgraph = String::from("@prefix : <http://ex/> .\n_:g :is :a-default-subject .\n_:g {\n");
        for i in 0..400 {
            bgraph.push_str(&format!(":s{i} :p _:x{i} .\n"));
        }
        bgraph.push_str("}\n:after :p :default .\n");
        differential(&bgraph, 24);

        // 4. The SAME named graph re-opened in non-adjacent blocks (first-occurrence ordering must
        //    survive: g_a, then g_b, then g_a again must keep [g_a, g_b]), plus prefixed names,
        //    typed/lang literals (incl. an interior `.`), and SPARQL-style `PREFIX` directives.
        let mut reopen = String::from("PREFIX : <http://ex/>\nPREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n");
        for i in 0..300 {
            reopen.push_str(&format!(
                "GRAPH :g_a {{ :s{i} :p \"v.{i}\"@en . }}\n\
                 :top{i} :p \"{i}\"^^xsd:integer .\n\
                 GRAPH :g_b {{ :s{i} :p :o{i} . }}\n"
            ));
        }
        for i in 0..50 {
            reopen.push_str(&format!("GRAPH :g_a {{ :late{i} :p :o{i} . }}\n"));
        }
        differential(&reopen, 24);

        // 5. Mid-document `@prefix` REDEFINITION: statements before vs after the redefinition must
        //    each parse under the correct snapshot (the chunker splits a group at any directive
        //    change and replays the in-scope directives verbatim).
        let mut redef = String::from("@prefix p: <http://a/> .\n");
        for i in 0..150 {
            redef.push_str(&format!("p:s{i} p:p p:o{i} .\nGRAPH p:g {{ p:s{i} p:p p:o{i} . }}\n"));
        }
        redef.push_str("@prefix p: <http://b/> .\n");
        for i in 0..150 {
            redef.push_str(&format!("p:s{i} p:p p:o{i} .\nGRAPH p:g {{ p:s{i} p:p p:o{i} . }}\n"));
        }
        differential(&redef, 24);

        // 6. Comments + blank lines interleaved (a chunk boundary may land on one); a comment text
        //    contains a `.` and braces to confirm they are skipped, not treated as structure.
        let mut sparse = String::from("@prefix : <http://ex/> .\n");
        for i in 0..300 {
            sparse.push_str("# a comment . with a dot and { fake brace }\n\n");
            if i % 2 == 0 {
                sparse.push_str(&format!("GRAPH :g {{ :s{i} :p :o{i} . }}\n"));
            } else {
                sparse.push_str(&format!(":s{i} :p :o{i} .\n"));
            }
        }
        differential(&sparse, 24);
    }

    /// [OPUS-4.8] (sq-m2pc) `parse_to_triples` / `parse_to_triples_with_base` (and the
    /// `load_str` wrappers over them) must REJECT a format string they do not recognise,
    /// instead of the old catch-all that silently parsed everything-unknown as Turtle and
    /// returned `Ok`. The Turtle alias set (`turtle`/`ttl`/`text/turtle`/`application/turtle`)
    /// keeps working, as do the line-based + trig formats; only the genuinely-unknown rest
    /// errors.
    #[test]
    fn parse_to_triples_rejects_unknown_format() {
        // Valid Turtle body — would parse cleanly IF routed to the Turtle arm, so a passing
        // assertion below proves the format string (not the document) is what got rejected.
        let ttl = "<http://ex/s> <http://ex/p> <http://ex/o> .";

        // Every accepted Turtle alias still parses to the one triple.
        for fmt in ["turtle", "ttl", "text/turtle", "application/turtle"] {
            let g = Graph::load_str(ttl, fmt)
                .unwrap_or_else(|e| panic!("alias {fmt:?} must parse as Turtle, got {e}"));
            assert_eq!(g.len(), 1, "alias {fmt:?}");
            // and the with_base entry agrees.
            let gb = Graph::load_str_with_base(ttl, fmt, "http://base/")
                .unwrap_or_else(|e| panic!("alias {fmt:?} (base) must parse, got {e}"));
            assert_eq!(gb.len(), 1, "alias {fmt:?} (base)");
        }

        // The line-based / trig formats (and their `nt`/`nq` extension aliases) are
        // unaffected — they route to their own parser, not the catch-all.
        assert!(Graph::load_str(ttl, "ntriples").is_ok());
        assert!(Graph::load_str(ttl, "nt").is_ok(), "nt is an N-Triples alias");
        assert!(Graph::parse_to_triples(ttl, "application/n-triples").is_ok());

        // Unknown / typo'd / unsupported strings now ERROR instead of silently parsing as
        // Turtle. Empty string is included — it must not be a Turtle alias either.
        // [OPUS-4.8] sq-f47w1 (survey §B1): `"rdfxml"` moved OUT of this always-bogus list —
        // it is an accepted format when the OPT-IN `rdfxml` feature is built (asserted below /
        // in `rdfxml_load_str_round_trip`), and only bogus when the feature is OFF (asserted in
        // the `not(feature = "rdfxml")` block), exactly mirroring how `"jsonld"` is gated.
        for bogus in ["bogusfmt", "turtl", "Turtle", "n3", ""] {
            // (`Result::Ok` carries `(Dict, Vec<…>)`, which is not `Debug`, so `expect_err`
            // cannot be used — match on the error directly.)
            let e = match Graph::parse_to_triples(ttl, bogus) {
                Err(e) => e,
                Ok(_) => panic!("unknown format {bogus:?} must error, not fall back to Turtle"),
            };
            assert!(e.contains(bogus), "error should name the bad format: {e}");
            assert!(
                Graph::parse_to_triples_with_base(ttl, bogus, "http://base/").is_err(),
                "with_base path must also reject {bogus:?}"
            );
            // The public `load_str` wrappers propagate the same error.
            assert!(Graph::load_str(ttl, bogus).is_err(), "load_str must reject {bogus:?}");
        }

        // [OPUS-4.8] sq-f47w1: with the OPT-IN `rdfxml` feature OFF, the RDF/XML aliases are
        // NOT recognised — they must ERROR (fall through to `unknown_format_err`), NOT be
        // mis-parsed as Turtle, exactly as `"jsonld"` does in a non-`jsonld` build.
        #[cfg(not(feature = "rdfxml"))]
        for off in ["rdfxml", "rdf-xml", "application/rdf+xml"] {
            assert!(
                Graph::parse_to_triples(ttl, off).is_err(),
                "without the `rdfxml` feature {off:?} must error"
            );
            assert!(
                Graph::parse_to_triples_with_base(ttl, off, "http://base/").is_err(),
                "without the `rdfxml` feature {off:?} (base) must error"
            );
        }
    }

    /// [OPUS-4.8] (sq-01yr) `load_dataset_serial` (the serial dataset reference, reached by the
    /// `jsonld`/non-`parallel` paths and the TriG-parallel serial fallback) had its OWN
    /// independent `_ => TriGParser` catch-all, so ANY string that was not N-Quads/JSON-LD —
    /// including typos and unsupported formats — silently parsed as TriG and returned `Ok`.
    /// It now gates TriG on its explicit alias set (mirroring the `parse_to_triples` Turtle fix,
    /// sq-m2pc) and rejects the unknown rest. The public `load_dataset` entry — which routes
    /// non-dataset formats to `load_str` first — must agree on the rejection.
    #[test]
    fn load_dataset_rejects_unknown_format() {
        // A one-quad N-Quads body — valid for the dataset path, so a passing assertion proves
        // the FORMAT string (not the document) is what gets rejected below.
        let nq = "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .";
        // A one-statement TriG body in the default graph.
        let trig = "<http://ex/s> <http://ex/p> <http://ex/o> .";

        // Accepted dataset aliases still load through the serial reference.
        for fmt in ["nquads", "n-quads", "nq", "application/n-quads"] {
            let g = Graph::load_dataset_serial(nq, fmt)
                .unwrap_or_else(|e| panic!("alias {fmt:?} must parse as N-Quads, got {e}"));
            assert_eq!(g.named.len() + g.len(), 1, "alias {fmt:?}");
        }
        for fmt in ["trig", "application/trig"] {
            let g = Graph::load_dataset_serial(trig, fmt)
                .unwrap_or_else(|e| panic!("alias {fmt:?} must parse as TriG, got {e}"));
            assert_eq!(g.len(), 1, "alias {fmt:?}");
        }

        // Unknown / typo'd / unsupported strings now ERROR instead of silently parsing as TriG.
        // `trg` is the load-bearing case: previously it hit the catch-all and parsed as TriG.
        // [OPUS-4.8] sq-f47w1 (survey §B1): `"rdfxml"` moved OUT of this list. RDF/XML has no
        // named-graph syntax, so it is NOT a dataset format — `load_dataset_serial` rejects it
        // regardless of the feature (asserted below, unconditionally), but the public
        // `load_dataset` routes a non-dataset format to `load_str`, which ACCEPTS RDF/XML when
        // the OPT-IN `rdfxml` feature is built, so the blanket `load_dataset(...).is_err()`
        // assertion no longer holds for it. Handled in the dedicated block after this loop.
        for bogus in ["bogusfmt", "trg", "TriG", ""] {
            let e = match Graph::load_dataset_serial(trig, bogus) {
                Err(e) => e,
                Ok(_) => panic!("unknown format {bogus:?} must error, not fall back to TriG"),
            };
            assert!(e.contains(bogus), "error should name the bad format: {e}");
            // The public `load_dataset` entry must also reject it: it routes a non-dataset
            // format to `load_str`, which rejects the unknown string under sq-m2pc.
            assert!(
                Graph::load_dataset(trig, bogus).is_err(),
                "public load_dataset must reject {bogus:?}"
            );
        }

        // [OPUS-4.8] sq-f47w1: the RDF/XML aliases are NOT dataset formats — `load_dataset_serial`
        // (the per-graph serial reference) rejects them whether or not the `rdfxml` feature is
        // built, because RDF/XML carries no named graphs and never reaches a quad-bearing parser.
        for off in ["rdfxml", "rdf-xml", "application/rdf+xml"] {
            assert!(
                Graph::load_dataset_serial(trig, off).is_err(),
                "RDF/XML {off:?} is not a dataset serial format — must error"
            );
        }
    }

    /// [OPUS-4.8] (sq-ev37) The public `Graph::load_dataset("…","trig")` entry point (which under
    /// the `parallel` feature dispatches to `load_trig_parallel`) must agree with the serial
    /// reference on a dataset large enough to genuinely fan out — the end-to-end witness that the
    /// dispatch + per-graph build are wired correctly, not just the internals.
    #[cfg(feature = "parallel")]
    #[test]
    fn load_dataset_trig_public_entry_matches_serial() {
        let mut trig = String::from("@prefix : <http://ex/> .\n_:shared :a :b .\n");
        for i in 0..3000 {
            if i % 3 == 0 {
                trig.push_str(&format!(":s{i} :p _:n{i} .\n"));
            } else {
                trig.push_str(&format!("GRAPH :g1 {{ :s{i} :p _:n{i} . }}\n"));
            }
        }
        trig.push_str("GRAPH :g1 { _:shared :c :d . }\n");
        let pub_g = Graph::load_dataset(&trig, "trig").unwrap();
        let ser = Graph::load_dataset_serial(&trig, "trig").unwrap();
        assert_eq!(pub_g.len(), ser.len());
        assert_eq!(pub_g.named.len(), ser.named.len(), "named graph count");
        assert_eq!(canon_dataset(&pub_g), canon_dataset(&ser), "public load_dataset must equal serial");
    }

    /// [FABLE-5] (sq-tonhr.2) `load_dataset_with_base` — the DATASET companion to
    /// `load_str_with_base` — must resolve a TriG document's relative IRIs (subjects, objects
    /// AND graph names) against the given base while preserving named graphs, ignore the base
    /// for the no-relative-IRI N-Quads format, reject an invalid base, and keep rejecting
    /// unknown formats (the sq-m2pc no-silent-Turtle-fallback contract).
    #[test]
    fn load_dataset_with_base_resolves_and_preserves_named_graphs() {
        let trig = "<s> <p> <o> .\nGRAPH <g> { <s2> <p2> \"lit\" . }\n";
        let g = Graph::load_dataset_with_base(trig, "trig", "http://base.example/dir/").unwrap();
        assert_eq!(g.len(), 1, "default graph triple count");
        assert!(
            g.id_of(&Term::NamedNode(
                oxrdf::NamedNode::new("http://base.example/dir/s").unwrap()
            ))
            .is_some(),
            "relative subject must resolve against the base"
        );
        let gname = Term::NamedNode(oxrdf::NamedNode::new("http://base.example/dir/g").unwrap());
        let named = g.named_graph(&gname).expect("named graph resolved against the base");
        assert_eq!(named.len(), 1, "named graph triple count");
        // N-Quads: absolute IRIs only — base has no effect, quads still bucket per graph.
        let nq = "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .\n";
        let g = Graph::load_dataset_with_base(nq, "nquads", "http://base.example/").unwrap();
        assert_eq!(g.len(), 0, "N-Quads named-graph quad must not land in the default graph");
        assert!(g.named_graph(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/g").unwrap())).is_some());
        // An invalid base IRI is a loud error, and a relative-IRI TriG doc with NO usable base
        // (the serial oxttl reference behaviour) stays an error through the with-base entry too.
        assert!(Graph::load_dataset_with_base(trig, "trig", "not a base iri").is_err());
        // Unknown formats keep erroring (never silently parsed as TriG/Turtle).
        assert!(Graph::load_dataset_with_base(trig, "nosuch", "http://base.example/").is_err());
        // Non-dataset formats defer to load_str_with_base (named-graph-free fold path).
        let ttl = "<s> <p> <o> .\n";
        let g = Graph::load_dataset_with_base(ttl, "turtle", "http://base.example/dir/").unwrap();
        assert_eq!(g.len(), 1);
        assert!(g.named.is_empty());
    }

    /// [OPUS-4.8] (sq-ev37) Malformed / not-cleanly-splittable TriG must NOT be silently accepted:
    /// either `trig_chunks` bails (`None` → serial) or some chunk fails to re-parse (→ serial), and
    /// either way the public loader returns the SAME `Err` the serial oxttl parser does. This pins
    /// the one failure the chunked-vs-serial oracle cannot catch — an over-eager split that happens
    /// to parse invalid input as valid.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_trig_rejects_malformed() {
        let cases = [
            // Unclosed graph block.
            "@prefix : <http://ex/> .\n:a :p :b .\nGRAPH :g { :x :y :z .\n:c :d :e .\n",
            // A `}` with no matching `{`.
            "@prefix : <http://ex/> .\n:a :p :b .\n} :c :d :e .\n:f :g :h .\n",
            // Missing `.` between two default-graph triples (oxttl rejects the fused statement).
            "@prefix : <http://ex/> .\n:a :p :b\n:c :d :e .\n:f :g :h .\n",
            // An undefined prefix used inside a block (`undef:` was never declared).
            "@prefix : <http://ex/> .\n:a :p :b .\nGRAPH :g { :x undef:y :z . }\n:c :d :e .\n",
        ];
        for src in cases {
            let serial_err = Graph::load_dataset_serial(src, "trig").is_err();
            let public_err = Graph::load_dataset(src, "trig").is_err();
            assert_eq!(public_err, serial_err, "rejection parity differs for {src:?}");
            assert!(serial_err, "case was expected to be invalid TriG: {src:?}");
        }
    }

    /// [OPUS-4.8] (sq-25r3) The public `Graph::load_dataset("…","nquads")` entry point (which
    /// dispatches to the parallel path under the `parallel` feature) must agree with the serial
    /// reference on a dataset with default + named graphs and a cross-chunk blank node — the
    /// end-to-end witness that the dispatch + build are wired correctly, not just the internals.
    #[cfg(feature = "parallel")]
    #[test]
    fn load_dataset_nquads_public_entry_matches_serial() {
        let mut nq = String::from("_:shared <http://ex/a> <http://ex/b> .\n");
        for i in 0..2000 {
            let g = if i % 3 == 0 { "" } else { " <http://ex/g1>" };
            nq.push_str(&format!("<http://ex/s{i}> <http://ex/p> _:n{i}{g} .\n"));
        }
        nq.push_str("_:shared <http://ex/c> <http://ex/d> <http://ex/g1> .\n");
        let pub_g = Graph::load_dataset(&nq, "nquads").unwrap();
        let ser = Graph::load_dataset_serial(&nq, "nquads").unwrap();
        assert_eq!(pub_g.len(), ser.len());
        assert_eq!(pub_g.named.len(), ser.named.len(), "named graph count");
        assert_eq!(canon_dataset(&pub_g), canon_dataset(&ser), "public load_dataset must equal serial");
    }

    /// [OPUS-4.8] (sq-25r3 / sq-ev37) Loading a small TriG dataset through the public
    /// `load_dataset` must split the default and named graphs correctly. (Under the `parallel`
    /// feature this now routes through the chunk-parallel `load_trig_parallel`, which falls back to
    /// serial for a document this small; the result is the same either way — the byte-identical
    /// equivalence on larger, genuinely-fanned-out documents is pinned by
    /// `parallel_trig_matches_serial`.)
    #[test]
    fn load_dataset_trig_still_serial_and_correct() {
        let trig = "@prefix : <http://ex/> .\n\
                    :a :p :b .\n\
                    :g1 { :x :q :y . :x :q :z . }\n\
                    :g2 { :m :n :o . }\n";
        let g = Graph::load_dataset(trig, "trig").unwrap();
        assert_eq!(g.len(), 1, "default graph triple");
        let names: Vec<String> = g.named.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(names, vec!["<http://ex/g1>".to_string(), "<http://ex/g2>".to_string()], "named graphs in document order");
        assert_eq!(g.named[0].1.len(), 2, "g1 has 2 triples");
        assert_eq!(g.named[1].1.len(), 1, "g2 has 1 triple");
    }

    /// [OPUS-4.8] (T2/T3 rejection parity) Malformed Turtle must still be REJECTED — the
    /// parallel split is never allowed to silently accept invalid input. Whether the splitter
    /// fans out (then a chunk fails oxttl → serial redo) or bails directly, `parse_turtle_chunked`
    /// must return `Err`, exactly as the serial oxttl parser does. An over-eager split that
    /// accidentally parsed invalid Turtle as valid is the one failure the chunked-vs-serial
    /// oracle cannot catch, so it is pinned here directly.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_turtle_rejects_malformed() {
        // Each is invalid Turtle for a distinct reason; size them past the 2-statement minimum so
        // the splitter actually engages where it can.
        let bad: &[&str] = &[
            // Unknown prefix used in the body (the body chunk must fail to parse).
            "@prefix ex: <http://ex/> .\nex:s ex:p ex:o .\nbad:s bad:p bad:o .\n",
            // Missing terminator on a statement.
            "@prefix ex: <http://ex/> .\nex:s ex:p ex:o\nex:s2 ex:p ex:o2 .\n",
            // An interspersed @prefix that is itself malformed (no IRI).
            "@prefix ex: <http://ex/> .\nex:s ex:p ex:o .\n@prefix bad .\nex:s2 ex:p ex:o2 .\n",
            // A statement that uses a prefix declared only LATER (Turtle requires prior decl) —
            // the snapshot must NOT retro-actively make it valid.
            "@prefix a: <http://a/> .\na:s a:p later:o .\n@prefix later: <http://l/> .\na:s2 a:p a:o .\n",
            // Bare unescaped control / illegal token sequence.
            "@prefix ex: <http://ex/> .\nex:s ex:p \"unterminated .\nex:s2 ex:p ex:o2 .\n",
            // [OPUS-4.8] (T3 SPARQL-form rejection parity) SPARQL-style `BASE` with an illegal
            // trailing `.` (W3C turtle-syntax-bad-base-03). The SPARQL-directive splitter ends the
            // span at the `>` and leaves the `.` in the stream; the chunk that then begins with `.`
            // must fail oxttl → serial redo → reject. A splitter that swallowed the `.` would
            // silently ACCEPT this invalid document — the over-eager-split corruption class.
            "BASE <http://b/> .\n<s> <http://b/p> <http://b/o> .\n<s2> <http://b/p> <http://b/o2> .\n",
            // SPARQL-style `PREFIX` with an illegal trailing `.`.
            "PREFIX p: <http://b/> .\np:s p:p p:o .\np:s2 p:p p:o2 .\n",
            // SPARQL-style `PREFIX` whose prefixed name is used in the body but the directive's
            // IRIREF is missing (truncated) — must reject, not be silently snapshotted.
            "PREFIX p: badnoiriref\np:s p:p p:o .\np:s2 p:p p:o2 .\n",
        ];
        for (k, src) in bad.iter().enumerate() {
            // Reference: the serial oxttl parser rejects it.
            let mut sd = Dict::new();
            assert!(
                parse_turtle_chunk(src.as_bytes(), &mut sd).is_err(),
                "case {k} should be invalid Turtle (serial oxttl)"
            );
            // The parallel path must reject it too (fan-out + chunk-fail-serial-redo, or a direct
            // bail then serial), at several fan-out targets.
            for target in [1usize, 2, 8, 32] {
                assert!(
                    parse_turtle_chunked(src.as_bytes(), target).is_err(),
                    "case {k} target {target}: parallel path must reject malformed Turtle"
                );
            }
        }
    }

    /// [OPUS-4.8] (B4 — sparq-turtle-path REJECTION ORACLE) The public `Graph::parse_to_triples`
    /// `"turtle"` path MUST reject malformed Turtle, and reject EXACTLY what the oxttl serial
    /// parser rejects (oxttl is the differential reference). This is the load-bearing gate for the
    /// T1 borrowed-slice interner spike: T1 changes how `parse_turtle_chunk` interns oxttl's
    /// already-parsed terms, so it cannot itself accept invalid syntax — but the chunked-vs-serial
    /// byte-identity oracle (`parallel_turtle_bnodes_match_serial`) only proves chunked ≡ serial,
    /// NOT that either rejects what the spec/oxttl rejects. An over-eager accept would be a silent
    /// corruption invisible to that oracle. So we pin REJECTION here directly, on the PUBLIC entry
    /// point (`parse_to_triples`, which dispatches to `parse_turtle_parallel` under `parallel`),
    /// for a dozen-plus malformed inputs spanning the categories the brief calls out: bad IRIs,
    /// bad escapes, unterminated strings, bad prefixes, plus structural errors.
    ///
    /// The assertion is DIFFERENTIAL, not just "sparq errors": for each input we first confirm the
    /// oxttl serial reference rejects it (so the corpus stays honest as oxttl evolves), then assert
    /// the public sparq turtle path rejects it too. Positive controls confirm the harness is not
    /// vacuously rejecting everything.
    #[test]
    fn turtle_path_rejection_oracle() {
        // Differential reference: serial oxttl (the same parser the chunk worker drives).
        let oxttl_serial_rejects = |src: &str| -> bool {
            oxttl::TurtleParser::new()
                .for_slice(src.as_bytes())
                .any(|r| r.is_err())
        };

        // Malformed Turtle, one invalid reason each. A leading valid statement is included where
        // it helps the parallel splitter actually engage (so the chunk path, not only the 1T
        // direct parse, is exercised by the same corpus when run under --features parallel).
        let bad: &[(&str, &str)] = &[
            // ---- bad prefixes / prefixed names ----
            ("undeclared prefix in body",
             "@prefix ex: <http://ex/> .\nex:s ex:p ex:o .\nbad:s bad:p bad:o .\n"),
            ("@prefix without an IRI",
             "@prefix ex: <http://ex/> .\nex:s ex:p ex:o .\n@prefix bad .\nex:s2 ex:p ex:o2 .\n"),
            ("prefix used before its declaration",
             "@prefix a: <http://a/> .\na:s a:p later:o .\n@prefix later: <http://l/> .\na:s2 a:p a:o .\n"),
            ("@prefix missing the colon",
             "@prefix ex <http://ex/> .\nex:s ex:p ex:o .\n"),
            // ---- bad IRIs ----
            ("unterminated IRI ref (no closing >)",
             "@prefix ex: <http://ex/> .\nex:s ex:p <http://no-close .\nex:s2 ex:p ex:o2 .\n"),
            ("space inside an IRI ref",
             "<http://ex/ s> <http://ex/p> <http://ex/o> .\n"),
            // ---- bad string escapes ----
            ("invalid string escape \\q",
             "@prefix ex: <http://ex/> .\nex:s ex:p \"bad \\q escape\" .\nex:s2 ex:p ex:o2 .\n"),
            ("truncated \\u escape (too few hex digits)",
             "@prefix ex: <http://ex/> .\nex:s ex:p \"bad \\u12\" .\nex:s2 ex:p ex:o2 .\n"),
            // ---- unterminated strings ----
            ("unterminated single-quoted string",
             "@prefix ex: <http://ex/> .\nex:s ex:p \"unterminated .\nex:s2 ex:p ex:o2 .\n"),
            ("unterminated triple-quoted string",
             "@prefix ex: <http://ex/> .\nex:s ex:p \"\"\"open\nstill open .\n"),
            // ---- structural ----
            ("missing statement terminator",
             "@prefix ex: <http://ex/> .\nex:s ex:p ex:o\nex:s2 ex:p ex:o2 .\n"),
            ("predicate with no object",
             "@prefix ex: <http://ex/> .\nex:s ex:p .\nex:s2 ex:p ex:o2 .\n"),
            ("literal in subject position",
             "@prefix ex: <http://ex/> .\n\"lit\" ex:p ex:o .\n"),
            ("bad numeric literal (double dot)",
             "@prefix ex: <http://ex/> .\nex:s ex:p 1.2.3 .\nex:s2 ex:p ex:o2 .\n"),
            ("unmatched [ property-list",
             "@prefix ex: <http://ex/> .\nex:s ex:p [ ex:q ex:r .\nex:s2 ex:p ex:o2 .\n"),
            ("unmatched ( collection",
             "@prefix ex: <http://ex/> .\nex:s ex:p ( ex:a ex:b .\nex:s2 ex:p ex:o2 .\n"),
        ];

        for (why, src) in bad {
            assert!(
                oxttl_serial_rejects(src),
                "corpus invariant: oxttl reference must reject `{why}` (update the corpus if oxttl changed)"
            );
            assert!(
                Graph::parse_to_triples(src, "turtle").is_err(),
                "sparq turtle path must REJECT malformed Turtle: {why}"
            );
        }

        // Positive controls: well-formed Turtle MUST parse (so the oracle is not vacuous), covering
        // the constructs the T1 interner has to keep handling — prefixed names, full IRIs, blank
        // nodes, collections, language tags, datatyped + plain literals, RDF 1.2 triple terms.
        let good: &[&str] = &[
            "@prefix ex: <http://ex/> .\nex:s ex:p ex:o .\n",
            "<http://ex/s> <http://ex/p> \"plain\" .\n",
            "@prefix ex: <http://ex/> .\nex:s ex:p [ ex:q ( 1 2 3 ) ] ; ex:r _:b .\n",
            "@prefix ex: <http://ex/> .\nex:s ex:p \"hi\"@en , \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "@prefix ex: <http://ex/> .\n<< ex:s ex:p ex:o >> ex:said ex:alice .\n",
        ];
        for src in good {
            assert!(
                !oxttl_serial_rejects(src),
                "corpus invariant: oxttl reference must ACCEPT this well-formed Turtle"
            );
            assert!(
                Graph::parse_to_triples(src, "turtle").is_ok(),
                "sparq turtle path must ACCEPT well-formed Turtle: {src:?}"
            );
        }
    }

    /// [OPUS-4.8] (T2) Differential test for the `memchr`-based `next_terminator` against the
    /// pre-T2 scalar reference. The SIMD-skip must return the SAME offset (or the same `None`)
    /// for every interesting Turtle shape — comments, all four string-quote forms with escapes,
    /// IRIs, decimals (`.`-not-a-terminator), PN_LOCAL_ESC `\#`/`\.`, and EOF-truncation — so it
    /// is a behaviour-preserving speedup, not a re-derivation of the splitter's grammar.
    #[cfg(feature = "parallel")]
    #[test]
    fn next_terminator_memchr_matches_scalar() {
        // The exact pre-T2 scalar scanner, kept here as the oracle.
        fn scalar(bytes: &[u8], start: usize) -> Option<usize> {
            let n = bytes.len();
            let mut i = start;
            while i < n {
                match bytes[i] {
                    b'#' => {
                        while i < n && bytes[i] != b'\n' {
                            i += 1;
                        }
                    }
                    b'<' => {
                        i += 1;
                        while i < n && bytes[i] != b'>' {
                            i += 1;
                        }
                        if i >= n {
                            return None;
                        }
                        i += 1;
                    }
                    q @ (b'"' | b'\'') => {
                        let triple = i + 2 < n && bytes[i + 1] == q && bytes[i + 2] == q;
                        if triple {
                            i += 3;
                            loop {
                                if i >= n {
                                    return None;
                                }
                                if bytes[i] == b'\\' {
                                    i += 2;
                                } else if bytes[i] == q
                                    && i + 2 < n
                                    && bytes[i + 1] == q
                                    && bytes[i + 2] == q
                                {
                                    i += 3;
                                    break;
                                } else {
                                    i += 1;
                                }
                            }
                        } else {
                            i += 1;
                            while i < n && bytes[i] != q {
                                i += if bytes[i] == b'\\' { 2 } else { 1 };
                            }
                            if i >= n {
                                return None;
                            }
                            i += 1;
                        }
                    }
                    b'.' => match bytes.get(i + 1) {
                        None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'#') => return Some(i + 1),
                        _ => i += 1,
                    },
                    b'\\' => {
                        if i + 1 >= n {
                            return None;
                        }
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            None
        }

        let cases: &[&str] = &[
            ":s :p :o .\n",
            ":s :p :o .",                                  // EOF terminator
            ":s :p 3.14 ; :q 2.5 .\n",                     // decimals are not terminators
            ":s :p \"a.b.c\" .\n",                          // dot inside a string
            ":s :p \"x\\\"y.z\" .\n",                       // escaped quote, then dot in string
            ":s :p 'single.quoted' .\n",                    // single-quote string
            ":s :p \"\"\"l1 . still\nl2.\"\"\" .\n",        // triple-quoted, dots + newline inside
            ":s :p '''a.b''' .\n",                          // triple single-quote
            ":s :p <http://x.y/a.b.c> .\n",                 // dots inside an IRI
            "ex:s ex:p ex:foo\\#bar .\n",                   // PN_LOCAL_ESC \# (review 1398)
            "ex:s ex:p ex:a\\.b .\n",                       // PN_LOCAL_ESC \. — \. is NOT a term
            ":s :p :o .# trailing comment\n:s2 :p :o2 .\n", // comment right after terminator
            "# leading comment with . inside\n:s :p :o .\n",
            ":s :p :o",                                     // no terminator, no EOF dot → None
            ":s :p \"unterminated",                         // unterminated string → None
            ":s :p <unterminated",                          // unterminated IRI → None
            ":s :p \"\"\"unterminated\n.",                  // unterminated triple-quote → None
            "ex:s ex:p ex:x\\",                             // trailing backslash → None
            "",
            ".",                                            // bare dot at EOF is a terminator
            ".5 .\n",                                       // leading decimal then real terminator
            // [OPUS-4.8] (sq-98w7z.1) Quote/backslash-FREE bodies: the bounded second `memchr3`
            // pass (introduced with the O(n²) fix) takes the `b == None` arm here, so the split
            // offset must still match the scalar oracle exactly. An all-IRI N-Triples-shaped run
            // is precisely the shape that used to trigger the quadratic full-tail re-scan.
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n<http://ex/s2> <http://ex/p> <http://ex/o2> .\n",
            ":a :b :c . :d :e :f . :g :h :i .\n",           // several quote-free statements in a row
        ];
        for (k, c) in cases.iter().enumerate() {
            let b = c.as_bytes();
            // From every start offset, not just 0 — turtle_chunks resumes mid-buffer.
            for start in 0..=b.len() {
                assert_eq!(
                    next_terminator(b, start),
                    scalar(b, start),
                    "case {k} {c:?} at start {start}"
                );
            }
        }
    }

    /// [OPUS-4.8] (sq-3ui0, gh-45) A dataset with MULTIPLE named graphs must survive a
    /// save→open round-trip LOSSLESSLY: every named graph's IRI + triples reopen exactly,
    /// including a graph the default graph does not name. Sorted-dump equality of the whole
    /// dataset (default + every named graph) on both sides is the data-loss guard — before
    /// this fix `open` dropped `named` entirely (total data loss for PSS's per-resource
    /// named graphs).
    #[cfg(feature = "mmap")]
    #[test]
    fn save_open_named_graphs_roundtrip_lossless() {
        type Triple = (String, String, String);
        type Dataset = Vec<(String, Vec<Triple>)>;
        // Dump a single graph's triples as a sorted N-Triples-ish vector.
        fn dump_one(gg: &Graph) -> Vec<Triple> {
            let scan = gg.store.scan(&[None, None, None]);
            let mut v: Vec<Triple> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (gg.dict.term(spo[0]).to_string(), gg.dict.term(spo[1]).to_string(), gg.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        }
        // Dump the WHOLE dataset: default graph under key "", each named graph under its name.
        fn dump_dataset(gg: &Graph) -> Dataset {
            let mut out: Dataset = vec![(String::new(), dump_one(gg))];
            for (name, sub) in &gg.named {
                out.push((name.to_string(), dump_one(sub)));
            }
            out.sort();
            out
        }

        // A dataset shaped like PSS: a default graph plus several named graphs whose IRI ==
        // the "resource" IRI (one of them with a literal + lang-tag, one a single triple).
        let nq = concat!(
            "<http://ex/default-s> <http://ex/p> <http://ex/default-o> .\n",
            "<http://res/a> <http://ex/p> \"value a\" <http://res/a> .\n",
            "<http://res/a> <http://ex/label> \"caf\\u00e9\"@fr <http://res/a> .\n",
            "<http://res/b> <http://ex/p> <http://ex/o-b> <http://res/b> .\n",
            "<http://res/b/.acl> <http://acl#mode> <http://acl#Read> <http://res/b/.acl> .\n",
        );
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        assert_eq!(g.named.len(), 3, "fixture should have 3 named graphs");
        let before = dump_dataset(&g);

        let dir = std::env::temp_dir().join(format!("sparq_named_rt_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        g.save(&dir).unwrap();
        // The manifest + one sub-directory per named graph must exist.
        assert!(dir.join("named.bin").exists(), "named-graph manifest not persisted");
        for i in 0..3 {
            assert!(dir.join("named").join(i.to_string()).join("perm0.bin").exists(), "named sub-graph {i} not persisted");
        }

        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.named.len(), 3, "named graphs dropped on reopen");
        let after = dump_dataset(&g2);
        assert_eq!(before, after, "named-graph dataset not losslessly round-tripped");

        // Every reopened named graph must carry its OWN per-graph WAL (durable updates).
        for (_, sub) in &g2.named {
            assert!(sub.wal.is_some(), "reopened named graph lacks its own WAL");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-3ui0) The block-COMPRESSED save path must round-trip named graphs too.
    #[cfg(feature = "mmap")]
    #[test]
    fn save_compressed_named_graphs_roundtrip_lossless() {
        let nq = concat!(
            "<http://ex/d> <http://ex/p> <http://ex/o> .\n",
            "<http://res/x> <http://ex/p> <http://ex/ox> <http://res/x> .\n",
            "<http://res/y> <http://ex/p> <http://ex/oy> <http://res/y> .\n",
        );
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        let dir = std::env::temp_dir().join(format!("sparq_named_zrt_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        g.save_compressed(&dir).unwrap();
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.named.len(), 2);
        let named_of = |gg: &Graph, name: &str| {
            gg.named.iter().find(|(n, _)| n.to_string() == format!("<{name}>")).map(|(_, s)| s.len())
        };
        assert_eq!(named_of(&g2, "http://res/x"), Some(1));
        assert_eq!(named_of(&g2, "http://res/y"), Some(1));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-3ui0) A DEFAULT-GRAPH-ONLY save writes NO `named/` subtree and NO
    /// `named.bin`, so its on-disk layout is unchanged from before this feature
    /// (byte-identity preserved for the common single-graph case).
    #[cfg(feature = "mmap")]
    #[test]
    fn default_only_save_has_no_named_artifacts() {
        let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .\n", "ntriples").unwrap();
        assert!(g.named.is_empty());
        let dir = std::env::temp_dir().join(format!("sparq_default_only_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        g.save(&dir).unwrap();
        assert!(!dir.join("named.bin").exists(), "default-only save must not write a named manifest");
        assert!(!dir.join("named").exists(), "default-only save must not write a named subtree");
        let g2 = Graph::open(&dir).unwrap();
        assert!(g2.named.is_empty());
        assert_eq!(g2.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-3ui0) A named-graph manifest with an UNKNOWN format version (or bad
    /// magic) is a hard error on open — never silently misread as the current format.
    #[cfg(feature = "mmap")]
    #[test]
    fn named_manifest_unknown_version_is_rejected() {
        let nq = "<http://res/x> <http://ex/p> <http://ex/o> <http://res/x> .\n";
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        let dir = std::env::temp_dir().join(format!("sparq_named_badver_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        g.save(&dir).unwrap();
        // Corrupt the manifest's version field (bytes 4..8) to a future version.
        let mut bytes = std::fs::read(dir.join("named.bin")).unwrap();
        bytes[4..8].copy_from_slice(&999u32.to_le_bytes());
        std::fs::write(dir.join("named.bin"), &bytes).unwrap();
        match Graph::open(&dir) {
            Ok(_) => panic!("unknown manifest version must NOT open silently"),
            Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "unknown version must be a hard InvalidData error"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (Copilot review #69, finding 3) A hostile/corrupt manifest that declares a
    /// HUGE entry `count` behind a tiny file must fail as a clean `InvalidData` (truncated)
    /// error — NEVER an unbounded `Vec::with_capacity` allocation (OOM/abort). The reservation
    /// is clamped to the file's remaining length, and the decode loop then errors on the
    /// missing entry bytes.
    #[cfg(feature = "mmap")]
    #[test]
    fn named_manifest_hostile_count_is_bounded_not_oom() {
        let nq = "<http://res/x> <http://ex/p> <http://ex/o> <http://res/x> .\n";
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        let dir = std::env::temp_dir().join(format!("sparq_named_hostile_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        g.save(&dir).unwrap();
        // Set the count field (bytes 8..12) to ~4 billion — a real `with_capacity(count)` would
        // try to reserve tens of GB (an uncatchable abort) before any bounds check ran.
        let mut bytes = std::fs::read(dir.join("named.bin")).unwrap();
        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(dir.join("named.bin"), &bytes).unwrap();
        // Must return cleanly (the clamp + the bounded decode loop), not OOM/panic.
        match Graph::open(&dir) {
            Ok(_) => panic!("a manifest whose count exceeds the file must NOT open"),
            Err(e) => assert_eq!(
                e.kind(),
                std::io::ErrorKind::InvalidData,
                "a hostile count must be a clean InvalidData error (truncated manifest), not an OOM"
            ),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (Copilot review #69, findings 1+2) The manifest write is ATOMIC: it goes
    /// through a temp file + rename, so the canonical `named.bin` is always a COMPLETE image,
    /// never a torn one. This asserts the temp file is cleaned up (renamed away, not left
    /// behind) and that a simulated CRASH that leaves a stale `named.bin.tmp` behind does NOT
    /// affect a subsequent open (the canonical manifest is the only commit point).
    #[cfg(feature = "mmap")]
    #[test]
    fn named_manifest_write_is_atomic_temp_then_rename() {
        let nq = concat!(
            "<http://ex/d> <http://ex/p> <http://ex/o> .\n",
            "<http://res/x> <http://ex/p> <http://ex/ox> <http://res/x> .\n",
            "<http://res/y> <http://ex/p> <http://ex/oy> <http://res/y> .\n",
        );
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        let dir = std::env::temp_dir().join(format!("sparq_named_atomic_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        g.save(&dir).unwrap();
        // The atomic write must leave the canonical manifest and NO leftover temp file.
        assert!(dir.join("named.bin").exists(), "canonical manifest missing after atomic write");
        assert!(!dir.join("named.bin.tmp").exists(), "temp manifest must be renamed away, not left behind");
        // Simulate a crash mid-write of a *previous* attempt: a stale, GARBAGE temp file is on
        // disk. It must be IGNORED on open (only `named.bin` is the commit point) — `open_named`
        // reads `named.bin`, never `named.bin.tmp`, so the torn temp can never be misread.
        std::fs::write(dir.join("named.bin.tmp"), b"torn-garbage-not-a-manifest").unwrap();
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.named.len(), 2, "stale temp file must not affect the open");
        // A fresh atomic write (into a clean dir) must leave NO leftover temp file: the rename
        // consumed it. (We save the in-memory copy to a SEPARATE dir — re-saving an mmap'd
        // graph back over its own source files is unsupported on every save path here.)
        let dir2 = std::env::temp_dir().join(format!("sparq_named_atomic2_{}", std::process::id()));
        std::fs::remove_dir_all(&dir2).ok();
        g.save(&dir2).unwrap();
        assert!(!dir2.join("named.bin.tmp").exists(), "atomic write must rename the temp file away");
        let g3 = Graph::open(&dir2).unwrap();
        assert_eq!(g3.named.len(), 2, "named graphs lost after atomic write");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&dir2).ok();
    }

    /// [OPUS-4.8] (sq-3ui0, gh-45) PER-GRAPH WAL durability: a NEW named graph created via
    /// `apply_delta_nquads` against a DIRECTORY-BACKED graph is WAL-logged + manifested at
    /// once, so a crash-style reopen (no `save`/`compact` in between) still recovers both
    /// the new graph AND a subsequent named-graph update — the named-graph analogue of the
    /// default-graph WAL recovery.
    #[cfg(feature = "mmap")]
    #[test]
    fn named_graph_updates_are_wal_durable() {
        let dir = std::env::temp_dir().join(format!("sparq_named_wal_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        // Persist an empty (default-only) directory-backed graph, then open it for updates.
        Graph::load_str("", "ntriples").unwrap().save(&dir).unwrap();
        {
            let mut g = Graph::open(&dir).unwrap();
            // Create a brand-new named graph (auto-created) and add a triple to it.
            g.apply_delta_nquads("<http://res/a> <http://ex/p> <http://ex/o1> <http://res/a> .\n", "").unwrap();
            // A second batch into the SAME named graph (exercises the per-graph WAL append).
            g.apply_delta_nquads("<http://res/a> <http://ex/p> <http://ex/o2> <http://res/a> .\n", "").unwrap();
            // A second named graph too.
            g.apply_delta_nquads("<http://res/b> <http://ex/p> <http://ex/o3> <http://res/b> .\n", "").unwrap();
            // Deliberately DROP `g` WITHOUT save()/compact() — only the WALs are on disk.
        }
        // Reopen: the named graphs + every WAL-logged triple must recover.
        let g2 = Graph::open(&dir).unwrap();
        let named_of = |gg: &Graph, name: &str| {
            gg.named.iter().find(|(n, _)| n.to_string() == format!("<{name}>")).map(|(_, s)| s.len())
        };
        assert_eq!(named_of(&g2, "http://res/a"), Some(2), "named graph a lost a WAL-logged triple");
        assert_eq!(named_of(&g2, "http://res/b"), Some(1), "named graph b not recovered from WAL");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (gh-1122) `insert_triple` / `remove_triple` against a DIRECTORY-BACKED graph
    /// flow through the SAME durable `apply_delta` path as a batch: each is WAL-logged + fsync'd,
    /// so a crash-style reopen (no save/compact in between) recovers the insert and honours the
    /// remove. This is the load-bearing durability invariant of the single-triple convenience API.
    #[cfg(feature = "mmap")]
    #[test]
    fn single_triple_mutations_are_wal_durable() {
        let dir = std::env::temp_dir().join(format!("sparq_single_triple_wal_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str("", "ntriples").unwrap().save(&dir).unwrap();
        let s = NamedNode::new_unchecked("http://ex/s");
        let p = NamedNode::new_unchecked("http://ex/p");
        let keep = Term::NamedNode(NamedNode::new_unchecked("http://ex/keep"));
        let gone = Term::NamedNode(NamedNode::new_unchecked("http://ex/gone"));
        {
            let mut g = Graph::open(&dir).unwrap();
            g.insert_triple(s.clone(), p.clone(), keep.clone()).unwrap();
            g.insert_triple(s.clone(), p.clone(), gone.clone()).unwrap();
            // Retract one of them through the single-triple remove path.
            g.remove_triple(s.clone(), p.clone(), gone.clone()).unwrap();
            // Drop WITHOUT save()/compact(): only the WAL records are on disk.
        }
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.len(), 1, "exactly the un-retracted insert must recover from the WAL");
        assert!(g2.id_of(&keep).is_some(), "the kept triple's object must be present");
        assert!(
            g2.pattern(Some(&Term::NamedNode(s)), Some(&p), Some(&gone)).is_none()
                || g2.store.scan(&[None, None, None]).rows.len() == 1,
            "the removed triple must not survive the reopen"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(feature = "mmap")]
    #[test]
    fn save_open_mmap_roundtrip() {
        // Build a graph, persist it, re-open with the indexes memory-mapped, and check
        // the store + dictionary are structurally identical (every triple round-trips).
        let mut nt = String::new();
        for i in 0..3000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 211,
                i % 13,
                i % 500
            ));
        }
        nt.push_str("<http://ex/n0> <http://ex/name> \"caf\\u00e9\"@fr .\n");
        // Temporal literals so the temporals.bin round-trip below has real cells:
        // zoned + floating dateTimes (sub-second), a date, and an ill-formed dateTime
        // (must stay uncached on both sides).
        nt.push_str("<http://ex/n1> <http://ex/at> \"2024-03-15T13:00:00.25Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n");
        nt.push_str("<http://ex/n2> <http://ex/at> \"2024-03-15T13:00:00\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n");
        nt.push_str("<http://ex/n3> <http://ex/on> \"2024-03-15\"^^<http://www.w3.org/2001/XMLSchema#date> .\n");
        nt.push_str("<http://ex/n4> <http://ex/at> \"not-a-date\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n");
        let g = Graph::load_str(&nt, "ntriples").unwrap();
        let dir = std::env::temp_dir().join(format!("sparq_mmap_test_{}", std::process::id()));
        g.save(&dir).unwrap();
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g.len(), g2.len());
        assert_eq!(g.dict.len(), g2.dict.len());
        let dump = |gg: &Graph| {
            let scan = gg.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (gg.dict.term(spo[0]).to_string(), gg.dict.term(spo[1]).to_string(), gg.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(dump(&g), dump(&g2), "mmap-reopened store differs");

        // The numeric-value cache must round-trip through its memory-mapped form: every
        // numeric literal resolves to the same f64 (and non-numerics to None) as before.
        assert!(dir.join("numerics.bin").exists(), "numerics cache not persisted");
        assert!(matches!(g2.numerics, NumData::Mapped(..)), "numerics not mmap'd on open");
        for v in [0u32, 1, 42, 250, 499] {
            let lit = Term::Literal(Literal::new_typed_literal(v.to_string(), xsd::INTEGER));
            if let Some(id) = g.id_of(&lit) {
                assert_eq!(g.numeric_value(id), Some(v as f64));
                assert_eq!(g2.numeric_value(id), Some(v as f64), "mmap'd numeric differs for {v}");
            }
        }
        // A non-numeric term (the language-tagged literal) must be None in both.
        if let Some(id) = g.id_of(&Term::Literal(Literal::new_language_tagged_literal("café", "fr").unwrap())) {
            assert_eq!(g2.numeric_value(id), None);
        }

        // The temporal-value cache must round-trip through its memory-mapped form too:
        // every cached cell (instant bits, tz presence, family) identical, and
        // non-temporal terms None in both.
        assert!(dir.join("temporals.bin").exists(), "temporals cache not persisted");
        assert!(matches!(g2.temporals, TempData::Mapped(..)), "temporals not mmap'd on open");
        for i in 1..=g.dict.len() as Id {
            match (g.temporal_value(i), g2.temporal_value(i)) {
                (None, None) => {}
                (Some(a), Some(b)) => {
                    assert_eq!(a.instant.to_bits(), b.instant.to_bits(), "mmap'd instant differs for id {i}");
                    assert_eq!(a.has_tz, b.has_tz);
                    assert_eq!(a.kind, b.kind);
                }
                (a, b) => panic!("temporal cache presence differs for id {i}: {a:?} vs {b:?}"),
            }
        }
        // The dictionary is memory-mapped (zero resident term storage) and lookup still
        // round-trips: every term resolves to the same id and back to the same term.
        assert!(dir.join("dict-terms.bin").exists(), "mmap dict not persisted");
        for s in 0..50u32 {
            let t = Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/n{}", s % 211)));
            assert_eq!(g.id_of(&t), g2.id_of(&t), "mmap dict lookup differs for {t}");
        }
        // Per-predicate stats are persisted (no POS/PSO re-scan on open) and identical.
        assert!(dir.join("predstats.bin").exists(), "pred stats not persisted");
        for p in 0..13u32 {
            let pred = NamedNode::new_unchecked(format!("http://ex/p{p}"));
            if let Some(pid) = g.id_of(&Term::NamedNode(pred)) {
                assert_eq!(g.store.pred_stat(pid), g2.store.pred_stat(pid), "pred_stat differs for p{p}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] Regression for review 1325 (sub-finding 2): a graph directory carrying the
    /// LEGACY single-file `dict.bin` dictionary (saved before the mmap dict format) must still
    /// open via `Graph::open` (falling back from the absent `dict-meta.bin`), with every triple
    /// round-tripping identically.
    #[cfg(feature = "mmap")]
    #[test]
    fn open_falls_back_to_legacy_dict_bin() {
        let mut nt = String::new();
        for i in 0..400u32 {
            nt.push_str(&format!("<http://ex/n{}> <http://ex/p{}> <http://ex/o{}> .\n", i % 97, i % 7, i % 53));
        }
        nt.push_str("<http://ex/n0> <http://ex/name> \"caf\\u00e9\"@fr .\n");
        let g = Graph::load_str(&nt, "ntriples").unwrap();
        // The in-memory build produces an arena dict (base == 0), so `Dict::save` applies.
        let dir = std::env::temp_dir().join(format!("sparq_legacy_dict_test_{}", std::process::id()));
        g.save(&dir).unwrap();
        // Simulate a LEGACY directory: drop every mmap-dict file and write the single-file
        // `dict.bin` instead. The permutation/numerics/temporals files are left untouched.
        for f in ["dict-meta.bin", "dict-terms.bin", "dict-offs.bin", "dict-hash.bin", "dict-hid.bin"] {
            std::fs::remove_file(dir.join(f)).ok();
        }
        g.dict.save(&dir.join("dict.bin")).unwrap();
        assert!(!dir.join("dict-meta.bin").exists());
        assert!(dir.join("dict.bin").exists());
        // `Graph::open` must succeed via the legacy fallback and round-trip every triple.
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g.len(), g2.len());
        assert_eq!(g.dict.len(), g2.dict.len());
        let dump = |gg: &Graph| {
            let scan = gg.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (gg.dict.term(spo[0]).to_string(), gg.dict.term(spo[1]).to_string(), gg.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(dump(&g), dump(&g2), "legacy-dict-reopened store differs");
        for s in 0..50u32 {
            let t = Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/n{}", s % 97)));
            assert_eq!(g.id_of(&t), g2.id_of(&t), "legacy dict lookup differs for {t}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Graph-level COMPRESSED persistence: `save_compressed` → `open` must round-trip the
    /// full graph (terms, numerics, pred stats) identically to the raw path, including a
    /// pending delta-overlay being folded into the compressed permutations on save.
    #[cfg(feature = "mmap")]
    #[test]
    fn compressed_graph_roundtrip() {
        let mut nt = String::new();
        for i in 0..4000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 311,
                i % 11,
                i % 700
            ));
        }
        let mut g = Graph::load_str(&nt, "ntriples").unwrap();
        // A pending overlay (one delete + one insert) must be folded into the compressed save.
        let del = [g.id_of(&Term::NamedNode(NamedNode::new_unchecked("http://ex/n1"))).unwrap(),
                   g.id_of(&Term::NamedNode(NamedNode::new_unchecked("http://ex/p1"))).unwrap(),
                   g.id_of(&Term::Literal(Literal::new_typed_literal("1", xsd::INTEGER))).unwrap()];
        g.store.apply_delta(&[[del[0], del[1], del[0]]], &[del]);
        assert!(g.store.has_overlay());

        let base = std::env::temp_dir().join(format!("sparq_cgraph_{}", std::process::id()));
        let (raw_dir, cmp_dir) = (base.join("raw"), base.join("cmp"));
        g.save(&raw_dir).unwrap();
        g.save_compressed(&cmp_dir).unwrap();

        let a = Graph::open(&raw_dir).unwrap();
        let mut b = Graph::open(&cmp_dir).unwrap();
        let dump = |gg: &Graph| {
            let scan = gg.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (gg.dict.term(spo[0]).to_string(), gg.dict.term(spo[1]).to_string(), gg.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(g.len(), b.len(), "overlay not folded into the compressed save");
        assert_eq!(dump(&a), dump(&b), "compressed graph differs from raw graph");
        // Numerics still resolve through the compressed-opened graph.
        let lit = Term::Literal(Literal::new_typed_literal("42", xsd::INTEGER));
        if let Some(id) = b.id_of(&lit) {
            assert_eq!(b.numeric_value(id), Some(42.0));
        }
        // Load-time decompression: identical content, perms now on the heap.
        let lazy_heap = b.store.heap_bytes();
        b.decompress_indexes();
        assert!(b.store.heap_bytes() > lazy_heap, "decompress_indexes must move perms to the heap");
        assert_eq!(dump(&a), dump(&b), "decompressed graph differs");
        std::fs::remove_dir_all(&base).ok();
    }

    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_matches_in_memory() {
        // External-memory build with a TINY chunk so the triples spill across many runs
        // and exercise the k-way merge + per-permutation re-sort. The on-disk result must
        // be byte-for-byte identical to an in-memory load → save (same dedup, same order).
        let mut nt = String::new();
        for i in 0..5000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 173,
                i % 7,
                i % 400
            ));
        }
        // A duplicate line (must be deduped) + a non-integer literal with a language tag.
        nt.push_str("<http://ex/n0> <http://ex/p0> \"0\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n");
        nt.push_str("<http://ex/n0> <http://ex/name> \"caf\\u00e9\"@fr .\n");

        let base = std::env::temp_dir().join(format!("sparq_ext_{}", std::process::id()));
        let mem_dir = base.join("mem");
        let ext_dir = base.join("ext");

        // In-memory load → save (reference), vs streaming external build (chunk = 256).
        let g = Graph::load_str(&nt, "ntriples").unwrap();
        g.save(&mem_dir).unwrap();
        Graph::build_external(nt.as_bytes(), "ntriples", &ext_dir, 256).unwrap();

        let mem = Graph::open(&mem_dir).unwrap();
        let ext = Graph::open(&ext_dir).unwrap();
        assert_eq!(mem.len(), ext.len(), "triple count differs");
        assert_eq!(mem.dict.len(), ext.dict.len(), "dict size differs");

        // Every BUILT permutation file must be byte-identical (same sort, same dedup).
        for &perm in store::BUILT {
            let f = format!("perm{}.bin", perm as usize);
            let a = std::fs::read(mem_dir.join(&f)).unwrap();
            let b = std::fs::read(ext_dir.join(&f)).unwrap();
            assert_eq!(a, b, "permutation {f} differs between in-memory and external build");
        }

        // And the data round-trips through terms.
        let dump = |gg: &Graph| {
            let scan = gg.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (gg.dict.term(spo[0]).to_string(), gg.dict.term(spo[1]).to_string(), gg.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(dump(&mem), dump(&ext), "external-built store differs");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq) Dumps a whole DATASET (default graph + every named graph) as a
    /// sorted set of `(graph, s, p, o)` string tuples — the lossless per-graph quad set used
    /// to compare two opened stores. The default graph's name is the empty string `""`.
    #[cfg(feature = "mmap")]
    fn dump_quads(g: &Graph) -> Vec<(String, String, String, String)> {
        fn dump_graph(g: &Graph, gname: &str, out: &mut Vec<(String, String, String, String)>) {
            let scan = g.store.scan(&[None, None, None]);
            for r in scan.rows.iter() {
                let spo = scan.to_spo(r);
                out.push((
                    gname.to_string(),
                    g.dict.term(spo[0]).to_string(),
                    g.dict.term(spo[1]).to_string(),
                    g.dict.term(spo[2]).to_string(),
                ));
            }
        }
        let mut v = Vec::new();
        dump_graph(g, "", &mut v);
        for (name, sub) in &g.named {
            dump_graph(sub, &name.to_string(), &mut v);
        }
        v.sort();
        v
    }

    /// [OPUS-4.8] (sq-5atq) An N-Quads dataset with several named graphs builds via the
    /// OUT-OF-CORE quad path (`build_external_quads`) and `open`s LOSSLESSLY — the opened
    /// store's per-graph quad set equals the input quad set. A TINY chunk forces the per-graph
    /// external SPO sort to spill across many runs + k-way merge (the genuine bounded-memory
    /// path), so the corpus is large enough that several graphs each overflow the chunk.
    /// Covers the edge cases the brief names: a graph with ONE quad, a DEFAULT graph mixed
    /// with named graphs in the same stream, and DUPLICATE quads across graphs (the same
    /// triple in two graphs is two distinct dataset quads; a duplicate within one graph
    /// dedups to one).
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_lossless_open() {
        let mut nq = String::new();
        // Three named graphs each large enough to overflow a tiny chunk, plus default-graph
        // quads interleaved in the same stream.
        for i in 0..2000u32 {
            let g = i % 3;
            nq.push_str(&format!(
                "<http://ex/s{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> <http://ex/g{}> .\n",
                i % 137, i % 11, i % 90, g
            ));
            if i % 5 == 0 {
                // Default-graph quad interleaved (no 4th term).
                nq.push_str(&format!(
                    "<http://ex/s{}> <http://ex/d{}> \"{}\" .\n",
                    i % 137, i % 7, i % 90
                ));
            }
        }
        // A graph with exactly ONE quad.
        nq.push_str("<http://ex/only> <http://ex/p> <http://ex/o> <http://ex/single> .\n");
        // A blank-node-subject quad in a named graph (round-trips losslessly per graph).
        nq.push_str("_:b0 <http://ex/p> _:b1 <http://ex/g0> .\n");
        // DUPLICATE quad within one graph (dedups) + the SAME triple in another graph (kept).
        nq.push_str("<http://ex/dup> <http://ex/p> <http://ex/o> <http://ex/g0> .\n");
        nq.push_str("<http://ex/dup> <http://ex/p> <http://ex/o> <http://ex/g0> .\n");
        nq.push_str("<http://ex/dup> <http://ex/p> <http://ex/o> <http://ex/g1> .\n");

        let base = std::env::temp_dir().join(format!("sparq_extq_{}", std::process::id()));
        let ext_dir = base.join("ext");

        // Build out-of-core with a TINY chunk (genuinely exercises the per-graph spill path).
        Graph::build_external_quads(nq.as_bytes(), "nquads", &ext_dir, 64).unwrap();
        let ext = Graph::open(&ext_dir).unwrap();

        // Reference: the in-RAM dataset load (its quad set is the ground truth).
        let mem_g = Graph::load_dataset(&nq, "nquads").unwrap();
        assert_eq!(
            dump_quads(&mem_g),
            dump_quads(&ext),
            "out-of-core quad build is not lossless vs the in-RAM dataset load"
        );
        // The manifest must exist (there ARE named graphs) and the named-graph set must match.
        assert!(ext_dir.join("named.bin").exists(), "named manifest not written by quad build");
        assert_eq!(ext.named.len(), mem_g.named.len(), "named-graph count differs");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq) DIFFERENTIAL: the out-of-core `build_external_quads` opened store
    /// equals the in-RAM dataset load + `save` (`save_named`) opened store — same on-disk
    /// layout family, same per-graph quad set, same named-graph ordering. This pins the
    /// out-of-core path to the existing in-RAM persistence as its reference.
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_differential_vs_save_named() {
        let mut nq = String::new();
        for i in 0..1500u32 {
            nq.push_str(&format!(
                "<http://ex/s{}> <http://ex/p{}> <http://ex/o{}> <http://ex/graph/{}> .\n",
                i % 97, i % 5, i % 60, i % 4
            ));
        }
        nq.push_str("<http://ex/x> <http://ex/y> \"z\"@en .\n"); // default-graph quad
        nq.push_str("<http://ex/lone> <http://ex/p> <http://ex/o> <http://ex/graph/lonely> .\n");

        let base = std::env::temp_dir().join(format!("sparq_extq_diff_{}", std::process::id()));
        let mem_dir = base.join("mem");
        let ext_dir = base.join("ext");

        // In-RAM dataset load → save (the reference on-disk layout), vs out-of-core build.
        let mem_g = Graph::load_dataset(&nq, "nquads").unwrap();
        mem_g.save(&mem_dir).unwrap();
        Graph::build_external_quads(nq.as_bytes(), "nquads", &ext_dir, 128).unwrap();

        let mem = Graph::open(&mem_dir).unwrap();
        let ext = Graph::open(&ext_dir).unwrap();
        assert_eq!(
            dump_quads(&mem),
            dump_quads(&ext),
            "out-of-core quad build differs from in-RAM save_named"
        );
        // Named-graph ordering (manifest order = first-occurrence) must match.
        let mem_names: Vec<String> = mem.named.iter().map(|(n, _)| n.to_string()).collect();
        let ext_names: Vec<String> = ext.named.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(mem_names, ext_names, "named-graph manifest ordering differs");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq) Edge case: a DEFAULT-GRAPH-ONLY N-Quads stream (no named graphs)
    /// must build a clean default-only directory — NO `named/` subtree, NO `named.bin` —
    /// and open losslessly, matching the N-Triples `build_external` default-only layout.
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_default_only_no_manifest() {
        let nq = "<http://ex/a> <http://ex/b> <http://ex/c> .\n\
                  <http://ex/a> <http://ex/b> \"lit\" .\n";
        let base = std::env::temp_dir().join(format!("sparq_extq_def_{}", std::process::id()));
        let ext_dir = base.join("ext");
        Graph::build_external_quads(nq.as_bytes(), "nquads", &ext_dir, 64).unwrap();
        assert!(!ext_dir.join("named.bin").exists(), "default-only quad build must not write a manifest");
        assert!(!ext_dir.join("named").exists(), "default-only quad build must not write a named subtree");
        let ext = Graph::open(&ext_dir).unwrap();
        let mem = Graph::load_dataset(nq, "nquads").unwrap();
        assert_eq!(dump_quads(&mem), dump_quads(&ext), "default-only quad build not lossless");
        assert!(ext.named.is_empty());
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq) Edge case: an EMPTY default graph that still has named graphs —
    /// every quad carries a graph name, so the default graph has zero triples. The default
    /// store in `dir` must be a valid empty store and the named graphs must round-trip.
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_empty_default_graph() {
        let nq = "<http://ex/a> <http://ex/b> <http://ex/c> <http://ex/g1> .\n\
                  <http://ex/d> <http://ex/e> <http://ex/f> <http://ex/g2> .\n";
        let base = std::env::temp_dir().join(format!("sparq_extq_emptydef_{}", std::process::id()));
        let ext_dir = base.join("ext");
        Graph::build_external_quads(nq.as_bytes(), "nquads", &ext_dir, 64).unwrap();
        let ext = Graph::open(&ext_dir).unwrap();
        assert_eq!(ext.len(), 0, "default graph must be empty");
        assert_eq!(ext.named.len(), 2, "two named graphs expected");
        let mem = Graph::load_dataset(nq, "nquads").unwrap();
        assert_eq!(dump_quads(&mem), dump_quads(&ext), "empty-default quad build not lossless");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq) The TriG out-of-core path mirrors N-Quads: a small TriG dataset
    /// builds via `build_external_quads` and opens losslessly vs the in-RAM TriG load. (TriG
    /// reuses the exact same partition-by-graph + per-graph external build as N-Quads — only
    /// the parser differs — so this is a smoke test of the format wiring, not a separate path.)
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_trig_lossless_open() {
        let trig = "@prefix ex: <http://ex/> .\n\
                    ex:s ex:p ex:o .\n\
                    ex:g1 { ex:a ex:b ex:c . ex:a ex:b ex:d . }\n\
                    ex:g2 { ex:x ex:y ex:z . }\n";
        let base = std::env::temp_dir().join(format!("sparq_extq_trig_{}", std::process::id()));
        let ext_dir = base.join("ext");
        Graph::build_external_quads(trig.as_bytes(), "trig", &ext_dir, 64).unwrap();
        let ext = Graph::open(&ext_dir).unwrap();
        let mem = Graph::load_dataset(trig, "trig").unwrap();
        assert_eq!(dump_quads(&mem), dump_quads(&ext), "TriG out-of-core build not lossless");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq, PR #187 review) DATA-LOSS regression: `build_external_quads` must
    /// VALIDATE the `format` BEFORE any destructive filesystem mutation. A caller that passes
    /// an unsupported format must get an `Err` and find any EXISTING dataset at `dir` fully
    /// intact — the function must not have deleted the prior store's named subtree/manifest
    /// (or anything else) before rejecting the format.
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_unsupported_format_preserves_dir() {
        let base = std::env::temp_dir().join(format!("sparq_extq_badfmt_{}", std::process::id()));
        let ext_dir = base.join("ext");

        // First build a real dataset with named graphs (so `named/` + `named.bin` exist).
        let nq = "<http://ex/a> <http://ex/b> <http://ex/c> <http://ex/g1> .\n\
                  <http://ex/d> <http://ex/e> <http://ex/f> <http://ex/g2> .\n\
                  <http://ex/x> <http://ex/y> <http://ex/z> .\n";
        Graph::build_external_quads(nq.as_bytes(), "nquads", &ext_dir, 64).unwrap();
        assert!(ext_dir.join("named.bin").exists(), "precondition: manifest present");
        assert!(ext_dir.join("named").exists(), "precondition: named subtree present");
        let before = dump_quads(&Graph::open(&ext_dir).unwrap());

        // Now call with an UNSUPPORTED format. It must return Err and leave `dir` untouched.
        let err = Graph::build_external_quads(b"whatever".as_slice(), "turtle", &ext_dir, 64)
            .expect_err("unsupported quad format must be rejected");
        assert!(err.contains("unsupported"), "unexpected error: {err}");

        // The prior dataset's on-disk artefacts must STILL be present and still open losslessly.
        assert!(ext_dir.join("named.bin").exists(), "manifest was destroyed before format check");
        assert!(ext_dir.join("named").exists(), "named subtree was destroyed before format check");
        assert!(!ext_dir.join("quads-spill").exists(), "a spill dir leaked from the rejected call");
        let after = dump_quads(&Graph::open(&ext_dir).unwrap());
        assert_eq!(before, after, "existing dataset corrupted by a rejected-format call");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq, PR #187 review) RESOURCE-LEAK regression: an error MID-BUILD (here
    /// a parse error after some quads have already spilled) must leave NO `quads-spill/` behind
    /// — the scope guard cleans the spill directory on every error/early-return path, not only
    /// on success.
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_error_cleans_spill_dir() {
        let base = std::env::temp_dir().join(format!("sparq_extq_leak_{}", std::process::id()));
        let ext_dir = base.join("ext");

        // Valid quads first (forcing spill files to be created across several graphs), then a
        // MALFORMED line that makes the N-Quads parser yield an Err mid-stream.
        let mut nq = String::new();
        for i in 0..10u32 {
            nq.push_str(&format!(
                "<http://ex/s{i}> <http://ex/p> <http://ex/o> <http://ex/g{i}> .\n"
            ));
        }
        nq.push_str("this is not valid n-quads !!!\n");

        let res = Graph::build_external_quads(nq.as_bytes(), "nquads", &ext_dir, 64);
        assert!(res.is_err(), "a malformed quad stream must produce an Err");
        assert!(
            !ext_dir.join("quads-spill").exists(),
            "spill dir leaked after a mid-build parse error"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-5atq, PR #187 review) FD-EXHAUSTION regression: with MANY more named
    /// graphs than the open-writer budget, the partition pass must NOT hold one FD per graph.
    /// We pin the budget to a tiny cap via `SPARQ_QUADS_SPILL_MAX_OPEN` and feed many graphs
    /// with INTERLEAVED quads (so most routes hit an evicted, reopened writer) — the build
    /// must still succeed and round-trip losslessly, proving the bounded-pool reopen-in-append
    /// path routes every quad to the right per-graph spill while capping open FDs.
    #[cfg(feature = "mmap")]
    #[test]
    fn build_external_quads_bounded_open_writers() {
        const N_GRAPHS: u32 = 200;
        // Interleave: round-robin across all graphs many times so writers are evicted and
        // reopened repeatedly (append path), not written once contiguously.
        let mut nq = String::new();
        for round in 0..6u32 {
            for g in 0..N_GRAPHS {
                nq.push_str(&format!(
                    "<http://ex/s{round}> <http://ex/p{}> <http://ex/o{}> <http://ex/g{g}> .\n",
                    g % 3,
                    round
                ));
            }
        }
        // Plus some default-graph quads interleaved (the default slot shares the same pool).
        for round in 0..6u32 {
            nq.push_str(&format!("<http://ex/d{round}> <http://ex/dp> <http://ex/do> .\n"));
        }

        let base = std::env::temp_dir().join(format!("sparq_extq_fd_{}", std::process::id()));
        let ext_dir = base.join("ext");

        // Force a tiny open-writer budget (4) although there are 200 graphs.
        // SAFETY: single-threaded test; we restore/remove the var before returning.
        unsafe { std::env::set_var("SPARQ_QUADS_SPILL_MAX_OPEN", "4") };
        let build = Graph::build_external_quads(nq.as_bytes(), "nquads", &ext_dir, 64);
        // SAFETY: single-threaded test; removes the var set above before returning so the
        // process env is left clean. Edition-2024 made `remove_var` `unsafe`. [OPUS-4.8 sq-8wbn]
        unsafe { std::env::remove_var("SPARQ_QUADS_SPILL_MAX_OPEN") };
        build.expect("bounded-writer-pool build must succeed with a tiny FD budget");

        let ext = Graph::open(&ext_dir).unwrap();
        let mem = Graph::load_dataset(&nq, "nquads").unwrap();
        assert_eq!(
            dump_quads(&mem),
            dump_quads(&ext),
            "bounded-writer-pool build not lossless (a reopen must append, not truncate)"
        );
        assert_eq!(ext.named.len(), N_GRAPHS as usize, "all named graphs must be present");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] The SPILLED external build (`dict-spill`) must produce a store that is
    /// BYTE-IDENTICAL to the in-RAM sharded path's: same dedup, same ids, same on-disk
    /// dictionary + permutation + numeric/temporal cache files. This is `dictspill.rs`'s
    /// core correctness contract (research/external-dictionary.md). Driven with a TINY
    /// memory budget so the per-shard dedup caches overflow and get cleared at batch
    /// boundaries (the EPOCH path) and the external sorts spill across many runs — the
    /// previously-uncovered `intern_batch`/`consolidate`/`remap_staged`/`ShardWindow`
    /// pipeline. The dataset deliberately mixes inline integers (passthrough), repeated
    /// IRIs (prefix factoring + dedup), language-tagged + datatyped literals, a numeric
    /// literal (numerics.bin), an xsd:dateTime (temporals.bin), and blank nodes.
    #[cfg(feature = "dict-spill")]
    #[test]
    fn dict_spill_build_byte_identical_to_sharded() {
        use store::BUILT;
        let mut nt = String::new();
        for i in 0..6000u32 {
            nt.push_str(&format!(
                "<http://ex/subj/{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 211,
                i % 13,
                i % 500
            ));
        }
        // Non-inline objects exercising every dictionary record shape.
        nt.push_str("<http://ex/subj/0> <http://ex/name> \"caf\\u00e9\"@fr .\n");
        nt.push_str("<http://ex/subj/0> <http://ex/name> \"plain string\" .\n");
        nt.push_str("<http://ex/subj/1> <http://ex/score> \"3.14\"^^<http://www.w3.org/2001/XMLSchema#double> .\n");
        nt.push_str("<http://ex/subj/2> <http://ex/when> \"2021-03-04T05:06:07Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n");
        nt.push_str("_:b0 <http://ex/p0> _:b1 .\n");
        // A duplicate (must dedup to one triple) and a repeated term across batches.
        nt.push_str("<http://ex/subj/0> <http://ex/name> \"plain string\" .\n");

        let base = std::env::temp_dir().join(format!("sparq_spill_{}", std::process::id()));
        let sharded_dir = base.join("sharded");
        let spill_dir = base.join("spill");

        // Reference: the default sharded in-RAM external build.
        Graph::build_external_opts(nt.as_bytes(), "ntriples", &sharded_dir, 256, true).unwrap();

        // SPILLED build with a tiny budget (forces cache eviction/epochs + many sort runs)
        // — drive build_external_spill DIRECTLY with an explicit SpillConfig (no env-var
        // race with parallel tests). disk_floor 0 so it never aborts in CI sandboxes.
        let cfg = dictspill::SpillConfig { mem_budget: 64 << 10, disk_floor: 0 };
        Graph::build_external_spill(nt.as_bytes(), "ntriples", &spill_dir, 256, &cfg).unwrap();

        let shg = Graph::open(&sharded_dir).unwrap();
        let spg = Graph::open(&spill_dir).unwrap();
        assert_eq!(shg.len(), spg.len(), "triple count differs (spill vs sharded)");
        assert_eq!(shg.dict.len(), spg.dict.len(), "dict size differs (spill vs sharded)");

        // Every on-disk file the spill path streams must be byte-identical to the sharded
        // path's — the design's central claim.
        let files = [
            "dict-meta.bin", "dict-terms.bin", "dict-offs.bin",
            "dict-hash.bin", "dict-hid.bin", "numerics.bin", "temporals.bin",
        ];
        for f in files {
            let a = std::fs::read(sharded_dir.join(f))
                .unwrap_or_else(|e| panic!("sharded {f}: {e}"));
            let b = std::fs::read(spill_dir.join(f))
                .unwrap_or_else(|e| panic!("spill {f}: {e}"));
            assert_eq!(a, b, "dictionary file {f} differs between spill and sharded build");
        }
        for &perm in BUILT {
            let f = format!("perm{}.bin", perm as usize);
            let a = std::fs::read(sharded_dir.join(&f)).unwrap();
            let b = std::fs::read(spill_dir.join(&f)).unwrap();
            assert_eq!(a, b, "permutation {f} differs between spill and sharded build");
        }

        // And the materialized triples round-trip identically through terms.
        let dump = |gg: &Graph| {
            let scan = gg.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (gg.dict.term(spo[0]).to_string(), gg.dict.term(spo[1]).to_string(), gg.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(dump(&shg), dump(&spg), "spill-built store differs from sharded");
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] The spilled external build still supports its mmap'd read-back path, and
    /// it rejects non-N-Triples formats with a clear error (the documented restriction).
    /// [OPUS-4.8] (sq-jvbr) It now ACCEPTS RDF 1.2 triple terms (previously rejected): the
    /// `SpillInterner` interns them structurally into an in-RAM triple-term arena finalised
    /// after the leaf consolidation (`finalize_triple_terms`), so a triple-term document
    /// builds and opens. Full differential coverage vs the sharded/serial/in-memory paths is
    /// in `dict_spill_triple_terms_match_in_memory` below; this just checks the smoke path.
    #[cfg(feature = "dict-spill")]
    #[test]
    fn dict_spill_rejects_non_ntriples_and_opens_mmap() {
        let dir = std::env::temp_dir().join(format!("sparq_spill_fmt_{}", std::process::id()));
        let cfg = dictspill::SpillConfig { mem_budget: 1 << 20, disk_floor: 0 };
        let err = Graph::build_external_spill(b"@prefix : <x> .".as_slice(), "turtle", &dir, 256, &cfg)
            .expect_err("spill build must reject non-ntriples");
        assert!(err.contains("N-Triples"), "error must name the restriction: {err}");

        // RDF 1.2 triple terms now build through the dict-spill path (sq-jvbr) and open.
        let tt = "<http://ex/r1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> \
                  <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> .\n";
        let tt_dir = dir.join("tt");
        Graph::build_external_spill(tt.as_bytes(), "ntriples", &tt_dir, 64, &cfg)
            .expect("dict-spill build must now accept triple terms (sq-jvbr)");
        let tg = Graph::open(&tt_dir).unwrap();
        assert_eq!(tg.len(), 1, "the single reifier statement is materialised");
        let ttobj = Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::NamedNode::new("http://ex/a").unwrap(),
            oxrdf::NamedNode::new("http://ex/b").unwrap(),
            Term::NamedNode(oxrdf::NamedNode::new("http://ex/c").unwrap()),
        )));
        assert!(tg.id_of(&ttobj).is_some(), "the triple term `<<( a b c )>>` must be a first-class dict term");

        let nt = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                  <http://ex/a> <http://ex/p> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n";
        let ok_dir = dir.join("ok");
        Graph::build_external_spill(nt.as_bytes(), "ntriples", &ok_dir, 64, &cfg).unwrap();
        // `Graph::open` mmaps the `dict-meta.bin` dictionary the spill build streams.
        let g = Graph::open(&ok_dir).unwrap();
        assert_eq!(g.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-jvbr) The DICT-SPILL external builder now interns RDF 1.2 triple terms
    /// `<<( … )>>` structurally: leaf components spill/dedup as usual, while triple-term
    /// OCCURRENCES go to an in-RAM arena (rare reification metadata, like the sharded path's
    /// serial second pass) and are finalised — content-addressed by their components' FINAL
    /// ids, deduped, assigned ids AFTER every leaf — in `finalize_triple_terms`. The built
    /// store must round-trip identically (term-level, order-independent) to the in-memory
    /// loader and the serial/sharded external builds. A tiny memory budget forces cache
    /// eviction/epochs + many sort runs, exercising the staged-id remap windows under spill.
    ///
    /// Coverage (mirrors the sharded differential test):
    /// - a triple term shared by TWO statements (must dedup to ONE id);
    /// - components in distinct namespaces (route to different leaf shards);
    /// - a leaf SHARED between a plain triple and a triple-term component;
    /// - a NESTED triple term (a triple term in another's object slot);
    /// - a blank-node component;
    /// - bulk rows so the document spans multiple parse blocks AND triggers epoch resets.
    #[cfg(feature = "dict-spill")]
    #[test]
    fn dict_spill_triple_terms_match_in_memory() {
        use store::BUILT;
        let mut nt = String::new();
        for i in 0..3000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 211,
                i % 11,
                i % 500
            ));
            if i % 500 == 0 {
                // Shared triple term across two statements; cross-namespace components.
                nt.push_str("<http://ex/r1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <http://alpha.example/alice> <http://beta.example/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n");
                nt.push_str("<http://ex/r1b> <http://ex/sameAs> <<( <http://alpha.example/alice> <http://beta.example/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n");
            }
        }
        // A leaf shared between a plain triple and a triple-term component.
        nt.push_str("<http://ex/knows> <http://ex/about> <http://alpha.example/alice> .\n");
        // Blank-node component.
        nt.push_str("<http://ex/r2> <http://ex/about> <<( _:b0 <http://ex/p> \"v\" )>> .\n");
        // Nested triple term.
        nt.push_str("<http://ex/r3> <http://ex/nested> <<( <http://gamma.example/x> <http://delta.example/q> <<( <http://eps.example/a> <http://zeta.example/b> <http://eta.example/c> )>> )>> .\n");

        let mem = Graph::load_str(&nt, "ntriples").expect("in-memory loader must accept triple terms");

        let base = std::env::temp_dir().join(format!("sparq_spill_tt_{}", std::process::id()));
        let spill_dir = base.join("spill");
        // Tiny budget -> epoch resets + many sort runs (exercises the remap windows).
        let cfg = dictspill::SpillConfig { mem_budget: 64 << 10, disk_floor: 0 };
        Graph::build_external_spill(nt.as_bytes(), "ntriples", &spill_dir, 256, &cfg)
            .expect("dict-spill build must accept triple terms (sq-jvbr)");
        let sp = Graph::open(&spill_dir).unwrap();
        assert_eq!(sp.len(), mem.len(), "spill triple count differs from in-memory");
        assert_eq!(sp.dict.len(), mem.dict.len(), "spill dict size differs from in-memory");

        let dump = |g: &Graph| {
            let scan = g.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (g.dict.term(spo[0]).to_string(), g.dict.term(spo[1]).to_string(), g.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(dump(&sp), dump(&mem), "spill-built store differs from in-memory (triple terms)");

        // The shared triple term must be ONE dict entry referenced by both reifiers.
        let tt = Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::NamedNode::new("http://alpha.example/alice").unwrap(),
            oxrdf::NamedNode::new("http://beta.example/age").unwrap(),
            Term::Literal(Literal::new_typed_literal("30", xsd::INTEGER)),
        )));
        let tt_id = sp.id_of(&tt).expect("the shared triple term must be in the spill dict");
        let r1 = sp.id_of(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/r1").unwrap())).unwrap();
        let r1b = sp.id_of(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/r1b").unwrap())).unwrap();
        let obj_of = |s: Id| {
            let scan = sp.store.scan(&[Some(s), None, None]);
            assert_eq!(scan.rows.len(), 1, "reifier must have exactly one statement");
            scan.to_spo(&scan.rows[0])[2]
        };
        assert_eq!(obj_of(r1), tt_id, "<r1>'s object is the shared triple term");
        assert_eq!(obj_of(r1b), tt_id, "<r1b>'s object is the SAME shared triple-term id");

        // Sanity: the triple-term-free permutation/dict bytes are still produced (no perm
        // file is empty), proving the staged-triple remap fed every triple through.
        for &perm in BUILT {
            let f = spill_dir.join(format!("perm{}.bin", perm as usize));
            assert!(std::fs::metadata(&f).map(|m| m.len() > 0).unwrap_or(false), "perm {f:?} must be non-empty");
        }
        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-t3rt) The SHARDED external builder now handles RDF 1.2 triple terms
    /// `<<( … )>>`: it interns them structurally into a dedicated triple shard
    /// (`ShardedDict::intern_partials`), preserving the triple-term↔id bijection ACROSS
    /// shards (a triple term's components are hash-routed to leaf shards, while the triple
    /// itself is content-addressed by its components' resolved ids and appended after every
    /// leaf). The result must round-trip identically to the SERIAL external path and the
    /// in-memory loader — both of which already intern triple terms structurally via
    /// `merge_remap`. The differential test below mirrors the N-Quads loader's style:
    /// compare every path's round-tripped triples (term-level, order-independent).
    ///
    /// Coverage of the cross-shard sharing the bead calls out:
    /// - a triple term whose three components hash-route to DIFFERENT leaf shards;
    /// - the SAME triple term referenced by two statements (must dedup to ONE triple id);
    /// - a leaf SHARED between a plain triple and a triple-term component;
    /// - NESTED triple terms (a triple term inside a triple term's object);
    /// - a blank-node component;
    /// - enough bulk rows that the input spans multiple parse blocks/chunks (cross-chunk
    ///   partial merge), with the reification metadata interleaved among them.
    ///
    /// (The dict-spill path now also supports triple terms — sq-jvbr; see
    /// `dict_spill_triple_terms_match_in_memory`.)
    #[cfg(all(feature = "mmap", feature = "parallel"))]
    #[test]
    fn external_build_triple_terms_sharded_matches_serial_and_memory() {
        // Bulk rows so the document spans multiple parse blocks/chunks; the reification
        // metadata (triple terms) is interleaved so triple terms occur in several partials.
        let mut nt = String::new();
        for i in 0..3000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 211,
                i % 11,
                i % 500
            ));
            // Sprinkle reification metadata through the stream.
            if i % 500 == 0 {
                // A triple term shared by TWO statements (must dedup to one triple id), with
                // components in distinct namespaces (so they hash to different leaf shards).
                nt.push_str("<http://ex/r1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <http://alpha.example/alice> <http://beta.example/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n");
                nt.push_str("<http://ex/r1b> <http://ex/sameAs> <<( <http://alpha.example/alice> <http://beta.example/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n");
            }
        }
        // A leaf SHARED between a plain triple and a triple-term component (the object
        // `<http://alpha.example/alice>` is both a plain object below and a TT subject above).
        nt.push_str("<http://ex/knows> <http://ex/about> <http://alpha.example/alice> .\n");
        // A blank-node component inside a triple term.
        nt.push_str("<http://ex/r2> <http://ex/about> <<( _:b0 <http://ex/p> \"v\" )>> .\n");
        // NESTED triple terms (a TT in the object slot of a TT), distinct shards per leaf.
        nt.push_str("<http://ex/r3> <http://ex/nested> <<( <http://gamma.example/x> <http://delta.example/q> <<( <http://eps.example/a> <http://zeta.example/b> <http://eta.example/c> )>> )>> .\n");

        // Reference paths: in-memory load (serial `merge_remap` fallback for triple terms)
        // and the SERIAL external build (sharded = false) — both intern triple terms
        // structurally and are byte-compatible with each other.
        let mem = Graph::load_str(&nt, "ntriples").expect("in-memory loader must accept triple terms");

        let base = std::env::temp_dir().join(format!("sparq_ext_tt_{}", std::process::id()));
        let ser_dir = base.join("serial");
        let sh_dir = base.join("sharded");
        Graph::build_external_opts(nt.as_bytes(), "ntriples", &ser_dir, 256, false)
            .expect("serial external build must accept triple terms");
        // The PATH UNDER TEST: the sharded external builder must now ACCEPT triple terms.
        Graph::build_external_opts(nt.as_bytes(), "ntriples", &sh_dir, 256, true)
            .expect("sharded external build must now accept triple terms (sq-t3rt)");

        let ser = Graph::open(&ser_dir).unwrap();
        let sh = Graph::open(&sh_dir).unwrap();
        assert_eq!(sh.len(), mem.len(), "sharded external triple count differs from in-memory");
        assert_eq!(sh.len(), ser.len(), "sharded external triple count differs from serial");
        assert_eq!(sh.dict.len(), mem.dict.len(), "sharded external dict size differs from in-memory");
        assert_eq!(sh.dict.len(), ser.dict.len(), "sharded external dict size differs from serial");

        // Round-trip every stored triple back to its terms (recursively expanding triple
        // terms via `Dict::term`), sort, and compare — id-assignment-order independent. This
        // proves the cross-shard triple-term bijection: the same triple term shared by two
        // statements resolves to ONE term, and nested/blank/cross-namespace components all
        // reconstruct correctly.
        let dump = |g: &Graph| {
            let scan = g.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (g.dict.term(spo[0]).to_string(), g.dict.term(spo[1]).to_string(), g.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        let d_mem = dump(&mem);
        let d_ser = dump(&ser);
        let d_sh = dump(&sh);
        assert_eq!(d_sh, d_mem, "sharded external build differs from in-memory load (triple terms)");
        assert_eq!(d_sh, d_ser, "sharded external build differs from serial external (triple terms)");

        // Cross-shard dedup is OBSERVABLE: the triple term shared by <r1>/<r1b> must be ONE
        // dictionary entry referenced by both statements, i.e. their objects share an id.
        let tt = Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::NamedNode::new("http://alpha.example/alice").unwrap(),
            oxrdf::NamedNode::new("http://beta.example/age").unwrap(),
            Term::Literal(Literal::new_typed_literal("30", xsd::INTEGER)),
        )));
        let tt_id = sh.id_of(&tt).expect("the shared triple term must be in the sharded dict");
        let r1 = sh.id_of(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/r1").unwrap())).unwrap();
        let r1b = sh.id_of(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/r1b").unwrap())).unwrap();
        let n_r1 = sh.store.scan(&[Some(r1), None, None]).rows.len();
        let n_r1b = sh.store.scan(&[Some(r1b), None, None]).rows.len();
        assert_eq!(n_r1, 1, "<r1> must have exactly one statement");
        assert_eq!(n_r1b, 1, "<r1b> must have exactly one statement");
        // Both reifier statements point at the SAME triple-term id (the bijection held).
        let obj_of = |s: Id| {
            let scan = sh.store.scan(&[Some(s), None, None]);
            scan.to_spo(&scan.rows[0])[2]
        };
        assert_eq!(obj_of(r1), tt_id, "<r1>'s object is the shared triple term");
        assert_eq!(obj_of(r1b), tt_id, "<r1b>'s object is the SAME shared triple term id");

        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-t3rt) The PIPELINED sharded IN-RAM loader (`load_reader_parallel` →
    /// `load_ntriples_pipelined` → `sharded_extend` → `ShardedDict::intern_partials`) shares
    /// the same consolidation as the sharded external builder, so the same fix lets it
    /// handle triple terms too (it would previously have PANICKED in a shard worker via
    /// `intern_parts`). Verify it round-trips identically to the serial `load_reader`.
    #[cfg(feature = "parallel")]
    #[test]
    fn load_reader_parallel_handles_triple_terms() {
        let mut nt = String::new();
        for i in 0..1500u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 97,
                i % 7,
                i % 300
            ));
            if i % 300 == 0 {
                nt.push_str("<http://ex/r1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <http://alpha.example/s> <http://beta.example/p> <<( <http://gamma.example/a> <http://delta.example/b> \"x\" )>> )>> .\n");
            }
        }
        let par = Graph::load_reader_parallel(nt.as_bytes(), "ntriples").unwrap();
        let seq = Graph::load_reader(nt.as_bytes(), "ntriples").unwrap();
        assert_eq!(par.len(), seq.len());
        assert_eq!(par.dict.len(), seq.dict.len());
        let dump = |g: &Graph| {
            let scan = g.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (g.dict.term(spo[0]).to_string(), g.dict.term(spo[1]).to_string(), g.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(dump(&par), dump(&seq));
    }

    #[test]
    fn ntriples_parallel_matches_sequential() {
        // >4KB so the input spans multiple parallel chunks; subjects/predicates repeat
        // across chunks (exercising the partial-dict merge) and the objects are inline
        // integers (exercising the inline-id passthrough in the remap).
        let mut nt = String::new();
        for i in 0..2000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 137,
                i % 11,
                i % 500
            ));
        }
        // The byte-level parser's risky paths, cross-checked against oxttl: escapes
        // (quote / backslash / newline / \\u), language tags, typed + simple literals,
        // a comment, and a different IRI namespace.
        nt.push_str("# a comment line\n");
        nt.push_str("<http://ex/s> <http://other.org/p> \"a \\\"q\\\" b\\nc \\\\ d \\u00e9\" .\n");
        nt.push_str("<http://ex/s> <http://ex/name> \"caf\\u00e9\"@fr .\n");
        nt.push_str("<http://ex/s> <http://ex/v> \"1.5\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n");
        nt.push_str("<http://ex/s> <http://ex/plain> \"just a string\" .\n");
        nt.push_str("<http://ex/s> <http://ex/big> \"\\U0001F600 grin\" .\n");
        // [OPUS-4.8] (sq-hxgb) RDF 1.2 triple-term objects `<<( s p o )>>` (object-only),
        // including a NESTED one and a blank-node component — cross-checked against oxttl's
        // `NTriplesParser` (the sequential `load_reader` path, which already accepts them via
        // the `rdf-12` feature). This is the byte-parser-vs-oxttl oracle for triple terms.
        nt.push_str("<http://ex/r1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <http://ex/alice> <http://ex/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n");
        nt.push_str("<http://ex/r2> <http://ex/about> <<( _:b0 <http://ex/p> \"v\" )>> .\n");
        nt.push_str("<http://ex/r3> <http://ex/nested> <<( <http://ex/x> <http://ex/q> <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> )>> .\n");
        let par = Graph::load_str(&nt, "ntriples").unwrap(); // parallel (when feature on)
        let seq = Graph::load_reader(nt.as_bytes(), "ntriples").unwrap(); // sequential
        assert_eq!(par.len(), seq.len());
        assert_eq!(par.dict.len(), seq.dict.len());
        // Full structural equality independent of id-assignment order: map every stored
        // triple back to its terms, sort, compare.
        let dump = |g: &Graph| {
            let scan = g.store.scan(&[None, None, None]);
            let mut v: Vec<(String, String, String)> = scan
                .rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    (g.dict.term(spo[0]).to_string(), g.dict.term(spo[1]).to_string(), g.dict.term(spo[2]).to_string())
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(dump(&par), dump(&seq));
    }

    /// [OPUS-4.8] (sq-87bq) The NON-streaming in-memory merge (`merge_partials`, behind
    /// `parse_ntriples_parallel` / the per-graph N-Quads merge) must consolidate RDF 1.2
    /// triple terms through the SHARDED path — its earlier `has_triple_terms ⇒ serial`
    /// guard was stale (the sharded interner gained structural triple-term support in
    /// sq-t3rt). This test FORCES the sharded branch (≥2 threads in a scoped rayon pool) on
    /// triple-term partials and pins the result to the serial `merge_remap` reference, so the
    /// fix is covered deterministically regardless of the ambient thread count.
    #[cfg(feature = "parallel")]
    #[test]
    fn merge_partials_sharded_consolidates_triple_terms() {
        // Several blocks' worth of triples with interleaved reification metadata, components
        // in distinct namespaces (different leaf shards), a shared triple term (cross-shard
        // dedup), a blank-node component, and a nested triple term.
        let mut nt = String::new();
        for i in 0..4000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 211,
                i % 13,
                i % 700
            ));
            if i % 700 == 0 {
                // Same triple term referenced by two statements -> must dedup to one id.
                nt.push_str("<http://ex/r1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <http://alpha.example/alice> <http://beta.example/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n");
                nt.push_str("<http://ex/r1b> <http://ex/sameAs> <<( <http://alpha.example/alice> <http://beta.example/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n");
            }
        }
        nt.push_str("<http://ex/r2> <http://ex/about> <<( _:b0 <http://ex/p> \"v\" )>> .\n");
        nt.push_str("<http://ex/r3> <http://ex/nested> <<( <http://gamma.example/x> <http://delta.example/q> <<( <http://eps.example/a> <http://zeta.example/b> <http://eta.example/c> )>> )>> .\n");
        let bytes = nt.as_bytes();

        // Round-trip a (dict, triples) pair to its sorted term-set (recursively expanding
        // triple terms via `Dict::term`), so the comparison is id-assignment-order
        // independent.
        let dump = |dict: &Dict, triples: &[[Id; 3]]| {
            let mut v: Vec<(String, String, String)> = triples
                .iter()
                .map(|&[s, p, o]| (dict.term(s).to_string(), dict.term(p).to_string(), dict.term(o).to_string()))
                .collect();
            v.sort();
            v
        };

        // Reference: serial `merge_remap` consolidation (single thread -> serial branch).
        let serial = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| merge_partials(parse_block(bytes).unwrap()));
        // Under test: the SHARDED branch (>=2 threads).
        let sharded = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(|| merge_partials(parse_block(bytes).unwrap()));

        assert_eq!(sharded.1.len(), serial.1.len(), "sharded triple count differs from serial");
        assert_eq!(sharded.0.len(), serial.0.len(), "sharded dict size differs from serial");
        assert_eq!(
            dump(&sharded.0, &sharded.1),
            dump(&serial.0, &serial.1),
            "sharded merge_partials differs from serial reference on triple-term input"
        );

        // Cross-shard dedup is OBSERVABLE: the triple term shared by <r1>/<r1b> is ONE dict
        // entry, so both statements' objects carry the same id.
        let tt = Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::NamedNode::new("http://alpha.example/alice").unwrap(),
            oxrdf::NamedNode::new("http://beta.example/age").unwrap(),
            Term::Literal(Literal::new_typed_literal("30", xsd::INTEGER)),
        )));
        let tt_id = sharded.0.lookup(&tt);
        assert!(tt_id != 0, "the shared triple term must be in the sharded dict");
        let r1 = sharded.0.lookup(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/r1").unwrap()));
        let r1b = sharded.0.lookup(&Term::NamedNode(oxrdf::NamedNode::new("http://ex/r1b").unwrap()));
        let obj_of = |subj: Id| sharded.1.iter().find(|t| t[0] == subj).map(|t| t[2]);
        assert_eq!(obj_of(r1), Some(tt_id), "<r1>'s object is the shared triple term");
        assert_eq!(obj_of(r1b), Some(tt_id), "<r1b>'s object is the SAME shared triple-term id");
    }

    /// [OPUS-4.8] (sq-7d3dj.2) End-to-end proof that pre-sizing the per-chunk partial `Dict` on
    /// the parallel ingest path is a PURE capacity hint. Consolidating partials whose dicts were
    /// reserved with `Dict::with_capacity((e-s)/AVG_NT_LINE_BYTES)` (the change under test) vs
    /// partials built from `Dict::new()` (the pre-change baseline) must yield: byte-identical
    /// id-triples INCLUDING order, an identical term↔id bijection, AND an identical final
    /// `dict.heap_bytes()`. The last assertion is the ratchet guard: `dict_bytes_per_term` /
    /// `store_bytes_per_triple` count `capacity()`, and this pins that a partial's reservation
    /// never leaks into the final dict (`merge_partials` builds it independently), so the
    /// deterministic byte ratchets are provably neutral.
    #[cfg(feature = "parallel")]
    #[test]
    fn presizing_is_pure_capacity_hint() {
        // Multi-block corpus: repeated predicate/type IRIs (heavy dict reuse), distinct
        // subjects/objects, typed + language literals, blank nodes, and a nested triple term.
        let mut nt = String::new();
        for i in 0..2000u32 {
            nt.push_str(&format!("<http://ex/n{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Person> .\n", i));
            nt.push_str(&format!("<http://ex/n{}> <http://ex/name> \"name{}\"@en .\n", i, i));
            nt.push_str(&format!("<http://ex/n{}> <http://ex/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", i, i % 91));
            nt.push_str(&format!("_:b{} <http://ex/reifies> <<( <http://ex/n{}> <http://ex/knows> <http://ex/n{}> )>> .\n", i, i, (i + 1) % 2000));
        }
        let bytes = nt.as_bytes();

        // Split at newline boundaries into several chunks (the same shape `parse_block` produces),
        // then build TWO partial sets differing ONLY in the partial dict's reserved capacity.
        // `parse_chunk` is identical in both, so the sole variable is the partial-dict capacity.
        let bounds = newline_chunk_bounds(bytes, 8);
        let build = |presize: bool| -> Vec<(Dict, Vec<[Id; 3]>)> {
            bounds
                .iter()
                .map(|&(s, e)| {
                    let mut d = if presize {
                        Dict::with_capacity((e - s) / nt::AVG_NT_LINE_BYTES)
                    } else {
                        Dict::new()
                    };
                    let t = nt::parse_chunk(&bytes[s..e], &mut d).unwrap();
                    (d, t)
                })
                .collect()
        };

        // Consolidate BOTH through the identical merge under one fixed thread pool, so the two
        // runs take the same merge branch and differ only by the isolated capacity variable.
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        let (dict_pre, tri_pre) = pool.install(|| merge_partials(build(true)));
        let (dict_plain, tri_plain) = pool.install(|| merge_partials(build(false)));

        // (1) Identical id-triples INCLUDING order.
        assert_eq!(tri_pre, tri_plain, "pre-sizing changed the merged id-triples or their order");
        // (2) Identical term set + bijection.
        assert_eq!(dict_pre.len(), dict_plain.len(), "pre-sizing changed the distinct-term count");
        for id in 1..=dict_pre.len() as Id {
            assert_eq!(dict_pre.term(id), dict_plain.term(id), "pre-sizing changed the term for id {}", id);
        }
        // (3) Identical FINAL dict footprint — `heap_bytes` counts `capacity()`, so this pins
        // that the partial reservation does NOT reach the ratcheted final dict.
        assert_eq!(
            dict_pre.heap_bytes(),
            dict_plain.heap_bytes(),
            "pre-sizing changed the final dict heap bytes (the dict_bytes_per_term ratchet would move)"
        );
    }

    /// The streaming pipelined loader must be byte-exact against the serial loader for
    /// readers with SHORT reads (a gzip/zstd decompressor returns a fraction of the block
    /// per `read()` — the measured streaming-ingest defect), read boundaries landing
    /// mid-line, an EOF mid-line (final line without a trailing newline), and empty input.
    #[cfg(feature = "parallel")]
    #[test]
    fn load_reader_parallel_short_reads_match_sequential() {
        /// Returns at most `max` bytes per `read()` call.
        struct ShortReader<'a> {
            data: &'a [u8],
            pos: usize,
            max: usize,
        }
        impl std::io::Read for ShortReader<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = (self.data.len() - self.pos).min(self.max).min(buf.len());
                buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            }
        }
        let mut nt = String::new();
        for i in 0..3000u32 {
            nt.push_str(&format!(
                "<http://ex/n{}> <http://ex/p{}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                i % 211,
                i % 13,
                i % 700
            ));
        }
        nt.push_str("<http://ex/last> <http://ex/p0> \"no trailing newline\" ."); // EOF mid-line
        let seq = Graph::load_reader(nt.as_bytes(), "ntriples").unwrap();
        // 7 B forces hundreds of reads per line; the others land boundaries mid-line.
        for max in [7usize, 1024, 1 << 16] {
            let par = Graph::load_reader_parallel(ShortReader { data: nt.as_bytes(), pos: 0, max }, "ntriples").unwrap();
            assert_eq!(par.len(), seq.len(), "triple count differs at max={max}");
            assert_eq!(par.dict.len(), seq.dict.len(), "dict size differs at max={max}");
            assert_eq!(dump_terms(&par), dump_terms(&seq), "stored triples differ at max={max}");
        }
        // MULTI-BLOCK: a small block size (vs the production 32 MiB) makes the same
        // document span ~60 blocks, with triple lines (each ~70-100 B, never a multiple
        // of 4096) straddling every block boundary and partial lines carried across
        // pipelined rounds — and the per-block dict merges must still agree.
        for (block, max) in [(4096usize, 997usize), (4096, 1 << 16), (8192, 7)] {
            let par = Graph::load_ntriples_pipelined(ShortReader { data: nt.as_bytes(), pos: 0, max }, block).unwrap();
            assert_eq!(par.len(), seq.len(), "triple count differs at block={block} max={max}");
            assert_eq!(par.dict.len(), seq.dict.len(), "dict size differs at block={block} max={max}");
            assert_eq!(dump_terms(&par), dump_terms(&seq), "stored triples differ at block={block} max={max}");
        }
        // A single line LONGER than the block size (carry outgrows the block).
        let long = format!("<http://ex/s> <http://ex/p> \"{}\" .\n<http://ex/s2> <http://ex/p> \"x\" .", "y".repeat(20_000));
        let lseq = Graph::load_reader(long.as_bytes(), "ntriples").unwrap();
        let lpar = Graph::load_ntriples_pipelined(ShortReader { data: long.as_bytes(), pos: 0, max: 333 }, 4096).unwrap();
        assert_eq!(lpar.len(), lseq.len());
        assert_eq!(dump_terms(&lpar), dump_terms(&lseq));
        // Empty input and input with no final newline at all.
        let empty = Graph::load_reader_parallel(ShortReader { data: b"", pos: 0, max: 7 }, "ntriples").unwrap();
        assert_eq!(empty.len(), 0);
        let one = Graph::load_reader_parallel(
            ShortReader { data: b"<http://ex/s> <http://ex/p> <http://ex/o> .", pos: 0, max: 3 },
            "ntriples",
        )
        .unwrap();
        assert_eq!(one.len(), 1);
        // A malformed document must surface the parse error, not hang the pipeline.
        let bad = Graph::load_reader_parallel(ShortReader { data: b"not ntriples\n", pos: 0, max: 5 }, "ntriples");
        assert!(bad.is_err());
    }

    #[test]
    fn load_and_scan() {
        let ttl = "@prefix ex: <http://ex/> . ex:a ex:p ex:b, ex:c . ex:d ex:p ex:b .";
        let g = Graph::load_str(ttl, "turtle").unwrap();
        assert_eq!(g.len(), 3);

        let p = NamedNode::new("http://ex/p").unwrap();
        let b = Term::NamedNode(NamedNode::new("http://ex/b").unwrap());
        // ?s ex:p ex:b  -> ex:a and ex:d
        let pat = g.pattern(None, Some(&p), Some(&b)).unwrap();
        let scan = g.store.scan(&pat);
        assert_eq!(scan.rows.len(), 2);
    }

    /// [OPUS-4.8] sq-bif — `is_integer_datatype` decides whether a datatype's values are
    /// exact integers (an `xsd:integer` subtype), which gates the exact-lexical disambiguation
    /// path. It must accept `xsd:integer` and every derived integer subtype, but REJECT the
    /// other numeric datatypes (`decimal` is exact but not an integer; `double`/`float` ARE
    /// their f64) and every non-numeric datatype. Previously it had no direct test, so a typo
    /// dropping a subtype — or accidentally admitting `decimal` — would go unnoticed.
    #[test]
    fn is_integer_datatype_accepts_integer_subtypes_only() {
        // Every integer family member is accepted.
        for dt in [
            xsd::INTEGER,
            xsd::LONG,
            xsd::INT,
            xsd::SHORT,
            xsd::BYTE,
            xsd::NON_NEGATIVE_INTEGER,
            xsd::POSITIVE_INTEGER,
            xsd::NON_POSITIVE_INTEGER,
            xsd::NEGATIVE_INTEGER,
            xsd::UNSIGNED_INT,
            xsd::UNSIGNED_LONG,
            xsd::UNSIGNED_SHORT,
            xsd::UNSIGNED_BYTE,
        ] {
            assert!(is_integer_datatype(dt.as_str()), "{} must be an integer datatype", dt.as_str());
        }
        // Numeric-but-not-integer and non-numeric datatypes are rejected.
        for dt in [xsd::DECIMAL, xsd::DOUBLE, xsd::FLOAT, xsd::STRING, xsd::BOOLEAN, xsd::DATE_TIME] {
            assert!(!is_integer_datatype(dt.as_str()), "{} must NOT be an integer datatype", dt.as_str());
        }
        assert!(!is_integer_datatype("http://ex/custom"), "an unknown IRI is not an integer datatype");
        assert!(!is_integer_datatype(""), "the empty datatype is not an integer datatype");
    }

    /// [OPUS-4.8] sq-bif — `exact_numeric_lexical` is the disambiguator the engine reaches for
    /// only when two operands' f64 values compare EQUAL: it returns the EXACT lexical of an
    /// integer-subtype or `xsd:decimal` literal (so `9007199254740993` ≠ `9007199254740992`
    /// survives the f64 collapse at 2^53) and `None` for everything whose value already IS its
    /// f64 (float/double) or is non-numeric. It had no test. We pin: the inline-integer path,
    /// the dictionary integer path (including a value beyond f64's exact range), the decimal
    /// path, and every `None` case.
    #[test]
    fn exact_numeric_lexical_returns_exact_form_or_none() {
        // A big integer past 2^53 (where f64 cannot represent consecutive integers) and its
        // neighbour: both are dictionary terms (out of the inline range), and their EXACT
        // lexicals must survive even though their f64s are equal.
        let big = "9007199254740993"; // 2^53 + 1
        let big_neighbour = "9007199254740992"; // 2^53 — same f64 as `big`
        let nt = format!(
            "<http://ex/s> <http://ex/i> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
             <http://ex/s> <http://ex/big> \"{big}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
             <http://ex/s> <http://ex/big2> \"{big_neighbour}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
             <http://ex/s> <http://ex/long> \"-5\"^^<http://www.w3.org/2001/XMLSchema#long> .\n\
             <http://ex/s> <http://ex/dec> \"1.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n\
             <http://ex/s> <http://ex/dbl> \"1.5\"^^<http://www.w3.org/2001/XMLSchema#double> .\n\
             <http://ex/s> <http://ex/flt> \"1.5\"^^<http://www.w3.org/2001/XMLSchema#float> .\n\
             <http://ex/s> <http://ex/str> \"hello\" .\n\
             <http://ex/s> <http://ex/lang> \"bonjour\"@fr .\n"
        );
        let g = Graph::load_str(&nt, "ntriples").unwrap();

        let lexical = |value: &str, dt: oxrdf::NamedNodeRef| -> Option<String> {
            let lit = Term::Literal(Literal::new_typed_literal(value, dt));
            g.exact_numeric_lexical(g.id_of(&lit).unwrap_or_else(|| panic!("{value} not interned")))
        };

        // Inline integer (small, in range): the inline id formats its value directly.
        let small_id = g.id_of(&Term::Literal(Literal::new_typed_literal("42", xsd::INTEGER))).unwrap();
        assert!(dict::is_inline(small_id), "42 is an inline integer");
        assert_eq!(g.exact_numeric_lexical(small_id).as_deref(), Some("42"));

        // Big dictionary integers: the EXACT lexical is preserved, disambiguating the f64 tie.
        assert_eq!(lexical(big, xsd::INTEGER).as_deref(), Some(big));
        assert_eq!(lexical(big_neighbour, xsd::INTEGER).as_deref(), Some(big_neighbour));
        // The two lexicals differ even though they are the same f64 (the whole point).
        assert_eq!(g.numeric_value(g.id_of(&Term::Literal(Literal::new_typed_literal(big, xsd::INTEGER))).unwrap()),
                   g.numeric_value(g.id_of(&Term::Literal(Literal::new_typed_literal(big_neighbour, xsd::INTEGER))).unwrap()),
                   "the two big integers collapse to the same f64");
        assert_ne!(lexical(big, xsd::INTEGER), lexical(big_neighbour, xsd::INTEGER));

        // An integer SUBTYPE (long) is also exact.
        assert_eq!(lexical("-5", xsd::LONG).as_deref(), Some("-5"));
        // `xsd:decimal` is exact (the trailing zero of "1.50" is preserved verbatim).
        assert_eq!(lexical("1.50", xsd::DECIMAL).as_deref(), Some("1.50"));

        // None: double / float values ARE their f64 (no exact-lexical disambiguation needed).
        assert_eq!(lexical("1.5", xsd::DOUBLE), None);
        assert_eq!(lexical("1.5", xsd::FLOAT), None);
        // None: a plain string and a language-tagged literal are not numeric.
        assert_eq!(g.exact_numeric_lexical(g.id_of(&Term::Literal(Literal::new_simple_literal("hello"))).unwrap()), None);
        assert_eq!(
            g.exact_numeric_lexical(g.id_of(&Term::Literal(Literal::new_language_tagged_literal_unchecked("bonjour", "fr"))).unwrap()),
            None
        );
        // None: an IRI is not numeric.
        assert_eq!(g.exact_numeric_lexical(g.id_of(&Term::NamedNode(NamedNode::new_unchecked("http://ex/s"))).unwrap()), None);
    }

    /// [OPUS-4.8] sq-lr2ii — `decimal_significant_digits` counts significant digits, ignoring
    /// leading integer zeros and leading fractional zeros, and rejects non-decimals.
    #[test]
    fn decimal_significant_digits_counts() {
        assert_eq!(decimal_significant_digits("1"), 1);
        assert_eq!(decimal_significant_digits("1.5"), 2);
        assert_eq!(decimal_significant_digits("007.50"), 2, "leading int zeros + trailing frac zeros dropped: 7.5");
        assert_eq!(decimal_significant_digits("0.00123"), 3, "leading fractional zeros are not significant");
        assert_eq!(decimal_significant_digits("-1.000000000000000001"), 19, "sign stripped; 19 sig digits");
        assert_eq!(decimal_significant_digits("+2.5"), 2);
        assert_eq!(decimal_significant_digits("0.0"), 0, "zero has no significant digits");
        // Non-decimal lexicals are treated as unsafe (usize::MAX > 15).
        assert_eq!(decimal_significant_digits("1e5"), usize::MAX);
        assert_eq!(decimal_significant_digits(""), usize::MAX);
        assert_eq!(decimal_significant_digits("abc"), usize::MAX);
    }

    /// [OPUS-4.8] sq-lr2ii — `has_high_precision_decimal` is the engine's sargable-safety gate:
    /// TRUE iff the graph holds an `xsd:decimal` with > 15 significant digits (an f64-inexact
    /// value). Integers past 2^53, floats/doubles, and <=15-digit decimals do NOT set it.
    #[test]
    fn has_high_precision_decimal_flags_only_inexact_decimals() {
        let xsd_ = |ty: &str| format!("http://www.w3.org/2001/XMLSchema#{ty}");
        let build = |obj: &str| {
            let nt = format!("<http://ex/s> <http://ex/p> {obj} .\n");
            Graph::load_str(&nt, "ntriples").unwrap()
        };

        // The bug value: 19-sig-digit decimal collapsing onto f64 1.0.
        assert!(build(&format!("\"1.000000000000000001\"^^<{}>", xsd_("decimal"))).has_high_precision_decimal());
        // A <1 high-precision decimal.
        assert!(build(&format!("\"0.1234567890123456789\"^^<{}>", xsd_("decimal"))).has_high_precision_decimal());

        // NOT flagged: an exactly-representable decimal (<=15 sig digits).
        assert!(!build(&format!("\"1.5\"^^<{}>", xsd_("decimal"))).has_high_precision_decimal());
        assert!(!build(&format!("\"123456789012345\"^^<{}>", xsd_("decimal"))).has_high_precision_decimal(), "15 sig digits round-trips");
        // NOT flagged: a big integer (constant-side guard's job, not this one).
        assert!(!build(&format!("\"9007199254740993\"^^<{}>", xsd_("integer"))).has_high_precision_decimal());
        // NOT flagged: a high-precision double — its value IS its f64.
        assert!(!build(&format!("\"1.000000000000000001\"^^<{}>", xsd_("double"))).has_high_precision_decimal());
        // NOT flagged: an empty / non-numeric graph.
        assert!(!build("\"hello\"").has_high_precision_decimal());
        assert!(!Graph::new().has_high_precision_decimal());
    }

    /// [OPUS-4.8] sq-lr2ii — the memoised flag must NOT go stale: inserting an f64-inexact
    /// decimal via a delta AFTER the flag was computed as `false` must flip it to `true`.
    #[test]
    fn has_high_precision_decimal_memo_survives_delta_insert() {
        let mut g = Graph::load_str("<http://ex/s> <http://ex/p> \"3\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", "ntriples").unwrap();
        // Compute (and memoise) the flag as false over the integer-only graph.
        assert!(!g.has_high_precision_decimal());
        // Insert a high-precision decimal; the memo must be invalidated + recomputed to true.
        let s = Term::NamedNode(NamedNode::new_unchecked("http://ex/s"));
        let p = Term::NamedNode(NamedNode::new_unchecked("http://ex/d"));
        let o = Term::Literal(Literal::new_typed_literal("2.000000000000000003", xsd::DECIMAL));
        g.apply_delta(&[[s, p, o]], &[]).unwrap();
        assert!(g.has_high_precision_decimal(), "delta-inserted inexact decimal must flip the memo");
    }

    /// The full sorted term-triple set of a graph (overlay merged), for state comparison.
    fn dump_terms(g: &Graph) -> Vec<(String, String, String)> {
        let scan = g.store.scan(&[None, None, None]);
        let mut v: Vec<(String, String, String)> = scan
            .rows
            .iter()
            .map(|r| {
                let spo = scan.to_spo(r);
                (g.dict.term(spo[0]).to_string(), g.dict.term(spo[1]).to_string(), g.dict.term(spo[2]).to_string())
            })
            .collect();
        v.sort();
        v
    }

    fn term_iri(n: u32, ns: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{ns}{n}")))
    }

    /// DIFFERENTIAL: a random sequence of 200 insert/delete batches applied through the
    /// delta-overlay must yield exactly the same sorted triple set as applying each batch
    /// by full set-ops + rebuild — including across periodic `compact()` calls.
    #[test]
    fn overlay_differential_200_batches() {
        let base_ttl = "@prefix : <http://ex/> . :s0 :p0 :o0 . :s1 :p0 :o1 . :s2 :p1 :o2 .";
        let mut g = Graph::load_str(base_ttl, "turtle").unwrap();
        let mut reference: std::collections::BTreeSet<(String, String, String)> =
            dump_terms(&g).into_iter().collect();

        let mut st = 0xDEADBEEFu32;
        let mut rng = move || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        let mk = |r: &mut dyn FnMut() -> u32| -> [Term; 3] {
            // A small term universe so inserts/deletes collide often (the hard cases:
            // re-insert, delete-of-pending-insert, delete-absent, re-insert-of-deleted).
            [term_iri(r() % 30, "s"), term_iri(r() % 5, "p"), term_iri(r() % 40, "o")]
        };
        for batch in 0..200 {
            let n_ins = (rng() % 6) as usize;
            let n_del = (rng() % 6) as usize;
            let inserts: Vec<[Term; 3]> = (0..n_ins).map(|_| mk(&mut rng)).collect();
            let deletes: Vec<[Term; 3]> = (0..n_del).map(|_| mk(&mut rng)).collect();

            // Reference semantics: deletes first, then inserts (SPARQL order).
            for [s, p, o] in &deletes {
                reference.remove(&(s.to_string(), p.to_string(), o.to_string()));
            }
            for [s, p, o] in &inserts {
                reference.insert((s.to_string(), p.to_string(), o.to_string()));
            }
            g.apply_delta(&inserts, &deletes).unwrap();

            if batch % 50 == 49 {
                g.compact().unwrap(); // fold the overlay periodically
                assert!(!g.store.has_overlay());
            }
            let got = dump_terms(&g);
            let want: Vec<(String, String, String)> = reference.iter().cloned().collect();
            assert_eq!(got, want, "state diverged after batch {batch}");
            assert_eq!(g.len(), reference.len(), "len diverged after batch {batch}");
        }
    }

    /// Durability roundtrip: updates on a directory-backed graph are WAL-logged and
    /// replayed on re-open; `compact()` folds them into a new persisted base atomically
    /// and truncates the log; the compacted state re-opens identically.
    #[cfg(feature = "mmap")]
    #[test]
    fn wal_replay_and_compact_roundtrip() {
        let mut nt = String::new();
        for i in 0..2000u32 {
            nt.push_str(&format!("<http://ex/s{}> <http://ex/p{}> <http://ex/o{}> .\n", i % 97, i % 7, i % 311));
        }
        let dir = std::env::temp_dir().join(format!("sparq_wal_rt_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str(&nt, "ntriples").unwrap().save(&dir).unwrap();

        // Open from disk, apply two update batches (one with a NEW term + a numeric
        // literal — exercising dict append + numeric-cache growth), drop WITHOUT compacting.
        let expected = {
            let mut g = Graph::open(&dir).unwrap();
            let lit = Term::Literal(Literal::new_typed_literal("31250000000", xsd::INTEGER));
            g.apply_delta(
                &[[term_iri(1000, "s"), term_iri(50, "p"), lit.clone()], [term_iri(1, "s"), term_iri(1, "p"), term_iri(1, "o")]],
                &[[term_iri(0, "s"), term_iri(0, "p"), term_iri(0, "o")]],
            )
            .unwrap();
            g.apply_delta(&[], &[[term_iri(1, "s"), term_iri(1, "p"), term_iri(1, "o")]]).unwrap();
            assert!(g.store.has_overlay());
            // The appended numeric literal must be live in the numeric cache.
            let id = g.id_of(&lit).expect("appended literal must be interned");
            assert_eq!(g.numeric_value(id), Some(31_250_000_000.0));
            dump_terms(&g)
        };
        assert!(dir.join("wal.log").metadata().unwrap().len() > 0, "WAL must persist the batches");

        // Re-open: the WAL replays into the overlay — identical state.
        let mut g2 = Graph::open(&dir).unwrap();
        assert!(g2.store.has_overlay(), "replayed WAL must rebuild the overlay");
        assert_eq!(dump_terms(&g2), expected, "WAL replay must restore the exact state");
        let lit = Term::Literal(Literal::new_typed_literal("31250000000", xsd::INTEGER));
        let id = g2.id_of(&lit).expect("replayed literal must be interned");
        assert_eq!(g2.numeric_value(id), Some(31_250_000_000.0), "numeric cache must cover replayed terms");

        // Compact: overlay folded, base re-persisted atomically, WAL truncated.
        g2.compact().unwrap();
        assert!(!g2.store.has_overlay());
        assert_eq!(dump_terms(&g2), expected);
        assert_eq!(dir.join("wal.log").metadata().unwrap().len(), 0, "compact must truncate the WAL");
        assert!(!dir.with_extension("compact-new").exists());
        assert!(!dir.with_extension("compact-old").exists());
        drop(g2);

        // The compacted base re-opens to the same state, with nothing left to replay.
        let g3 = Graph::open(&dir).unwrap();
        assert!(!g3.store.has_overlay());
        assert_eq!(dump_terms(&g3), expected);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Kill-during-write simulation: a WAL whose trailing BATCH is torn (header, body, or
    /// commit trailer) must replay only the fully-COMMITTED batches and truncate the torn
    /// tail. [OPUS-4.8] (review 1593) Each `apply_delta` is one atomic batch frame:
    /// `[magic][n][body_len] <records> [crc][commit]`.
    #[cfg(feature = "mmap")]
    #[test]
    fn wal_torn_batch_replay() {
        let dir = std::env::temp_dir().join(format!("sparq_wal_torn_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str("<http://ex/s0> <http://ex/p0> <http://ex/o0> .", "ntriples").unwrap().save(&dir).unwrap();

        {
            let mut g = Graph::open(&dir).unwrap();
            g.apply_delta(&[[term_iri(1, "s"), term_iri(1, "p"), term_iri(1, "o")]], &[]).unwrap();
            g.apply_delta(&[[term_iri(2, "s"), term_iri(2, "p"), term_iri(2, "o")]], &[]).unwrap();
        }
        let wal_path = dir.join("wal.log");
        let full = std::fs::read(&wal_path).unwrap();
        // Length of the FIRST batch frame: 12 (header) + body_len + 12 (crc+commit).
        let body_len = u32::from_le_bytes([full[8], full[9], full[10], full[11]]) as usize;
        let first_batch_len = 12 + body_len + 12;
        assert!(full.len() > first_batch_len, "test needs two batches");

        // (a) Sever the SECOND batch mid-body (a crash before its commit trailer is synced):
        //     the uncommitted batch is discarded WHOLE — never partially applied.
        let torn = &full[..first_batch_len + 14]; // header + a few body bytes, no trailer
        std::fs::write(&wal_path, torn).unwrap();
        let g = Graph::open(&dir).unwrap();
        let got = dump_terms(&g);
        assert_eq!(got.len(), 2, "only the first committed batch replays");
        assert!(got.iter().any(|(s, _, _)| s.contains("s1")));
        assert!(!got.iter().any(|(s, _, _)| s.contains("s2")));
        assert_eq!(std::fs::read(&wal_path).unwrap().len(), first_batch_len, "torn tail truncated to the batch boundary");

        // (b) Corrupt the FIRST batch's body (flip a byte inside it): the checksum fails, so
        //     the whole batch is rejected and replay stops at the base — never a partial apply.
        let mut corrupt = full[..first_batch_len].to_vec();
        corrupt[14] ^= 0xFF; // a byte inside the first record's body
        std::fs::write(&wal_path, &corrupt).unwrap();
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(dump_terms(&g2).len(), 1, "checksum-failing batch must not apply");
        assert_eq!(std::fs::read(&wal_path).unwrap().len(), 0, "corrupt leading batch truncates the whole log");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (review 1593) A MULTI-record batch must be atomic: if a crash severs the
    /// batch after some of its records are on disk but before the commit trailer, NONE of the
    /// batch's records may be applied (the pre-fix per-record WAL would apply a prefix).
    #[cfg(feature = "mmap")]
    #[test]
    fn wal_torn_multi_record_batch_is_all_or_nothing() {
        let dir = std::env::temp_dir().join(format!("sparq_wal_multi_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str("<http://ex/s0> <http://ex/p0> <http://ex/o0> .", "ntriples").unwrap().save(&dir).unwrap();
        {
            let mut g = Graph::open(&dir).unwrap();
            // ONE batch of three inserts.
            let ins: Vec<[Term; 3]> = (1..=3)
                .map(|i| [term_iri(i, "s"), term_iri(i, "p"), term_iri(i, "o")])
                .collect();
            g.apply_delta(&ins, &[]).unwrap();
        }
        let wal_path = dir.join("wal.log");
        let full = std::fs::read(&wal_path).unwrap();
        let body_len = u32::from_le_bytes([full[8], full[9], full[10], full[11]]) as usize;
        // Drop the commit trailer (last 12 bytes) AND part of the last record — a torn append
        // with two of the three records' bytes present but no commit point.
        let torn = &full[..12 + body_len - 5]; // header + most of the body, no trailer
        std::fs::write(&wal_path, torn).unwrap();
        let g = Graph::open(&dir).unwrap();
        // NONE of the three records may have applied — only the base triple remains.
        assert_eq!(dump_terms(&g).len(), 1, "uncommitted multi-record batch must apply nothing");
        assert_eq!(std::fs::read(&wal_path).unwrap().len(), 0, "the torn batch is the whole log → truncated empty");
        std::fs::remove_dir_all(&dir).ok();
    }

    // [OPUS-4.8] (sq-ycle) ---- atomic multi-op UPDATE redo journal (`txn.log`) -----------------

    /// A unique temp dir per test (pid + a process-local counter) so the journal tests never
    /// collide when run in parallel; cleaned with `remove_dir_all` at the end of each test.
    #[cfg(feature = "mmap")]
    fn txn_tmp_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("sparq_txn_{tag}_{}_{n}", std::process::id()))
    }

    /// [OPUS-4.8] (sq-ycle) The named-graph term used by the journal tests.
    #[cfg(feature = "mmap")]
    fn named(n: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{n}")))
    }

    /// [OPUS-4.8] (sq-ycle) The single named-graph triple the torn-tail test starts with.
    #[cfg(feature = "mmap")]
    fn base_named() -> Vec<(String, String, String)> {
        vec![("<http://ex/n0>".into(), "<http://ex/p>".into(), "<http://ex/o0>".into())]
    }

    /// [OPUS-4.8] (sq-ycle) TORN-TAIL guarantee: a multi-slot redo frame (a default-graph record
    /// PLUS a named-graph record) whose commit trailer + part of the last record were severed by a
    /// crash must apply NOTHING on `open`, and the torn `txn.log` must be truncated to empty.
    /// Mirrors `wal_torn_multi_record_batch_is_all_or_nothing` for the parent-level journal.
    #[cfg(feature = "mmap")]
    #[test]
    fn txn_journal_torn_tail_is_all_or_nothing() {
        let dir = txn_tmp_dir("torn");
        std::fs::remove_dir_all(&dir).ok();
        // Base graph with one default triple + one named graph holding one triple — so a torn
        // frame redo would, if it leaked, change BOTH slots (and the assert below would fire).
        Graph::load_dataset(
            "<http://ex/base> <http://ex/p> <http://ex/o> .\n\
             <http://ex/n0> <http://ex/p> <http://ex/o0> <http://ex/g> .",
            "nquads",
        )
        .unwrap()
        .save(&dir)
        .unwrap();
        let base = dump_terms(&Graph::open(&dir).unwrap());

        // Write ONE valid frame with a default-slot insert AND a named-slot insert.
        let records: Vec<(bool, Option<Term>, [Term; 3])> = vec![
            (true, None, [term_iri(1, "s"), term_iri(1, "p"), term_iri(1, "o")]),
            (true, Some(named("g")), [term_iri(2, "s"), term_iri(2, "p"), term_iri(2, "o")]),
        ];
        {
            let mut j = TxnJournal::open(&dir).unwrap();
            j.append(&records).unwrap();
        }
        let path = TxnJournal::path(&dir);
        let full = std::fs::read(&path).unwrap();
        assert!(full.len() > 24, "frame must have a header + body + trailer");
        // Sever the commit trailer (last 12 bytes) AND part of the last record (a torn append).
        let torn = &full[..full.len() - 12 - 5];
        std::fs::write(&path, torn).unwrap();

        let g = Graph::open(&dir).unwrap();
        // NONE of the journaled records applied: the state equals the pre-frame base, and neither
        // the default `s1` triple nor the named `s2` triple is present.
        assert_eq!(dump_terms(&g), base, "torn frame must apply nothing to the default graph");
        let gnamed = g.named.iter().find(|(n, _)| n == &named("g")).map(|(_, sub)| dump_terms(sub));
        assert_eq!(gnamed, Some(base_named()), "torn frame must apply nothing to the named graph");
        assert_eq!(std::fs::read(&path).unwrap().len(), 0, "torn journal is truncated to empty on open");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-ycle) The PURE journal guarantee that underwrites atomicity: a committed
    /// `txn.log` frame ALONE — with NO per-graph WAL materialisation at all — reconstructs the
    /// FULL all-or-nothing record set on `open`, across BOTH the default graph and a named graph
    /// (the PSS `INSERT DATA { GRAPH <r> ... ; GRAPH <parent> contains <r> }` shape). This is the
    /// reliable simulation of "a crash after the single commit fsync but before materialisation":
    /// the journal redo heals the desync. The journal is truncated afterwards.
    #[cfg(feature = "mmap")]
    #[test]
    fn apply_effects_multi_slot_is_atomic_via_journal() {
        let dir = txn_tmp_dir("multislot");
        std::fs::remove_dir_all(&dir).ok();
        // Empty base default graph; no named graphs yet (the frame must CREATE the named graph).
        Graph::load_str("", "ntriples").unwrap().save(&dir).unwrap();

        // The PSS body resolved to records: a child-graph triple under <r>, and a containment
        // triple under <parent>. Commit them to the journal with NO materialisation, then close.
        let r = named("r");
        let parent = named("parent");
        let child = [
            term_iri(1, "x"),
            Term::NamedNode(NamedNode::new_unchecked("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")),
            named("Resource"),
        ];
        let contains = [
            parent.clone(),
            Term::NamedNode(NamedNode::new_unchecked("http://www.w3.org/ns/ldp#contains")),
            r.clone(),
        ];
        let records: Vec<(bool, Option<Term>, [Term; 3])> = vec![
            (true, Some(r.clone()), child.clone()),
            (true, Some(parent.clone()), contains.clone()),
        ];
        {
            let mut g = Graph::open(&dir).unwrap();
            g.commit_txn(&records).unwrap(); // THE commit point — NO materialisation follows.
            assert!(TxnJournal::path(&dir).metadata().unwrap().len() > 0, "frame must be on disk");
            // Drop without materialising: simulates a crash right after the commit fsync.
        }
        // Reopen: the journal redo must reconstruct BOTH slots (child + containment), all-or-nothing.
        let g = Graph::open(&dir).unwrap();
        let r_sub = g.named.iter().find(|(n, _)| n == &r).map(|(_, s)| dump_terms(s));
        let p_sub = g.named.iter().find(|(n, _)| n == &parent).map(|(_, s)| dump_terms(s));
        assert_eq!(
            r_sub,
            Some(vec![(child[0].to_string(), child[1].to_string(), child[2].to_string())]),
            "journal redo must materialise the child graph <r>"
        );
        assert_eq!(
            p_sub,
            Some(vec![(contains[0].to_string(), contains[1].to_string(), contains[2].to_string())]),
            "journal redo must materialise the containment under <parent>"
        );
        assert_eq!(std::fs::read(TxnJournal::path(&dir)).unwrap().len(), 0, "journal truncated after redo");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (Copilot #135) DOUBLE-RESTART DURABILITY (default graph): the load-bearing
    /// regression test for the recovery-durability fix. A committed `txn.log` frame with NO
    /// per-graph WAL materialisation (the crash-after-commit-fsync / before-materialisation window)
    /// must be REDONE INTO THE DURABLE WAL on the first reopen and then SURVIVE a SECOND reopen.
    ///
    /// Under the OLD behaviour (`apply_delta_mem` then truncate) the first reopen applied the record
    /// only IN MEMORY and erased `txn.log`, so the data was present after the first open but GONE
    /// after the second — exactly the durability defect this asserts against. The second reopen is
    /// what distinguishes "durable" from "in-memory-on-first-open".
    #[cfg(feature = "mmap")]
    #[test]
    fn txn_default_redo_survives_double_restart() {
        let dir = txn_tmp_dir("double_default");
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str("<http://ex/base> <http://ex/p> <http://ex/o> .", "ntriples").unwrap().save(&dir).unwrap();

        // Two default-graph inserts committed to the journal with NO materialisation, then dropped
        // — simulating a crash right after the single commit fsync, before any per-graph WAL write.
        let a = [term_iri(1, "s"), term_iri(1, "p"), term_iri(1, "o")];
        let b = [term_iri(2, "s"), term_iri(2, "p"), term_iri(2, "o")];
        let records: Vec<(bool, Option<Term>, [Term; 3])> = vec![(true, None, a.clone()), (true, None, b.clone())];
        {
            let mut g = Graph::open(&dir).unwrap();
            g.commit_txn(&records).unwrap(); // THE commit point — NO materialisation follows.
            assert!(TxnJournal::path(&dir).metadata().unwrap().len() > 0, "frame must be on disk");
        }

        let want = |g: &Graph| {
            let d = dump_terms(g);
            assert!(d.iter().any(|(s, _, _)| s.contains("/s1")), "record a must be present");
            assert!(d.iter().any(|(s, _, _)| s.contains("/s2")), "record b must be present");
            assert!(d.iter().any(|(s, _, _)| s.contains("/base")), "base must be present");
        };

        // First reopen: recovery REDOES the frame INTO THE DURABLE WAL, then truncates `txn.log`.
        {
            let g = Graph::open(&dir).unwrap();
            want(&g);
            assert_eq!(std::fs::read(TxnJournal::path(&dir)).unwrap().len(), 0, "journal truncated after redo");
            // The WAL — not the (now empty) journal — must now hold the recovered records.
            assert!(Wal::path(&dir).metadata().unwrap().len() > 0, "records must be durably in the per-graph WAL");
        }
        // SECOND reopen: `txn.log` is empty, so the data can ONLY come from the durable WAL. This is
        // the assertion the OLD in-memory-only redo failed.
        {
            let g = Graph::open(&dir).unwrap();
            want(&g);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (Copilot #135) DOUBLE-RESTART DURABILITY (named graphs): the named-graph twin of
    /// `txn_default_redo_survives_double_restart` — the PSS `INSERT DATA { GRAPH <r> ... ; GRAPH
    /// <parent> contains <r> }` shape. A committed frame that CREATES a named graph and writes a
    /// containment triple under another, with NO materialisation, must be redone into the durable
    /// per-graph WALs on the first reopen and STILL be present after a SECOND reopen (`txn.log`
    /// empty → the data must come from each sub-graph's own WAL).
    #[cfg(feature = "mmap")]
    #[test]
    fn txn_named_redo_survives_double_restart() {
        let dir = txn_tmp_dir("double_named");
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str("", "ntriples").unwrap().save(&dir).unwrap();

        let r = named("r");
        let parent = named("parent");
        let child = [
            term_iri(1, "x"),
            Term::NamedNode(NamedNode::new_unchecked("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")),
            named("Resource"),
        ];
        let contains = [
            parent.clone(),
            Term::NamedNode(NamedNode::new_unchecked("http://www.w3.org/ns/ldp#contains")),
            r.clone(),
        ];
        let records: Vec<(bool, Option<Term>, [Term; 3])> =
            vec![(true, Some(r.clone()), child.clone()), (true, Some(parent.clone()), contains.clone())];
        {
            let mut g = Graph::open(&dir).unwrap();
            g.commit_txn(&records).unwrap(); // THE commit point — NO materialisation follows.
            assert!(TxnJournal::path(&dir).metadata().unwrap().len() > 0, "frame must be on disk");
        }

        let want = |g: &Graph| {
            let r_sub = g.named.iter().find(|(n, _)| n == &r).map(|(_, s)| dump_terms(s));
            let p_sub = g.named.iter().find(|(n, _)| n == &parent).map(|(_, s)| dump_terms(s));
            assert_eq!(
                r_sub,
                Some(vec![(child[0].to_string(), child[1].to_string(), child[2].to_string())]),
                "child graph <r> must be present"
            );
            assert_eq!(
                p_sub,
                Some(vec![(contains[0].to_string(), contains[1].to_string(), contains[2].to_string())]),
                "containment under <parent> must be present"
            );
        };

        // First reopen: recovery redoes the frame into each sub-graph's DURABLE WAL, then truncates.
        {
            let g = Graph::open(&dir).unwrap();
            want(&g);
            assert_eq!(std::fs::read(TxnJournal::path(&dir)).unwrap().len(), 0, "journal truncated after redo");
        }
        // SECOND reopen: `txn.log` empty → both records must come from the per-graph WALs (durable).
        {
            let g = Graph::open(&dir).unwrap();
            want(&g);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-ycle) An UNCOMMITTED frame (no commit trailer written — the crash landed
    /// before the single fsync completed the trailer) must apply NOTHING and leave the pre-body
    /// state intact; the torn tail is truncated.
    #[cfg(feature = "mmap")]
    #[test]
    fn txn_uncommitted_frame_applies_nothing() {
        let dir = txn_tmp_dir("uncommitted");
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str("<http://ex/base> <http://ex/p> <http://ex/o> .", "ntriples").unwrap().save(&dir).unwrap();
        let base = dump_terms(&Graph::open(&dir).unwrap());

        // A valid frame, then strip JUST the commit trailer (the last 12 bytes) — header + body
        // present, no commit marker → an uncommitted frame.
        let records: Vec<(bool, Option<Term>, [Term; 3])> =
            vec![(true, None, [term_iri(9, "s"), term_iri(9, "p"), term_iri(9, "o")])];
        {
            let mut j = TxnJournal::open(&dir).unwrap();
            j.append(&records).unwrap();
        }
        let path = TxnJournal::path(&dir);
        let full = std::fs::read(&path).unwrap();
        std::fs::write(&path, &full[..full.len() - 12]).unwrap(); // drop the commit trailer

        let g = Graph::open(&dir).unwrap();
        assert_eq!(dump_terms(&g), base, "uncommitted frame must apply nothing");
        assert_eq!(std::fs::read(&path).unwrap().len(), 0, "uncommitted tail truncated to empty");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-ycle) IDEMPOTENCY: a committed `txn.log` frame whose records were ALSO
    /// already materialised into the per-graph WALs (the normal, crash-free case) must, on `open`,
    /// yield EXACTLY the committed set — no duplicate triples (set semantics) — and truncate the
    /// journal. This is what guarantees the redundant-on-crash redo is harmless in the happy path.
    #[cfg(feature = "mmap")]
    #[test]
    fn txn_idempotent_when_already_materialized() {
        let dir = txn_tmp_dir("idempotent");
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_dataset(
            "<http://ex/base> <http://ex/p> <http://ex/o> .\n\
             <http://ex/n0> <http://ex/p> <http://ex/o0> <http://ex/g> .",
            "nquads",
        )
        .unwrap()
        .save(&dir)
        .unwrap();

        let g_default = [term_iri(1, "s"), term_iri(1, "p"), term_iri(1, "o")];
        let g_named = [term_iri(2, "s"), term_iri(2, "p"), term_iri(2, "o")];
        let records: Vec<(bool, Option<Term>, [Term; 3])> =
            vec![(true, None, g_default.clone()), (true, Some(named("g")), g_named.clone())];
        {
            let mut g = Graph::open(&dir).unwrap();
            // Commit the frame AND materialise the SAME records into the per-graph WALs (the
            // crash-free path: both the journal and the per-graph WALs hold the change).
            g.commit_txn(&records).unwrap();
            g.apply_delta(std::slice::from_ref(&g_default), &[]).unwrap();
            let i = g.ensure_named(&named("g")).unwrap();
            g.named[i].1.apply_delta(std::slice::from_ref(&g_named), &[]).unwrap();
        }
        // Reopen: the per-graph WALs replay the records, THEN the journal redoes the SAME records —
        // set semantics means no duplicates; the committed set is present exactly once.
        let g = Graph::open(&dir).unwrap();
        let mut def = dump_terms(&g);
        def.retain(|(s, _, _)| s.contains("/s1"));
        assert_eq!(def.len(), 1, "the default record is present exactly once (no duplicate)");
        let nsub = g.named.iter().find(|(n, _)| n == &named("g")).map(|(_, s)| {
            let mut v = dump_terms(s);
            v.retain(|(x, _, _)| x.contains("/s2"));
            v
        });
        assert_eq!(nsub.map(|v| v.len()), Some(1), "the named record is present exactly once");
        assert_eq!(std::fs::read(TxnJournal::path(&dir)).unwrap().len(), 0, "journal truncated after idempotent redo");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (review 1593) Compaction directory swap must be crash-recoverable: if `dir`
    /// is missing but the new base survives in `compact-new`, `Graph::open` completes the
    /// swap; if only `compact-old` survives, it rolls back — `dir` is never lost.
    #[cfg(feature = "mmap")]
    #[test]
    fn compact_swap_crash_recovery() {
        let base = std::env::temp_dir().join(format!("sparq_compact_recover_{}", std::process::id()));
        let dir = base.join("g");
        // Helper: build a graph dir with one triple naming `tag`.
        let build = |d: &std::path::Path, tag: &str| {
            std::fs::remove_dir_all(d).ok();
            Graph::load_str(&format!("<http://ex/{tag}> <http://ex/p> <http://ex/o> ."), "ntriples")
                .unwrap()
                .save(d)
                .unwrap();
        };

        // (a) Crash BETWEEN the two renames: dir missing, both siblings present. Recovery must
        //     COMPLETE the swap by promoting compact-new (the fully-synced new base).
        build(&dir.with_extension("compact-old"), "old");
        build(&dir.with_extension("compact-new"), "new");
        assert!(!dir.exists());
        let g = Graph::open(&dir).unwrap();
        assert!(dump_terms(&g).iter().any(|(s, _, _)| s.contains("/new")), "must promote compact-new");
        assert!(!dir.with_extension("compact-new").exists());
        assert!(!dir.with_extension("compact-old").exists());

        // (b) Crash BEFORE the new base was ready: dir missing, only compact-old present.
        //     Recovery must ROLL BACK by restoring compact-old.
        std::fs::remove_dir_all(&dir).ok();
        build(&dir.with_extension("compact-old"), "old");
        assert!(!dir.exists());
        let g = Graph::open(&dir).unwrap();
        assert!(dump_terms(&g).iter().any(|(s, _, _)| s.contains("/old")), "must roll back to compact-old");
        assert!(!dir.with_extension("compact-old").exists());

        std::fs::remove_dir_all(&base).ok();
    }

    /// [OPUS-4.8] (sq-ft7u) The PUBLIC `restore_into_durable` seam: replacing a directory-backed
    /// store's contents with a fresh image must (a) round-trip the fresh live triple set after the
    /// swap AND after a reopen (the on-disk base IS the restored image), and (b) leave NO sibling
    /// cruft. The restored graph must also be WAL-backed (a subsequent durable update survives a
    /// reopen), proving the re-open carried a real WAL.
    #[cfg(feature = "mmap")]
    #[test]
    fn restore_into_durable_round_trips_and_is_wal_backed() {
        let dir = std::env::temp_dir().join(format!("sparq_restore_durable_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        // Establish an ORIGINAL durable store with one triple.
        Graph::load_str("<http://ex/orig> <http://ex/p> <http://ex/o> .", "ntriples")
            .unwrap()
            .save(&dir)
            .unwrap();
        assert_eq!(Graph::open(&dir).unwrap().len(), 1);

        // Restore a DIFFERENT, larger fresh image THROUGH to the durable dir.
        let mut fresh = Graph::load_str(
            "<http://ex/r1> <http://ex/p> <http://ex/o1> .\n\
             <http://ex/r2> <http://ex/p> <http://ex/o2> .",
            "ntriples",
        )
        .unwrap();
        // A named graph too, to exercise the whole sub-tree write.
        fresh
            .apply_delta_nquads(
                "<http://ex/n> <http://ex/q> <http://ex/w> <http://ex/g> .",
                "",
            )
            .unwrap();
        let restored = Graph::restore_into_durable(&dir, fresh).unwrap();
        // The original triple is GONE; the restored set is present (round-trip of the live set).
        let live = dump_terms(&restored);
        assert!(!live.iter().any(|(s, _, _)| s.contains("/orig")), "original content replaced");
        assert!(live.iter().any(|(s, _, _)| s.contains("/r1")), "restored content present");
        assert_eq!(restored.len(), 2, "two default-graph triples after restore");
        // No sibling leftovers from the swap.
        assert!(!dir.with_extension("compact-new").exists());
        assert!(!dir.with_extension("compact-old").exists());

        // The restored graph is genuinely WAL-backed: a durable update + a fresh reopen sees it.
        let mut g = restored;
        g.apply_delta_nquads("<http://ex/post> <http://ex/p> <http://ex/v> .", "").unwrap();
        drop(g);
        let reopened = Graph::open(&dir).unwrap();
        let live2 = dump_terms(&reopened);
        assert!(live2.iter().any(|(s, _, _)| s.contains("/r1")), "restored content survives reopen");
        assert!(live2.iter().any(|(s, _, _)| s.contains("/post")), "post-restore update is WAL-durable");
        assert!(!live2.iter().any(|(s, _, _)| s.contains("/orig")), "original never resurrects");
        let g_name = Term::NamedNode(NamedNode::new_unchecked("http://ex/g".to_string()));
        let named = reopened
            .named_graph(&g_name)
            .expect("the restored named graph survives the reopen");
        assert_eq!(named.len(), 1, "the named graph holds its one triple after reopen");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (review 1593) A durable CLEAR of a directory-backed graph must SURVIVE a
    /// reopen. The pre-fix CLEAR replaced the graph with a fresh in-memory empty graph,
    /// dropping the WAL/dir association, so the on-disk base was untouched and a reopen
    /// restored the cleared data. `clear_default_durable` retracts the triples through the
    /// WAL instead, so the emptiness is persistent.
    #[cfg(feature = "mmap")]
    #[test]
    fn clear_default_durable_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("sparq_clear_durable_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let mut nt = String::new();
        for i in 0..20u32 {
            nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"));
        }
        Graph::load_str(&nt, "ntriples").unwrap().save(&dir).unwrap();

        {
            let mut g = Graph::open(&dir).unwrap();
            assert_eq!(dump_terms(&g).len(), 20);
            g.clear_default_durable().unwrap();
            assert_eq!(dump_terms(&g).len(), 0, "clear empties the graph in memory");
            assert!(g.wal.is_some(), "the WAL/dir association is preserved");
        }
        // Reopen: the WAL-logged retractions replay, so the graph stays empty.
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(dump_terms(&g2).len(), 0, "durable CLEAR must survive the reopen");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-glw2) `clear_named_durable` empties an EXISTING named sub-graph durably
    /// (WAL-logged retraction) while PRESERVING its slot/dir/WAL — the named-graph twin of
    /// `clear_default_durable_survives_reopen`. The pre-fix engine path did `entry.1 =
    /// empty_graph()`, dropping the sub-graph's WAL so the clear survived only a compaction.
    #[cfg(feature = "mmap")]
    #[test]
    fn clear_named_durable_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("sparq_clear_named_durable_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_dataset(
            "<http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .\n\
             <http://ex/z> <http://ex/q> <http://ex/w> <http://ex/g1> .",
            "nquads",
        )
        .unwrap()
        .save(&dir)
        .unwrap();
        let g1 = |g: &Graph| g.named.iter().find(|(n, _)| n.to_string().contains("g1")).map(|(_, s)| s.len());

        {
            let mut g = Graph::open(&dir).unwrap();
            assert_eq!(g1(&g), Some(2));
            let cleared =
                g.clear_named_durable(&Term::NamedNode(NamedNode::new_unchecked("http://ex/g1"))).unwrap();
            assert!(cleared, "an existing graph reports cleared = true");
            assert_eq!(g1(&g), Some(0), "cleared in memory, slot kept");
            // A clear of an ABSENT graph is a no-op reporting false (CLEAR no-op-on-absent).
            assert!(!g
                .clear_named_durable(&Term::NamedNode(NamedNode::new_unchecked("http://ex/absent")))
                .unwrap());
        }
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g1(&g2), Some(0), "durable CLEAR of a named graph survives the reopen (slot present, empty)");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-glw2) `drop_named_durable` removes a named sub-graph's directory AND its
    /// manifest entry durably (so a reopen does NOT resurrect it), renumbers the surviving
    /// sub-tree to stay positional, and re-opens survivors with correctly-indexed WALs (a
    /// post-drop mutation of a survivor persists). The pre-fix engine path did
    /// `graph.named.retain(...)`, leaving `named/<i>/` + the manifest entry so a reopen brought
    /// the dropped graph back.
    #[cfg(feature = "mmap")]
    #[test]
    fn drop_named_durable_removes_subdir_and_manifest_and_renumbers() {
        let dir = std::env::temp_dir().join(format!("sparq_drop_named_durable_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_dataset(
            "<http://ex/a1> <http://ex/p> <http://ex/o> <http://ex/g1> .\n\
             <http://ex/a2> <http://ex/p> <http://ex/o> <http://ex/g2> .\n\
             <http://ex/a3> <http://ex/p> <http://ex/o> <http://ex/g3> .",
            "nquads",
        )
        .unwrap()
        .save(&dir)
        .unwrap();
        let nn = |s: &str| Term::NamedNode(NamedNode::new_unchecked(s));
        let count =
            |g: &Graph, frag: &str| g.named.iter().find(|(n, _)| n.to_string().contains(frag)).map(|(_, s)| s.len());

        {
            let mut g = Graph::open(&dir).unwrap();
            assert_eq!(g.named.len(), 3);
            // The on-disk sub-tree has 3 positional sub-dirs before the drop.
            assert!(dir.join("named").join("0").exists() && dir.join("named").join("2").exists());
            // Drop the MIDDLE graph; g3 must renumber down a slot.
            assert!(g.drop_named_durable(&nn("http://ex/g2")).unwrap(), "existing graph reports dropped = true");
            assert_eq!(g.named.len(), 2);
            assert_eq!(count(&g, "g2"), None);
            // The highest old sub-dir is gone (3 -> 2 sub-dirs after renumber).
            assert!(!dir.join("named").join("2").exists(), "drop renumbered the sub-tree to 2 contiguous slots");
            // A post-drop mutation of a survivor proves its WAL was re-indexed correctly.
            g.apply_delta_nquads("<http://ex/extra> <http://ex/p> <http://ex/o> <http://ex/g3> .", "").unwrap();
            assert_eq!(count(&g, "g3"), Some(2));
            // Dropping an ABSENT graph is a no-op reporting false (DROP no-op-on-absent).
            assert!(!g.drop_named_durable(&nn("http://ex/absent")).unwrap());
        }
        // Reopen: dropped graph stays gone; survivors keep their (renumbered) contents.
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.named.len(), 2, "exactly the two survivors after reopen");
        assert_eq!(count(&g2, "g2"), None, "dropped graph is NOT resurrected on reopen");
        assert_eq!(count(&g2, "g1"), Some(1));
        assert_eq!(count(&g2, "g3"), Some(2), "renumbered survivor keeps its original + post-drop triple");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-glw2) Dropping the LAST named graph removes the whole `named/` sub-tree
    /// and the `named.bin` manifest, so the directory persists as a clean default-graph-only
    /// store across a reopen.
    #[cfg(feature = "mmap")]
    #[test]
    fn drop_last_named_graph_clears_manifest_and_subtree() {
        let dir = std::env::temp_dir().join(format!("sparq_drop_last_named_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_dataset(
            "<http://ex/d> <http://ex/p> <http://ex/e> .\n\
             <http://ex/x> <http://ex/q> <http://ex/y> <http://ex/only> .",
            "nquads",
        )
        .unwrap()
        .save(&dir)
        .unwrap();
        assert!(dir.join("named.bin").exists() && dir.join("named").exists());
        {
            let mut g = Graph::open(&dir).unwrap();
            assert!(g
                .drop_named_durable(&Term::NamedNode(NamedNode::new_unchecked("http://ex/only")))
                .unwrap());
            assert_eq!(g.named.len(), 0);
        }
        assert!(!dir.join("named.bin").exists(), "manifest removed when the last named graph is dropped");
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.named.len(), 0, "no named graphs resurrected; default graph intact");
        assert_eq!(g2.len(), 1, "the default-graph triple survives");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-glw2, Copilot #123) The BATCH `drop_all_named_durable` (DROP NAMED / the
    /// named part of DROP ALL) removes EVERY named sub-graph and the manifest durably in ONE
    /// pass — no survivor renumbering — and leaves the directory as a clean default-only store
    /// across a reopen. Exercises the O(n) batch path on a directory-backed dataset with several
    /// named graphs: all gone after restart, default graph intact. (The old engine loop dropped
    /// them one at a time, O(n²); this asserts the single-rewrite batch path produces the same
    /// durable end-state.)
    #[cfg(feature = "mmap")]
    #[test]
    fn drop_all_named_durable_batch_survives_restart() {
        let dir = std::env::temp_dir().join(format!("sparq_drop_all_named_batch_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_dataset(
            "<http://ex/d> <http://ex/p> <http://ex/e> .\n\
             <http://ex/a1> <http://ex/p> <http://ex/o> <http://ex/g1> .\n\
             <http://ex/a2> <http://ex/p> <http://ex/o> <http://ex/g2> .\n\
             <http://ex/a3> <http://ex/p> <http://ex/o> <http://ex/g3> .\n\
             <http://ex/a4> <http://ex/p> <http://ex/o> <http://ex/g4> .",
            "nquads",
        )
        .unwrap()
        .save(&dir)
        .unwrap();
        // Four positional sub-dirs + manifest before the batch drop.
        assert!(dir.join("named.bin").exists());
        for i in 0..4 {
            assert!(dir.join("named").join(i.to_string()).exists(), "sub-dir {i} present before DROP ALL");
        }
        {
            let mut g = Graph::open(&dir).unwrap();
            assert_eq!(g.named.len(), 4);
            // ONE batch call drops all four (no per-graph renumber/manifest-rewrite loop).
            g.drop_all_named_durable().unwrap();
            assert_eq!(g.named.len(), 0, "every named graph gone in memory after the batch drop");
            // A second batch drop is an idempotent no-op (nothing left to remove).
            g.drop_all_named_durable().unwrap();
            assert_eq!(g.named.len(), 0);
        }
        // The whole named sub-tree + manifest are gone on disk (no staging siblings left behind).
        assert!(!dir.join("named.bin").exists(), "manifest removed by the batch drop");
        assert!(!dir.join("named").exists(), "named/ sub-tree removed by the batch drop");
        assert!(!dir.join("named.drop-new").exists() && !dir.join("named.drop-old").exists());
        // Restart: all named graphs stay gone; the default graph survives.
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.named.len(), 0, "no named graph resurrected after restart");
        assert_eq!(g2.len(), 1, "the default-graph triple survives the batch DROP ALL");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `load_str_with_base`: relative IRIs (subjects, predicates, objects, @prefix-free
    /// docs) resolve against the supplied base, in Turtle and TriG; an invalid base is
    /// a clean error; line-based formats ignore the base.
    #[test]
    fn load_str_with_base_resolves_relative_iris() {
        let g = Graph::load_str_with_base("<a> <p> <../up/o> .", "turtle", "http://ex/dir/").unwrap();
        let scan = g.store.scan(&[None, None, None]);
        let t = scan.to_spo(&scan.rows.as_ref()[0]);
        assert_eq!(g.dict.term(t[0]).to_string(), "<http://ex/dir/a>");
        assert_eq!(g.dict.term(t[1]).to_string(), "<http://ex/dir/p>");
        assert_eq!(g.dict.term(t[2]).to_string(), "<http://ex/up/o>");

        // A document's own @base still wins over the parameter (RFC 3986 layering).
        let g = Graph::load_str_with_base("@base <http://other/> . <a> <p> <o> .", "turtle", "http://ex/").unwrap();
        let scan = g.store.scan(&[None, None, None]);
        let t = scan.to_spo(&scan.rows.as_ref()[0]);
        assert_eq!(g.dict.term(t[0]).to_string(), "<http://other/a>");

        // TriG routes through the base-aware parser too (named graphs folded, as load_str).
        let g = Graph::load_str_with_base("<g> { <a> <p> <o> . }", "trig", "http://ex/").unwrap();
        assert_eq!(g.len(), 1);

        // N-Triples has no relative IRIs: base is accepted and ignored.
        let g = Graph::load_str_with_base("<http://ex/a> <http://ex/p> <http://ex/o> .", "ntriples", "http://b/")
            .unwrap();
        assert_eq!(g.len(), 1);

        assert!(Graph::load_str_with_base("<a> <p> <o> .", "turtle", "not a iri").is_err());
    }

    /// `Dict::iter` covers exactly the real ids `1..=len()` in order, with parts that
    /// reconstruct to the same terms as `term()`.
    #[test]
    fn dict_iter_is_dense_and_complete() {
        let g = Graph::load_str(
            "@prefix : <http://ex/> . :a :p \"lit\"@en . :a :q 99999999999 . _:b :p :a .",
            "turtle",
        )
        .unwrap();
        let d = &g.dict;
        let items: Vec<(Id, String)> = d.iter().map(|(id, _)| (id, d.term(id).to_string())).collect();
        assert_eq!(items.len(), d.len());
        assert_eq!(d.iter().len(), d.len(), "ExactSizeIterator must agree with len()");
        for (i, (id, _)) in items.iter().enumerate() {
            assert_eq!(*id, (i + 1) as Id, "ids must be dense and 1-based");
        }
        let all: Vec<String> = items.into_iter().map(|(_, t)| t).collect();
        assert!(all.iter().any(|t| t == "<http://ex/a>"));
        assert!(all.iter().any(|t| t == "\"lit\"@en"));
    }

    /// `Graph::iter_ids` / `iter_ids_sorted`: full coverage in canonical order, the
    /// requested sort column actually sorted, and the delta-overlay merged in.
    #[test]
    fn graph_iter_ids_orders_and_overlay() {
        let mut g = Graph::load_str(
            "@prefix : <http://ex/> . :b :p :x . :a :p :y . :a :q :x . :c :r 5 .",
            "turtle",
        )
        .unwrap();
        let spo: Vec<[Id; 3]> = g.iter_ids().collect();
        assert_eq!(spo.len(), g.len());
        assert!(spo.windows(2).all(|w| w[0] <= w[1]), "iter_ids must be (S,P,O)-sorted");
        // Same triple set through the object-sorted order.
        let mut by_obj: Vec<[Id; 3]> = g.iter_ids_sorted(2).collect();
        assert!(by_obj.windows(2).all(|w| w[0][2] <= w[1][2]), "col 2 must be sorted");
        let mut spo_sorted = spo.clone();
        spo_sorted.sort_unstable();
        by_obj.sort_unstable();
        assert_eq!(spo_sorted, by_obj);
        // Distinct subjects via run boundaries == 3 (:a :b :c).
        let mut subjects: Vec<Id> = spo.iter().map(|t| t[0]).collect();
        subjects.dedup();
        assert_eq!(subjects.len(), 3);
        // Overlay: an applied delta must appear in (and disappear from) the iterator.
        let t = |s: &str| Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{s}")));
        g.apply_delta(&[[t("z"), t("p"), t("x")]], &[]).unwrap();
        assert_eq!(g.iter_ids().len(), 5);
        assert_eq!(g.iter_ids().count(), 5);
        g.apply_delta(&[], &[[t("z"), t("p"), t("x")]]).unwrap();
        assert_eq!(g.iter_ids().count(), 4);
    }

    /// REGRESSION (sparq-py TODO): `Graph::save` on a graph whose numeric cache went
    /// SPARSE (`into_compressed`) must not panic — the dense cache is recomputed from
    /// the dictionary on save, and the round-trip preserves numeric query behaviour.
    #[cfg(feature = "mmap")]
    #[test]
    fn save_after_into_compressed_sparse_numerics() {
        let dir = std::env::temp_dir().join(format!("sparq_sparse_save_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let g = Graph::load_str(
            "@prefix : <http://ex/> . :a :v 1.5 . :b :v 2.5 . :c :p :a .",
            "turtle",
        )
        .unwrap()
        .into_compressed();
        g.save(&dir).unwrap();
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(g2.len(), 3);
        // The numeric cache must have round-tripped: 1.5 / 2.5 still resolve.
        let vals: Vec<f64> =
            g2.dict.iter().filter_map(|(id, _)| g2.numeric_value(id)).collect();
        assert!(vals.contains(&1.5) && vals.contains(&2.5), "numerics must survive: {vals:?}");
        // And save_compressed takes the same path.
        std::fs::remove_dir_all(&dir).ok();
        g.save_compressed(&dir).unwrap();
        assert_eq!(Graph::open(&dir).unwrap().len(), 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-7ph8) The streamed numerics/temporals save must write BYTE-IDENTICAL
    /// `numerics.bin`/`temporals.bin` to the old dense-materialise path — for a DENSE-owned
    /// cache, a SPARSE cache (`into_compressed`), AND a graph carrying temporal literals — so
    /// the bounded-RSS finalize is purely a memory optimisation, never an on-disk format change.
    /// We prove it by comparing the streamed files against a reference dense computation done
    /// directly off the dictionary, and by checking the exact on-disk sizes (`n*8`, `n*9`).
    #[cfg(feature = "mmap")]
    #[test]
    fn streamed_caches_byte_identical_to_dense() {
        // A mix: numeric literals, temporal literals (dateTime + date), and many plain IRIs/
        // strings so the caches are MOSTLY empty — the case that used to spike a dense buffer.
        let ttl = "@prefix : <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             :a :v 1.5 . :b :v 2.5 . :c :n \"42\"^^xsd:integer .\n\
             :d :t \"2020-01-02T03:04:05Z\"^^xsd:dateTime .\n\
             :e :t \"2021-06-07\"^^xsd:date .\n\
             :f :label \"just a string\" . :g :p :a . :h :p :b .";

        // Reference dense bytes computed straight from a dict (the pre-streaming layout):
        // `n` LE f64 numerics; then `n` LE f64 temporal instants followed by `n` flag bytes.
        let reference = |dict: &Dict| -> (Vec<u8>, Vec<u8>) {
            let n = dict.len();
            let mut num = Vec::new();
            let mut inst = Vec::new();
            let mut flags = Vec::new();
            for id in 1..=n as Id {
                num.extend_from_slice(&numeric_of(&dict.term(id)).to_le_bytes());
                match temporal_of_id(dict, id) {
                    Some(t) => {
                        inst.extend_from_slice(&t.instant.to_le_bytes());
                        flags.push(temp_flag(t));
                    }
                    None => {
                        inst.extend_from_slice(&f64::NAN.to_le_bytes());
                        flags.push(0);
                    }
                }
            }
            inst.extend_from_slice(&flags); // temporals.bin = instants || flags
            (num, inst)
        };

        for sparse in [false, true] {
            for compressed in [false, true] {
                let g = {
                    let g = Graph::load_str(ttl, "turtle").unwrap();
                    if sparse { g.into_compressed() } else { g }
                };
                let (want_num, want_temp) = reference(&g.dict);
                let dir = std::env::temp_dir()
                    .join(format!("sparq_stream_caches_{}_{sparse}_{compressed}", std::process::id()));
                std::fs::remove_dir_all(&dir).ok();
                if compressed {
                    g.save_compressed(&dir).unwrap();
                } else {
                    g.save(&dir).unwrap();
                }

                let got_num = std::fs::read(dir.join("numerics.bin")).unwrap();
                let got_temp = std::fs::read(dir.join("temporals.bin")).unwrap();
                let n = g.dict.len();
                assert_eq!(got_num.len(), n * 8, "numerics.bin size (sparse={sparse} compressed={compressed})");
                assert_eq!(got_temp.len(), n * 9, "temporals.bin size (sparse={sparse} compressed={compressed})");
                assert_eq!(got_num, want_num, "streamed numerics != dense (sparse={sparse} compressed={compressed})");
                assert_eq!(got_temp, want_temp, "streamed temporals != dense (sparse={sparse} compressed={compressed})");

                // And the caches still resolve after a re-open (mmap path).
                let g2 = Graph::open(&dir).unwrap();
                // 42 is an xsd:integer, inline-encoded into its id (never in numerics.bin); the
                // decimals 1.5/2.5 are the cache entries that must round-trip.
                let nums: Vec<f64> = g2.dict.iter().filter_map(|(id, _)| g2.numeric_value(id)).collect();
                assert!(nums.contains(&1.5) && nums.contains(&2.5), "numerics survive: {nums:?}");
                let temporal_count = g2.dict.iter().filter(|(id, _)| g2.temporal_value(*id).is_some()).count();
                assert_eq!(temporal_count, 2, "both temporal literals survive (sparse={sparse} compressed={compressed})");
                std::fs::remove_dir_all(&dir).ok();
            }
        }
    }

    /// [SONNET-4.6] (sq-7d3dj.6) The borrowed-path `numerics_of` must produce the SAME f64 cache
    /// as the old owned-path for every dictionary id — including numerics (xsd:decimal/double/float/
    /// integer-subtypes), temporals, plain IRIs, strings, and lang-tagged literals. This is the
    /// load-bearing invariant for the allocation-removal optimisation: the cache bytes are identical
    /// whether built via `dict.term(id)` (old) or `dict.term_parts(id)` (new borrowed path).
    #[test]
    fn numerics_of_borrowed_path_byte_identical_to_owned() {
        let ttl = "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
            @prefix ex: <http://ex/> .\n\
            ex:a ex:v \"1.5\"^^xsd:decimal .\n\
            ex:b ex:v \"2.5\"^^xsd:double .\n\
            ex:c ex:v \"3.0\"^^xsd:float .\n\
            ex:d ex:v \"42\"^^xsd:integer .\n\
            ex:e ex:v \"100\"^^xsd:long .\n\
            ex:f ex:v \"hello\" .\n\
            ex:g ex:v \"bonjour\"@fr .\n\
            ex:h ex:v ex:iri .\n\
            ex:i ex:v \"2020-01-01T00:00:00Z\"^^xsd:dateTime .";
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let dict = &g.dict;
        let n = dict.len();
        // Owned-path reference: the old `numerics_of` logic byte-for-byte.
        let owned: Vec<f64> = (0..n).map(|i| numeric_of(&dict.term(i as Id + 1))).collect();
        // Borrowed-path: current `numerics_of`.
        let borrowed = numerics_of(dict);
        assert_eq!(owned.len(), borrowed.len(), "length mismatch");
        for (idx, (a, b)) in owned.iter().zip(borrowed.iter()).enumerate() {
            let id = idx as Id + 1;
            // Compare as bits: NaN == NaN in the cache (we want byte-identical, not IEEE equality).
            assert_eq!(
                a.to_bits(), b.to_bits(),
                "id {}: owned bits {:016x} != borrowed bits {:016x}",
                id, a.to_bits(), b.to_bits()
            );
        }
    }

    /// [FABLE-5] (sq-9781x) Direct unit test for the shared public `parse_xsd_f64`: the XSD
    /// lexical space (specials `NaN`/`INF`/`+INF`/`-INF`), the Rust-only spellings it MUST
    /// reject, ordinary decimals/exponents, and untrimmed padding (this fn does NOT trim —
    /// callers trim). Kept in lock-step with the substrate re-export's own test.
    #[test]
    fn parse_xsd_f64_shared_acceptance_set() {
        assert_eq!(parse_xsd_f64("NaN").map(f64::to_bits), Some(f64::NAN.to_bits()));
        assert_eq!(parse_xsd_f64("INF"), Some(f64::INFINITY));
        assert_eq!(parse_xsd_f64("+INF"), Some(f64::INFINITY));
        assert_eq!(parse_xsd_f64("-INF"), Some(f64::NEG_INFINITY));
        assert_eq!(parse_xsd_f64("6"), Some(6.0));
        assert_eq!(parse_xsd_f64("+1"), Some(1.0));
        assert_eq!(parse_xsd_f64("1.5E2"), Some(150.0));
        assert_eq!(parse_xsd_f64("-0.0").map(f64::to_bits), Some((-0.0f64).to_bits()));
        // Rust-`FromStr`-only spellings the XSD lexical space forbids: rejected.
        for bad in ["inf", "+inf", "-inf", "infinity", "-infinity", "Infinity", "nan", "NAN"] {
            // Positional arg (not inline `{bad}`) to dodge the CodeQL false positive. [FABLE-5]
            assert_eq!(parse_xsd_f64(bad), None, "must reject {:?}", bad);
        }
        // Ill-formed / non-numeric: rejected.
        for bad in ["", "abc", "1_000", "0x1p4", " 1", "1 "] {
            assert_eq!(parse_xsd_f64(bad), None, "must reject {:?}", bad);
        }
    }

    /// [FABLE-5] (sq-74oy4 / sq-6b1lj) DIRECT unit test of the public DATATYPE-AWARE cache
    /// acceptance `numeric_cache_value` (and thereby `cached_numeric_f64` /
    /// `numeric_datatype_wellformed`): trims, per-datatype well-formedness, and the exact
    /// scale-0 integer rule (`"5."`/`"5.0"` accepted as integers, `"5.5"` not). This is the
    /// in-crate coverage anchor for the new fns; the CROSS-crate agreement with
    /// `Num::of_literal` is pinned in `sparq-substrate`'s `cache_f64_seam_vs_as_numeric_differential`.
    #[test]
    fn numeric_cache_value_datatype_aware_acceptance() {
        let xi = xsd::INTEGER.as_str();
        let xd = xsd::DECIMAL.as_str();
        let xdbl = xsd::DOUBLE.as_str();
        let xf = xsd::FLOAT.as_str();
        // integers: scale-0 (after trailing-zero normalisation), i128-fit, trimmed.
        assert_eq!(numeric_cache_value("5", xi), Some(5.0));
        assert_eq!(numeric_cache_value(" 5 ", xi), Some(5.0)); // XSD collapse: trimmed
        assert_eq!(numeric_cache_value("+7", xi), Some(7.0));
        assert_eq!(numeric_cache_value("5.", xi), Some(5.0)); // trailing dot, no fraction
        assert_eq!(numeric_cache_value("5.0", xi), Some(5.0)); // trailing-zero fraction
        assert_eq!(numeric_cache_value("5.5", xi), None); // fraction on an integer
        assert_eq!(numeric_cache_value(".5", xi), None); // no integer part, scale 1
        assert_eq!(numeric_cache_value("1E2", xi), None); // exponent on an integer
        assert_eq!(numeric_cache_value("9999999999999999999999999999999999999999", xi), None); // >i128
        // decimals: any scale, no exponent, i128-fit.
        assert_eq!(numeric_cache_value("1.5", xd), Some(1.5));
        assert_eq!(numeric_cache_value(".5", xd), Some(0.5));
        assert_eq!(numeric_cache_value("007.50", xd), Some(7.5));
        assert_eq!(numeric_cache_value("1E2", xd), None); // exponent on a decimal
        assert_eq!(numeric_cache_value("9999999999999999999999999999999999999999.5", xd), None);
        // float/double: XSD doubleRep spellings; genuine NaN reads back as a cache MISS.
        assert_eq!(numeric_cache_value("1.5E2", xdbl), Some(150.0));
        assert_eq!(numeric_cache_value("INF", xdbl), Some(f64::INFINITY));
        assert_eq!(numeric_cache_value("3.0", xf), Some(3.0));
        assert_eq!(numeric_cache_value("inf", xdbl), None); // Rust-only spelling
        assert_eq!(numeric_cache_value("NaN", xdbl), None); // genuine NaN -> sentinel miss
        // non-numeric datatype: always None.
        assert_eq!(numeric_cache_value("5", xsd::STRING.as_str()), None);
    }

    /// [FABLE-5] (sq-9781x / sq-6b1lj) PLUMBING check that the numeric-value cache's decision
    /// matches the cache's OWN documented acceptance function `numeric_cache_value` (the
    /// DATATYPE-AWARE gate over the shared `parse_xsd_f64`): a cache HIT
    /// (`numeric_value(id).is_some()`) is EQUIVALENT to `numeric_cache_value(value, datatype)`
    /// being `Some`, and the cached f64 is EXACTLY that value. Because the model here is the
    /// SAME function the cache calls, this pins the wiring (build path ⟺ the acceptance fn),
    /// not the cross-crate agreement with `Num::of_literal` — that TRUE differential is
    /// `sparq_substrate::numeric` `cache_f64_seam_vs_as_numeric_differential`. A stored-NaN
    /// value folds to a cache MISS (the cache uses NaN as its "not cached" sentinel, so a
    /// genuine `NaN`^^xsd:double falls through to the evaluator). Covers whitespace padding,
    /// leading `+`, the XSD specials, the Rust-only spellings, exponent forms, empty/`abc`,
    /// high-precision decimals, `2^53 ± 1`, and the sq-6b1lj per-datatype-ill-formed lexicals.
    #[test]
    fn numeric_cache_hit_matches_shared_parse_xsd_f64_plumbing() {
        // (lexical, datatype-suffix). Each becomes one distinct dictionary literal.
        let cases: &[(&str, &str)] = &[
            (" 1", "xsd:integer"),      // whitespace-padded (leading) — trim-then-parse
            ("1 ", "xsd:integer"),      // whitespace-padded (trailing)
            ("\t2\n", "xsd:integer"),   // other whitespace
            ("+3", "xsd:integer"),      // leading +
            ("INF", "xsd:double"),      // XSD special
            ("-INF", "xsd:double"),     // XSD special
            ("+INF", "xsd:double"),     // XSD special
            ("NaN", "xsd:double"),      // XSD special -> NaN value -> cache MISS (sentinel)
            ("inf", "xsd:double"),      // Rust-only spelling -> REJECTED
            ("infinity", "xsd:double"), // Rust-only spelling -> REJECTED
            ("Infinity", "xsd:double"), // Rust-only spelling -> REJECTED
            ("nan", "xsd:double"),      // Rust-only spelling -> REJECTED
            ("1.5E2", "xsd:double"),    // exponent
            ("-2.5e-1", "xsd:double"),  // exponent, negative
            ("abc", "xsd:integer"),     // not a number -> REJECTED
            ("", "xsd:integer"),        // empty -> REJECTED
            ("1.000000000000000001", "xsd:decimal"), // high-precision decimal
            ("9007199254740993", "xsd:integer"),     // 2^53 + 1
            ("9007199254740991", "xsd:integer"),     // 2^53 - 1
            ("3.0", "xsd:float"),       // ordinary float
            ("007.50", "xsd:decimal"),  // leading zeros
            // sq-6b1lj: datatype-ill-formed lexicals -> cache MISS (of_literal type-errors).
            ("1.5", "xsd:integer"),     // fraction on an integer -> REJECTED
            ("1E2", "xsd:decimal"),     // exponent on a decimal -> REJECTED
            ("1E2", "xsd:integer"),     // exponent on an integer -> REJECTED
        ];
        let mut ttl = String::from(
            "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n@prefix ex: <http://ex/> .\n",
        );
        // Use a fresh subject per case so every literal is a distinct object term (equal
        // (lexical, datatype) pairs would intern to one id, but ours are all distinct).
        for (i, (lex, dt)) in cases.iter().enumerate() {
            // Escape the lexical for a Turtle string literal (only \, ", \t, \n occur here).
            let esc: String = lex
                .chars()
                .flat_map(|c| match c {
                    '\\' => vec!['\\', '\\'],
                    '"' => vec!['\\', '"'],
                    '\t' => vec!['\\', 't'],
                    '\n' => vec!['\\', 'n'],
                    other => vec![other],
                })
                .collect();
            ttl.push_str(&format!("ex:s{} ex:v \"{}\"^^{} .\n", i, esc, dt));
        }
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        // For each object literal, compare the cache decision to the evaluator model.
        for (id, tp) in g.dict.iter() {
            let dict::TermParts::Lit { value, datatype, lang: None } = tp else { continue };
            if !is_numeric_datatype_str(datatype) {
                continue;
            }
            // Model: the cache's OWN datatype-aware acceptance (`numeric_cache_value`), which
            // trims, gates on per-datatype well-formedness, and folds a NaN value to a miss.
            let model = numeric_cache_value(value, datatype);
            let cached = g.numeric_value(id);
            match (model, cached) {
                (Some(m), Some(c)) => assert_eq!(
                    m.to_bits(),
                    c.to_bits(),
                    "id {}: lexical {:?} cache f64 {:016x} != evaluator f64 {:016x}",
                    id,
                    value,
                    c.to_bits(),
                    m.to_bits()
                ),
                (None, None) => {}
                (m, c) => panic!(
                    "cache/evaluator disagree on acceptance for lexical {:?}: model={:?} cache={:?}",
                    value,
                    m,
                    c
                ),
            }
        }
    }

    /// [FABLE-5] (sq-9781x) The specific latent bugs the alignment fixes, asserted directly
    /// against the pre-fix raw `str::parse::<f64>` behaviour: (a) a whitespace-padded numeric
    /// literal now HITS the cache (was a miss); (b) a Rust-only `inf`/`nan` spelling now MISSES
    /// the cache (the raw parse wrongly cached `inf`/`NaN`), matching the evaluator's rejection.
    #[test]
    fn numeric_cache_alignment_fixes_both_divergences() {
        let ttl = "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
            @prefix ex: <http://ex/> .\n\
            ex:pad  ex:v \" 7 \"^^xsd:decimal .\n\
            ex:inf  ex:v \"inf\"^^xsd:double .\n\
            ex:nan  ex:v \"nan\"^^xsd:double .\n\
            ex:good ex:v \"INF\"^^xsd:double .";
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let by_val = |needle: &str| -> Option<f64> {
            g.dict.iter().find_map(|(id, tp)| match tp {
                dict::TermParts::Lit { value, .. } if value == needle => Some(g.numeric_value(id)),
                _ => None,
            }).flatten()
        };
        // (a) previously a raw-parse MISS, now a value-7.0 HIT (trim-then-parse).
        assert_eq!(by_val(" 7 "), Some(7.0), "padded decimal must now hit the cache");
        // (b) previously a raw-parse HIT storing inf/NaN, now a MISS (XSD rejects the spelling).
        assert_eq!(by_val("inf"), None, "'inf'^^xsd:double must NOT hit the cache");
        assert_eq!(by_val("nan"), None, "'nan'^^xsd:double must NOT hit the cache");
        // The genuine XSD spelling still hits with the infinity value.
        assert_eq!(by_val("INF"), Some(f64::INFINITY), "'INF'^^xsd:double still hits");
    }

    // [SONNET-4.6] Exercise the outlined storage dispatch directly so the coverage
    // ratchet sees every non-mmap cache shape, not only the dominant owned fast path.
    #[test]
    fn numeric_cache_lookup_covers_outlined_storage_variants() {
        let mut sparse_values = rustc_hash::FxHashMap::default();
        sparse_values.insert(2, 2.5);
        let sparse = NumData::Sparse(sparse_values);
        assert_eq!(sparse.lookup(2), Some(2.5));
        assert_eq!(sparse.lookup(1), None);

        let base = std::sync::Arc::new(NumData::Owned(vec![1.5, f64::NAN]));
        let mut extra = rustc_hash::FxHashMap::default();
        extra.insert(3, 3.5);
        let forked = NumData::Forked { base, extra };
        assert_eq!(forked.lookup(1), Some(1.5));
        assert_eq!(forked.lookup(2), None);
        assert_eq!(forked.lookup(3), Some(3.5));
    }

    /// [OPUS-4.8] (sq-x32t) Recursively reads every regular file under `dir` and returns true iff
    /// `needle`'s bytes appear in ANY of them — used to prove a deleted term's bytes are PHYSICALLY
    /// gone from the on-disk store (not merely logically hidden by the overlay).
    #[cfg(feature = "mmap")]
    fn on_disk_contains(dir: &std::path::Path, needle: &[u8]) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if on_disk_contains(&p, needle) {
                    return true;
                }
            } else if let Ok(bytes) = std::fs::read(&p) {
                if bytes.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
            }
        }
        false
    }

    /// [OPUS-4.8] (sq-x32t, epic sq-toze.33) ERASURE-COMPLETENESS — the load-bearing test. After
    /// INSERT + DELETE + DROP GRAPH, a `vacuum` (the admin WAL compact/vacuum) must (1) preserve the
    /// LIVE triple set exactly (round-trip), and (2) physically REMOVE the erased data's bytes from
    /// the on-disk store — including a deleted LITERAL VALUE that is no longer referenced (the
    /// dictionary purge), not just hide it behind the delta-overlay. We prove (2) by asserting the
    /// distinctive deleted bytes are present on disk BEFORE the vacuum (so the test can actually
    /// fail) and ABSENT after, and by re-opening to confirm durability.
    #[cfg(feature = "mmap")]
    #[test]
    fn vacuum_erases_deleted_data_bytes_on_disk() {
        let dir = std::env::temp_dir().join(format!("sparq_vacuum_erase_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        // Seed: a default-graph triple to KEEP, a default-graph triple whose object literal will be
        // DELETED, and a NAMED graph that will be DROPped. Distinctive byte markers per datum.
        let g0 = Graph::load_dataset(
            "<http://ex/keep> <http://ex/p> \"KEEP-ME-LIVE-MARKER\" .\n\
             <http://ex/alice> <http://ex/ssn> \"SECRET-SSN-555-00-1234\" .\n\
             <http://ex/n1> <http://ex/p> \"NAMED-GRAPH-DROP-MARKER\" <http://ex/secretgraph> .",
            "nquads",
        )
        .unwrap();
        g0.save(&dir).unwrap();

        // The marked secrets are on disk to begin with (dictionary blob holds the literal values).
        assert!(on_disk_contains(&dir, b"SECRET-SSN-555-00-1234"), "deleted-literal seed must be on disk first");
        assert!(on_disk_contains(&dir, b"NAMED-GRAPH-DROP-MARKER"), "dropped-graph seed must be on disk first");

        {
            let mut g = Graph::open(&dir).unwrap();
            // Logical erasure: DELETE the SSN triple and DROP the named graph (via retraction).
            g.apply_delta(
                &[],
                &[[
                    Term::NamedNode(NamedNode::new_unchecked("http://ex/alice")),
                    Term::NamedNode(NamedNode::new_unchecked("http://ex/ssn")),
                    Term::Literal(Literal::new_simple_literal("SECRET-SSN-555-00-1234")),
                ]],
            )
            .unwrap();
            g.drop_named_durable(&named("secretgraph")).unwrap();

            // Logically hidden — but the WAL/dict bytes still hold the secrets pre-vacuum.
            assert_eq!(dump_terms(&g).len(), 1, "only the KEEP triple remains live");
            assert!(on_disk_contains(&dir, b"SECRET-SSN-555-00-1234"), "pre-vacuum: still on disk (logical only)");

            // VACUUM: physically rewrite the store + purge the dictionary.
            g.vacuum().unwrap();

            // Round-trip: the live set is preserved exactly.
            let live = dump_terms(&g);
            assert_eq!(live.len(), 1);
            assert_eq!(live[0].0, "<http://ex/keep>");
            assert!(g.named.is_empty(), "dropped named graph is gone");
        }

        // The KEEP marker survives; the erased data's bytes are PHYSICALLY GONE from disk.
        assert!(on_disk_contains(&dir, b"KEEP-ME-LIVE-MARKER"), "live data must survive the vacuum");
        assert!(!on_disk_contains(&dir, b"SECRET-SSN-555-00-1234"), "DELETED literal value bytes must be physically erased");
        assert!(!on_disk_contains(&dir, b"NAMED-GRAPH-DROP-MARKER"), "DROPped-graph data bytes must be physically erased");

        // Durability: a fresh open sees exactly the live triple and nothing resurrects.
        let g2 = Graph::open(&dir).unwrap();
        assert_eq!(dump_terms(&g2).len(), 1, "erasure survives reopen");
        assert!(g2.named.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-x32t) `vacuum` ROUND-TRIP equality across the whole dataset (default + named),
    /// independent of the byte-level erasure check: the post-vacuum quad set must EQUAL the pre-vacuum
    /// live quad set exactly, and survive a reopen.
    #[cfg(feature = "mmap")]
    #[test]
    fn vacuum_preserves_live_dataset_exactly() {
        let dir = std::env::temp_dir().join(format!("sparq_vacuum_roundtrip_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let mut nq = String::new();
        for i in 0..40u32 {
            nq.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"));
            nq.push_str(&format!("<http://ex/s{i}> <http://ex/q> <http://ex/o{i}> <http://ex/g{}> .\n", i % 3));
        }
        Graph::load_dataset(&nq, "nquads").unwrap().save(&dir).unwrap();

        let mut g = Graph::open(&dir).unwrap();
        // Mutate (insert + delete + drop a graph) so the overlay is non-trivial before vacuum.
        g.apply_delta(
            &[[
                Term::NamedNode(NamedNode::new_unchecked("http://ex/new")),
                Term::NamedNode(NamedNode::new_unchecked("http://ex/p")),
                Term::NamedNode(NamedNode::new_unchecked("http://ex/v")),
            ]],
            &[[
                Term::NamedNode(NamedNode::new_unchecked("http://ex/s0")),
                Term::NamedNode(NamedNode::new_unchecked("http://ex/p")),
                Term::NamedNode(NamedNode::new_unchecked("http://ex/o0")),
            ]],
        )
        .unwrap();
        g.drop_named_durable(&named("g0")).unwrap();

        let before = dump_quads(&g);
        g.vacuum().unwrap();
        let after = dump_quads(&g);
        assert_eq!(before, after, "vacuum must preserve the live dataset exactly (default + named)");

        let reopened = dump_quads(&Graph::open(&dir).unwrap());
        assert_eq!(before, reopened, "the vacuumed dataset must survive a reopen unchanged");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-x32t) CRASH-SAFETY of `vacuum`: it uses the SAME rollback-safe directory swap
    /// (`<dir>.compact-new` / `<dir>.compact-old` siblings, healed by `recover_compaction` on open)
    /// as `compact`, so an interrupted vacuum must never lose or corrupt the store. We simulate the
    /// two crash windows directly on the sibling dirs and assert the open recovers a VALID store.
    #[cfg(feature = "mmap")]
    #[test]
    fn vacuum_swap_crash_recovery() {
        let base = std::env::temp_dir().join(format!("sparq_vacuum_recover_{}", std::process::id()));
        let dir = base.join("g");
        let build = |d: &std::path::Path, tag: &str| {
            std::fs::remove_dir_all(d).ok();
            Graph::load_str(&format!("<http://ex/{tag}> <http://ex/p> <http://ex/o> ."), "ntriples")
                .unwrap()
                .save(d)
                .unwrap();
        };
        // (a) Crash BETWEEN the two renames (dir missing, both siblings present): the open must
        //     COMPLETE the swap by promoting the fully-synced new base.
        build(&dir.with_extension("compact-old"), "old");
        build(&dir.with_extension("compact-new"), "new");
        assert!(!dir.exists());
        let g = Graph::open(&dir).unwrap();
        assert!(dump_terms(&g).iter().any(|(s, _, _)| s.contains("/new")), "must promote the new base");
        assert!(!dir.with_extension("compact-new").exists() && !dir.with_extension("compact-old").exists());
        // (b) Crash BEFORE the new base was ready (dir missing, only old present): roll back.
        std::fs::remove_dir_all(&dir).ok();
        build(&dir.with_extension("compact-old"), "old");
        assert!(!dir.exists());
        let g = Graph::open(&dir).unwrap();
        assert!(dump_terms(&g).iter().any(|(s, _, _)| s.contains("/old")), "must roll back to the old base");
        std::fs::remove_dir_all(&base).ok();
    }

    // ---- [OPUS-4.8] gh-1118 / gh-1122: ergonomic constructors + single-triple mutation ----

    /// gh-1118: `Graph::new()` and `Graph::default()` both yield an EMPTY in-memory graph
    /// (no error handling, no `load_str("", …)` workaround) and are interchangeable.
    #[test]
    fn new_and_default_are_empty_graphs() {
        let n = Graph::new();
        assert!(n.is_empty(), "Graph::new() must be empty");
        assert_eq!(n.len(), 0, "Graph::new() must have zero triples");
        assert!(n.named.is_empty(), "Graph::new() must have no named graphs");
        let d = Graph::default();
        assert!(d.is_empty(), "Graph::default() must be empty");
        assert_eq!(d.len(), 0, "Graph::default() must have zero triples");
        // An empty graph is immediately usable for incremental build-up.
        let mut g = Graph::new();
        g.insert_triple(
            NamedNode::new_unchecked("http://ex/s"),
            NamedNode::new_unchecked("http://ex/p"),
            NamedNode::new_unchecked("http://ex/o"),
        )
        .unwrap();
        assert_eq!(g.len(), 1);
    }

    /// gh-1122: `insert_triple` takes oxrdf terms directly, makes the triple queryable
    /// (REAL overlay path — `id_of` resolves the interned terms and a pattern scan finds the
    /// row), and is SET-VALUED — re-inserting the same triple is a no-op, not a duplicate.
    #[test]
    fn insert_triple_interns_and_is_set_valued() {
        let mut g = Graph::new();
        let s = NamedNode::new_unchecked("http://ex/alice");
        let p = NamedNode::new_unchecked("http://schema.org/age");
        let o = Term::Literal(Literal::new_typed_literal("30", xsd::INTEGER));
        // Mixed term kinds: NamedNode (subject), NamedNode (predicate), Literal (object).
        g.insert_triple(s.clone(), p.clone(), o.clone()).unwrap();
        assert_eq!(g.len(), 1);
        // Real path: the terms are interned, so the pattern scan finds exactly this row.
        let pat = g
            .pattern(Some(&Term::NamedNode(s.clone())), Some(&p), Some(&o))
            .expect("all three terms interned");
        assert_eq!(g.store.scan(&pat).rows.len(), 1, "the inserted triple must be queryable");
        // Set semantics: re-inserting the identical triple does not grow the graph.
        g.insert_triple(s, p, o).unwrap();
        assert_eq!(g.len(), 1, "re-inserting an existing triple is a no-op");
    }

    /// gh-1122: `remove_triple` retracts a present triple and is a NO-OP (never an error)
    /// for an absent one — the retraction twin of `insert_triple`.
    #[test]
    fn remove_triple_retracts_and_absent_is_noop() {
        let mut g = Graph::new();
        let s = NamedNode::new_unchecked("http://ex/s");
        let p = NamedNode::new_unchecked("http://ex/p");
        let o = Term::NamedNode(NamedNode::new_unchecked("http://ex/o"));
        let other = Term::NamedNode(NamedNode::new_unchecked("http://ex/never"));
        g.insert_triple(s.clone(), p.clone(), o.clone()).unwrap();
        assert_eq!(g.len(), 1);
        // Removing an absent triple is a no-op (the object term is not even in the dict).
        g.remove_triple(s.clone(), p.clone(), other).unwrap();
        assert_eq!(g.len(), 1, "removing an absent triple must not change the graph");
        // Removing the present triple retracts it.
        g.remove_triple(s, p, o).unwrap();
        assert!(g.is_empty(), "removing the present triple must empty the graph");
    }

    /// gh-1122: the load-bearing invariant — a graph built with `Graph::new()` +
    /// `insert_triple` is RESULT-EQUIVALENT to the same triples parsed from Turtle text
    /// (same materialised term set), so the convenience API is not a separate semantics.
    #[test]
    fn insert_triple_equivalent_to_text_load() {
        let mut built = Graph::new();
        let s = NamedNode::new_unchecked("http://ex/alice");
        built
            .insert_triple(
                s.clone(),
                NamedNode::new_unchecked("http://ex/name"),
                Term::Literal(Literal::new_simple_literal("Alice")),
            )
            .unwrap();
        built
            .insert_triple(
                s,
                NamedNode::new_unchecked("http://ex/knows"),
                NamedNode::new_unchecked("http://ex/bob"),
            )
            .unwrap();
        let loaded = Graph::load_str(
            "<http://ex/alice> <http://ex/name> \"Alice\" .\n\
             <http://ex/alice> <http://ex/knows> <http://ex/bob> .",
            "ntriples",
        )
        .unwrap();
        let mut a = dump_terms(&built);
        let mut b = dump_terms(&loaded);
        a.sort();
        b.sort();
        assert_eq!(a, b, "insert_triple build must match the text-loaded graph");
    }
}

#[cfg(test)]
mod dir_roundtrip_test {
    /// `apply_delta_nquads` routes per graph: default-graph lines to the main graph,
    /// named-graph lines to the matching sub-graph (auto-created on first insert),
    /// deletes-before-inserts, bnodes matched by label, absent-graph deletes a no-op.
    #[test]
    fn apply_delta_nquads_routes_per_graph() {
        use oxrdf::{Literal, NamedNode, Term};
        let mut g = crate::Graph::load_dataset(
            "<http://ex/a> <http://ex/p> \"keep\" .\n\
             <http://ex/a> <http://ex/p> \"drop\" .\n\
             _:b <http://ex/p> \"bnode\" .\n\
             <http://ex/x> <http://ex/p> \"g1-drop\" <http://ex/g1> .",
            "nquads",
        )
        .unwrap();
        g.apply_delta_nquads(
            // inserts: default graph, existing named graph, and a NEW named graph
            "<http://ex/a> <http://ex/p> \"new\" .\n\
             <http://ex/x> <http://ex/p> \"g1-new\" <http://ex/g1> .\n\
             <http://ex/y> <http://ex/p> \"g2-new\" <http://ex/g2> .",
            // deletes: default-graph literal, a bnode triple BY LABEL, and one
            // against an absent graph (must be a no-op, not an error)
            "<http://ex/a> <http://ex/p> \"drop\" .\n\
             _:b <http://ex/p> \"bnode\" .\n\
             <http://ex/x> <http://ex/p> \"g1-drop\" <http://ex/g1> .\n\
             <http://ex/z> <http://ex/p> \"nope\" <http://ex/absent> .",
        )
        .unwrap();
        let lit = |v: &str| Term::Literal(Literal::new_simple_literal(v));
        assert_eq!(g.len(), 2); // keep + new (drop and the bnode triple retracted)
        assert!(g.id_of(&lit("keep")).is_some() && g.id_of(&lit("new")).is_some());
        let named = |g: &crate::Graph, name: &str| {
            let name = Term::NamedNode(NamedNode::new_unchecked(name));
            g.named.iter().find(|(n, _)| *n == name).map(|(_, sub)| sub.len())
        };
        assert_eq!(named(&g, "http://ex/g1"), Some(1)); // g1-drop out, g1-new in
        assert_eq!(named(&g, "http://ex/g2"), Some(1)); // auto-created
        assert_eq!(named(&g, "http://ex/absent"), None); // delete-only: never created
    }

    #[test]
    fn dir_literal_roundtrip() {
        let g = crate::Graph::load_str("@prefix : <http://ex/> . :a :p \"abc\"@en--ltr .", "turtle").unwrap();
        let scan = g.store.scan(&[None, None, None]);
        let t = scan.to_spo(&scan.rows.as_ref()[0]);
        assert_eq!(g.dict.term(t[2]).to_string(), "\"abc\"@en--ltr");
    }

    /// [OPUS-4.8] (sq-langcase / KamiQuasi #1119) Cross-format CONSISTENCY: loading the SAME
    /// language-tagged triple as N-Triples, N-Quads, and Turtle must yield the SAME stored term —
    /// a lowercased tag (`en-us`), not the format-dependent `en-US` (N-Triples) / `en-us` (Turtle)
    /// split that existed before the byte parser was taught to normalise the tag. This is the
    /// public-API regression for the reported bug: pick the literal object back out via the store
    /// scan and compare its serialised form across all three load paths.
    #[test]
    fn language_tag_casing_consistent_across_formats() {
        let only_obj = |g: &crate::Graph| -> String {
            let scan = g.store.scan(&[None, None, None]);
            let t = scan.to_spo(&scan.rows.as_ref()[0]);
            g.dict.term(t[2]).to_string()
        };
        let nt = crate::Graph::load_str("<http://ex/s> <http://ex/p> \"hi\"@en-US .\n", "ntriples").unwrap();
        let ttl = crate::Graph::load_str("<http://ex/s> <http://ex/p> \"hi\"@en-US .\n", "turtle").unwrap();
        let nq = crate::Graph::load_str("<http://ex/s> <http://ex/p> \"hi\"@en-US .\n", "nquads").unwrap();
        assert_eq!(only_obj(&nt), "\"hi\"@en-us", "N-Triples must lowercase the language tag");
        assert_eq!(only_obj(&ttl), "\"hi\"@en-us", "Turtle must lowercase the language tag");
        assert_eq!(only_obj(&nq), "\"hi\"@en-us", "N-Quads must lowercase the language tag");
        // The three load paths agree (the format-dependent split is gone).
        assert_eq!(only_obj(&nt), only_obj(&ttl));
        assert_eq!(only_obj(&nt), only_obj(&nq));
    }

    /// Fork-after-compact must stay O(delta): compaction folds the numeric/temporal
    /// caches into a FRESH shared base but keeps them in the shareable `Forked`
    /// shape (Arc base + empty side map) and re-freezes the dict — so the next
    /// `fork()` is Arc bumps, never an O(dict) cache/dict re-freeze. (Roborev
    /// finding on the initial fold-to-flat implementation.)
    #[test]
    fn compact_keeps_caches_shareable_for_the_next_fork() {
        use crate::{Graph, NumData, TempData};
        use oxrdf::vocab::xsd;
        use oxrdf::{Literal, NamedNode, Term};
        let g = Graph::load_str(
            "@prefix : <http://ex/> . :a :age 30 . :b :p :c .",
            "turtle",
        )
        .unwrap();
        let mut f = g.fork();
        let lit = |v: &str| Term::Literal(Literal::new_typed_literal(v, xsd::DECIMAL));
        let iri = |s: &str| Term::NamedNode(NamedNode::new_unchecked(s));
        f.apply_delta(&[[iri("http://ex/n"), iri("http://ex/age"), lit("4.5")]], &[]).unwrap();
        f.compact().unwrap();
        assert_eq!(f.pending_delta_len(), 0);
        // Caches stay in the shareable Forked shape with an EMPTY side map…
        assert!(
            matches!(&f.numerics, NumData::Forked { extra, .. } if extra.is_empty()),
            "numeric cache must be folded into a fresh shared base"
        );
        assert!(
            matches!(&f.temporals, TempData::Forked { extra, .. } if extra.is_empty()),
            "temporal cache must be folded into a fresh shared base"
        );
        // …and still answer: the folded base covers the post-fork term.
        let id = f.id_of(&lit("4.5")).unwrap();
        assert_eq!(f.numeric_value(id), Some(4.5));
        // The next fork shares those bases (no re-freeze) and keeps working.
        let f2 = f.fork();
        let same_base = match (&f.numerics, &f2.numerics) {
            (NumData::Forked { base: a, .. }, NumData::Forked { base: b, .. }) => {
                std::sync::Arc::ptr_eq(a, b)
            }
            _ => false,
        };
        assert!(same_base, "fork after compact must share the folded cache base");
        assert_eq!(f2.numeric_value(id), Some(4.5));
    }
}
