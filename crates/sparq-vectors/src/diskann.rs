//! A **persistent, on-disk Vamana ANN index** (`.spqg`) over a [`VectorStore`]:
//! a fixed-degree proximity graph laid out for memory-mapped, locality-friendly reads, so a
//! built index is **saved once and reopened without a rebuild** (the gap [`ann`](crate::ann)'s
//! in-RAM HNSW left — see that module's persistence note and this crate's open beads
//! (`bd list -l area:sparq-vectors`)).
//!
//! # Why a second index
//!
//! `VectorIndex` wraps `instant-distance`'s HNSW, which is rebuilt
//! from the mmap'd store on every open (~33 s release for 50k×32 on an M1 — see the README
//! throughput table). `instant-distance` is a closed graph: its adjacency is not exposed, so
//! it cannot be laid out on disk. This index is therefore **self-contained** — we build the
//! Vamana graph ourselves and own its on-disk encoding end to end.
//!
//! # Algorithm (Vamana, the DiskANN graph)
//!
//! Build (in RAM, once): start from a random `R`-regular graph, then for each node run a
//! greedy search from a fixed medoid to collect visited candidates and `RobustPrune` them to
//! at most `R` out-neighbours (the α-pruned edge set that gives DiskANN its short search
//! paths); edges are made undirected and re-pruned when a node exceeds degree `R`. Two passes
//! (α = 1.0 then α = `alpha`) per the DiskANN paper. Search: greedy beam search of width `L`
//! from the medoid. Distance is **cosine**, identical to [`ann`](crate::ann): vectors are
//! L2-normalized at build time and Euclidean-on-unit-vectors is rank-equivalent to cosine
//! (`cos = 1 − d²/2`), so reported scores and rankings match the exact/HNSW searchers.
//!
//! # On-disk format (`.spqg`, version 2, little-endian)
//!
//! ```text
//! offset 0    magic       b"SPQG"                       4 bytes
//! offset 4    version     u32 = 2                       4 bytes
//! offset 8    dim         u32                           4 bytes
//! offset 12   degree R    u32   (max out-neighbours)    4 bytes
//! offset 16   count       u64   (nodes = store vectors) 8 bytes
//! offset 24   medoid      u32   (entry-point slot)      4 bytes
//! offset 28   enc_tag     u32   (0 = none, 1 = PQ cache) 4 bytes
//! offset 32   fingerprint graph fingerprint            24 bytes
//!             (dict_len: u64, triple_count: u64, content_hash: u64)
//! offset 56   nodes       [count × NODE_RECORD]         count·record bytes
//!
//! one NODE_RECORD (all fields little-endian, 4-byte aligned):
//!   id        u32                  the store's dictionary term id
//!   degree    u32                  number of valid neighbours (≤ R)
//!   vector    [dim] f32            the L2-NORMALIZED vector (search reads it here)
//!   nbrs      [R] u32              neighbour SLOT indices; entries ≥ degree are padding
//!
//! (sq-qamd) the optional PQ_SECTION (present iff enc_tag == 1, appended after nodes):
//!   magic     b"SPQP"              4 bytes
//!   cb_len    u64                  length of the codebook block that follows
//!   codebook  [cb_len] bytes       ProductQuantizer::to_bytes (dim, m, k + centroids)
//!   stride    u32                  code length M (== codebook M)
//!   codes     [count × M] bytes    per-slot PQ codes, slot order (== node-record order)
//! ```
//!
//! A node's vector and its adjacency are **co-located in one record** (the DiskANN locality
//! property): one contiguous read per visited node — the OS pages in a record and gets both
//! the distance input and the next hops, no scatter. Records start at multiples of 4 from the
//! page-aligned map, so the `f32` cast is always aligned (same discipline as `.spqv`).
//!
//! # Honest scope vs. full DiskANN
//!
//! The default build is the **Vamana on-disk graph with full-precision vectors searched from the
//! mmap**. Full DiskANN additionally keeps a **PQ-compressed** copy of every vector resident in RAM
//! to rank candidates without touching disk, reading full-precision vectors only to re-rank the
//! beam. That quantization layer (sq-nq5,
//! [`quant`](crate::quant) — [`ProductQuantizer`] + [`EncodedStore`] + [`DistanceTable`])
//! is now **wired into the search path** (sq-qamd): build with
//! [`DiskAnnIndex::build_with_pq`] and the greedy search ranks each visited node's neighbours by an
//! ADC [`DistanceTable`] lookup against the in-RAM codes (no disk), reading the full-precision
//! vector from the mmap only for the final beam it re-ranks — DiskANN's "search on PQ, re-rank on
//! disk" loop. Without the PQ section (the plain [`build`](DiskAnnIndex::build) /
//! [`build_with`](DiskAnnIndex::build_with) path) `search_slots` is unchanged: it computes every
//! candidate distance from the full-precision mmap, exactly as before. At the scales where the graph
//! fits in page cache (the regime the plain index unblocks: skip the per-process rebuild) the two
//! are recall-equivalent up to the PQ approximation; PQ matters most when the vectors themselves
//! exceed RAM. The `.spqg` header's 4 reserved bytes carry an **encoding tag** (`0` = no cache,
//! `1` = a trailing PQ section), so the PQ variant is a backwards-compatible addition: a plain
//! reader sees the tag and a longer file but the node-record layout is byte-identical.
//!
//! (sq-32i5) The header also carries a **graph fingerprint** (offset 32..56) binding
//! the index to the graph it was built against — see [`crate::fingerprint`] and
//! [`DiskAnnIndex::check_graph`]. A query by term resolves through the caller's graph dictionary,
//! so an index built against a different graph generation would silently mis-resolve; the checked
//! query entry points reject that. Version-1 files (no fingerprint, 32-byte header) still open but
//! are reported as unverifiable.
//!
//! (sq-wlzi) **ID-KEYED STALENESS CONTRACT.** Each node record stores its term's
//! build-time dictionary id, and neighbour entries are stored slots — so this index, like the
//! [`VectorStore`] under it, is valid ONLY against the **exact graph generation it was built
//! against**. To serve it, persist that graph (`Graph::save`) and reopen THAT graph (`Graph::open`,
//! which mmaps the **frozen** dict id order — both gated by `sparq-core`'s `mmap` feature) to resolve
//! query terms — **never re-parse the source RDF** (`Graph::load_str` et al.): sparq-core's parallel
//! sharded dict merge assigns thread-count-dependent ids, so a re-parse gives a *different*
//! `id → term` binding and `nearest_term` mis-resolves. `check_graph` is a
//! backstop, **not** a sufficient guard — the sq-xhiv fingerprint is thread-count-stable, so it PASSES
//! a re-parse of the same RDF whose ids permuted. See [`crate::fingerprint`] for the full rationale.

use crate::fingerprint::{self, Fingerprint, FINGERPRINT_LEN};
use crate::quant::{DistanceTable, EncodedStore, PqConfig, ProductQuantizer};
// (sq-98c) The read backing is shared with `.spqv`: a memory map on native targets,
// f32-aligned owned bytes on wasm32 / `open_from_bytes` (memmap2 is target-gated out of wasm
// builds in Cargo.toml). Every record read derefs to `[u8]`, so the paths are identical.
use crate::store::{open_backing, Bytes, VectorStore};
use oxrdf::Term;
use sparq_core::dict::Id;
use sparq_core::Graph;
use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};

