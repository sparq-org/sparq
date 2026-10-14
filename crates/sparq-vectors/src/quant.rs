//! **Vector quantization** for large stores (sibling task **sq-nq5**): shrink the
//! per-vector memory footprint so 100M-scale stores fit a candidate cache in RAM, at a measured
//! cost in recall. Two encoders, both round-trippable and both able to *estimate* a distance
//! from the compressed codes alone:
//!
//! - [`ScalarQuantizer`] — **scalar quantization (SQ)**: each dimension is mapped to its own
//!   `[min, max]` range and linearly quantized `f32 → u8` (256 levels). 4× smaller (4 bytes →
//!   1 byte per dimension), trivially [`reconstruct`](ScalarQuantizer::reconstruct)ed for
//!   re-ranking. The standard cheap first step.
//! - [`ProductQuantizer`] — **product quantization (PQ)**: the vector is split into `M`
//!   contiguous subspaces, a `k`-means codebook of `K` centroids (default 256, so one code byte
//!   per subspace) is learned per subspace, and each vector is stored as its `M` nearest-centroid
//!   ids — `M` bytes total, independent of the original dimension. The classic 8–32× compression
//!   for billion-scale ANN. Distance to a full-precision query is computed by **asymmetric
//!   distance computation (ADC)**: precompute, per subspace, the distance from the query's
//!   sub-vector to all `K` centroids ([`DistanceTable`]), then a code's estimated distance is the
//!   sum of `M` table lookups — no decompression, one add per subspace.
//!
//! # Cosine convention (matches [`ann`](crate::ann) and [`diskann`](crate::diskann))
//!
//! Quantizers here are built over **L2-normalized** vectors and estimate **squared Euclidean**
//! distance, which is rank-equivalent to cosine on unit vectors (`cos = 1 − d²/2`). [`fit`] (both
//! quantizers) normalizes its training inputs, [`encode`] normalizes the vector it encodes, and
//! [`DistanceTable::new`] normalizes the query — so a caller can hand raw store vectors straight
//! in and the resulting ranking is cosine, identical to the exact/HNSW/DiskANN searchers. The
//! `cosine_*` helpers convert an estimated `d²` back to a cosine score for callers that want one.
//!
//! [`fit`]: ProductQuantizer::fit
//! [`encode`]: ProductQuantizer::encode
//!
//! # Intended use with [`DiskAnnIndex`](crate::diskann::DiskAnnIndex)
//!
//! This module is the **PQ-compressed in-RAM candidate cache** that closes `diskann.rs`'s honest
//! gap to *full* DiskANN (see that module's "Honest scope" note). The wiring, kept here as the
//! contract rather than baked into the `.spqg` format prematurely (no driving recall budget yet —
//! see this crate's open beads — `bd list -l area:sparq-vectors`):
//!
//! 1. After building the `.spqg` graph, [`fit`](ProductQuantizer::fit) a [`ProductQuantizer`] on
//!    the store's vectors and [`encode_store`](ProductQuantizer::encode_store) every vector into a
//!    flat `count × M`-byte code array — small enough to stay resident (e.g. 100M × 16 B = 1.6 GB
//!    where the f32 vectors would be 100M × dim × 4 B).
//! 2. The Vamana greedy search ranks candidates with [`DistanceTable`] lookups against those codes
//!    (RAM-only, no disk), and reads the full-precision vector from the mmap **only** for the
//!    final beam to re-rank — DiskANN's "search on PQ, re-rank on disk" loop.
//! 3. The codebook + SQ/PQ params persist alongside the graph; the `.spqg` header's 4 reserved
//!    bytes carry an encoding tag so this lands as a backwards-compatible format bump.
//!
//! The encoders/decoders/distance-estimators are complete and tested here; `search_slots` in
//! `diskann.rs` is unchanged (it still searches full-precision from the mmap), so nothing regresses
//! and the integration is a localized follow-up, not a rewrite.

use crate::store::VectorStore;
use sparq_core::dict::Id;

// ───────────────────────────── shared helpers ─────────────────────────────

