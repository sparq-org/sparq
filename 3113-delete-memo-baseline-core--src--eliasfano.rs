//! [SONNET-4.6] sq-96hp1 (survey §A2 / `research/data-structures.md` §B1) — OPT-IN
//! Elias-Fano column codecs with **compressed seek**: `next_geq(target)` answered *directly on
//! the compressed representation*, without decoding a block first.
//!
//! # Why this exists
//!
//! [`crate::compress`] stores a permutation as fixed-count blocks of lexicographic-delta +
//! LEB128-varint rows. That is a good *ratio* codec, but it is **decode-then-search**: a seek
//! for the first row ≥ some target must decode a whole 128-row block even when it wants one
//! row. Elias-Fano is alone in the high-ratio family in supporting near-O(1) successor
//! (`NextGEQ`) queries on the compressed data — exactly the primitive merge-join galloping,
//! scan↔scan block pruning and a Leapfrog `seek()` all call.
//!
//! # Status: PROTOTYPE, MEASUREMENT-GATED — not wired into the store
//!
//! This module deliberately ships the codec and the A/B measurement harness ONLY. It is **not**
//! routed as an alternate `PermData` variant and no join calls it yet, because the bead that
//! commissioned it makes adoption conditional on measurement: Elias-Fano can *lose* to the
//! incumbent block codec on cache-resident streaming scans, where its select/rank is
//! pointer-chasing while the varint path is a linear byte walk. ZSTD/varint blocks stay the
//! default. The A/B — seek latency, scan throughput, bytes moved, heap, on several column
//! shapes — is `measure_ef_vs_block_varint` below (`#[ignore]`d; run it explicitly). It is
//! **NATIVE-ONLY**: it and its varint baseline are `#[cfg(not(target_arch = "wasm32"))]`,
//! because they time with [`std::time::Instant`], which is unavailable on
//! `wasm32-unknown-unknown`. **`wasm32` measurement is therefore UNRESOLVED** — it needs a
//! `performance.now()`-based harness under a wasm test runner, which this prototype does not
//! ship, and it must be answered before any adoption decision that claims a wasm win.
//! **No performance number is asserted here or in any doc.**
//!
//! # What is implemented
//!
//! * [`EliasFano`] — plain (non-partitioned) Elias-Fano over a **non-decreasing** `u64`
//!   sequence. Quasi-succinct: ~`2 + log2(u/n)` bits per value plus a small sampled
//!   select index.
//! * [`PartitionedEliasFano`] — fixed-size-partition PEF: an upper-level [`EliasFano`] over the
//!   partition endpoints, plus one [`EliasFano`] per partition encoding values relative to the
//!   previous partition's endpoint. This is the *fixed-size* partitioning, NOT the
//!   DP-optimal partitioning of Ottaviano/Venturini (SIGIR'14) — it captures PEF's locality
//!   win without the optimisation pass, which is the right first prototype for an A/B.
//!
//! Both are pure `std` (no new dependency, no SIMD intrinsics), so the same code compiles for
//! `wasm32` as for native — which is the point: EF's win is meant to come from moving fewer
//! bytes, not from wide lanes. That is a property of the *source*, not a measured claim: no
//! harness here compares the two codecs' behaviour on `wasm32` (see the status note above).
//!
//! # Applicability to a permutation column
//!
//! Elias-Fano requires a **monotone** sequence. A permutation's trailing column is monotone
//! *within* one leading-prefix group (e.g. the objects of one `(s, p)` in SPO), not across the
//! whole column — so the eventual routing encodes one EF per group, or per partition of a
//! group-aligned run. Choosing that segmentation, and the per-relation NDV/clustering gate that
//! picks EF over ZSTD, is the adoption step and is out of scope here.

/// Sampling rate of the select directories: [`EliasFano`] stores the bit position of every
/// `SAMPLE`-th one and every `SAMPLE`-th zero, and `select` scans forward from the sample.
/// 64 keeps both directories at ~1 bit per value combined while bounding the forward scan to
/// at most 63 further set bits.
const SAMPLE: usize = 64;

/// Position of the `k`-th (0-indexed) set bit of `word`. `k` must be less than
/// `word.count_ones()`; callers establish that from a popcount before calling.
#[inline]
fn select_in_word(mut word: u64, k: u32) -> usize {
    for _ in 0..k {
        word &= word - 1; // clear the lowest set bit
    }
    word.trailing_zeros() as usize
}