/// First four bytes of every `.spqg` file.
pub const SPQG_MAGIC: [u8; 4] = *b"SPQG";
/// Current on-disk graph format version. (sq-32i5) v2 adds the 24-byte graph
/// fingerprint block at offset 32; v1 files (32-byte header) still open but cannot be verified.
pub const SPQG_VERSION: u32 = 2;
/// Header length of a version-1 file (no fingerprint block).
const HEADER_LEN_V1: usize = 32;
/// Header length of the current (version-2) format: the v1 header + the fingerprint block.
const HEADER_LEN: usize = HEADER_LEN_V1 + FINGERPRINT_LEN;
/// Byte offset of the encoding tag (the 4 reserved bytes of the v1/v2 header).
const ENC_TAG_OFFSET: usize = 28;
/// Encoding tag: no PQ candidate cache — the file ends at the last node record (default).
const ENC_TAG_NONE: u32 = 0;
/// (sq-qamd) Encoding tag: a **PQ candidate-cache section** follows the node records
/// (codebook + per-slot codes). See `PqSection` for the trailing block's layout.
const ENC_TAG_PQ: u32 = 1;
/// First four bytes of the trailing PQ section (a guard against a truncated/garbled tail).
const PQ_SECTION_MAGIC: [u8; 4] = *b"SPQP";

/// Vamana build/search parameters. Defaults follow the DiskANN paper's small-graph regime
/// and are tuned to clear [`ann`](crate::ann)'s recall@10 ≥ 0.95 gate on the same synthetic
/// set the HNSW index is measured on.
#[derive(Clone, Copy, Debug)]
pub struct VamanaConfig {
    /// Max out-degree `R` (neighbours per node). Higher = better recall, larger file.
    pub degree: usize,
    /// Build-time beam width `L` (candidate list size during construction; must be ≥ `degree`).
    pub build_beam: usize,
    /// Search-time beam width `L` (the recall knob; must be ≥ the `k` you query).
    pub search_beam: usize,
    /// RobustPrune relaxation `α` ≥ 1.0 (1.0 = pure nearest; >1 keeps longer-range edges
    /// that shorten search paths). DiskANN uses 1.2.
    pub alpha: f32,
    /// Graph-construction RNG seed — fixed by default so builds are reproducible.
    pub seed: u64,
}

impl Default for VamanaConfig {
    fn default() -> Self {
        VamanaConfig {
            degree: 32,
            build_beam: 100,
            search_beam: 100,
            alpha: 1.2,
            seed: 0x5350_5147_0001,
        }
    }
}

/// Squared Euclidean distance over two equal-length unit vectors (rank-equivalent to cosine;
/// avoids the `sqrt` that the greedy search does not need). Eight accumulator lanes so the
/// loop auto-vectorizes — mirrors [`ann::cosine`](crate::ann::cosine)'s shape.
fn sq_dist(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    const LANES: usize = 8;
    let mut acc = [0f32; LANES];
    let chunks = a.len() / LANES;
    for c in 0..chunks {
        for l in 0..LANES {
            let d = a[c * LANES + l] - b[c * LANES + l];
            acc[l] += d * d;
        }
    }
    for i in chunks * LANES..a.len() {
        let d = a[i] - b[i];
        acc[0] += d * d;
    }
    acc.iter().sum()
}

/// L2-normalizes `v`; `None` for an all-zero vector (no direction). Stored vectors are never
/// zero ([`VectorStore::put`] rejects them), so `None` only arises for a degenerate query.
fn normalized(v: &[f32]) -> Option<Vec<f32>> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (norm > 0.0).then(|| v.iter().map(|x| x / norm).collect())
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A `(distance, slot)` ordered nearest-first. `f32` distances are finite here (unit vectors),
/// so `total_cmp` never sees a NaN; ties break on slot for determinism.
#[derive(Clone, Copy, PartialEq)]
struct Cand {
    dist: f32,
    slot: u32,
}
impl Eq for Cand {}
impl Ord for Cand {
    fn cmp(&self, other: &Self) -> Ordering {
        self.dist.total_cmp(&other.dist).then(self.slot.cmp(&other.slot))
    }
}
impl PartialOrd for Cand {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ───────────────────────────── build (in RAM) ─────────────────────────────

/// The in-RAM Vamana graph during construction: normalized vectors + per-node adjacency.
struct Builder {
    dim: usize,
    degree: usize,
    /// Normalized vectors, row-major, slot order (matches the store's `iter` order).
    vectors: Vec<f32>,
    ids: Vec<Id>,
    /// Out-neighbour slots per node (≤ `degree` after pruning).
    adj: Vec<Vec<u32>>,
    medoid: u32,
}

impl Builder {
    fn vector(&self, slot: u32) -> &[f32] {
        let s = slot as usize * self.dim;
        &self.vectors[s..s + self.dim]
    }

    fn dist(&self, a: u32, b: u32) -> f32 {
        sq_dist(self.vector(a), self.vector(b))
    }

    /// Greedy search from `start` toward `query`'s slot, returning the set of slots *visited*
    /// (the candidate pool RobustPrune draws from). The working set is kept to `beam` closest.
    fn greedy_visit(&self, query: u32, start: u32, beam: usize) -> Vec<u32> {
        let n = self.ids.len();
        let qv = self.vector(query);
        let mut working: Vec<Cand> = Vec::with_capacity(beam + self.degree);
        let mut visited: Vec<u32> = Vec::new();
        let mut in_working = vec![false; n];
        let mut expanded = vec![false; n];
        working.push(Cand { dist: sq_dist(qv, self.vector(start)), slot: start });
        in_working[start as usize] = true;
        loop {
            let next = working
                .iter()
                .filter(|c| !expanded[c.slot as usize])
                .min()
                .copied();
            let Some(cur) = next else { break };
            expanded[cur.slot as usize] = true;
            visited.push(cur.slot);
            for &nbr in &self.adj[cur.slot as usize] {
                if in_working[nbr as usize] {
                    continue;
                }
                in_working[nbr as usize] = true;
                let d = sq_dist(qv, self.vector(nbr));
                working.push(Cand { dist: d, slot: nbr });
            }
            if working.len() > beam {
                working.sort_unstable();
                working.truncate(beam);
            }
        }
        visited
    }

    /// DiskANN's RobustPrune: from candidate slots `cands` (plus `node`'s current neighbours),
    /// pick ≤ `degree` out-neighbours for `node`, keeping `p` only if no already-kept `k` is
    /// `alpha`× closer to `p` than `node` is — the diversification that bounds search paths.
    fn robust_prune(&self, node: u32, mut cands: Vec<u32>, alpha: f32) -> Vec<u32> {
        cands.extend_from_slice(&self.adj[node as usize]);
        cands.retain(|&c| c != node);
        cands.sort_unstable();
        cands.dedup();
        // Closest candidate to `node` first.
        cands.sort_by(|&a, &b| self.dist(node, a).total_cmp(&self.dist(node, b)));
        let mut kept: Vec<u32> = Vec::with_capacity(self.degree);
        for p in cands {
            if kept.len() >= self.degree {
                break;
            }
            // Keep p unless some already-kept k is alpha× closer to p than node is.
            let keep = kept.iter().all(|&k| alpha * self.dist(k, p) > self.dist(node, p));
            if keep {
                kept.push(p);
            }
        }
        kept
    }