/// L2 norm of `v`.
fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// L2-normalizes `v` into `out` (same length); leaves an all-zero vector unchanged (no
/// direction). Stored vectors are never zero ([`VectorStore::put`] rejects them); a degenerate
/// zero query simply yields a zero row, whose distances are uninformative but never panic.
fn normalize_into(v: &[f32], out: &mut [f32]) {
    let n = norm(v);
    if n > 0.0 {
        for (o, &x) in out.iter_mut().zip(v) {
            *o = x / n;
        }
    } else {
        out.copy_from_slice(v);
    }
}

/// Squared Euclidean distance over two equal-length vectors. Eight accumulator lanes so the loop
/// auto-vectorizes — mirrors [`ann::cosine`](crate::ann::cosine) and [`diskann`]'s `sq_dist`.
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

/// Converts an estimated squared-Euclidean distance over unit vectors back to cosine similarity
/// (`cos = 1 − d²/2`). The inverse of the mapping the exact/HNSW/DiskANN searchers use, so a
/// quantized estimate is directly comparable to their scores.
pub fn cosine_from_sq_dist(sq: f32) -> f32 {
    1.0 - sq / 2.0
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// ───────────────────────────── scalar quantization ─────────────────────────────

/// **Per-dimension scalar quantizer** (`f32 → u8`, 4× smaller). Each of the `dim` dimensions is
/// linearly mapped from its own learned `[min, max]` range onto the 256 integer levels `0..=255`;
/// [`encode`](Self::encode) rounds to the nearest level and [`reconstruct`](Self::reconstruct)
/// inverts it. The max per-component error is half a quantization step (`range / 510`), so the
/// reconstruction error is bounded and small for the bounded-range, normalized vectors this crate
/// stores — see the round-trip test. SQ codes can serve as a cheap re-rank input or a 4×-smaller
/// candidate cache on their own.
#[derive(Clone, Debug)]
pub struct ScalarQuantizer {
    dim: usize,
    /// Per-dimension lower bound (the value mapped to level 0).
    min: Vec<f32>,
    /// Per-dimension `(max − min) / 255` step; 0 for a constant dimension (encodes to level 0).
    step: Vec<f32>,
}

impl ScalarQuantizer {
    /// Learns the per-dimension `[min, max]` ranges from `vectors` (each L2-normalized first, the
    /// cosine convention) and returns the fitted quantizer. Empty input yields a degenerate
    /// quantizer whose ranges are all zero (everything encodes to level 0); callers fit on a
    /// non-empty store. Errors on a dimension mismatch.
    pub fn fit<'a, I>(dim: usize, vectors: I) -> Result<ScalarQuantizer, String>
    where
        I: IntoIterator<Item = &'a [f32]>,
    {
        if dim == 0 {
            return Err("scalar quantizer dim must be > 0".into());
        }
        let mut min = vec![f32::INFINITY; dim];
        let mut max = vec![f32::NEG_INFINITY; dim];
        let mut buf = vec![0f32; dim];
        let mut seen = false;
        for v in vectors {
            if v.len() != dim {
                return Err(format!("vector has dim {}, quantizer has dim {dim}", v.len()));
            }
            normalize_into(v, &mut buf);
            for d in 0..dim {
                min[d] = min[d].min(buf[d]);
                max[d] = max[d].max(buf[d]);
            }
            seen = true;
        }
        if !seen {
            // Degenerate: no training data. Zero ranges → all-zero codes, reconstruct → 0.
            return Ok(ScalarQuantizer { dim, min: vec![0.0; dim], step: vec![0.0; dim] });
        }
        let step: Vec<f32> = (0..dim)
            .map(|d| {
                let r = max[d] - min[d];
                if r > 0.0 {
                    r / 255.0
                } else {
                    0.0
                }
            })
            .collect();
        Ok(ScalarQuantizer { dim, min, step })
    }

    /// The vector dimension this quantizer was fitted for.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Encodes one vector to `dim` bytes (L2-normalized first). A component below the learned min
    /// clamps to 0 and above the learned max clamps to 255, so an out-of-training-range query
    /// vector still encodes safely. Panics on a dimension mismatch (a programming error).
    pub fn encode(&self, v: &[f32]) -> Vec<u8> {
        assert_eq!(v.len(), self.dim, "encode dim {} != quantizer dim {}", v.len(), self.dim);
        let mut buf = vec![0f32; self.dim];
        normalize_into(v, &mut buf);
        let mut out = vec![0u8; self.dim];
        self.encode_normalized_into(&buf, &mut out);
        out
    }

    /// Encodes an already-normalized vector into `out` (length `dim`). Internal fast path for
    /// [`encode_store`](Self::encode_store) so the normalize buffer is reused across the store.
    fn encode_normalized_into(&self, norm_v: &[f32], out: &mut [u8]) {
        for d in 0..self.dim {
            out[d] = if self.step[d] > 0.0 {
                let level = ((norm_v[d] - self.min[d]) / self.step[d]).round();
                level.clamp(0.0, 255.0) as u8
            } else {
                0
            };
        }
    }

    /// Reconstructs an approximate (still normalized-space) vector from its `dim` code bytes:
    /// `min[d] + code[d] · step[d]`. The input to re-ranking — recompute an exact cosine against
    /// this if you want SQ-only ranking, or treat it as the lossy preview before a full-precision
    /// re-rank. Panics on a code length mismatch.
    pub fn reconstruct(&self, code: &[u8]) -> Vec<f32> {
        assert_eq!(code.len(), self.dim, "code len {} != quantizer dim {}", code.len(), self.dim);
        (0..self.dim).map(|d| self.min[d] + code[d] as f32 * self.step[d]).collect()
    }

    /// Encodes every vector in `store` into a flat `count × dim`-byte array (slot order, matching
    /// [`VectorStore::iter`]), paired with the slot→id mapping so a caller can map a ranked slot
    /// back to a term id. The 4×-smaller candidate cache. Errors on a store/quantizer dim
    /// mismatch.
    pub fn encode_store(&self, store: &VectorStore) -> Result<EncodedStore, String> {
        if store.dim() != self.dim {
            return Err(format!("store dim {} != quantizer dim {}", store.dim(), self.dim));
        }
        let mut codes = Vec::with_capacity(store.len() * self.dim);
        let mut ids = Vec::with_capacity(store.len());
        let mut buf = vec![0f32; self.dim];
        let mut row = vec![0u8; self.dim];
        for (id, v) in store.iter() {
            normalize_into(v, &mut buf);
            self.encode_normalized_into(&buf, &mut row);
            codes.extend_from_slice(&row);
            ids.push(id);
        }
        Ok(EncodedStore { ids, codes, stride: self.dim })
    }
}

// ───────────────────────────── product quantization ─────────────────────────────

/// **Product quantizer** (`M` bytes per vector, 8–32× smaller). The vector is split into `M`
/// contiguous subspaces; a `k`-means codebook of `K` (= [`Self::k`], default 256) centroids is
/// learned per subspace, and a vector is encoded as the `M` ids of its nearest centroid in each
/// subspace. With `K = 256` each id is one byte, so a code is exactly `M` bytes regardless of the
/// original dimension. Distance to a full-precision query uses [`DistanceTable`] (ADC).
///
/// `dim` need not be divisible by `M`: subspaces are sized `ceil(dim/M)` for the first
/// `dim mod M` of them and `floor(dim/M)` for the rest (every dimension covered exactly once).
#[derive(Clone, Debug)]
pub struct ProductQuantizer {
    dim: usize,
    m: usize,
    k: usize,
    /// Subspace `s` spans dimensions `sub_offsets[s] .. sub_offsets[s+1]` (len `m + 1`).
    sub_offsets: Vec<usize>,
    /// Codebooks, one per subspace: `centroids[s]` is `k` rows of `sub_len(s)` f32 each, row-major.
    centroids: Vec<Vec<f32>>,
}

/// Tuning for [`ProductQuantizer::fit`].
#[derive(Clone, Copy, Debug)]
pub struct PqConfig {
    /// Number of subspaces `M` (= code length in bytes when `k = 256`). Higher `M` = better
    /// recall, larger codes. Must divide the work so each subspace has ≥ 1 dimension (`M ≤ dim`).
    pub m: usize,
    /// Centroids per subspace `K`. 256 keeps a code byte-addressable (the standard choice);
    /// larger `K` would need wider codes, but [`ProductQuantizer::encode`] returns `u8`, so `fit`
    /// rejects `k > 256` for that reason.
    pub k: usize,
    /// Lloyd's-algorithm iterations for each subspace's k-means. ~10–25 is plenty for the small
    /// subspace dimensions PQ produces.
    pub iters: usize,
    /// k-means++ / assignment RNG seed — fixed by default so codebooks are reproducible.
    pub seed: u64,
}

impl Default for PqConfig {
    fn default() -> Self {
        // M = 16 subspaces × 256 centroids: 16 bytes/vector. For dim=32 (the synthetic gate set)
        // that is two dims per subspace, 8× compression vs 128-byte f32; for dim=128 it is 32×.
        PqConfig { m: 16, k: 256, iters: 20, seed: 0x5350_5051_0001 }
    }
}

impl ProductQuantizer {
    /// Learns the per-subspace codebooks from `vectors` (each L2-normalized first — the cosine
    /// convention) by `k`-means (k-means++ seeding, Lloyd iterations). Errors on an invalid config
    /// (`m == 0`, `m > dim`, `k == 0`, `k > 256`), a dimension mismatch, or an empty training set
    /// (k-means needs at least one point; a caller fits on a non-empty store).
    pub fn fit<'a, I>(dim: usize, vectors: I, cfg: PqConfig) -> Result<ProductQuantizer, String>
    where
        I: IntoIterator<Item = &'a [f32]>,
    {
        if dim == 0 {
            return Err("product quantizer dim must be > 0".into());
        }
        if cfg.m == 0 || cfg.m > dim {
            return Err(format!("M must be in 1..={dim} (got {})", cfg.m));
        }
        if cfg.k == 0 || cfg.k > 256 {
            return Err(format!("K must be in 1..=256 for u8 codes (got {})", cfg.k));
        }
        let sub_offsets = subspace_offsets(dim, cfg.m);

        // Collect normalized training vectors once (k-means makes many passes).
        let mut train: Vec<f32> = Vec::new();
        let mut n = 0usize;
        let mut buf = vec![0f32; dim];
        for v in vectors {
            if v.len() != dim {
                return Err(format!("vector has dim {}, quantizer has dim {dim}", v.len()));
            }
            normalize_into(v, &mut buf);
            train.extend_from_slice(&buf);
            n += 1;
        }
        if n == 0 {
            return Err("product quantizer needs at least one training vector".into());
        }

        let mut state = cfg.seed;
        let centroids: Vec<Vec<f32>> = (0..cfg.m)
            .map(|s| {
                let (lo, hi) = (sub_offsets[s], sub_offsets[s + 1]);
                let sub_len = hi - lo;
                // Gather subspace s of every training vector contiguously for cache-friendly kmeans.
                let mut sub: Vec<f32> = Vec::with_capacity(n * sub_len);
                for r in 0..n {
                    sub.extend_from_slice(&train[r * dim + lo..r * dim + hi]);
                }
                kmeans(&sub, n, sub_len, cfg.k, cfg.iters, &mut state)
            })
            .collect();

        Ok(ProductQuantizer { dim, m: cfg.m, k: cfg.k, sub_offsets, centroids })
    }

    /// The vector dimension this quantizer was fitted for.
    pub fn dim(&self) -> usize {
        self.dim
    }
    /// Number of subspaces `M` (= code length in bytes).
    pub fn m(&self) -> usize {
        self.m
    }
    /// Centroids per subspace `K`.
    pub fn k(&self) -> usize {
        self.k
    }

    /// Length of subspace `s`.
    fn sub_len(&self, s: usize) -> usize {
        self.sub_offsets[s + 1] - self.sub_offsets[s]
    }

    /// The `c`-th centroid of subspace `s`.
    fn centroid(&self, s: usize, c: usize) -> &[f32] {
        let len = self.sub_len(s);
        &self.centroids[s][c * len..(c + 1) * len]
    }

    /// Encodes one vector to its `M`-byte PQ code (L2-normalized first): for each subspace, the id
    /// of the nearest centroid. Panics on a dimension mismatch (a programming error).
    pub fn encode(&self, v: &[f32]) -> Vec<u8> {
        assert_eq!(v.len(), self.dim, "encode dim {} != quantizer dim {}", v.len(), self.dim);
        let mut buf = vec![0f32; self.dim];
        normalize_into(v, &mut buf);
        let mut out = vec![0u8; self.m];
        self.encode_normalized_into(&buf, &mut out);
        out
    }

    /// Encodes an already-normalized vector into `out` (length `M`). Internal fast path for
    /// [`encode_store`](Self::encode_store).
    fn encode_normalized_into(&self, norm_v: &[f32], out: &mut [u8]) {
        for (s, code_s) in out.iter_mut().enumerate() {
            let (lo, hi) = (self.sub_offsets[s], self.sub_offsets[s + 1]);
            let subv = &norm_v[lo..hi];
            let mut best = 0usize;
            let mut best_d = f32::INFINITY;
            for c in 0..self.k {
                let d = sq_dist(subv, self.centroid(s, c));
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            *code_s = best as u8;
        }
    }

    /// Reconstructs an approximate (normalized-space) vector by concatenating the chosen centroid
    /// of each subspace — the PQ re-rank preview. Panics on a code length mismatch.
    pub fn reconstruct(&self, code: &[u8]) -> Vec<f32> {
        assert_eq!(code.len(), self.m, "code len {} != M {}", code.len(), self.m);
        let mut out = vec![0f32; self.dim];
        for (s, &c) in code.iter().enumerate() {
            let (lo, hi) = (self.sub_offsets[s], self.sub_offsets[s + 1]);
            out[lo..hi].copy_from_slice(self.centroid(s, c as usize));
        }
        out
    }

    /// (sq-qamd) Serializes the codebook to a self-describing little-endian byte block
    /// — `dim, m, k` (u32 each) then every centroid value as f32 — so it can be persisted alongside
    /// the on-disk graph and reloaded with [`from_bytes`](Self::from_bytes). The subspace layout is
    /// recomputed from `dim`/`m` on load (it is a pure function of the two), so only the learned
    /// centroids are stored. Round-trips exactly (no lossy step).
    pub fn to_bytes(&self) -> Vec<u8> {
        let n_centroid_vals: usize = self.centroids.iter().map(Vec::len).sum();
        let mut out = Vec::with_capacity(12 + n_centroid_vals * 4);
        out.extend_from_slice(&(self.dim as u32).to_le_bytes());
        out.extend_from_slice(&(self.m as u32).to_le_bytes());
        out.extend_from_slice(&(self.k as u32).to_le_bytes());
        for sub in &self.centroids {
            for &val in sub {
                out.extend_from_slice(&val.to_le_bytes());
            }
        }
        out
    }

    /// (sq-qamd) Reconstructs a quantizer from [`to_bytes`](Self::to_bytes). Errors on a
    /// truncated/over-long block or an invalid header (`m == 0`, `m > dim`, `k == 0`, `k > 256`) —
    /// the same validity envelope [`fit`](Self::fit) enforces, so a decoded quantizer is always sound.
    pub fn from_bytes(bytes: &[u8]) -> Result<ProductQuantizer, String> {
        if bytes.len() < 12 {
            return Err("PQ codebook block truncated (no header)".into());
        }
        let rd = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
        let (dim, m, k) = (rd(0), rd(4), rd(8));
        if dim == 0 {
            return Err("PQ codebook dim must be > 0".into());
        }
        if m == 0 || m > dim {
            return Err(format!("PQ codebook M must be in 1..={dim} (got {m})"));
        }
        if k == 0 || k > 256 {
            return Err(format!("PQ codebook K must be in 1..=256 (got {k})"));
        }
        let sub_offsets = subspace_offsets(dim, m);
        let mut centroids: Vec<Vec<f32>> = Vec::with_capacity(m);
        let mut off = 12;
        for s in 0..m {
            let sub_len = sub_offsets[s + 1] - sub_offsets[s];
            let want = k * sub_len;
            let need = off + want * 4;
            if bytes.len() < need {
                return Err("PQ codebook block truncated (centroids)".into());
            }
            let mut sub = Vec::with_capacity(want);
            for c in 0..want {
                let p = off + c * 4;
                sub.push(f32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()));
            }
            centroids.push(sub);
            off = need;
        }
        if off != bytes.len() {
            return Err(format!("PQ codebook block has {} trailing bytes", bytes.len() - off));
        }
        Ok(ProductQuantizer { dim, m, k, sub_offsets, centroids })
    }

    /// Encodes every vector in `store` into a flat `count × M`-byte code array (slot order,
    /// matching [`VectorStore::iter`]) plus the slot→id mapping. This is the in-RAM candidate
    /// cache for [`DiskAnnIndex`](crate::diskann::DiskAnnIndex). Errors on a dim mismatch.
    pub fn encode_store(&self, store: &VectorStore) -> Result<EncodedStore, String> {
        if store.dim() != self.dim {
            return Err(format!("store dim {} != quantizer dim {}", store.dim(), self.dim));
        }
        let mut codes = Vec::with_capacity(store.len() * self.m);
        let mut ids = Vec::with_capacity(store.len());
        let mut buf = vec![0f32; self.dim];
        let mut row = vec![0u8; self.m];
        for (id, v) in store.iter() {
            normalize_into(v, &mut buf);
            self.encode_normalized_into(&buf, &mut row);
            codes.extend_from_slice(&row);
            ids.push(id);
        }
        Ok(EncodedStore { ids, codes, stride: self.m })
    }
}

/// The flat `subspace_offsets`: subspace boundaries when splitting `dim` into `m` parts as evenly
/// as possible (`ceil` for the first `dim % m`, `floor` for the rest). Length `m + 1`.
fn subspace_offsets(dim: usize, m: usize) -> Vec<usize> {
    let base = dim / m;
    let rem = dim % m;
    let mut offsets = Vec::with_capacity(m + 1);
    let mut acc = 0;
    offsets.push(0);
    for s in 0..m {
        acc += base + usize::from(s < rem);
        offsets.push(acc);
    }
    offsets
}

/// k-means (k-means++ seed + Lloyd iterations) over `n` points of `sub_len` dims each (`data` is
/// row-major, `n × sub_len`). Returns `k` centroids row-major (`k × sub_len`). When `n < k`, the
/// `n` points become the first `n` centroids and the rest are duplicated from point 0 (so empty
/// clusters never produce NaN centroids — they just never win an assignment).
fn kmeans(data: &[f32], n: usize, sub_len: usize, k: usize, iters: usize, state: &mut u64) -> Vec<f32> {
    debug_assert_eq!(data.len(), n * sub_len);
    let point = |i: usize| &data[i * sub_len..(i + 1) * sub_len];
    let mut centroids = vec![0f32; k * sub_len];
    let set = |c: &mut [f32], i: usize, src: &[f32]| {
        c[i * sub_len..(i + 1) * sub_len].copy_from_slice(src);
    };

    // k-means++ seeding: first centre uniform, each next with prob ∝ D²(point, nearest chosen).
    let first = (splitmix64(state) % n as u64) as usize;
    set(&mut centroids, 0, point(first));
    let mut d2 = vec![f32::INFINITY; n];
    for c in 1..k {
        // Update nearest-centre squared distance with the newly added centre c-1.
        let added = &centroids[(c - 1) * sub_len..c * sub_len].to_vec();
        let mut total = 0f64;
        for (i, d2_i) in d2.iter_mut().enumerate() {
            let d = sq_dist(point(i), added);
            if d < *d2_i {
                *d2_i = d;
            }
            total += *d2_i as f64;
        }
        let chosen = if total <= 0.0 {
            // All remaining points coincide with chosen centres: fall back to a uniform pick.
            (splitmix64(state) % n as u64) as usize
        } else {
            let target = (splitmix64(state) as f64 / u64::MAX as f64) * total;
            let mut acc = 0f64;
            let mut pick = n - 1;
            for (i, &d) in d2.iter().enumerate() {
                acc += d as f64;
                if acc >= target {
                    pick = i;
                    break;
                }
            }
            pick
        };
        set(&mut centroids, c, point(chosen));
    }

    // Lloyd iterations.
    let mut assign = vec![0usize; n];
    let mut sums = vec![0f32; k * sub_len];
    let mut counts = vec![0u32; k];
    for _ in 0..iters {
        // Assignment.
        let mut changed = false;
        for (i, assign_i) in assign.iter_mut().enumerate() {
            let p = point(i);
            let mut best = 0usize;
            let mut best_d = f32::INFINITY;
            for c in 0..k {
                let d = sq_dist(p, &centroids[c * sub_len..(c + 1) * sub_len]);
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            if *assign_i != best {
                *assign_i = best;
                changed = true;
            }
        }
        // Update.
        sums.iter_mut().for_each(|x| *x = 0.0);
        counts.iter_mut().for_each(|x| *x = 0);
        for (i, &c) in assign.iter().enumerate() {
            counts[c] += 1;
            let p = point(i);
            for (acc, &x) in sums[c * sub_len..(c + 1) * sub_len].iter_mut().zip(p) {
                *acc += x;
            }
        }
        for (c, &count) in counts.iter().enumerate() {
            if count > 0 {
                let inv = 1.0 / count as f32;
                for (cv, &acc) in centroids[c * sub_len..(c + 1) * sub_len]
                    .iter_mut()
                    .zip(&sums[c * sub_len..(c + 1) * sub_len])
                {
                    *cv = acc * inv;
                }
            }
            // Empty cluster: leave the centroid where it is (it simply never wins).
        }
        if !changed {
            break;
        }
    }
    centroids
}

/// **Asymmetric distance table** for one query against a [`ProductQuantizer`]: per subspace, the
/// squared-Euclidean distance from the query's sub-vector to each of the `K` centroids. A code's
/// estimated `d²` is then `Σₛ table[s][code[s]]` — `M` adds, no decompression. The "asymmetric"
/// of ADC: the query stays full-precision, only the database vectors are quantized, which is more
/// accurate than quantizing both (symmetric DC).
pub struct DistanceTable {
    m: usize,
    k: usize,
    /// `m × k` squared distances, row-major (`tables[s * k + c]`).
    tables: Vec<f32>,
}

impl DistanceTable {
    /// Builds the table for `query` (L2-normalized first, the cosine convention) against `pq`.
    /// Panics on a dimension mismatch.
    pub fn new(pq: &ProductQuantizer, query: &[f32]) -> DistanceTable {
        assert_eq!(query.len(), pq.dim, "query dim {} != quantizer dim {}", query.len(), pq.dim);
        let mut buf = vec![0f32; pq.dim];
        normalize_into(query, &mut buf);
        let mut tables = vec![0f32; pq.m * pq.k];
        for s in 0..pq.m {
            let (lo, hi) = (pq.sub_offsets[s], pq.sub_offsets[s + 1]);
            let subq = &buf[lo..hi];
            for c in 0..pq.k {
                tables[s * pq.k + c] = sq_dist(subq, pq.centroid(s, c));
            }
        }
        DistanceTable { m: pq.m, k: pq.k, tables }
    }

    /// Estimated squared-Euclidean distance from the query to a PQ `code` (the ADC sum). Panics on
    /// a code length mismatch.
    pub fn distance(&self, code: &[u8]) -> f32 {
        assert_eq!(code.len(), self.m, "code len {} != M {}", code.len(), self.m);
        let mut acc = 0f32;
        for (s, &c) in code.iter().enumerate() {
            acc += self.tables[s * self.k + c as usize];
        }
        acc
    }

    /// Estimated cosine similarity from the query to a PQ `code` (`1 − d²/2`), directly comparable
    /// to the exact/HNSW/DiskANN cosine scores.
    pub fn cosine(&self, code: &[u8]) -> f32 {
        cosine_from_sq_dist(self.distance(code))
    }
}

// ───────────────────────────── encoded store ─────────────────────────────

/// A store's vectors encoded to fixed-length codes (SQ or PQ) plus the slot→id mapping — the
/// **in-RAM candidate cache**. `codes` is `len × stride` bytes in slot order (matching
/// [`VectorStore::iter`]); `stride` is `dim` for [`ScalarQuantizer`] or `M` for
/// [`ProductQuantizer`]. Ranking over this cache then re-ranking the top candidates against the
/// full-precision store (or [`DiskAnnIndex`](crate::diskann::DiskAnnIndex)'s mmap) is the DiskANN
/// search loop.
#[derive(Clone, Debug)]
pub struct EncodedStore {
    ids: Vec<Id>,
    codes: Vec<u8>,
    stride: usize,
}

impl EncodedStore {
    /// (sq-qamd) Builds an [`EncodedStore`] from a slot→id mapping and the flat
    /// `len × stride` code array (slot order) — the inverse of the `ids`/`codes` an encoder
    /// produces, used to reload a persisted candidate cache. Errors if `codes.len()` is not
    /// `ids.len() × stride` (or `stride == 0` with codes present), so a malformed block can't
    /// produce out-of-bounds [`code`](Self::code) reads.
    pub fn from_parts(ids: Vec<Id>, codes: Vec<u8>, stride: usize) -> Result<EncodedStore, String> {
        if stride == 0 && !codes.is_empty() {
            return Err("EncodedStore stride must be > 0 when codes are present".into());
        }
        let expect = ids.len().checked_mul(stride).ok_or("EncodedStore code length overflow")?;
        if codes.len() != expect {
            return Err(format!(
                "EncodedStore codes len {} != ids {} × stride {stride}",
                codes.len(),
                ids.len()
            ));
        }
        Ok(EncodedStore { ids, codes, stride })
    }

    /// Number of encoded vectors.
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    /// Bytes per code (`dim` for SQ, `M` for PQ).
    pub fn stride(&self) -> usize {
        self.stride
    }
    /// The term id at `slot`.
    pub fn id(&self, slot: usize) -> Id {
        self.ids[slot]
    }
    /// The code bytes for `slot`.
    pub fn code(&self, slot: usize) -> &[u8] {
        &self.codes[slot * self.stride..(slot + 1) * self.stride]
    }
    /// (sq-qamd) The flat `len × stride` code array (slot order) — for persisting the
    /// candidate cache; pair with [`from_parts`](Self::from_parts) to reload it.
    pub fn codes(&self) -> &[u8] {
        &self.codes
    }
    /// Iterates `(slot, id, code)` in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, Id, &[u8])> + '_ {
        (0..self.len()).map(move |slot| (slot, self.ids[slot], self.code(slot)))
    }

    /// PQ-only convenience: rank every code by ADC distance to `table` and return the top-`k`
    /// `(Id, cosine)` pairs, best first — the RAM-only candidate ranking that
    /// [`DiskAnnIndex`](crate::diskann::DiskAnnIndex) would re-rank on disk. Ties break on
    /// ascending id, matching [`nearest_exact`](crate::ann::nearest_exact). The caller must pass a
    /// table built from the same [`ProductQuantizer`] that produced these codes (the `stride`
    /// must equal the table's `M`); a mismatch panics in [`DistanceTable::distance`].
    pub fn rank_pq(&self, table: &DistanceTable, k: usize) -> Vec<(Id, f32)> {
        let mut scored: Vec<(Id, f32)> = self
            .iter()
            .map(|(_, id, code)| (id, table.distance(code)))
            .collect();
        // Ascending distance = descending cosine; tie-break ascending id.
        scored.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        scored.truncate(k);
        scored.into_iter().map(|(id, d)| (id, cosine_from_sq_dist(d))).collect()
    }
}