/// A non-decreasing `u64` sequence in Elias-Fano form, supporting random access
/// ([`EliasFano::get`]), sequential decode ([`EliasFano::iter`]) and **compressed successor
/// search** ([`EliasFano::next_geq`]).
///
/// Layout (the classical one): with `n` values and universe `u = max + 1`, each value is split
/// into `l = floor(log2(u / n))` low bits — packed contiguously in `lo` — and the remaining
/// high bits `h_i`, written into `hi` as a bit at position `h_i + i`. Because `h` is
/// non-decreasing and bounded by ~`n`, `hi` holds `n` ones and ~`n` zeros, and the number of
/// zeros before the `i`-th one is exactly `h_i`. That identity is what makes successor search
/// a `select0` plus a short scan of one high-bucket.
pub struct EliasFano {
    /// High-bits bitvector, `hi_bits` significant bits, little-endian within each word.
    hi: Vec<u64>,
    /// Low bits, `l` per value, packed with no padding between values.
    lo: Vec<u64>,
    /// Bit position of the `SAMPLE * k`-th one, for `k = 0, 1, …`.
    sel1: Vec<u32>,
    /// Bit position of the `SAMPLE * k`-th zero, for `k = 0, 1, …`.
    sel0: Vec<u32>,
    /// Number of low bits per value (0 when the sequence is denser than its universe).
    l: u32,
    /// Number of encoded values.
    n: usize,
    /// Significant bit count of `hi` (`n + (max >> l) + 1`).
    hi_bits: usize,
}

impl EliasFano {
    /// Encodes `values`, which MUST be non-decreasing.
    ///
    /// Returns `None` when `values` is not non-decreasing (the caller has mis-segmented the
    /// column — EF is undefined on a non-monotone sequence), or when the high-bits vector would
    /// exceed `u32::MAX` bits, which is the addressing limit of the sampled select directories.
    /// An empty slice encodes successfully to an empty sequence.
    #[must_use]
    pub fn encode(values: &[u64]) -> Option<Self> {
        let n = values.len();
        if n == 0 {
            return Some(Self {
                hi: Vec::new(),
                lo: Vec::new(),
                sel1: Vec::new(),
                sel0: Vec::new(),
                l: 0,
                n: 0,
                hi_bits: 0,
            });
        }
        if values.windows(2).any(|w| w[0] > w[1]) {
            return None;
        }
        let max = values[n - 1];
        // `l = floor(log2(u / n))` balances the two halves: it makes the high-bits vector hold
        // ~n zeros, so `hi` stays ~2n bits regardless of how large the values are.
        let q = max.saturating_add(1) / n as u64;
        let l = if q == 0 { 0 } else { 63 - q.leading_zeros() };
        let h_max = max >> l;
        // +1 trailing zero so `select0(h_max)` is always in range.
        let hi_bits = (n as u64).checked_add(h_max)?.checked_add(1)?;
        if hi_bits > u64::from(u32::MAX) {
            return None;
        }
        let hi_bits = hi_bits as usize;

        let mut hi = vec![0u64; hi_bits.div_ceil(64)];
        let lo_bits = n * l as usize;
        let mut lo = vec![0u64; lo_bits.div_ceil(64)];
        let lo_mask = if l == 0 { 0 } else { u64::MAX >> (64 - l) };
        for (i, &v) in values.iter().enumerate() {
            let p = (v >> l) as usize + i;
            hi[p / 64] |= 1u64 << (p % 64);
            if l > 0 {
                let bit = i * l as usize;
                let (w, off) = (bit / 64, bit % 64);
                let low = v & lo_mask;
                lo[w] |= low << off;
                if off + l as usize > 64 {
                    lo[w + 1] |= low >> (64 - off);
                }
            }
        }

        // One linear pass to sample both select directories. O(hi_bits) at build time only.
        let mut sel1 = Vec::with_capacity(n / SAMPLE + 1);
        let mut sel0 = Vec::with_capacity(hi_bits / SAMPLE + 1);
        let (mut ones, mut zeros) = (0usize, 0usize);
        for p in 0..hi_bits {
            if hi[p / 64] >> (p % 64) & 1 == 1 {
                if ones % SAMPLE == 0 {
                    sel1.push(p as u32);
                }
                ones += 1;
            } else {
                if zeros % SAMPLE == 0 {
                    sel0.push(p as u32);
                }
                zeros += 1;
            }
        }
        Some(Self { hi, lo, sel1, sel0, l, n, hi_bits })
    }

    /// Number of encoded values.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    /// `true` when no values are encoded.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// The `l` low bits of value `i`.
    #[inline]
    fn low(&self, i: usize) -> u64 {
        if self.l == 0 {
            return 0;
        }
        let bit = i * self.l as usize;
        let (w, off) = (bit / 64, bit % 64);
        let mut x = self.lo[w] >> off;
        if off + self.l as usize > 64 {
            x |= self.lo[w + 1] << (64 - off);
        }
        x & (u64::MAX >> (64 - self.l))
    }