    /// Add the directed edge node→nbr, re-pruning node if it overflows `degree`.
    fn add_edge(&mut self, node: u32, nbr: u32, alpha: f32) {
        if node == nbr || self.adj[node as usize].contains(&nbr) {
            return;
        }
        self.adj[node as usize].push(nbr);
        if self.adj[node as usize].len() > self.degree {
            let cands = std::mem::take(&mut self.adj[node as usize]);
            self.adj[node as usize] = self.robust_prune(node, cands, alpha);
        }
    }
}

/// Builds the Vamana graph over every vector in `store`, returning the in-RAM [`Builder`].
fn build_graph(store: &VectorStore, cfg: &VamanaConfig) -> Builder {
    let dim = store.dim();
    let mut vectors: Vec<f32> = Vec::with_capacity(store.len() * dim);
    let mut ids: Vec<Id> = Vec::with_capacity(store.len());
    for (id, v) in store.iter() {
        let nv = normalized(v).expect("stores never hold zero vectors (put rejects them)");
        vectors.extend_from_slice(&nv);
        ids.push(id);
    }
    let n = ids.len();
    let degree = cfg.degree.max(1);

    // Medoid (greedy entry point): the node closest to the centroid — O(n·dim), an adequate
    // start for greedy search (the exact O(n²) medoid is not worth it here).
    let medoid = if n == 0 {
        0
    } else {
        let mut centroid = vec![0f32; dim];
        for s in 0..n {
            for (c, &x) in centroid.iter_mut().zip(&vectors[s * dim..(s + 1) * dim]) {
                *c += x;
            }
        }
        for c in centroid.iter_mut() {
            *c /= n as f32;
        }
        (0..n)
            .min_by(|&a, &b| {
                sq_dist(&vectors[a * dim..(a + 1) * dim], &centroid)
                    .total_cmp(&sq_dist(&vectors[b * dim..(b + 1) * dim], &centroid))
            })
            .unwrap_or(0) as u32
    };

    // Random initial graph so greedy search has edges to follow on pass one.
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    if n > 1 {
        let mut state = cfg.seed;
        let want = degree.min(n - 1);
        for (s, out) in adj.iter_mut().enumerate() {
            while out.len() < want {
                let cand = (splitmix64(&mut state) % n as u64) as u32;
                if cand as usize != s && !out.contains(&cand) {
                    out.push(cand);
                }
            }
        }
    }

    let mut b = Builder { dim, degree, vectors, ids, adj, medoid };

    if n > 1 {
        // Random processing order (DiskANN). Two passes: α=1.0 then α=cfg.alpha.
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut state = cfg.seed ^ 0xABCD_1234;
        for i in (1..order.len()).rev() {
            let j = (splitmix64(&mut state) % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        for &alpha in &[1.0f32, cfg.alpha.max(1.0)] {
            for &node in &order {
                let visited = b.greedy_visit(node, b.medoid, cfg.build_beam.max(degree));
                let pruned = b.robust_prune(node, visited, alpha);
                b.adj[node as usize] = pruned.clone();
                // Make edges undirected (Vamana): add node into each neighbour's list.
                for &nbr in &pruned {
                    b.add_edge(nbr, node, alpha);
                }
            }
        }
    }
    b
}

// ───────────────────────────── persistence ─────────────────────────────

/// Byte length of one node record: id + degree + vector + neighbour slots.
const fn record_len(dim: usize, degree: usize) -> usize {
    8 + dim * 4 + degree * 4
}

/// (sq-qamd) The trailing PQ candidate-cache section: the fitted quantizer plus every
/// vector's PQ code, ready to be appended after the node records when the encoding tag is `1`.
struct PqSection {
    pq: ProductQuantizer,
    codes: EncodedStore,
}

impl PqSection {
    /// Serializes the section to the on-disk byte layout documented in the module header
    /// (`SPQP` magic, codebook length + codebook, stride, then the flat per-slot codes).
    fn to_bytes(&self) -> Vec<u8> {
        let cb = self.pq.to_bytes();
        let codes = self.codes.codes();
        let mut out = Vec::with_capacity(4 + 8 + cb.len() + 4 + codes.len());
        out.extend_from_slice(&PQ_SECTION_MAGIC);
        out.extend_from_slice(&(cb.len() as u64).to_le_bytes());
        out.extend_from_slice(&cb);
        out.extend_from_slice(&(self.codes.stride() as u32).to_le_bytes());
        out.extend_from_slice(codes);
        out
    }
}

/// Writes the built graph to `path` as a `.spqg` file (one node record per node). `fingerprint`
/// (offset 32..56) binds the index to its graph; `None` writes an unverifiable (all-zero) block.
/// `pq` (when `Some`) appends a `PqSection` after the node records and sets the encoding tag to
/// [`ENC_TAG_PQ`]; its `codes` MUST be in the same slot order as the node records.
fn write_graph(
    b: &Builder,
    path: &Path,
    fingerprint: Option<Fingerprint>,
    pq: Option<&PqSection>,
) -> Result<(), String> {
    if cfg!(target_endian = "big") {
        return Err(".spqg is a little-endian format; big-endian targets are unsupported".into());
    }
    let count = b.ids.len();
    let r = b.degree;
    let mut header = [0u8; HEADER_LEN];
    header[0..4].copy_from_slice(&SPQG_MAGIC);
    header[4..8].copy_from_slice(&SPQG_VERSION.to_le_bytes());
    header[8..12].copy_from_slice(&(b.dim as u32).to_le_bytes());
    header[12..16].copy_from_slice(&(r as u32).to_le_bytes());
    header[16..24].copy_from_slice(&(count as u64).to_le_bytes());
    header[24..28].copy_from_slice(&b.medoid.to_le_bytes());
    // (sq-qamd) Encoding tag at offset 28..32: `1` when a PQ section is appended below,
    // else `0`. (sq-32i5) Graph fingerprint at offset 32..56; `None` leaves a zeroed block, which
    // `check_graph` treats as unverifiable.
    let enc_tag = if pq.is_some() { ENC_TAG_PQ } else { ENC_TAG_NONE };
    header[ENC_TAG_OFFSET..ENC_TAG_OFFSET + 4].copy_from_slice(&enc_tag.to_le_bytes());
    if let Some(fp) = fingerprint {
        header[HEADER_LEN_V1..HEADER_LEN].copy_from_slice(&fp.to_bytes());
    }

    let file = std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut w = std::io::BufWriter::new(file);
    w.write_all(&header).map_err(|e| format!("write {}: {e}", path.display()))?;

    let mut rec = vec![0u8; record_len(b.dim, r)];
    for slot in 0..count {
        let nbrs = &b.adj[slot];
        let deg = nbrs.len().min(r) as u32;
        rec[0..4].copy_from_slice(&b.ids[slot].to_le_bytes());
        rec[4..8].copy_from_slice(&deg.to_le_bytes());
        let vec_off = 8;
        let v = &b.vectors[slot * b.dim..(slot + 1) * b.dim];
        // f32 → LE bytes (little-endian target asserted above). SAFETY: f32 has no invalid bit
        // patterns and align(f32) ≥ align(u8); the read borrows `b.vectors`.
        let v_bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, b.dim * 4) };
        rec[vec_off..vec_off + b.dim * 4].copy_from_slice(v_bytes);
        let nbr_off = vec_off + b.dim * 4;
        for (i, entry) in rec[nbr_off..].chunks_exact_mut(4).enumerate() {
            let val = if i < deg as usize { nbrs[i] } else { 0u32 };
            entry.copy_from_slice(&val.to_le_bytes());
        }
        w.write_all(&rec).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    // (sq-qamd) Trailing PQ candidate-cache section (encoding tag == 1).
    if let Some(section) = pq {
        w.write_all(&section.to_bytes())
            .map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    let f = w.into_inner().map_err(|e| format!("flush {}: {e}", path.display()))?;
    f.sync_all().map_err(|e| format!("fsync {}: {e}", path.display()))?;
    Ok(())
}

// ───────────────────────────── on-disk search ─────────────────────────────

/// A **persistent on-disk Vamana index** opened over a `.spqg` file (memory-mapped on native
/// targets) — the out-of-core counterpart to `VectorIndex`. Built once with
/// [`build`](Self::build) / [`build_with`](Self::build_with), reopened with [`open`](Self::open)
/// at near-zero cost (mmap + header validation, no rebuild) or from fetched/embedded bytes with
/// [`open_from_bytes`](Self::open_from_bytes) (the wasm/filesystem-less path — memmap2 is
/// target-gated out of wasm32 builds). Search reads node records directly
/// from the backing; see the module docs for the format and the honest scope vs. full DiskANN.
pub struct DiskAnnIndex {
    map: Bytes,
    dim: usize,
    degree: usize,
    count: usize,
    medoid: u32,
    record_len: usize,
    search_beam: usize,
    /// (sq-32i5) The graph fingerprint this index was built against, or `None` for a
    /// legacy version-1 file / an index built without a graph. See [`check_graph`](Self::check_graph).
    fingerprint: Option<Fingerprint>,
    /// Byte offset where node records begin: [`HEADER_LEN`] (v2) or [`HEADER_LEN_V1`] (legacy v1).
    /// Every record read keys off this so both versions are read correctly by the same code.
    data_offset: usize,
    /// (sq-qamd) The in-RAM PQ candidate cache (codebook + per-slot codes), or `None`
    /// for a plain index (encoding tag `0`). When present, [`search_slots`](Self::search_slots)
    /// ranks each visited node's neighbours by an ADC table lookup against these codes and re-ranks
    /// the final beam off the mmap; when `None` it computes every distance from the mmap as before.
    pq: Option<PqCache>,
}

/// (sq-qamd) The decoded PQ candidate cache: the fitted quantizer and the per-slot
/// codes (slot order == node-record order, so a graph slot indexes the codes directly).
struct PqCache {
    pq: ProductQuantizer,
    codes: EncodedStore,
}

impl DiskAnnIndex {
    /// Builds the Vamana graph over `store` with default parameters and writes it to `path`,
    /// then opens it. Equivalent to `build_with(store, path, VamanaConfig::default())`. The index
    /// is written WITHOUT a graph fingerprint (unverifiable — [`check_graph`](Self::check_graph)
    /// errors); use [`build_for`](Self::build_for) to bind it to its graph.
    pub fn build<P: AsRef<Path>>(store: &VectorStore, path: P) -> Result<DiskAnnIndex, String> {
        Self::build_with(store, path, VamanaConfig::default())
    }

    /// Builds the Vamana graph over `store` with `cfg`, writes the `.spqg` file at `path`, and
    /// opens it memory-mapped. The build is in RAM (one-off); the open is cheap forever after.
    /// Written without a fingerprint — see [`build`](Self::build) / [`build_with_for`](Self::build_with_for).
    pub fn build_with<P: AsRef<Path>>(
        store: &VectorStore,
        path: P,
        cfg: VamanaConfig,
    ) -> Result<DiskAnnIndex, String> {
        Self::build_inner(store, path, cfg, None, None)
    }

    /// (sq-32i5) Like [`build`](Self::build) but binds the index to `graph` (embeds its
    /// fingerprint), so [`check_graph`](Self::check_graph) / [`nearest_term_checked`](Self::nearest_term_checked)
    /// can reject a query against a different graph generation. Pass the graph whose term ids `store`
    /// is keyed by.
    pub fn build_for<P: AsRef<Path>>(
        store: &VectorStore,
        path: P,
        graph: &Graph,
    ) -> Result<DiskAnnIndex, String> {
        Self::build_with_for(store, path, VamanaConfig::default(), graph)
    }

    /// (sq-32i5) Like [`build_with`](Self::build_with) but binds the index to `graph`'s
    /// fingerprint (see [`build_for`](Self::build_for)).
    pub fn build_with_for<P: AsRef<Path>>(
        store: &VectorStore,
        path: P,
        cfg: VamanaConfig,
        graph: &Graph,
    ) -> Result<DiskAnnIndex, String> {
        Self::build_inner(store, path, cfg, Some(Fingerprint::of(graph)), None)
    }

    /// (sq-qamd) Builds the Vamana graph **with a PQ candidate cache**: in addition to
    /// the full-precision node records, it fits a [`ProductQuantizer`] over `store` (with `pq_cfg`),
    /// encodes every vector into the in-RAM code cache, and persists both alongside the graph (the
    /// trailing PQ section, encoding tag `1`). The opened index then searches DiskANN-style: rank
    /// candidates on the RAM codes (no disk), re-rank the final beam off the mmap. Recall is
    /// approximate (the PQ approximation) but no disk page is touched until the re-rank, so it scales
    /// to stores whose full-precision vectors exceed RAM.
    ///
    /// Errors if `pq_cfg` is invalid for `store`'s dimension (see [`ProductQuantizer::fit`]) or the
    /// store is empty (PQ needs at least one training vector — use the plain
    /// [`build_with`](Self::build_with) for an empty store).
    pub fn build_with_pq<P: AsRef<Path>>(
        store: &VectorStore,
        path: P,
        cfg: VamanaConfig,
        pq_cfg: PqConfig,
    ) -> Result<DiskAnnIndex, String> {
        let pq = ProductQuantizer::fit(store.dim(), store.iter().map(|(_, v)| v), pq_cfg)?;
        let codes = pq.encode_store(store)?;
        let section = PqSection { pq, codes };
        Self::build_inner(store, path, cfg, None, Some(section))
    }

    fn build_inner<P: AsRef<Path>>(
        store: &VectorStore,
        path: P,
        cfg: VamanaConfig,
        fingerprint: Option<Fingerprint>,
        pq: Option<PqSection>,
    ) -> Result<DiskAnnIndex, String> {
        if cfg.build_beam < cfg.degree {
            return Err(format!(
                "build_beam {} must be ≥ degree {} (the candidate pool can't be smaller than the out-degree)",
                cfg.build_beam, cfg.degree
            ));
        }
        let b = build_graph(store, &cfg);
        write_graph(&b, path.as_ref(), fingerprint, pq.as_ref())?;
        let mut idx = Self::open(path)?;
        idx.search_beam = cfg.search_beam;
        Ok(idx)
    }

    /// Opens a `.spqg` file memory-mapped, **without rebuilding** — the whole point of this
    /// index. Validates the header and that the file size matches `count` records so no later
    /// search can read out of bounds; the records themselves page in on access.
    ///
    /// (sq-32i5) A version-2 file's graph fingerprint is read for
    /// [`check_graph`](Self::check_graph). A legacy version-1 file (32-byte header, no fingerprint)
    /// still opens — its fingerprint is `None`, so `check_graph` reports it as unverifiable.
    ///
    /// (sq-98c) On wasm32 (memmap2 target-gated out) this reads the whole file into
    /// the same f32-aligned owned backing [`open_from_bytes`](Self::open_from_bytes) uses —
    /// identical validation, no map. `wasm32-unknown-unknown` has no filesystem, so there the
    /// read fails with a clean I/O error and `open_from_bytes` is the supported path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<DiskAnnIndex, String> {
        let path = path.as_ref();
        // (sq-98c) Memory map on native targets; on wasm32 (memmap2 target-gated out)
        // `open_backing` reads the whole file into the f32-aligned owned backing instead —
        // identical validation. `wasm32-unknown-unknown` has no filesystem, so there the read
        // fails with a clean I/O error and [`open_from_bytes`](Self::open_from_bytes) is the
        // supported path.
        let map = open_backing(path)?;
        Self::open_validated(map, &path.display().to_string())
    }

    /// Opens a `.spqg` document held entirely in memory — for environments without a
    /// filesystem (the bytes were fetched, embedded, or decompressed by the caller), the
    /// `.spqg` counterpart of [`VectorStore::open_from_bytes`]. Validation is identical to
    /// [`open`](Self::open); record reads borrow the owned buffer (f32-aligned, so the vector
    /// casts are as sound as on the page-aligned map) instead of a memory map. (sq-98c)
    pub fn open_from_bytes(bytes: Vec<u8>) -> Result<DiskAnnIndex, String> {
        Self::open_validated(Bytes::owned(bytes), "<bytes>")
    }

    /// Shared header/size validation behind [`open`](Self::open) and
    /// [`open_from_bytes`](Self::open_from_bytes). (sq-98c)
    fn open_validated(map: Bytes, origin: &str) -> Result<DiskAnnIndex, String> {
        if cfg!(target_endian = "big") {
            return Err(".spqg is a little-endian format; big-endian targets are unsupported".into());
        }
        if map.len() < HEADER_LEN_V1 {
            return Err(format!("{origin}: truncated header"));
        }
        if map[0..4] != SPQG_MAGIC {
            return Err(format!("{origin}: not a .spqg file (bad magic)"));
        }
        let version = u32::from_le_bytes(map[4..8].try_into().unwrap());
        // (sq-32i5) Both v1 (no fingerprint, 32-byte header) and v2 (fingerprint,
        // 56-byte header) open; the offset where node records begin depends on the version, so
        // every record read keys off `data_offset` below.
        let (data_offset, fingerprint): (usize, Option<Fingerprint>) = match version {
            1 => (HEADER_LEN_V1, None),
            2 => {
                if map.len() < HEADER_LEN {
                    return Err(format!("{origin}: truncated version-2 header (fingerprint block)"));
                }
                // (sq-32i5) All-zero block (a v2 index built without a graph binding)
                // → `None` ("unverifiable"), not a zero fingerprint that would surface as a
                // spurious "DIFFERENT graph" mismatch.
                (
                    HEADER_LEN,
                    Fingerprint::from_bytes_opt(&map[HEADER_LEN_V1..HEADER_LEN]),
                )
            }
            v => return Err(format!("{origin}: unsupported .spqg version {v}")),
        };
        let dim = u32::from_le_bytes(map[8..12].try_into().unwrap()) as usize;
        let degree = u32::from_le_bytes(map[12..16].try_into().unwrap()) as usize;
        let count: usize = u64::from_le_bytes(map[16..24].try_into().unwrap())
            .try_into()
            .map_err(|_| format!("{origin}: node count exceeds the address space"))?;
        let medoid = u32::from_le_bytes(map[24..28].try_into().unwrap());
        if dim == 0 {
            return Err(format!("{origin}: zero dimension"));
        }
        let record_len = record_len(dim, degree);
        // Checked size arithmetic — reject a malformed header before it wraps past the bounds check.
        // `nodes_end` is the byte just past the last node record; the file is exactly that long for
        // a plain index, or longer (a trailing PQ section) when the encoding tag is `1`.
        let nodes_end = count
            .checked_mul(record_len)
            .and_then(|body| body.checked_add(data_offset))
            .ok_or_else(|| {
                format!("{origin}: dim={dim} degree={degree} count={count} overflows the file size")
            })?;
        // (sq-qamd) Encoding tag at offset 28..32 (reserved bytes of the header).
        let enc_tag =
            u32::from_le_bytes(map[ENC_TAG_OFFSET..ENC_TAG_OFFSET + 4].try_into().unwrap());
        let pq = match enc_tag {
            ENC_TAG_NONE => {
                if map.len() != nodes_end {
                    return Err(format!(
                        "{origin}: file is {} bytes, expected {nodes_end} for dim={dim} degree={degree} count={count}",
                        map.len()
                    ));
                }
                None
            }
            ENC_TAG_PQ => {
                if map.len() < nodes_end {
                    return Err(format!(
                        "{origin}: file is {} bytes, shorter than the {nodes_end}-byte node body",
                        map.len()
                    ));
                }
                Some(Self::parse_pq_section(&map[nodes_end..], count, dim, origin)?)
            }
            t => return Err(format!("{origin}: unsupported encoding tag {t}")),
        };
        if count > 0 && medoid as usize >= count {
            return Err(format!("{origin}: medoid {medoid} out of range (count {count})"));
        }
        Ok(DiskAnnIndex {
            map,
            dim,
            degree,
            count,
            medoid,
            record_len,
            search_beam: VamanaConfig::default().search_beam,
            fingerprint,
            data_offset,
            pq,
        })
    }

    /// (sq-qamd) Parses the trailing PQ section (`tail` begins right after the last node
    /// record). Validates the magic, the codebook (which re-checks its own `dim`/`m`/`k`), that the
    /// codebook's `dim` matches the graph's, and that exactly `count × M` code bytes are present (no
    /// trailing slop) so a later code read is always in bounds. Errors are descriptive.
    fn parse_pq_section(
        tail: &[u8],
        count: usize,
        dim: usize,
        origin: &str,
    ) -> Result<PqCache, String> {
        let hdr = 4 + 8; // magic + codebook length
        if tail.len() < hdr {
            return Err(format!("{origin}: truncated PQ section header"));
        }
        if tail[0..4] != PQ_SECTION_MAGIC {
            return Err(format!("{origin}: PQ section has bad magic"));
        }
        let cb_len: usize = u64::from_le_bytes(tail[4..12].try_into().unwrap())
            .try_into()
            .map_err(|_| format!("{origin}: PQ codebook length exceeds the address space"))?;
        let cb_end = hdr
            .checked_add(cb_len)
            .and_then(|e| e.checked_add(4)) // + stride u32
            .ok_or_else(|| format!("{origin}: PQ section length overflows"))?;
        if tail.len() < cb_end {
            return Err(format!("{origin}: truncated PQ codebook"));
        }
        let pq = ProductQuantizer::from_bytes(&tail[hdr..hdr + cb_len])
            .map_err(|e| format!("{origin}: {e}"))?;
        if pq.dim() != dim {
            return Err(format!("{origin}: PQ codebook dim {} != graph dim {dim}", pq.dim()));
        }
        let stride = u32::from_le_bytes(tail[hdr + cb_len..cb_end].try_into().unwrap()) as usize;
        if stride != pq.m() {
            return Err(format!("{origin}: PQ section stride {stride} != codebook M {}", pq.m()));
        }
        let want_codes = count
            .checked_mul(stride)
            .ok_or_else(|| format!("{origin}: PQ code array length overflows"))?;
        if tail.len() != cb_end + want_codes {
            return Err(format!(
                "{origin}: PQ code array is {} bytes, expected {want_codes} (count {count} × M {stride})",
                tail.len() - cb_end
            ));
        }
        let codes =
            EncodedStore::from_parts((0..count as u32).collect(), tail[cb_end..].to_vec(), stride)
                .map_err(|e| format!("{origin}: {e}"))?;
        Ok(PqCache { pq, codes })
    }

    /// (sq-qamd) Whether this index carries an in-RAM PQ candidate cache (built via
    /// [`build_with_pq`](Self::build_with_pq) / persisted as the trailing PQ section). When `true`,
    /// [`nearest`](Self::nearest) ranks candidates on the codes and re-ranks the beam off the mmap;
    /// when `false` it searches full-precision throughout.
    pub fn has_pq_cache(&self) -> bool {
        self.pq.is_some()
    }

    /// The index's vector dimension.
    pub fn dim(&self) -> usize {
        self.dim
    }
    /// Number of indexed nodes (= store vectors at build time).
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The normalized vector stored in `slot`'s record, read directly from the map.
    fn node_vector(&self, slot: u32) -> &[f32] {
        let start = self.data_offset + slot as usize * self.record_len + 8;
        let bytes = &self.map[start..start + self.dim * 4];
        debug_assert_eq!(bytes.as_ptr() as usize % std::mem::align_of::<f32>(), 0);
        // SAFETY: the backing base is f32-aligned — a memory map is page-aligned, and the owned
        // backing (`open_from_bytes` / wasm32) is 4-byte-aligned by construction (`AlignedBytes`,
        // see store.rs) — and `start` is a multiple of 4 (data_offset [32 or 56] +
        // slot·record_len[a multiple of 4] + 8), so the pointer is f32-aligned; the range is in
        // bounds (validated in `open_validated`); f32 accepts any bit pattern; slice borrows map.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, self.dim) }
    }

    /// `slot`'s dictionary term id (record field 0).
    fn node_id(&self, slot: u32) -> Id {
        let start = self.data_offset + slot as usize * self.record_len;
        u32::from_le_bytes(self.map[start..start + 4].try_into().unwrap())
    }

    /// `slot`'s valid out-neighbour slots (the first `degree` neighbour entries).
    fn node_neighbours(&self, slot: u32) -> impl Iterator<Item = u32> + '_ {
        let start = self.data_offset + slot as usize * self.record_len;
        let deg = u32::from_le_bytes(self.map[start + 4..start + 8].try_into().unwrap()) as usize;
        let nbr_off = start + 8 + self.dim * 4;
        (0..deg.min(self.degree)).map(move |i| {
            let o = nbr_off + i * 4;
            u32::from_le_bytes(self.map[o..o + 4].try_into().unwrap())
        })
    }

    /// Greedy beam search of width `beam` from the medoid toward `query` (already normalized),
    /// returning the `k` best `(slot, cosine)` pairs, best first. Each visited node is one
    /// contiguous record read from the map.
    ///
    /// (sq-qamd) When the index carries a PQ candidate cache (built via
    /// [`build_with_pq`](Self::build_with_pq)) the beam is driven by RAM-only ADC distance
    /// estimates against the codes ([`search_slots_pq`](Self::search_slots_pq)) and only the final
    /// beam is re-ranked off the mmap; otherwise every candidate distance is the exact mmap
    /// distance, as before.
    fn search_slots(&self, query: &[f32], k: usize, beam: usize) -> Vec<(u32, f32)> {
        if self.count == 0 {
            return Vec::new();
        }
        if let Some(cache) = &self.pq {
            return self.search_slots_pq(cache, query, k, beam);
        }
        let beam = beam.max(k).max(1);
        let mut working: Vec<Cand> = Vec::with_capacity(beam + self.degree);
        let mut in_working = vec![false; self.count];
        let mut expanded = vec![false; self.count];
        let start = self.medoid;
        working.push(Cand { dist: sq_dist(query, self.node_vector(start)), slot: start });
        in_working[start as usize] = true;
        loop {
            let next = working.iter().filter(|c| !expanded[c.slot as usize]).min().copied();
            let Some(cur) = next else { break };
            expanded[cur.slot as usize] = true;
            let neighbours: Vec<u32> = self.node_neighbours(cur.slot).collect();
            for nbr in neighbours {
                if in_working[nbr as usize] {
                    continue;
                }
                in_working[nbr as usize] = true;
                let d = sq_dist(query, self.node_vector(nbr));
                working.push(Cand { dist: d, slot: nbr });
            }
            if working.len() > beam {
                working.sort_unstable();
                working.truncate(beam);
            }
        }
        working.sort_unstable();
        working.truncate(k);
        // d² over unit vectors → cosine: cos = 1 − d²/2.
        working.into_iter().map(|c| (c.slot, 1.0 - c.dist / 2.0)).collect()
    }

    /// (sq-qamd) DiskANN's "search on PQ, re-rank on disk" greedy beam search. The
    /// frontier `working` holds **PQ-estimated** `d²` (an ADC [`DistanceTable`] lookup against the
    /// in-RAM codes — no disk touched while traversing), so the beam is ranked from RAM. After the
    /// walk, the surviving beam is **re-ranked against the full-precision mmap vectors** and the top
    /// `k` of *those* exact distances are returned, so the reported cosine is exact even though the
    /// path was guided by the lossy codes. `query` is already L2-normalized (the cosine convention).
    fn search_slots_pq(
        &self,
        cache: &PqCache,
        query: &[f32],
        k: usize,
        beam: usize,
    ) -> Vec<(u32, f32)> {
        let beam = beam.max(k).max(1);
        let table = DistanceTable::new(&cache.pq, query);
        // PQ-estimated d² for a slot's code (slot order == code order).
        let pq_dist = |slot: u32| table.distance(cache.codes.code(slot as usize));
        let mut working: Vec<Cand> = Vec::with_capacity(beam + self.degree);
        let mut in_working = vec![false; self.count];
        let mut expanded = vec![false; self.count];
        let start = self.medoid;
        working.push(Cand { dist: pq_dist(start), slot: start });
        in_working[start as usize] = true;
        loop {
            let next = working.iter().filter(|c| !expanded[c.slot as usize]).min().copied();
            let Some(cur) = next else { break };
            expanded[cur.slot as usize] = true;
            let neighbours: Vec<u32> = self.node_neighbours(cur.slot).collect();
            for nbr in neighbours {
                if in_working[nbr as usize] {
                    continue;
                }
                in_working[nbr as usize] = true;
                working.push(Cand { dist: pq_dist(nbr), slot: nbr });
            }
            if working.len() > beam {
                working.sort_unstable();
                working.truncate(beam);
            }
        }
        // Re-rank the surviving beam against full-precision mmap vectors (the only disk reads), then
        // keep the top `k` by EXACT distance — so the returned cosine matches the full-precision
        // searchers even though the traversal was PQ-guided.
        working.sort_unstable();
        working.truncate(beam);
        let mut reranked: Vec<Cand> = working
            .into_iter()
            .map(|c| Cand { dist: sq_dist(query, self.node_vector(c.slot)), slot: c.slot })
            .collect();
        reranked.sort_unstable();
        reranked.truncate(k);
        // d² over unit vectors → cosine: cos = 1 − d²/2.
        reranked.into_iter().map(|c| (c.slot, 1.0 - c.dist / 2.0)).collect()
    }

    /// (sq-1wc1) **Filtered** greedy beam search: traverse the Vamana graph
    /// *predicate-agnostically* (expand through every neighbour, beam-truncated toward `query` —
    /// exactly as [`search_slots`](Self::search_slots) does, so connectivity is preserved) but only
    /// **accept** into the result the slots whose dictionary id passes `accept`. ACORN / NaviX-style
    /// predicate-aware acceptance. Returns the `k` accepted slots closest to `query`, best first.
    ///
    /// `accept` is the slot→keep predicate (the caller closes over an [`IdMask`](crate::IdMask),
    /// translating slot to id once per visited node). The traversal beam is `beam`; because accepted
    /// nodes are a subset of visited nodes, the caller widens `beam` over the unfiltered default so
    /// `k` accepted nodes can still be collected (see [`FilterConfig`](crate::FilterConfig)).
    #[cfg(feature = "filtered-ann")]
    fn search_slots_filtered(
        &self,
        query: &[f32],
        k: usize,
        beam: usize,
        accept: impl Fn(u32) -> bool,
    ) -> Vec<(u32, f32)> {
        if self.count == 0 {
            return Vec::new();
        }
        let beam = beam.max(k).max(1);
        // `working`: the traversal frontier (ALL nodes, mask-agnostic — drives expansion exactly as
        // the unfiltered search, so the graph's short paths are kept). `accepted`: masked nodes seen
        // so far, kept separately so a masked hit is never lost when the beam truncates it out.
        let mut working: Vec<Cand> = Vec::with_capacity(beam + self.degree);
        let mut accepted: Vec<Cand> = Vec::new();
        let mut in_working = vec![false; self.count];
        let mut expanded = vec![false; self.count];
        let consider = |slot: u32, dist: f32, accepted: &mut Vec<Cand>| {
            if accept(slot) {
                accepted.push(Cand { dist, slot });
            }
        };
        let start = self.medoid;
        let start_d = sq_dist(query, self.node_vector(start));
        working.push(Cand { dist: start_d, slot: start });
        in_working[start as usize] = true;
        consider(start, start_d, &mut accepted);
        loop {
            let next = working.iter().filter(|c| !expanded[c.slot as usize]).min().copied();
            let Some(cur) = next else { break };
            expanded[cur.slot as usize] = true;
            let neighbours: Vec<u32> = self.node_neighbours(cur.slot).collect();
            for nbr in neighbours {
                if in_working[nbr as usize] {
                    continue;
                }
                in_working[nbr as usize] = true;
                let d = sq_dist(query, self.node_vector(nbr));
                consider(nbr, d, &mut accepted);
                working.push(Cand { dist: d, slot: nbr });
            }
            if working.len() > beam {
                working.sort_unstable();
                working.truncate(beam);
            }
        }
        accepted.sort_unstable();
        accepted.dedup_by_key(|c| c.slot);
        accepted.truncate(k);
        // d² over unit vectors → cosine: cos = 1 − d²/2.
        accepted.into_iter().map(|c| (c.slot, 1.0 - c.dist / 2.0)).collect()
    }

    /// (sq-1wc1) **Predicate-constrained (filtered) approximate top-`k`**: like
    /// [`nearest`](Self::nearest) but the result is restricted to ids the `mask` permits — the
    /// RDF-native filtered-ANN path. The mask is the candidate id-set a SPARQL BGP selects (e.g.
    /// `?node a :Car`); only neighbours in it are returned, while the traversal still hops through
    /// non-matching nodes for connectivity.
    ///
    /// Strategy is chosen by the mask's **selectivity** ([`FilterConfig`](crate::FilterConfig)):
    /// a *very selective* mask is served by an exact **pre-filter** scan over just the masked ids
    /// (cheaper and exact than touching the whole graph), a *broad* mask by **filtered traversal**
    /// of the Vamana graph. Both honour the mask; see the [`filter`](crate::filter) module docs and
    /// `tests/filtered.rs` for the measured recall. Uses `FilterConfig::default`; for a custom
    /// threshold/beam use [`nearest_filtered_with`](Self::nearest_filtered_with).
    ///
    /// An empty mask returns no results (the BGP matched nothing); an all-zero query returns no
    /// results (same degenerate contract as [`nearest`](Self::nearest)).
    #[cfg(feature = "filtered-ann")]
    pub fn nearest_filtered(
        &self,
        query: &[f32],
        mask: &crate::IdMask,
        store: &VectorStore,
        k: usize,
    ) -> Vec<(Id, f32)> {
        self.nearest_filtered_with(query, mask, store, k, crate::FilterConfig::default())
    }

    /// (sq-1wc1) [`nearest_filtered`](Self::nearest_filtered) with an explicit
    /// [`FilterConfig`](crate::FilterConfig) (pre-filter ↔ traversal crossover and the traversal
    /// beam factor). The `store` is needed for the pre-filter strategy (it scans the masked vectors
    /// directly); for the traversal strategy it is unused, but the method takes it unconditionally so
    /// the strategy choice stays an internal detail.
    #[cfg(feature = "filtered-ann")]
    pub fn nearest_filtered_with(
        &self,
        query: &[f32],
        mask: &crate::IdMask,
        store: &VectorStore,
        k: usize,
        cfg: crate::FilterConfig,
    ) -> Vec<(Id, f32)> {
        assert_eq!(query.len(), self.dim, "query dim {} != index dim {}", query.len(), self.dim);
        if mask.is_empty() {
            return Vec::new();
        }
        // Selective mask → exact pre-filter scan over just the masked ids (exact AND cheaper than
        // walking the whole graph to accept a handful of nodes; also avoids a graph-connectivity
        // miss to an isolated accepted node). Broad mask → filtered traversal.
        if cfg.prefer_prefilter(mask.len(), self.count) {
            return crate::filter::nearest_exact_filtered(store, query, mask, k);
        }
        let Some(q) = normalized(query) else { return Vec::new() };
        let beam = (self.search_beam.max(k)) * cfg.traversal_beam_factor.max(1);
        self.search_slots_filtered(&q, k, beam, |slot| mask.contains(self.node_id(slot)))
            .into_iter()
            .map(|(slot, cos)| (self.node_id(slot), cos))
            .collect()
    }

    /// Approximate top-`k` ids by cosine similarity to `query`, best first. An all-zero
    /// `query` returns no results (same contract as [`nearest_exact`](crate::ann::nearest_exact)
    /// and `VectorIndex::nearest`).
    pub fn nearest(&self, query: &[f32], k: usize) -> Vec<(Id, f32)> {
        assert_eq!(query.len(), self.dim, "query dim {} != index dim {}", query.len(), self.dim);
        let Some(q) = normalized(query) else { return Vec::new() };
        self.search_slots(&q, k, self.search_beam.max(k))
            .into_iter()
            .map(|(slot, cos)| (self.node_id(slot), cos))
            .collect()
    }

    /// Approximate top-`k` neighbours of `term`: resolves it through the graph's dictionary,
    /// looks its vector up in `store`, excludes the term itself and maps neighbour ids back to
    /// [`Term`]s. Empty if the term is absent or unembedded. Mirrors
    /// `VectorIndex::nearest_term`.
    ///
    /// (sq-32i5) This does NOT verify the index/store match `graph` — pass a graph
    /// whose ids have shifted since build and the results are silently WRONG. Use
    /// [`nearest_term_checked`](Self::nearest_term_checked) (or call [`check_graph`](Self::check_graph)
    /// once after open) to make a mismatch a hard error.
    pub fn nearest_term(
        &self,
        term: &Term,
        graph: &Graph,
        store: &VectorStore,
        k: usize,
    ) -> Vec<(Term, f32)> {
        let Some(id) = graph.id_of(term) else { return Vec::new() };
        let Some(query) = store.get(id) else { return Vec::new() };
        self.nearest(query, k + 1)
            .into_iter()
            .filter(|&(n, _)| n != id)
            .take(k)
            .map(|(n, s)| (graph.dict().term(n), s))
            .collect()
    }

    /// (sq-32i5) The graph fingerprint this index was built against, or `None` for a
    /// legacy version-1 file / an index built without a graph. See [`check_graph`](Self::check_graph).
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        self.fingerprint
    }