    /// Bit position of the `i`-th one in `hi`. `i` must be `< self.n`.
    #[inline]
    fn select1(&self, i: usize) -> usize {
        let k = i / SAMPLE;
        let start = self.sel1[k] as usize;
        let mut rem = i - k * SAMPLE;
        let mut w = start / 64;
        let mut word = self.hi[w] & (u64::MAX << (start % 64));
        loop {
            let c = word.count_ones() as usize;
            if rem < c {
                return w * 64 + select_in_word(word, rem as u32);
            }
            rem -= c;
            w += 1;
            word = self.hi[w];
        }
    }

    /// The `w`-th word of `hi` complemented, with any bits past `hi_bits` cleared so they are
    /// not mistaken for real zeros by [`EliasFano::select0`].
    #[inline]
    fn zero_word(&self, w: usize) -> u64 {
        let x = !self.hi[w];
        let valid = self.hi_bits - w * 64;
        if valid < 64 { x & (u64::MAX >> (64 - valid)) } else { x }
    }

    /// Bit position of the `j`-th zero in `hi`. `j` must be less than the zero count.
    #[inline]
    fn select0(&self, j: usize) -> usize {
        let k = j / SAMPLE;
        let start = self.sel0[k] as usize;
        let mut rem = j - k * SAMPLE;
        let mut w = start / 64;
        let mut word = self.zero_word(w) & (u64::MAX << (start % 64));
        loop {
            let c = word.count_ones() as usize;
            if rem < c {
                return w * 64 + select_in_word(word, rem as u32);
            }
            rem -= c;
            w += 1;
            word = self.zero_word(w);
        }
    }

    /// Value at index `i` without a bounds check on `i < self.n` (callers establish it).
    #[inline]
    fn value_at(&self, i: usize) -> u64 {
        (((self.select1(i) - i) as u64) << self.l) | self.low(i)
    }

    /// Value at index `i`, or `None` when `i` is out of range.
    #[must_use]
    #[inline]
    pub fn get(&self, i: usize) -> Option<u64> {
        (i < self.n).then(|| self.value_at(i))
    }

    /// **Compressed seek.** Returns `(index, value)` of the first element ≥ `target`, or `None`
    /// when every element is smaller.
    ///
    /// This never decodes a block: it locates the high-bits bucket of `target` with one
    /// `select0`, then walks that single bucket comparing full values. The walk is bounded by
    /// the bucket's occupancy — one element on average, since `l` is chosen so the bucket count
    /// is ~`n` — after which the next element necessarily has a strictly larger high part and
    /// therefore already exceeds `target`.
    #[must_use]
    pub fn next_geq(&self, target: u64) -> Option<(usize, u64)> {
        if self.n == 0 || target > self.value_at(self.n - 1) {
            return None;
        }
        let ht = target >> self.l;
        // Elements with a high part < ht are exactly those before the (ht-1)-th zero, and the
        // position of the j-th zero is `j + #{i : h_i <= j}` — so this is that count.
        let mut i = if ht == 0 {
            0
        } else {
            let z = (ht - 1) as usize;
            self.select0(z) - z
        };
        while i < self.n {
            let v = self.value_at(i);
            if v >= target {
                return Some((i, v));
            }
            i += 1;
        }
        None
    }

    /// Sequential decode — the scan path. Walks the high-bits words directly instead of paying
    /// a `select1` per element, so it is the fair competitor to a varint block walk.
    #[must_use]
    pub fn iter(&self) -> EliasFanoIter<'_> {
        EliasFanoIter { ef: self, i: 0, w: 0, word: self.hi.first().copied().unwrap_or(0) }
    }

    /// Decodes the whole sequence back to a `Vec`. Round-trips [`EliasFano::encode`] exactly.
    #[must_use]
    pub fn decode_all(&self) -> Vec<u64> {
        self.iter().collect()
    }

    /// Resident heap bytes of the encoding (bit vectors plus both select directories).
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        (self.hi.capacity() + self.lo.capacity()) * size_of::<u64>()
            + (self.sel1.capacity() + self.sel0.capacity()) * size_of::<u32>()
    }
}

/// Sequential decoder over an [`EliasFano`], produced by [`EliasFano::iter`].
pub struct EliasFanoIter<'a> {
    ef: &'a EliasFano,
    /// Index of the next value to yield.
    i: usize,
    /// Word of `ef.hi` currently being consumed.
    w: usize,
    /// Remaining unconsumed set bits of word `w`.
    word: u64,
}

impl Iterator for EliasFanoIter<'_> {
    type Item = u64;

    #[inline]
    fn next(&mut self) -> Option<u64> {
        if self.i >= self.ef.n {
            return None;
        }
        // `i < n` guarantees another set bit exists, so this stays in bounds.
        while self.word == 0 {
            self.w += 1;
            self.word = self.ef.hi[self.w];
        }
        let p = self.w * 64 + self.word.trailing_zeros() as usize;
        self.word &= self.word - 1;
        let v = (((p - self.i) as u64) << self.ef.l) | self.ef.low(self.i);
        self.i += 1;
        Some(v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.ef.n - self.i;
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for EliasFanoIter<'_> {}

/// A non-decreasing `u64` sequence in **fixed-size-partition** Partitioned-Elias-Fano form.
///
/// The sequence is cut into partitions of [`PartitionedEliasFano::DEFAULT_PARTITION`] values.
/// An upper-level [`EliasFano`] holds the partitions' endpoints (their last values); each
/// partition is an [`EliasFano`] over values *relative* to the previous partition's endpoint.
/// Rebasing shrinks each partition's universe to its own local span, which is the whole point:
/// a clustered column pays `log2(local span / partition size)` low bits instead of
/// `log2(global span / n)`.
///
/// This is the fixed-size variant. The published PEF picks partition boundaries with a
/// dynamic program over three per-partition encodings; that optimisation is deliberately not
/// implemented until the A/B says the family is worth adopting at all.
///
/// **Read the space measurement with this caveat.** Each partition is a separate [`EliasFano`]
/// with its own four `Vec`s, so this prototype pays ~`size_of::<EliasFano>()` of per-partition
/// struct metadata on top of the coded bits — at [`Self::DEFAULT_PARTITION`] values per
/// partition that term dominates, and it is an artefact of the *layout*, not of partitioned
/// Elias-Fano. A production PEF concatenates every partition into one bit-stream behind an
/// offset index. So a "PEF is larger than plain EF" reading of the harness is a statement about
/// this prototype's allocation strategy; only the seek/scan *timings* speak to the algorithm.
pub struct PartitionedEliasFano {
    /// Values per partition (the last one may be shorter).
    part: usize,
    /// Endpoints (last value) of each partition, themselves Elias-Fano coded.
    upper: EliasFano,
    /// One codec per partition, over `value - bases[p]`.
    parts: Vec<EliasFano>,
    /// `bases[p]` is the previous partition's endpoint (`0` for partition 0).
    bases: Vec<u64>,
    /// Total number of encoded values.
    n: usize,
}

impl PartitionedEliasFano {
    /// Default values per partition. Matches [`crate::compress::BLOCK`] so a PEF partition and
    /// a block-codec block cover the same rows, which keeps the A/B an apples-to-apples
    /// comparison of the two codecs rather than of two blocking factors.
    pub const DEFAULT_PARTITION: usize = crate::compress::BLOCK;

    /// Encodes `values` (which MUST be non-decreasing) with [`Self::DEFAULT_PARTITION`].
    #[must_use]
    pub fn encode(values: &[u64]) -> Option<Self> {
        Self::encode_with_partition(values, Self::DEFAULT_PARTITION)
    }

    /// Encodes `values` (which MUST be non-decreasing) with an explicit partition size.
    ///
    /// Returns `None` when `part` is zero, when `values` is not non-decreasing, or when any
    /// sub-sequence exceeds [`EliasFano::encode`]'s addressing limit.
    #[must_use]
    pub fn encode_with_partition(values: &[u64], part: usize) -> Option<Self> {
        if part == 0 {
            return None;
        }
        if values.windows(2).any(|w| w[0] > w[1]) {
            return None;
        }
        let mut parts = Vec::with_capacity(values.len().div_ceil(part));
        let mut bases = Vec::with_capacity(parts.capacity());
        let mut endpoints = Vec::with_capacity(parts.capacity());
        let mut base = 0u64;
        let mut local = Vec::with_capacity(part);
        for chunk in values.chunks(part) {
            local.clear();
            local.extend(chunk.iter().map(|&v| v - base));
            parts.push(EliasFano::encode(&local)?);
            bases.push(base);
            let end = chunk[chunk.len() - 1];
            endpoints.push(end);
            base = end;
        }
        Some(Self { part, upper: EliasFano::encode(&endpoints)?, parts, bases, n: values.len() })
    }

    /// Number of encoded values.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    /// `true` when no values are encoded.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Value at index `i`, or `None` when `i` is out of range.
    #[must_use]
    #[inline]
    pub fn get(&self, i: usize) -> Option<u64> {
        let p = i / self.part;
        let ef = self.parts.get(p)?;
        ef.get(i % self.part).map(|v| v + self.bases[p])
    }

    /// **Compressed seek** — `(index, value)` of the first element ≥ `target`, or `None`.
    ///
    /// Two levels, both compressed: one `next_geq` over the endpoints picks the first partition
    /// that can contain the answer, then one `next_geq` inside it. No partition other than the
    /// chosen one is touched, and neither level is decoded.
    #[must_use]
    pub fn next_geq(&self, target: u64) -> Option<(usize, u64)> {
        let (p, _) = self.upper.next_geq(target)?;
        let base = self.bases[p];
        // Partition `p` is the FIRST whose endpoint ≥ target, so the answer is inside it. When
        // target < base the whole partition qualifies and the local target saturates to 0.
        let (j, v) = self.parts[p].next_geq(target.saturating_sub(base))?;
        Some((p * self.part + j, v + base))
    }

    /// Sequential decode — the scan path, partition by partition.
    pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.parts.iter().zip(self.bases.iter()).flat_map(|(ef, &b)| ef.iter().map(move |v| v + b))
    }

    /// Decodes the whole sequence back to a `Vec`.
    #[must_use]
    pub fn decode_all(&self) -> Vec<u64> {
        self.iter().collect()
    }

    /// Resident heap bytes: every partition, the endpoint index, and the base table.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.upper.heap_bytes()
            + self.parts.iter().map(EliasFano::heap_bytes).sum::<usize>()
            + self.parts.capacity() * size_of::<EliasFano>()
            + self.bases.capacity() * size_of::<u64>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Naive reference successor: the semantics `next_geq` must reproduce exactly.
    fn naive_next_geq(values: &[u64], target: u64) -> Option<(usize, u64)> {
        let i = values.partition_point(|&v| v < target);
        values.get(i).map(|&v| (i, v))
    }

    /// Dense run-length column: long runs of consecutive ids, the clustered shape a
    /// well-sorted permutation's trailing column actually has.
    fn clustered(n: usize) -> Vec<u64> {
        let mut v = Vec::with_capacity(n);
        let mut x = 3u64;
        while v.len() < n {
            let run = 1 + (x % 17);
            for _ in 0..run {
                if v.len() == n {
                    break;
                }
                v.push(x);
                x += 1;
            }
            x += 1 + (x % 4096); // a gap between clusters
        }
        v
    }

    /// Sparse column: a wide, near-uniform universe — EF's best case for ratio.
    fn sparse(n: usize) -> Vec<u64> {
        let mut v = Vec::with_capacity(n);
        let mut x = 0u64;
        for i in 0..n {
            x += 1 + (i as u64 * 2_654_435_761) % 100_000;
            v.push(x);
        }
        v
    }

    /// Duplicate-heavy column: EF is defined on non-DEcreasing input, and a trailing column
    /// with repeated values must round-trip and seek correctly.
    fn with_duplicates(n: usize) -> Vec<u64> {
        (0..n).map(|i| (i as u64) / 5).collect()
    }

    fn shapes(n: usize) -> Vec<(&'static str, Vec<u64>)> {
        vec![
            ("dense", (0..n as u64).collect()),
            ("clustered", clustered(n)),
            ("sparse", sparse(n)),
            ("duplicates", with_duplicates(n)),
        ]
    }

    #[test]
    fn encode_rejects_non_monotone_input() {
        assert!(EliasFano::encode(&[1, 5, 4]).is_none());
        assert!(PartitionedEliasFano::encode(&[1, 5, 4]).is_none());
        assert!(PartitionedEliasFano::encode_with_partition(&[1, 2, 3], 0).is_none());
        // equal neighbours are non-DEcreasing, hence accepted
        assert!(EliasFano::encode(&[1, 1, 1]).is_some());
    }

    #[test]
    fn empty_and_is_empty() {
        let ef = EliasFano::encode(&[]).unwrap();
        assert!(ef.is_empty());
        assert_eq!(ef.len(), 0);
        assert_eq!(ef.get(0), None);
        assert_eq!(ef.next_geq(0), None);
        assert!(ef.decode_all().is_empty());

        let pef = PartitionedEliasFano::encode(&[]).unwrap();
        assert!(pef.is_empty());
        assert_eq!(pef.len(), 0);
        assert_eq!(pef.get(0), None);
        assert_eq!(pef.next_geq(0), None);
        assert!(pef.decode_all().is_empty());
    }

    #[test]
    fn roundtrip_and_random_access_across_shapes() {
        for (name, values) in shapes(3_000) {
            let ef = EliasFano::encode(&values).unwrap();
            assert_eq!(ef.len(), values.len(), "{} len", name);
            assert!(!ef.is_empty(), "{}", name);
            assert_eq!(ef.decode_all(), values, "{} decode_all", name);
            for i in (0..values.len()).step_by(7) {
                assert_eq!(ef.get(i), Some(values[i]), "{} get({})", name, i);
            }
            assert_eq!(ef.get(values.len()), None, "{} get past end", name);

            let pef = PartitionedEliasFano::encode(&values).unwrap();
            assert_eq!(pef.len(), values.len(), "{} pef len", name);
            assert!(!pef.is_empty(), "{} pef", name);
            assert_eq!(pef.decode_all(), values, "{} pef decode_all", name);
            for i in (0..values.len()).step_by(7) {
                assert_eq!(pef.get(i), Some(values[i]), "{} pef get({})", name, i);
            }
            assert_eq!(pef.get(values.len()), None, "{} pef get past end", name);
        }
    }

    #[test]
    fn iter_matches_slice_and_reports_exact_len() {
        let values = clustered(500);
        let ef = EliasFano::encode(&values).unwrap();
        let mut it = ef.iter();
        assert_eq!(it.len(), values.len());
        assert_eq!(it.next(), Some(values[0]));
        assert_eq!(it.len(), values.len() - 1);
        assert!(it.eq(values[1..].iter().copied()));

        let pef = PartitionedEliasFano::encode(&values).unwrap();
        assert!(pef.iter().eq(values.iter().copied()));
    }

    #[test]
    fn next_geq_matches_naive_exhaustively() {
        for (name, values) in shapes(1_200) {
            let ef = EliasFano::encode(&values).unwrap();
            let pef = PartitionedEliasFano::encode_with_partition(&values, 64).unwrap();
            let max = *values.last().unwrap();
            // Probe every value, its neighbours, and past the end.
            let mut probes: Vec<u64> = values.iter().flat_map(|&v| [v.saturating_sub(1), v, v + 1]).collect();
            probes.extend([0, max, max + 1, u64::MAX]);
            for t in probes {
                let want = naive_next_geq(&values, t);
                assert_eq!(ef.next_geq(t), want, "{} ef next_geq({})", name, t);
                assert_eq!(pef.next_geq(t), want, "{} pef next_geq({})", name, t);
            }
        }
    }

    #[test]
    fn next_geq_spans_partition_boundaries() {
        // A gap that straddles a partition boundary is the case a two-level seek gets wrong if
        // the upper level is searched with the wrong comparison.
        let values: Vec<u64> = (0..256u64).map(|i| i * 1_000).collect();
        let pef = PartitionedEliasFano::encode_with_partition(&values, 16).unwrap();
        for t in [1u64, 15_001, 16_000, 16_001, 255_000, 255_001] {
            assert_eq!(pef.next_geq(t), naive_next_geq(&values, t), "next_geq({})", t);
        }
    }

    #[test]
    fn single_and_tiny_sequences() {
        for values in [vec![0u64], vec![7], vec![u64::MAX], vec![0, u64::MAX]] {
            let ef = EliasFano::encode(&values).unwrap();
            assert_eq!(ef.decode_all(), values);
            let pef = PartitionedEliasFano::encode_with_partition(&values, 1).unwrap();
            assert_eq!(pef.decode_all(), values);
            for &t in &[0u64, 1, u64::MAX] {
                assert_eq!(ef.next_geq(t), naive_next_geq(&values, t));
                assert_eq!(pef.next_geq(t), naive_next_geq(&values, t));
            }
        }
    }

    #[test]
    fn heap_bytes_is_reported_and_bounded() {
        let values = sparse(4_096);
        let ef = EliasFano::encode(&values).unwrap();
        let pef = PartitionedEliasFano::encode(&values).unwrap();
        // A codec that is not smaller than the raw `u64` array it replaces is not a codec.
        let raw = values.len() * size_of::<u64>();
        assert!(ef.heap_bytes() > 0 && ef.heap_bytes() < raw, "ef {} vs raw {}", ef.heap_bytes(), raw);
        assert!(pef.heap_bytes() > 0 && pef.heap_bytes() < raw, "pef {} vs raw {}", pef.heap_bytes(), raw);
    }

    proptest::proptest! {
        /// Differential against the naive successor over arbitrary non-decreasing sequences —
        /// the load-bearing property: `next_geq` is result-identical to a binary search on the
        /// decoded array, for both codecs.
        #[test]
        fn prop_roundtrip_and_next_geq(
            deltas in proptest::collection::vec(0u64..1_000, 1..300),
            targets in proptest::collection::vec(0u64..300_000, 1..40),
            part in 1usize..40,
        ) {
            let mut values = Vec::with_capacity(deltas.len());
            let mut x = 0u64;
            for d in deltas {
                x += d;
                values.push(x);
            }
            let ef = EliasFano::encode(&values).unwrap();
            let pef = PartitionedEliasFano::encode_with_partition(&values, part).unwrap();
            proptest::prop_assert_eq!(ef.decode_all(), values.clone());
            proptest::prop_assert_eq!(pef.decode_all(), values.clone());
            for t in targets {
                let want = naive_next_geq(&values, t);
                proptest::prop_assert_eq!(ef.next_geq(t), want);
                proptest::prop_assert_eq!(pef.next_geq(t), want);
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // The A/B measurement harness (sq-96hp1). NOT a gate, NOT an assertion — it prints.
    // ---------------------------------------------------------------------------------------

    /// The incumbent shape, reduced to one column: delta + LEB128-varint blocks with a sparse
    /// directory of (first value, byte offset). Seeking means binary-searching the directory
    /// and then **decoding the whole block** — the exact property Elias-Fano is meant to remove.
    #[cfg(not(target_arch = "wasm32"))]
    struct VarintBlocks {
        bytes: Vec<u8>,
        /// `(first value of block, byte offset)`.
        dir: Vec<(u64, u32)>,
        block: usize,
        n: usize,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl VarintBlocks {
        fn encode(values: &[u64], block: usize) -> Self {
            let (mut bytes, mut dir) = (Vec::new(), Vec::new());
            for chunk in values.chunks(block) {
                dir.push((chunk[0], bytes.len() as u32));
                let mut prev = chunk[0];
                for (k, &v) in chunk.iter().enumerate() {
                    let mut d = if k == 0 { v } else { v - prev };
                    prev = v;
                    while d >= 0x80 {
                        bytes.push((d as u8) | 0x80);
                        d >>= 7;
                    }
                    bytes.push(d as u8);
                }
            }
            Self { bytes, dir, block, n: values.len() }
        }

        fn decode_block(&self, b: usize, out: &mut Vec<u64>) {
            out.clear();
            let start = self.dir[b].1 as usize;
            let end = self.dir.get(b + 1).map_or(self.bytes.len(), |e| e.1 as usize);
            let count = (self.n - b * self.block).min(self.block);
            let mut pos = start;
            let mut acc = 0u64;
            for k in 0..count {
                let (mut x, mut shift) = (0u64, 0u32);
                loop {
                    let byte = self.bytes[pos];
                    pos += 1;
                    x |= u64::from(byte & 0x7f) << shift;
                    if byte & 0x80 == 0 {
                        break;
                    }
                    shift += 7;
                }
                acc = if k == 0 { x } else { acc + x };
                out.push(acc);
            }
            debug_assert_eq!(pos, end, "block {} varint stream must be consumed exactly", b);
        }

        /// Decode-then-search successor: the baseline `next_geq`.
        fn next_geq(&self, target: u64, scratch: &mut Vec<u64>) -> Option<(usize, u64)> {
            if self.dir.is_empty() {
                return None;
            }
            let b = self.dir.partition_point(|&(first, _)| first <= target).saturating_sub(1);
            for b in b..self.dir.len() {
                self.decode_block(b, scratch);
                let j = scratch.partition_point(|&v| v < target);
                if j < scratch.len() {
                    return Some((b * self.block + j, scratch[j]));
                }
            }
            None
        }

        fn scan_sum(&self, scratch: &mut Vec<u64>) -> u64 {
            let mut acc = 0u64;
            for b in 0..self.dir.len() {
                self.decode_block(b, scratch);
                acc = acc.wrapping_add(scratch.iter().copied().fold(0u64, u64::wrapping_add));
            }
            acc
        }

        fn heap_bytes(&self) -> usize {
            self.bytes.capacity() + self.dir.capacity() * size_of::<(u64, u32)>()
        }
    }

    /// [SONNET-4.6] sq-96hp1 — the MEASUREMENT this bead is gated on: plain-EF and
    /// Partitioned-EF versus the incumbent decode-then-search varint block codec, on four
    /// column shapes, reporting seek latency, scan throughput, bytes resident and bits/value.
    ///
    /// NATIVE-ONLY: it times with [`std::time::Instant`], which `wasm32-unknown-unknown` does
    /// not provide, so this harness (and its `VarintBlocks` baseline) is compiled out there
    /// and the `wasm32` half of the A/B remains UNRESOLVED — see the module status note.
    ///
    /// Every number it prints is a NON-canonical work-box measurement and asserts nothing; the
    /// adoption verdict belongs in the bead note after a run on the canonical native host. Run
    /// it with:
    /// `cargo test -p sparq-core --features elias-fano --lib measure_ef -- --ignored --nocapture`
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "spike measurement: run explicitly with --ignored --nocapture"]
    fn measure_ef_vs_block_varint() {
        use std::time::Instant;

        const N: usize = 1_000_000;
        const SEEKS: usize = 200_000;
        println!("\n===== sq-96hp1 Elias-Fano A/B (NON-canonical work-box; no number asserted) =====");
        println!(
            "  NOTE: PEF bits/value includes ~{} B/partition of per-partition struct metadata \
             (one Vec set per partition) — a layout artefact of this prototype, not of PEF.",
            size_of::<EliasFano>()
        );
        for (name, values) in shapes(N) {
            let raw_bytes = values.len() * size_of::<u64>();
            let ef = EliasFano::encode(&values).unwrap();
            let pef = PartitionedEliasFano::encode(&values).unwrap();
            let vb = VarintBlocks::encode(&values, crate::compress::BLOCK);
            let mut scratch = Vec::with_capacity(crate::compress::BLOCK);

            let span = *values.last().unwrap();
            // Deterministic pseudo-random probes spread over the value span.
            let probes: Vec<u64> = (0..SEEKS)
                .map(|i| (i as u64).wrapping_mul(2_654_435_761) % (span + 1))
                .collect();

            let t = Instant::now();
            let mut sink = 0u64;
            for &p in &probes {
                sink = sink.wrapping_add(ef.next_geq(p).map_or(0, |(_, v)| v));
            }
            let ef_seek = t.elapsed();

            let t = Instant::now();
            for &p in &probes {
                sink = sink.wrapping_add(pef.next_geq(p).map_or(0, |(_, v)| v));
            }
            let pef_seek = t.elapsed();

            let t = Instant::now();
            for &p in &probes {
                sink = sink.wrapping_add(vb.next_geq(p, &mut scratch).map_or(0, |(_, v)| v));
            }
            let vb_seek = t.elapsed();

            let t = Instant::now();
            sink = sink.wrapping_add(ef.iter().fold(0u64, u64::wrapping_add));
            let ef_scan = t.elapsed();
            let t = Instant::now();
            sink = sink.wrapping_add(pef.iter().fold(0u64, u64::wrapping_add));
            let pef_scan = t.elapsed();
            let t = Instant::now();
            sink = sink.wrapping_add(vb.scan_sum(&mut scratch));
            let vb_scan = t.elapsed();

            let bits = |b: usize| b as f64 * 8.0 / values.len() as f64;
            println!("--- shape={} n={} span={} raw={} B ---", name, values.len(), span, raw_bytes);
            println!(
                "  bits/value   EF={:>7.3}  PEF={:>7.3}  varint-blocks={:>7.3}",
                bits(ef.heap_bytes()), bits(pef.heap_bytes()), bits(vb.heap_bytes())
            );
            println!(
                "  heap bytes   EF={:>10}  PEF={:>10}  varint-blocks={:>10}",
                ef.heap_bytes(), pef.heap_bytes(), vb.heap_bytes()
            );
            println!(
                "  seek {} probes  EF={:>10?}  PEF={:>10?}  varint-blocks={:>10?}",
                SEEKS, ef_seek, pef_seek, vb_seek
            );
            println!(
                "  full scan       EF={:>10?}  PEF={:>10?}  varint-blocks={:>10?}",
                ef_scan, pef_scan, vb_scan
            );
            // Keep the compiler from eliding the work being timed.
            assert_ne!(sink, u64::MAX, "sink guard");
        }
        println!("===== end measurement — verdict in the bead note / PR body =====\n");
    }

    /// Coverage companion for [`measure_ef_vs_block_varint`]'s baseline: the varint-block
    /// reference must itself be correct, or the A/B compares against a broken competitor.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn varint_block_baseline_agrees_with_naive() {
        for (name, values) in shapes(700) {
            let vb = VarintBlocks::encode(&values, 64);
            let mut scratch = Vec::new();
            assert_eq!(vb.scan_sum(&mut scratch), values.iter().copied().fold(0, u64::wrapping_add), "{}", name);
            assert!(vb.heap_bytes() > 0, "{}", name);
            let max = *values.last().unwrap();
            for t in [0, 1, values[values.len() / 2], max, max + 1] {
                assert_eq!(
                    vb.next_geq(t, &mut scratch),
                    naive_next_geq(&values, t),
                    "{} varint next_geq({})",
                    name,
                    t
                );
            }
        }
    }
}