    /// (sq-32i5) **Checked open guard**: verifies this index was built against `graph`
    /// (and, since the index is queried alongside the store, that `store` matches it too) by
    /// recomputing `graph`'s fingerprint and comparing it to BOTH stored fingerprints. Returns a
    /// descriptive `Err` on any mismatch — the index and store are keyed by `graph`'s dictionary
    /// ids, so a mismatch means a query would silently resolve to the WRONG vectors. A legacy
    /// version-1 index/store (no stored fingerprint) also errors, as "unverifiable".
    ///
    /// Call once after [`open`](Self::open) (it is O(dict_len), not per-query). The store is checked
    /// as well because [`nearest_term`](Self::nearest_term) resolves the query vector through it.
    pub fn check_graph(&self, store: &VectorStore, graph: &Graph) -> fingerprint::CheckResult {
        let origin = "<.spqg index>";
        fingerprint::check_against(self.fingerprint, graph, fingerprint::Artifact::Index, origin)?;
        store.check_graph(graph)
    }

    /// (sq-32i5) [`nearest_term`](Self::nearest_term) with the staleness check: returns
    /// `Err` if this index or `store` was built against a different graph generation than `graph`
    /// (which would otherwise return silently-wrong neighbours), else `Ok` with the neighbours.
    pub fn nearest_term_checked(
        &self,
        term: &Term,
        graph: &Graph,
        store: &VectorStore,
        k: usize,
    ) -> Result<Vec<(Term, f32)>, String> {
        self.check_graph(store, graph)?;
        Ok(self.nearest_term(term, graph, store, k))
    }
}

/// Default path for a store's sibling `.spqg` graph artifact: the store path with its extension
/// replaced by `spqg` (so `entities.spqv` → `entities.spqg`). A convenience for the common
/// "graph lives next to the store" layout; any path works with [`DiskAnnIndex::build_with`].
pub fn sibling_graph_path(store_path: &Path) -> PathBuf {
    store_path.with_extension("spqg")
}

#[cfg(test)]
mod fingerprint_tests {
    // (sq-32i5) Checked-open tests for the `.spqg` on-disk index: build against graph A
    // then (1) query against A → OK + correct neighbours, (2) query against a DIFFERENT graph B →
    // descriptive Err, (3) fingerprint survives reopen, (4) a legacy version-1 `.spqg` opens (and
    // searches) but reports as unverifiable.
    use super::*;
    use crate::Fingerprint;
    use oxrdf::NamedNode;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmp(tag: &str, ext: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("sparq_fpg_{tag}_{}_{n}.{ext}", std::process::id()))
    }

    fn graph(ttl: &str) -> Graph {
        Graph::load_str(ttl, "turtle").expect("load test turtle")
    }

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    const A: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice ex:knows ex:bob .
        ex:bob ex:knows ex:carol .
    "#;
    const B: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:dave ex:likes ex:eve .
        ex:eve ex:likes ex:frank .
    "#;

    fn build_store(g: &Graph, path: &std::path::Path) -> VectorStore {
        let alice = g.id_of(&iri("http://example.org/alice")).unwrap();
        let bob = g.id_of(&iri("http://example.org/bob")).unwrap();
        let carol = g.id_of(&iri("http://example.org/carol")).unwrap();
        let mut s = VectorStore::create(path, 4).unwrap().with_fingerprint(g);
        s.put(alice, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        s.put(bob, &[0.9, 0.1, 0.0, 0.0]).unwrap();
        s.put(carol, &[0.0, 0.0, 0.0, 1.0]).unwrap();
        s.finalize().unwrap();
        s
    }

    #[test]
    fn build_for_then_query_against_build_graph_ok_and_correct() {
        let ga = graph(A);
        let store_path = tmp("ok", "spqv");
        let store = build_store(&ga, &store_path);
        let idx_path = tmp("ok", "spqg");
        let idx = DiskAnnIndex::build_for(&store, &idx_path, &ga).unwrap();
        // (1) Checked against the build graph → OK, and bob is alice's nearest.
        assert!(idx.check_graph(&store, &ga).is_ok());
        let got = idx
            .nearest_term_checked(&iri("http://example.org/alice"), &ga, &store, 1)
            .expect("checked query against the build graph must succeed");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, iri("http://example.org/bob"));
        std::fs::remove_file(&store_path).ok();
        std::fs::remove_file(&idx_path).ok();
    }

    #[test]
    fn query_against_different_graph_errs() {
        let ga = graph(A);
        let store_path = tmp("mm", "spqv");
        let store = build_store(&ga, &store_path);
        let idx_path = tmp("mm", "spqg");
        let idx = DiskAnnIndex::build_for(&store, &idx_path, &ga).unwrap();
        let gb = graph(B);
        // (2) Against a DIFFERENT graph → descriptive Err, not silently wrong neighbours.
        assert!(idx.check_graph(&store, &gb).is_err());
        let qerr = idx
            .nearest_term_checked(&iri("http://example.org/dave"), &gb, &store, 1)
            .expect_err("a checked query against a mismatched graph must error");
        assert!(qerr.contains("mismatch") || qerr.contains("wrong results"), "err: {qerr}");
        std::fs::remove_file(&store_path).ok();
        std::fs::remove_file(&idx_path).ok();
    }

    #[test]
    fn fingerprint_survives_reopen() {
        let ga = graph(A);
        let store_path = tmp("rt", "spqv");
        let store = build_store(&ga, &store_path);
        let idx_path = tmp("rt", "spqg");
        DiskAnnIndex::build_for(&store, &idx_path, &ga).unwrap();
        // (3) Reopen the .spqg and confirm the stored fingerprint equals the live one.
        let reopened = DiskAnnIndex::open(&idx_path).unwrap();
        assert_eq!(reopened.fingerprint(), Some(Fingerprint::of(&ga)));
        std::fs::remove_file(&store_path).ok();
        std::fs::remove_file(&idx_path).ok();
    }

    #[test]
    fn v2_index_built_without_graph_binding_is_unverifiable_not_mismatch() {
        // (sq-32i5) A v2 `.spqg` built WITHOUT a graph binding writes an all-zero
        // fingerprint block; on reopen it must decode to `None` (reported as "unverifiable"),
        // NOT a zero fingerprint that would surface as a spurious "DIFFERENT graph" mismatch.
        let ga = graph(A);
        let store_path = tmp("nob", "spqv");
        let store = build_store(&ga, &store_path);
        let idx_path = tmp("nob", "spqg");
        DiskAnnIndex::build(&store, &idx_path).unwrap(); // no graph binding
        let idx = DiskAnnIndex::open(&idx_path).unwrap();
        assert_eq!(
            idx.fingerprint(),
            None,
            "all-zero block must decode to None"
        );
        let err = idx
            .check_graph(&store, &ga)
            .expect_err("an unbound index must not be certified");
        assert!(
            err.contains("carries no graph fingerprint"),
            "err must say unverifiable, not mismatch: {err}"
        );
        std::fs::remove_file(&store_path).ok();
        std::fs::remove_file(&idx_path).ok();
    }

    #[test]
    fn legacy_v1_spqg_opens_but_is_unverifiable() {
        // (4) A version-1 `.spqg`: build the default (no-fingerprint) v2 file, then rewrite its
        // header to a genuine v1 (version=1, fingerprint block dropped) so the legacy open path is
        // exercised. It still opens and searches; check_graph reports it unverifiable.
        let ga = graph(A);
        let store_path = tmp("legacy", "spqv");
        let store = build_store(&ga, &store_path);
        let idx_path = tmp("legacy", "spqg");
        DiskAnnIndex::build(&store, &idx_path).unwrap();
        let mut bytes = std::fs::read(&idx_path).unwrap();
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        bytes.drain(HEADER_LEN_V1..HEADER_LEN); // remove the 24-byte fingerprint block
        std::fs::write(&idx_path, &bytes).unwrap();

        let idx = DiskAnnIndex::open(&idx_path).expect("a legacy v1 .spqg must still open");
        assert!(idx.fingerprint().is_none());
        // It still searches correctly against the build graph (data layout is offset-32).
        let got = idx.nearest_term(&iri("http://example.org/alice"), &ga, &store, 1);
        assert_eq!(got[0].0, iri("http://example.org/bob"));
        // ...but cannot be verified.
        assert!(idx.check_graph(&store, &ga).is_err());
        std::fs::remove_file(&store_path).ok();
        std::fs::remove_file(&idx_path).ok();
    }
}
