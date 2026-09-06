//! Block-compressed permutation index: a sorted `[[Id;3]]` permutation stored as
//! fixed-count blocks of lexicographic-delta + LEB128-varint rows, with a sparse
//! directory of (first-triple, byte-offset) for random access. Cuts the index from
//! 12 B/triple to ~4-6 B/triple (measured −55% to −69% on synthetic + real Wikidata)
//! while keeping pattern scans random-accessible: binary-search the directory, decode
//! only the blocks the pattern's key-range touches.
//!
//! This is the storage mode for the memory-bound paths (browser / out-of-core); the
//! native in-memory store keeps raw `[[Id;3]]` (no decode cost). The encoding is the
//! one validated by `sparq-cli probe-compress`.

use crate::dict::Id;

/// Rows per block. A scan decodes whole blocks, so smaller blocks mean less decode
/// waste per probe but a larger directory; 128 keeps the directory at ~0.16 B/triple.
pub const BLOCK: usize = 128;

/// Magic prefix of the COMPRESSED on-disk permutation file format (see
/// [`CompressedPerm::write_to`]). Distinguishes a compressed `perm{i}.bin` from the raw
/// little-endian `[u32;3]` format on open: a raw file starts with the FIRST (i.e.
/// smallest) sorted row, whose leading id would have to be 0x43515053 ≈ 1.13e9 to
/// collide — ids are assigned densely from 1, so the minimum row never gets close.
#[cfg(feature = "mmap")]
pub const FILE_MAGIC: [u8; 8] = *b"SPQCPRM1";

/// Appends `x` to `out` as an unsigned LEB128 varint.
#[inline]
fn put_varint(out: &mut Vec<u8>, mut x: u64) {
    while x >= 0x80 {
        out.push((x as u8) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}

/// [FABLE-5] sq-7d3dj.32.2.6 — appends `x` to `out` as a ZIGZAG-mapped LEB128 varint (used by
/// the SPQCPRM2 format to encode a col2 frame-of-reference offset, which can be negative). Zigzag
/// maps `0,-1,1,-2,2,…` to `0,1,2,3,4,…` so a small-magnitude signed offset stays a short varint.
/// Gated on `mmap` (not `spqcprm2`): the V2 reader ships with the on-disk store so any V2 file
/// always decodes, whether or not this build's emitter can produce V2.
#[cfg(feature = "mmap")]
#[inline]
fn put_zigzag_varint(out: &mut Vec<u8>, x: i64) {
    put_varint(out, ((x << 1) ^ (x >> 63)) as u64);
}

/// [FABLE-5] sq-7d3dj.32.2.6 — reads a zigzag-mapped LEB128 varint written by [`put_zigzag_varint`],
/// advancing `*pos`. The inverse zigzag maps the unsigned varint back to a signed offset.
#[cfg(feature = "mmap")]
#[inline]
fn get_zigzag_varint(buf: &[u8], pos: &mut usize) -> i64 {
    let u = get_varint(buf, pos);
    ((u >> 1) as i64) ^ -((u & 1) as i64)
}

/// Reads an unsigned LEB128 varint from `buf` at `*pos`, advancing `*pos`.
///
/// This is the TRUSTED hot-path reader: it indexes `buf` unchecked and is sound ONLY on a
/// block stream already proven well-formed. The in-memory encoder produces valid streams,
/// and a memory-mapped stream is fully validated once at open by
/// [`CompressedPerm::from_mmap`] (via [`get_varint_checked`] / [`validate_block`]) before
/// any scan reaches here — so an attacker-controlled `.spq` cannot drive this OOB. [OPUS-4.8]
#[inline]
fn get_varint(buf: &[u8], pos: &mut usize) -> u64 {
    let mut x = 0u64;
    let mut shift = 0;
    loop {
        let b = buf[*pos];
        *pos += 1;
        x |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return x;
        }
        shift += 7;
    }
}

/// [OPUS-4.8] sq-ed2i — bounds-/overflow-CHECKED varint reader for the UNTRUSTED mmap path.
/// Returns `None` if the buffer ends mid-varint or the encoding exceeds 64 bits (a hostile
/// file could otherwise make `get_varint` read past the mapping or shift past `u64`). Used
/// only by the one-time open-time validation, never on the scan hot path.
#[cfg(feature = "mmap")]
#[inline]
fn get_varint_checked(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut x = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos)?;
        *pos += 1;
        x |= ((b & 0x7f) as u64).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some(x);
        }
        shift += 7;
        if shift >= 64 {
            return None; // varint longer than a u64 can hold → malformed
        }
    }
}

/// The encoded block stream: built in RAM, or borrowed from a memory-mapped compressed
/// index file (the lazy out-of-core mode — the OS pages in only the blocks scans touch,
/// and the stream never counts against the heap).
enum Blocks {
    Owned(Vec<u8>),
    #[cfg(feature = "mmap")]
    Mapped { map: memmap2::Mmap, off: usize },
}

impl Blocks {
    #[inline]
    fn bytes(&self) -> &[u8] {
        match self {
            Blocks::Owned(v) => v,
            #[cfg(feature = "mmap")]
            Blocks::Mapped { map, off } => &map[*off..],
        }
    }
}

/// [OPUS-4.8] sq-wihld (survey §A1) — OPT-IN per-block Bloom filters on the leading
/// (column-0) ids of a block-compressed permutation. The directory's first-triple already
/// gives an implicit min/max zone map that prunes RANGE scans, but for an EQUALITY-BOUND
/// leading column (a point/prefix lookup, `lo[0] == hi[0]`) the zone map still leaves one
/// candidate block that must be decoded to discover it holds no matching row. A tiny per-block
/// Bloom bitset closes exactly that gap: probe the block's filter and skip its
/// `decode_block_at` when the id is provably absent.
///
/// WHERE THE MEASURED SKIP OPPORTUNITY IS ([SONNET-4.6] sq-v8ixk — see
/// `tests::measure_bloom_skip_and_sizing` / `tests::measure_bloom_selective_scan_latency`).
/// This module originally justified itself by an id falling inside SEVERAL overlapping blocks'
/// `[min,max]` spans. On the SYNTHETIC high-NDV columns the harness generates (WatDiv-shaped
/// skew — NOT a real Wikidata permutation) that shape does not show up: a PRESENT id's rows are
/// contiguous and the zone map already narrows the candidate window to about one block, so the
/// measured skip opportunity on a present-id point lookup is small. (An id spanning many blocks
/// is a LOW-NDV column — exactly what the density gate declines.) The larger measured skip
/// opportunity is the ABSENT equality-bound id — a subject/object bound by an earlier pattern
/// that has no rows in this permutation — where the filter can drop the single candidate block
/// without decoding it at all.
///
/// SCOPE OF THAT CLAIM (deliberately narrow): those are candidate-window and block-SKIP COUNTS
/// on synthetic distributions. Being pure functions of the column they are host-independent, but
/// they do NOT establish an end-to-end latency result in either direction, and a synthetic column
/// cannot settle the question for real workloads. No number is asserted here or in any doc, and
/// the harness's latency half is a work-box figure only — the canonical run, against the intended
/// real dataset on the perf host, is bead sq-0g6g and is still PENDING.
///
/// CORRECTNESS (load-bearing): zero false NEGATIVES by construction — a leading id that is
/// present in a block always probes as "maybe present", so no matching row is ever skipped.
/// A false POSITIVE costs one wasted block decode whose rows the existing range trim then
/// discards, so the `range` output is byte-identical to the no-Bloom path. The filter is an
/// in-RAM/build-time acceleration only: it is NEVER written to the on-disk `SPQCPRM1` format,
/// so a perm built with the feature on and one built with it off persist identically.
///
/// This module is compiled only under the `block-bloom` cargo feature; with the feature off
/// the `CompressedPerm` carries no `bloom` field and `range` is the original code verbatim.
#[cfg(feature = "block-bloom")]
mod block_bloom {
    use super::{Id, BLOCK};

    /// Bits per block filter. A block holds at most [`BLOCK`] (=128) rows, so at most 128
    /// distinct leading ids; 256 bits (32 bytes/block ≈ 0.25 B/triple) keeps the
    /// false-positive rate low at the [`HASHES`] count below while staying a flat,
    /// WASM-trivial bitset. The directory already costs ~0.13 B/triple, so this roughly
    /// triples the resident directory of a Bloom-enabled column — paid only on the high-NDV
    /// columns the density gate admits.
    /// (`pub(super)` only so the sq-v8ixk measurement harness in this file's `tests` module
    /// can print and sweep around the SHIPPED value instead of re-declaring it and drifting.)
    pub(super) const BITS: usize = 256;
    /// Machine words (u64) per block filter.
    const WORDS: usize = BITS / 64;
    /// Hash probes per id. Two independent probes (double hashing off one 64-bit hash) is
    /// the sweet spot for ~128 keys in 256 bits; more probes would over-fill the bitset.
    /// `pub(super)` for the same measurement-harness reason as [`BITS`].
    ///
    /// [SONNET-4.6] sq-v8ixk — DELIBERATELY LEFT AT 2. The sizing sweep in
    /// `tests::measure_bloom_skip_and_sizing` finds `HASHES = 1` markedly BETTER on DENSE ids
    /// (which is what the dictionary assigns) — better than the textbook Bloom formula predicts,
    /// at half the probe work — but the sweep's scattered-id CONTROL column shows that advantage
    /// is an artefact of FNV-1a's low bits behaving near-injectively on small consecutive ids,
    /// not of Bloom math: with the ids scattered across the 32-bit space the measured rates track
    /// theory and `HASHES = 2` wins as expected. Flipping to 1 would therefore trade a
    /// distribution-INDEPENDENT choice for one silently contingent on an id-density property
    /// nothing in this crate guarantees. It stays a candidate retune pending confirmation against
    /// a REAL Wikidata permutation on the canonical perf host (bead sq-0g6g); re-run the sweep
    /// there before changing this line.
    pub(super) const HASHES: u32 = 2;

    /// FNV-1a 64-bit hash of a leading-column id, the seed for double hashing. Deterministic
    /// and endian-stable (we hash the little-endian id bytes), so a filter built on one host
    /// probes identically on another — though filters are never serialised, this keeps the
    /// build reproducible.
    #[inline]
    fn hash_id(id: Id) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in id.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// A flat array of fixed-size per-block Bloom filters over each block's distinct
    /// column-0 ids, one filter per directory entry (same index space as the directory).
    pub struct BlockBloomDir {
        /// `WORDS` u64 words per block, laid out contiguously: block `b` occupies
        /// `words[b * WORDS .. (b + 1) * WORDS]`.
        words: Vec<u64>,
    }

    impl BlockBloomDir {
        /// Inserts `id` into block `b`'s filter via [`HASHES`] double-hashed probes.
        #[inline]
        fn insert(words: &mut [u64], b: usize, id: Id) {
            let h = hash_id(id);
            let (h1, h2) = (h as usize, ((h >> 32) | 1) as usize); // odd step ⇒ distinct probes
            let base = b * WORDS;
            for k in 0..HASHES as usize {
                let bit = (h1.wrapping_add(k.wrapping_mul(h2))) % BITS;
                words[base + bit / 64] |= 1u64 << (bit % 64);
            }
        }

        /// `true` if block `b`'s filter says `id` MIGHT be present (never a false negative);
        /// `false` ⇒ `id` is definitely absent, so the block can be skipped.
        #[inline]
        pub fn maybe_contains(&self, b: usize, id: Id) -> bool {
            let h = hash_id(id);
            let (h1, h2) = (h as usize, ((h >> 32) | 1) as usize);
            let base = b * WORDS;
            for k in 0..HASHES as usize {
                let bit = (h1.wrapping_add(k.wrapping_mul(h2))) % BITS;
                if self.words[base + bit / 64] & (1u64 << (bit % 64)) == 0 {
                    return false;
                }
            }
            true
        }

        /// Resident bytes of the filter array.
        #[inline]
        pub fn heap_bytes(&self) -> usize {
            self.words.capacity() * std::mem::size_of::<u64>()
        }

        /// Builds one filter per block over each block's distinct leading (column-0) ids,
        /// but ONLY when the leading column is high-NDV enough for a filter to ever skip a
        /// block. Returns `None` for a low-NDV leading column (e.g. a predicate-leading
        /// permutation, where every block's id set is tiny and overlapping blocks are rare),
        /// so dense columns pay no filter bytes. The density gate: build only if the average
        /// number of DISTINCT leading ids per block is at least [`MIN_AVG_DISTINCT_PER_BLOCK`]
        /// — i.e. the column actually varies fast enough within a block that an out-of-cluster
        /// point lookup lands in blocks that do not contain it.
        pub fn build(blocks: &[&[[Id; 3]]]) -> Option<Self> {
            if blocks.is_empty() {
                return None;
            }
            // First pass: estimate density (total distinct leading ids across blocks).
            let mut total_distinct: usize = 0;
            for chunk in blocks {
                let mut prev: Option<Id> = None;
                for r in chunk.iter() {
                    if prev != Some(r[0]) {
                        total_distinct += 1;
                        prev = Some(r[0]);
                    }
                }
            }
            let avg = total_distinct as f64 / blocks.len() as f64;
            if avg < MIN_AVG_DISTINCT_PER_BLOCK {
                return None; // low-NDV leading column: a Bloom filter would never skip.
            }
            // Second pass: build the filters. Rows within a block are sorted, so equal
            // leading ids are contiguous — insert each distinct id once.
            let mut words = vec![0u64; blocks.len() * WORDS];
            for (b, chunk) in blocks.iter().enumerate() {
                let mut prev: Option<Id> = None;
                for r in chunk.iter() {
                    if prev != Some(r[0]) {
                        Self::insert(&mut words, b, r[0]);
                        prev = Some(r[0]);
                    }
                }
            }
            Some(BlockBloomDir { words })
        }
    }

    /// Density gate (see [`BlockBloomDir::build`]). A column whose blocks average fewer than
    /// this many distinct leading ids is so clustered that the min/max zone map already
    /// prunes effectively and a Bloom filter would virtually never skip a block — so we skip
    /// the filter to keep the directory lean. Chosen conservatively (a full BLOCK of distinct
    /// ids is 128; this admits columns where at least an eighth of a block's rows start a new
    /// leading id).
    /// `pub(super)` for the same measurement-harness reason as [`BITS`].
    pub(super) const MIN_AVG_DISTINCT_PER_BLOCK: f64 = (BLOCK / 8) as f64;
}

/// A block-compressed, random-accessible sorted permutation.
pub struct CompressedPerm {
    /// One entry per block: (its first triple, its byte offset into `blocks`).
    dir: Vec<([Id; 3], u32)>,
    /// The concatenated encoded blocks.
    blocks: Blocks,
    len: usize,
    /// [OPUS-4.8] sq-wihld — OPT-IN (`block-bloom` feature) per-block Bloom filters over the
    /// leading column, parallel to `dir`. `None` when the feature built no filter for this
    /// perm (low-NDV leading column, empty perm, or a perm opened from disk — filters are not
    /// serialised). Used only to skip blocks on an equality-bound leading column in `range`.
    #[cfg(feature = "block-bloom")]
    bloom: Option<block_bloom::BlockBloomDir>,
    /// [FABLE-5] sq-7d3dj.32.2.6 / sq-7d3dj.32.2.7 — which block-stream encoding this perm's
    /// `blocks` uses. Present under `mmap` (the on-disk store, where the versioned format lives
    /// and `open` auto-detects V1 vs V2). With `mmap` off (the wasm in-RAM path) the struct is
    /// byte-for-byte the default — there is only SPQCPRM1 and the decode hot path is unchanged.
    /// A perm built by `encode` is always `V1`; `from_mmap` sets it from the file magic; only
    /// `encode_v2` (and a V2-emitting `write_to`/writer) produces `V2`.
    #[cfg(feature = "mmap")]
    format: Format,
}

/// [FABLE-5] sq-7d3dj.32.2.6 — the block-stream encoding a [`CompressedPerm`] carries. `V1` is
/// the shipped `SPQCPRM1` (col2 reset written ABSOLUTE); `V2` is `SPQCPRM2` (col2 reset frame-of-
/// reference: a zigzag delta from the block's first-row col2). The two decode through different
/// block readers, so a perm's `format` picks the reader — see `decode_block_at`. The reader for
/// BOTH ships with `mmap`; only whether a build can *emit* V2 is gated by the `spqcprm2` feature.
#[cfg(feature = "mmap")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// The shipped absolute-col2-reset encoding (`SPQCPRM1`).
    V1,
    /// The frame-of-reference col2-reset encoding (`SPQCPRM2`).
    V2,
}

// ===== [FABLE-5] sq-7d3dj.32.2.7 — the V2 EMIT config gate =====
//
// The V2 *reader* ships unconditionally with `mmap`, but a build EMITS `SPQCPRM1` by DEFAULT.
// Turning on the `spqcprm2` feature and selecting `EmitFormat::V2` (via `with_emit_format` in
// tests, or `SPARQ_EMIT_FORMAT=v2` in a process) makes the store's compressed save path and the
// streaming `CompressedPermWriter` write `SPQCPRM2` instead.
//
// Mirrors the `with_build_compressed` per-thread override (lib.rs): a thread-local read once
// on the writing thread, so a test never mutates the process-global environment (the
// `set_var`/`getenv` data race under parallel `cargo test`). With the feature OFF the whole
// gate compiles out and `emit_format()` is a `const V1`, so the default build cannot emit V2.
//
// ===== [OPUS-5] sq-sh7be — the V2-DEFAULT FLIP DECISION: **NO**, V2 stays opt-in =====
//
// The flip was gated on a clearly-positive B/triple measurement. It is not one, and settling
// that needed NO quiet-box run: the encoded length of a block is a PURE, DETERMINISTIC function
// of its rows (varint byte counts), so the size half of the decision is host-independent and
// reproducible anywhere. `v2_size_delta_is_shape_dependent_not_a_uniform_win` measures it
// directly over all six permutations and pins the finding:
//
//   * on a col2-CLUSTERED corpus (the shape the frame-of-reference was designed for) V2 is
//     materially SMALLER than V1, and the win widens with scale;
//   * on a corpus whose objects are spread over a wide domain V2 is consistently LARGER —
//     zigzag DOUBLES the offset's magnitude, so once `|r[2] - first_col2|` is comparable to
//     `r[2]` itself the frame offset costs a varint byte the absolute id did not.
//
// So V2 is a SHAPE-DEPENDENT TRADE, not a uniform win, and a blanket default flip would
// silently regress every non-clustered corpus. A canonical EC2 B/tri+RSS run cannot overturn
// that — the regression is arithmetic on the emitted bytes, not measurement noise — so the
// decision is taken now rather than left parked on a box that would not change the answer.
//
// The size regression above is on its own sufficient to block the flip, so DECODE cost was not
// what the decision turned on — and it is NOT claimed to be measured here. What IS pinned is an
// ENCODING property: a block containing no `reset_d1` row encodes BYTE-IDENTICALLY under both
// formats (asserted by `v2_stream_is_byte_identical_when_no_reset_d1`), i.e. the two encoders
// differ only in the `reset_d1` payload. That is a statement about emitted bytes, NOT about
// runtime: a V2 perm still decodes through the separate `decode_block_v2_at` (dispatched per
// block in `decode_block_at`, and capturing the frame origin once per block) whatever its rows
// look like, so equal bytes do not imply equal decode work. Quantifying V2's decode overhead
// would need a decode benchmark that covers that dispatch and per-block work; nobody has run
// one, and the flip does not depend on it.
//
// WHAT WOULD RE-OPEN THIS: not more benchmarking of the blanket flip, but a different design —
// a PER-PERMUTATION (or per-block) format choice that emits V2 only where it measures smaller,
// which needs a selector + a mixed-format directory and is its own piece of work. Both tests
// above go RED if the shape-dependence ever stops holding, which is the signal to revisit.

/// [FABLE-5] sq-7d3dj.32.2.7 — which block-stream [`Format`] a build EMITS when writing a
/// compressed permutation. Distinct from a perm's own `format` field: this is the *policy*
/// choice at write time, resolved by `emit_format`.
#[cfg(feature = "spqcprm2")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmitFormat {
    /// Write `SPQCPRM1` (the shipped default — full backward/forward compat).
    V1,
    /// Write `SPQCPRM2` (frame-of-reference col2 reset). Opt-in.
    V2,
}

#[cfg(feature = "spqcprm2")]
thread_local! {
    /// Per-thread override of the emit format. `None` ⇒ fall back to `SPARQ_EMIT_FORMAT`.
    /// Read once on the writing thread before any fan-out, exactly like `BUILD_COMPRESSED_OVERRIDE`.
    static EMIT_FORMAT_OVERRIDE: std::cell::Cell<Option<EmitFormat>> = const { std::cell::Cell::new(None) };
}

/// [FABLE-5] sq-7d3dj.32.2.7 — TEST/BENCH hook: run `f` with the compressed-perm emit format
/// forced to `fmt`, on THIS thread only, restoring the previous override afterwards. Used by the
/// V2 migration differentials so they never touch the global environment.
#[cfg(feature = "spqcprm2")]
#[doc(hidden)]
pub fn with_emit_format<R>(fmt: EmitFormat, f: impl FnOnce() -> R) -> R {
    let prev = EMIT_FORMAT_OVERRIDE.with(|c| c.replace(Some(fmt)));
    let r = f();
    EMIT_FORMAT_OVERRIDE.with(|c| c.set(prev));
    r
}

/// [FABLE-5] sq-7d3dj.32.2.7 — the [`Format`] a compressed save/build should EMIT. Resolves the
/// per-thread [`with_emit_format`] override, else `SPARQ_EMIT_FORMAT` (`v2`/`spqcprm2` ⇒ V2,
/// anything else ⇒ V1). With the `spqcprm2` feature OFF this is a `const` `V1` — the default
/// build never emits V2, so every shipped index stays `SPQCPRM1` bit-for-bit.
#[cfg(feature = "mmap")]
#[inline]
pub(crate) fn emit_format() -> Format {
    #[cfg(feature = "spqcprm2")]
    {
        if let Some(f) = EMIT_FORMAT_OVERRIDE.with(|c| c.get()) {
            return match f {
                EmitFormat::V1 => Format::V1,
                EmitFormat::V2 => Format::V2,
            };
        }
        if matches!(
            std::env::var("SPARQ_EMIT_FORMAT").as_deref(),
            Ok("v2") | Ok("V2") | Ok("spqcprm2") | Ok("SPQCPRM2")
        ) {
            return Format::V2;
        }
    }
    Format::V1
}

/// Encodes one block (`chunk`, 1..=`BLOCK` sorted rows) into `out`, appending its
/// `count` varint, first row absolute, then per-row lexicographic deltas. Shared by the
/// in-RAM [`CompressedPerm::encode`] and the streaming [`CompressedPermWriter`] so both
/// emit a BYTE-IDENTICAL block stream. [OPUS-4.8] sq-vkz7
#[inline]
fn encode_block(chunk: &[[Id; 3]], out: &mut Vec<u8>) {
    put_varint(out, chunk.len() as u64);
    // First row absolute.
    put_varint(out, chunk[0][0] as u64);
    put_varint(out, chunk[0][1] as u64);
    put_varint(out, chunk[0][2] as u64);
    // Remaining rows: lexicographic delta vs the previous row.
    for w in chunk.windows(2) {
        let (p, r) = (w[0], w[1]);
        let d0 = r[0] - p[0];
        put_varint(out, d0 as u64);
        if d0 == 0 {
            let d1 = r[1] - p[1];
            put_varint(out, d1 as u64);
            if d1 == 0 {
                put_varint(out, (r[2] - p[2]) as u64); // strictly increasing
            } else {
                put_varint(out, r[2] as u64); // col2 resets → absolute
            }
        } else {
            put_varint(out, r[1] as u64); // cols 1,2 reset → absolute
            put_varint(out, r[2] as u64);
        }
    }
}

/// [FABLE-5] sq-7d3dj.32.2.6 / sq-7d3dj.32.2.7 — magic prefix of the `SPQCPRM2` on-disk format
/// (frame-of-reference col2 reset), auto-detected on [`from_mmap`](CompressedPerm::from_mmap)
/// alongside [`FILE_MAGIC`] (`SPQCPRM1`). Ships with `mmap` so any V2 file always decodes; whether
/// this build *writes* it is a separate config gate (`EmitFormat`, behind `spqcprm2`). Held here
/// alongside the V1 magic so the two markers stay adjacent and distinct.
#[cfg(feature = "mmap")]
pub const FILE_MAGIC_V2: [u8; 8] = *b"SPQCPRM2";

/// [FABLE-5] sq-7d3dj.32.2.6 — encodes one block into the `SPQCPRM2` frame-of-reference
/// stream. Byte-for-byte identical to [`encode_block`] EXCEPT the `reset_d1` col2 (written when
/// `d0 == 0 && d1 != 0`): SPQCPRM1 emits `r[2]` absolute; SPQCPRM2 emits the zigzag delta
/// `r[2] - first_col2`, where `first_col2` is the block's first-row col2 (the frame origin). The
/// decoder ([`decode_block_v2_at`]) reads that same first-row col2 at block start, so it can add
/// the offset back with no extra state. All other write sites are unchanged, so a V2 block that
/// happens to contain no `reset_d1` row is byte-identical to its V1 block. Ships with `mmap` so a
/// V2-emitting `write_to`/writer is available whenever the store is, gated by the `spqcprm2`
/// emit-selector at the call site.
#[cfg(feature = "mmap")]
#[inline]
fn encode_block_v2(chunk: &[[Id; 3]], out: &mut Vec<u8>) {
    put_varint(out, chunk.len() as u64);
    let first_col2 = chunk[0][2];
    put_varint(out, chunk[0][0] as u64);
    put_varint(out, chunk[0][1] as u64);
    put_varint(out, chunk[0][2] as u64);
    for w in chunk.windows(2) {
        let (p, r) = (w[0], w[1]);
        let d0 = r[0] - p[0];
        put_varint(out, d0 as u64);
        if d0 == 0 {
            let d1 = r[1] - p[1];
            put_varint(out, d1 as u64);
            if d1 == 0 {
                put_varint(out, (r[2] - p[2]) as u64); // strictly increasing
            } else {
                // FRAME-OF-REFERENCE: col2 reset written as a signed delta from the block's
                // first-row col2 instead of the absolute id (the SPQCPRM1 `reset_d1` bucket).
                put_zigzag_varint(out, r[2] as i64 - first_col2 as i64);
            }
        } else {
            put_varint(out, r[1] as u64); // cols 1,2 reset → absolute (unchanged from V1)
            put_varint(out, r[2] as u64);
        }
    }
}

impl CompressedPerm {
    /// Encodes a sorted permutation (rows already in this permutation's column order).
    pub fn encode(rows: &[[Id; 3]]) -> Self {
        let mut dir = Vec::with_capacity(rows.len() / BLOCK + 1);
        let mut blocks = Vec::with_capacity(rows.len() * 6);
        for chunk in rows.chunks(BLOCK) {
            dir.push((chunk[0], blocks.len() as u32));
            encode_block(chunk, &mut blocks);
        }
        // [OPUS-4.8] sq-wihld — build the OPT-IN per-block Bloom directory over the leading
        // column (off the same chunks, so it is exactly aligned with `dir`). `build` returns
        // `None` on a low-NDV leading column, so dense columns pay nothing.
        #[cfg(feature = "block-bloom")]
        let bloom = block_bloom::BlockBloomDir::build(&rows.chunks(BLOCK).collect::<Vec<_>>());
        CompressedPerm {
            dir,
            blocks: Blocks::Owned(blocks),
            len: rows.len(),
            #[cfg(feature = "block-bloom")]
            bloom,
            #[cfg(feature = "mmap")]
            format: Format::V1,
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — SPIKE: encodes a sorted permutation into the `SPQCPRM2`
    /// frame-of-reference block stream. Identical to [`encode`](Self::encode) except the col2
    /// that resets after a middle-column change (the `reset_d1` bucket the sq-7d3dj.32.2.4
    /// attribution found dominant at scale) is written as a ZIGZAG DELTA from the block's
    /// first-row col2 rather than an absolute varint. When a block's objects cluster (the
    /// common case for related subject/predicate rows) the frame offset is smaller than the
    /// absolute id, so fewer varint bytes. Everything else — first row, `d0`/`d1`/`d2` deltas,
    /// and the `reset_d0` (cols 1,2 absolute) shape — is byte-for-byte the SPQCPRM1 encoder.
    ///
    /// The returned perm's `format` is `V2`, so its own `decode_all` / `range` decode through
    /// the matching reader. The store's default save path stays `SPQCPRM1`; a build opts into
    /// V2 emission via the `emit_format` config gate ([`with_emit_format`] / `SPARQ_EMIT_FORMAT`),
    /// which routes [`encode_emit`](Self::encode_emit) here.
    #[cfg(feature = "spqcprm2")]
    pub fn encode_v2(rows: &[[Id; 3]]) -> Self {
        let mut dir = Vec::with_capacity(rows.len() / BLOCK + 1);
        let mut blocks = Vec::with_capacity(rows.len() * 6);
        for chunk in rows.chunks(BLOCK) {
            dir.push((chunk[0], blocks.len() as u32));
            encode_block_v2(chunk, &mut blocks);
        }
        #[cfg(feature = "block-bloom")]
        let bloom = block_bloom::BlockBloomDir::build(&rows.chunks(BLOCK).collect::<Vec<_>>());
        CompressedPerm {
            dir,
            blocks: Blocks::Owned(blocks),
            len: rows.len(),
            #[cfg(feature = "block-bloom")]
            bloom,
            format: Format::V2,
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — encodes a sorted permutation in the block-stream `Format`
    /// the build's `emit_format` config gate selects: `SPQCPRM1` by default (bit-for-bit
    /// [`encode`](Self::encode)), or `SPQCPRM2` when a `spqcprm2` build has opted in. This is the
    /// single entry point BOTH compressed encode paths use — the on-disk save path and the
    /// in-RAM compressed profile (`TripleStore::from_triples_compressed`, sq-559dp) — so the
    /// emit policy is honoured in exactly one place. With the `spqcprm2` feature OFF (which
    /// includes every `mmap`-off wasm build, since `spqcprm2` implies `mmap`) the gate compiles
    /// out entirely and this is an inlined `encode` with no branch, so the default in-RAM block
    /// stream stays byte-identical.
    #[inline]
    pub fn encode_emit(rows: &[[Id; 3]]) -> Self {
        #[cfg(feature = "spqcprm2")]
        if emit_format() == Format::V2 {
            return Self::encode_v2(rows);
        }
        Self::encode(rows)
    }

    /// Writes the COMPRESSED on-disk permutation format (auto-detected on open by its magic;
    /// raw files keep working). The 8-byte magic is [`FILE_MAGIC`] (`SPQCPRM1`) for a `V1` perm
    /// and [`FILE_MAGIC_V2`] (`SPQCPRM2`) for a `V2` perm — [FABLE-5] sq-7d3dj.32.2.7. Layout past
    /// the magic is version-independent, all little-endian:
    ///
    /// ```text
    /// magic[8] | len u64 | n_blocks u64 | blocks_len u64
    /// directory: n_blocks × { first_row [u32;3], byte_off u32 }
    /// blocks:    blocks_len bytes (the delta+varint block stream of `encode`/`encode_v2`)
    /// ```
    ///
    /// The byte stream is exactly the in-memory encoding, so [`from_mmap`](Self::from_mmap)
    /// can serve scans straight off the mapped file with no transcode. A `V1` perm writes
    /// byte-for-byte as before the migration (the backward-compat invariant).
    #[cfg(feature = "mmap")]
    pub fn write_to<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        let blocks = self.blocks.bytes();
        w.write_all(self.file_magic())?;
        w.write_all(&(self.len as u64).to_le_bytes())?;
        w.write_all(&(self.dir.len() as u64).to_le_bytes())?;
        w.write_all(&(blocks.len() as u64).to_le_bytes())?;
        for &(key, off) in &self.dir {
            for c in key {
                w.write_all(&c.to_le_bytes())?;
            }
            w.write_all(&off.to_le_bytes())?;
        }
        w.write_all(blocks)
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — the 8-byte file magic that matches this perm's block-stream
    /// [`Format`]: `SPQCPRM1` for `V1`, `SPQCPRM2` for `V2`. `from_mmap` inverts this to pick the
    /// decode reader, so a perm always round-trips through its own format.
    #[cfg(feature = "mmap")]
    fn file_magic(&self) -> &'static [u8; 8] {
        match self.format {
            Format::V1 => &FILE_MAGIC,
            Format::V2 => &FILE_MAGIC_V2,
        }
    }

    /// Opens a compressed permutation from a memory-mapped file written by
    /// [`write_to`](Self::write_to). Only the sparse directory is copied to the heap
    /// (~0.13 B/triple); the block stream stays on disk, decoded block-wise per scan —
    /// the lazy out-of-core mode.
    #[cfg(feature = "mmap")]
    pub fn from_mmap(map: memmap2::Mmap) -> std::io::Result<Self> {
        let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("compressed perm: {m}"));
        let b: &[u8] = &map;
        if b.len() < 32 {
            return Err(bad("missing FILE_MAGIC header"));
        }
        // [FABLE-5] sq-7d3dj.32.2.7 — AUTO-DETECT the block-stream format from the 8-byte magic.
        // A `SPQCPRM1` file decodes byte-identically forever (the backward-compat soundness
        // invariant); a `SPQCPRM2` file decodes through the frame-of-reference reader
        // (`decode_block_v2_at`). ANY OTHER 8 bytes is a loud, clean error — the caller
        // (`TripleStore::open`) has already gated on a leading `FILE_MAGIC`/`FILE_MAGIC_V2` to
        // reach here, but re-check inside the constructor so a corrupted magic never silently
        // misdecodes (mutation-witness in the corruption oracle). The header layout past the
        // magic (len/n_blocks/blocks_len + directory) is version-independent.
        let magic: [u8; 8] = b[..8].try_into().unwrap();
        let format = if magic == FILE_MAGIC {
            Format::V1
        } else if magic == FILE_MAGIC_V2 {
            Format::V2
        } else {
            return Err(bad("missing FILE_MAGIC header"));
        };
        let rd64 = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        // [OPUS-4.8] sq-ed2i: the three header counts are attacker-controlled. Compute the
        // layout with CHECKED arithmetic — the original `32 + n_blocks*16` / `dir_end +
        // blocks_len` were plain `usize` ops that wrap on a hostile `n_blocks`/`blocks_len`
        // near `u64::MAX`, so the length-equality check could pass against an undersized
        // file and the directory loop then read OOB. On any overflow / length mismatch we
        // return a clean error instead.
        let (len, n_blocks, blocks_len) = (rd64(8), rd64(16), rd64(24));
        // Reject sizes that cannot fit this file before any allocation: each directory entry
        // is 16 bytes and lives between the 32-byte header and the block stream, so
        // `n_blocks` is bounded by the actual file length. This also caps the
        // `Vec::with_capacity` below, so a hostile `n_blocks` cannot trigger a huge alloc.
        let n_blocks = usize::try_from(n_blocks).map_err(|_| bad("n_blocks exceeds usize"))?;
        let blocks_len = usize::try_from(blocks_len).map_err(|_| bad("blocks_len exceeds usize"))?;
        let dir_bytes = n_blocks.checked_mul(16).ok_or_else(|| bad("directory size overflows"))?;
        let dir_end = dir_bytes.checked_add(32).ok_or_else(|| bad("directory end overflows"))?;
        let total = dir_end.checked_add(blocks_len).ok_or_else(|| bad("file size overflows"))?;
        if b.len() != total {
            return Err(bad("length does not match header"));
        }
        let len = usize::try_from(len).map_err(|_| bad("len exceeds usize"))?;
        let mut dir = Vec::with_capacity(n_blocks);
        for e in (32..dir_end).step_by(16) {
            let rd32 = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
            // [OPUS-4.8] sq-ed2i: every block byte-offset must point at a valid start inside
            // the block stream (a non-empty stream has its first block at 0); otherwise
            // `decode_block_at` would index past the mapping. `validate_blocks` below then
            // proves the block at each offset decodes fully in-bounds.
            let off = rd32(e + 12);
            if off as usize >= blocks_len {
                return Err(bad("a directory block offset is past the end of the block stream"));
            }
            dir.push(([rd32(e), rd32(e + 4), rd32(e + 8)], off));
        }
        // [OPUS-4.8] sq-wihld — the on-disk `SPQCPRM1` format carries NO Bloom filters (they
        // are an in-RAM acceleration, never serialised), so a memory-mapped perm has none. The
        // mmap/out-of-core path therefore behaves exactly as before this feature; the Bloom
        // skip only ever fires on an in-RAM `encode`d perm. This keeps the on-disk format and
        // the mmap scan path byte-identical regardless of the `block-bloom` feature.
        let perm = CompressedPerm {
            dir,
            blocks: Blocks::Mapped { map, off: dir_end },
            len,
            #[cfg(feature = "block-bloom")]
            bloom: None,
            // [FABLE-5] sq-7d3dj.32.2.7 — the block-stream format auto-detected from the file
            // magic above; `decode_block_at` dispatches on it, so a V1 file reads through the
            // shipped absolute-col2 reader and a V2 file through `decode_block_v2_at`.
            format,
        };
        // [OPUS-4.8] sq-ed2i: decode-validate every block ONCE, here, so the (unchecked,
        // hot-path) `get_varint`/`decode_block_at` are provably in-bounds on later scans.
        // A bounded one-pass walk: each block's varints must stay within the stream and the
        // decoded row count across all blocks must equal `len`. Any malformation → Err, no
        // panic / OOB / unbounded loop. Peak extra RAM is O(1) (we count, not collect).
        perm.validate_blocks().map_err(|m| bad(&m))?;
        Ok(perm)
    }

    /// [OPUS-4.8] sq-ed2i — one-time structural validation of the mapped block stream.
    /// Walks every block with the bounds-checked varint reader, proving each block decodes
    /// fully within the stream and that the total decoded row count matches the header
    /// `len`. Returns a description on the first malformation. After this passes, the
    /// unchecked hot-path decoders cannot read out of bounds on a corrupt file.
    #[cfg(feature = "mmap")]
    fn validate_blocks(&self) -> Result<(), String> {
        let buf = self.blocks.bytes();
        let mut total_rows: usize = 0;
        for &(_, off) in &self.dir {
            let mut pos = off as usize;
            self.validate_one_block(buf, &mut pos).map(|rows| total_rows += rows).ok_or_else(|| {
                "a compressed block is truncated or has a malformed varint".to_string()
            })?;
        }
        if total_rows != self.len {
            return Err(format!("decoded {total_rows} rows but header declares {}", self.len));
        }
        Ok(())
    }

    /// Validates a single block at `*pos` (advancing it past the block), returning its row
    /// count, or `None` on any out-of-bounds / malformed varint. Mirrors the shape of
    /// [`decode_block_at`] but reads through [`get_varint_checked`] and discards values.
    #[cfg(feature = "mmap")]
    fn validate_one_block(&self, buf: &[u8], pos: &mut usize) -> Option<usize> {
        let count = get_varint_checked(buf, pos)? as usize;
        if count == 0 || count > BLOCK {
            return None; // a block holds 1..=BLOCK rows by construction
        }
        // First row: three absolute varints.
        for _ in 0..3 {
            get_varint_checked(buf, pos)?;
        }
        for _ in 1..count {
            let d0 = get_varint_checked(buf, pos)?;
            if d0 == 0 {
                let d1 = get_varint_checked(buf, pos)?;
                get_varint_checked(buf, pos)?; // col2 (delta or absolute)
                let _ = d1;
            } else {
                get_varint_checked(buf, pos)?; // col1 absolute
                get_varint_checked(buf, pos)?; // col2 absolute
            }
        }
        Some(count)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Resident bytes (directory + block stream). A memory-mapped block stream counts 0:
    /// its resident pages are OS page cache, not process heap (same as a raw mmap'd perm).
    pub fn heap_bytes(&self) -> usize {
        let stream = match &self.blocks {
            Blocks::Owned(v) => v.capacity(),
            #[cfg(feature = "mmap")]
            Blocks::Mapped { .. } => 0,
        };
        // [OPUS-4.8] sq-wihld — the OPT-IN per-block Bloom directory (when built) is resident
        // heap; count it so `heap_bytes` stays honest for the memory-budget paths.
        #[cfg(feature = "block-bloom")]
        let bloom = self.bloom.as_ref().map_or(0, block_bloom::BlockBloomDir::heap_bytes);
        #[cfg(not(feature = "block-bloom"))]
        let bloom = 0;
        self.dir.capacity() * std::mem::size_of::<([Id; 3], u32)>() + stream + bloom
    }

    /// [FABLE-5] sq-559dp — TEST-ONLY: the block-stream format this perm's `blocks` carry, so
    /// the store-level emit-gate differentials (`store::tests`) can assert WHICH encoder ran
    /// without making the private `format` field part of the public API.
    #[cfg(all(test, feature = "spqcprm2"))]
    pub(crate) fn format(&self) -> Format {
        self.format
    }

    /// Decodes the block starting at byte `off` into `out` (appending).
    #[inline]
    fn decode_block_at(&self, off: usize, out: &mut Vec<[Id; 3]>) {
        // [FABLE-5] sq-7d3dj.32.2.6 / sq-7d3dj.32.2.7 — dispatch on the perm's block-stream
        // format. With `mmap` off (the wasm in-RAM path) there is only V1 and this is the
        // original code verbatim; with `mmap` on, a V2-magic file opened by `from_mmap` reads
        // through the frame-of-reference reader.
        #[cfg(feature = "mmap")]
        if self.format == Format::V2 {
            return self.decode_block_v2_at(off, out);
        }
        let buf = self.blocks.bytes();
        let mut pos = off;
        let count = get_varint(buf, &mut pos) as usize;
        let mut prev = [
            get_varint(buf, &mut pos) as Id,
            get_varint(buf, &mut pos) as Id,
            get_varint(buf, &mut pos) as Id,
        ];
        out.push(prev);
        for _ in 1..count {
            let d0 = get_varint(buf, &mut pos) as Id;
            // [OPUS-4.8] sq-ed2i: `from_mmap` proves every varint here is in-bounds, but a
            // TAMPERED (checksum-less) block can still hold a delta that makes `prev + d`
            // exceed `u32::MAX`. `+` panics on that overflow in debug; `wrapping_add` cannot
            // panic and matches release wrapping — the resulting (wrong) id is then handled
            // safely by the bounds-checked `Dict::record` (sq-ky2a), so a corrupt perm is
            // wrong-but-safe (the documented trusted-store boundary), never a panic / UB.
            let cur = if d0 == 0 {
                let d1 = get_varint(buf, &mut pos) as Id;
                if d1 == 0 {
                    [prev[0], prev[1], prev[2].wrapping_add(get_varint(buf, &mut pos) as Id)]
                } else {
                    [prev[0], prev[1].wrapping_add(d1), get_varint(buf, &mut pos) as Id]
                }
            } else {
                [prev[0].wrapping_add(d0), get_varint(buf, &mut pos) as Id, get_varint(buf, &mut pos) as Id]
            };
            out.push(cur);
            prev = cur;
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — decodes one `SPQCPRM2` frame-of-reference block at byte
    /// `off` into `out`. Mirrors [`decode_block_at`] exactly except the `reset_d1` col2 is
    /// reconstructed from the zigzag frame offset plus the block's first-row col2 (captured
    /// as `first_col2` from the absolute first row), inverting [`encode_block_v2`]. The same
    /// `wrapping_add` id-overflow safety as the V1 decoder applies (a tampered block is
    /// wrong-but-safe, never a panic / OOB — the documented trusted-store boundary).
    #[cfg(feature = "mmap")]
    #[inline]
    fn decode_block_v2_at(&self, off: usize, out: &mut Vec<[Id; 3]>) {
        let buf = self.blocks.bytes();
        let mut pos = off;
        let count = get_varint(buf, &mut pos) as usize;
        let mut prev = [
            get_varint(buf, &mut pos) as Id,
            get_varint(buf, &mut pos) as Id,
            get_varint(buf, &mut pos) as Id,
        ];
        let first_col2 = prev[2] as i64; // the frame origin the encoder used
        out.push(prev);
        for _ in 1..count {
            let d0 = get_varint(buf, &mut pos) as Id;
            let cur = if d0 == 0 {
                let d1 = get_varint(buf, &mut pos) as Id;
                if d1 == 0 {
                    [prev[0], prev[1], prev[2].wrapping_add(get_varint(buf, &mut pos) as Id)]
                } else {
                    // Invert the frame-of-reference: absolute col2 = first_col2 + signed offset.
                    let off2 = get_zigzag_varint(buf, &mut pos);
                    let abs = (first_col2.wrapping_add(off2)) as u64 as Id;
                    [prev[0], prev[1].wrapping_add(d1), abs]
                }
            } else {
                [prev[0].wrapping_add(d0), get_varint(buf, &mut pos) as Id, get_varint(buf, &mut pos) as Id]
            };
            out.push(cur);
            prev = cur;
        }
    }

    /// Decodes the whole permutation (for stats / full iteration).
    pub fn decode_all(&self) -> Vec<[Id; 3]> {
        let mut out = Vec::with_capacity(self.len);
        for &(_, off) in &self.dir {
            self.decode_block_at(off as usize, &mut out);
        }
        out
    }

    /// Exact count of rows in `[lo, hi]` without materializing the range: decode at most
    /// the two boundary blocks (the interior blocks are full and wholly inside the range),
    /// so this is O(block) regardless of how many rows the range spans — the planner's
    /// cheap cardinality estimate.
    pub fn count_range(&self, lo: [Id; 3], hi: [Id; 3]) -> usize {
        if self.dir.is_empty() || lo > hi {
            return 0;
        }
        let first = self.dir.partition_point(|&(k, _)| k <= lo).saturating_sub(1);
        let last = self.dir.partition_point(|&(k, _)| k <= hi).saturating_sub(1).max(first);
        let mut buf = Vec::with_capacity(BLOCK);
        self.decode_block_at(self.dir[first].1 as usize, &mut buf);
        if first == last {
            let s = buf.partition_point(|r| *r < lo);
            let e = buf.partition_point(|r| *r <= hi);
            return e - s;
        }
        // First block: rows >= lo. Interior blocks: full (BLOCK each). Last block: rows <= hi.
        let first_count = buf.len() - buf.partition_point(|r| *r < lo);
        buf.clear();
        self.decode_block_at(self.dir[last].1 as usize, &mut buf);
        let last_count = buf.partition_point(|r| *r <= hi);
        first_count + (last - first - 1) * BLOCK + last_count
    }

    /// Returns the rows in `[lo, hi]` (inclusive, comparing full triples) by decoding
    /// only the blocks that span that key range, then trimming. The result is sorted —
    /// identical to a binary-search range over the raw permutation.
    pub fn range(&self, lo: [Id; 3], hi: [Id; 3]) -> Vec<[Id; 3]> {
        if self.dir.is_empty() || lo > hi {
            return Vec::new();
        }
        // First block whose first-key could contain `lo`: the last block with first-key
        // <= lo (a key in [lo,hi] could start partway into that block).
        let first = self.dir.partition_point(|&(k, _)| k <= lo).saturating_sub(1);
        // Last block whose first-key <= hi (blocks after it are entirely > hi).
        let last = self.dir.partition_point(|&(k, _)| k <= hi).saturating_sub(1);
        let last = last.max(first);

        // [OPUS-4.8] sq-wihld — when the LEADING column is equality-bound (`lo[0] == hi[0]`,
        // i.e. a point/prefix lookup on a constant subject/object), an OPT-IN per-block Bloom
        // filter can skip blocks whose `[min,max]` zone-map span overlaps the constant but
        // that do not actually contain it (the high-NDV overlapping-block case the min/max map
        // cannot prune). The filter has zero false negatives, so any block it skips provably
        // holds no row with that leading id — the trimmed range below is unchanged. We only
        // consult it for the leading-equality shape; a range / full scan runs the loop as before.
        #[cfg(feature = "block-bloom")]
        let bloom_key: Option<Id> = match &self.bloom {
            Some(_) if lo[0] == hi[0] => Some(lo[0]),
            _ => None,
        };

        let mut decoded = Vec::with_capacity((last - first + 1) * BLOCK);
        for b in first..=last {
            #[cfg(feature = "block-bloom")]
            if let (Some(key), Some(bloom)) = (bloom_key, &self.bloom) {
                if !bloom.maybe_contains(b, key) {
                    continue; // block provably holds no row with this leading id.
                }
            }
            self.decode_block_at(self.dir[b].1 as usize, &mut decoded);
        }
        // Trim to the exact inclusive range.
        let s = decoded.partition_point(|r| *r < lo);
        let e = decoded.partition_point(|r| *r <= hi);
        decoded.drain(..s);
        decoded.truncate(e - s);
        decoded
    }

    /// [OPUS-4.8] sq-wihld — test-only: was a per-block Bloom directory built for this perm?
    /// Lets a test confirm the density gate admitted a high-NDV leading column (and so the
    /// skip path is actually exercised), versus declining a low-NDV one.
    #[cfg(all(test, feature = "block-bloom"))]
    fn has_bloom(&self) -> bool {
        self.bloom.is_some()
    }

    /// [OPUS-4.8] sq-wihld — test-only: for an equality-bound leading id `key`, returns
    /// `(candidate_blocks, bloom_skipped)` over the zone-map span the `range` loop would
    /// visit — `candidate_blocks` is the number of blocks whose `[min,max]` span overlaps the
    /// point lookup (what the min/max zone map alone leaves to decode) and `bloom_skipped` is
    /// how many of those the Bloom filter proves cannot contain `key` (so `range` skips their
    /// decode). A positive `bloom_skipped` proves the optimisation does real work; the value is
    /// purely diagnostic and never asserted as a performance number.
    #[cfg(all(test, feature = "block-bloom"))]
    fn bloom_skip_stats(&self, key: Id) -> (usize, usize) {
        let lo = [key, Id::MIN, Id::MIN];
        let hi = [key, Id::MAX, Id::MAX];
        if self.dir.is_empty() {
            return (0, 0);
        }
        let first = self.dir.partition_point(|&(k, _)| k <= lo).saturating_sub(1);
        let last = self.dir.partition_point(|&(k, _)| k <= hi).saturating_sub(1).max(first);
        let candidates = last - first + 1;
        let bloom = self.bloom.as_ref().expect("bloom_skip_stats requires a built filter");
        let skipped = (first..=last).filter(|&b| !bloom.maybe_contains(b, key)).count();
        (candidates, skipped)
    }
}

/// [OPUS-4.8] sq-vkz7 — STREAMING writer of the [`CompressedPerm`] on-disk format
/// ([`FILE_MAGIC`] `SPQCPRM1`, or [`FILE_MAGIC_V2`] `SPQCPRM2` when a `spqcprm2` build opts in
/// via the `emit_format` config gate — [FABLE-5] sq-7d3dj.32.2.7). Encodes the FoR+varint
/// block stream as sorted rows arrive, so the external-memory build can emit compressed perms
/// STRAIGHT FROM the merge tail — no raw-write-then-reopen-then-`decode_all`-then-`encode`
/// recompress second pass over an 84+ GB index. The byte stream it produces is BYTE-IDENTICAL to
/// `CompressedPerm::encode_emit(rows).write_to(w)` for the same `rows` and emit format (proven in
/// tests) — so in the shipped `V1` default it is byte-for-byte what it always was.
///
/// The format is `header[32] | directory | blocks`, so the directory (which we only finish
/// once the last block is sealed) must physically precede the block stream. We therefore
/// buffer the SPARSE directory in RAM (one 16-byte entry per [`BLOCK`] rows ≈ 0.13 B/triple
/// — the same directory the open path already holds resident) and stream the block bytes to
/// a side file; [`finish`](Self::finish) writes `header | directory` then appends the block
/// side file. The block COPY is over the already-compressed stream (~2.5× smaller than raw),
/// and there is no decode / re-sort — the saving the bead targets.
#[cfg(feature = "mmap")]
pub struct CompressedPermWriter {
    /// [FABLE-5] sq-7d3dj.32.2.7 — the block-stream [`Format`] this writer emits (fixed at
    /// construction from the `emit_format` config gate). Drives the per-block encoder and the
    /// file magic, so the streamed bytes match `encode_emit(rows).write_to` for the same format.
    format: Format,
    /// One (first-triple, byte-offset-into-blocks) entry per sealed block.
    dir: Vec<([Id; 3], u32)>,
    /// The current (not-yet-sealed) block's rows, up to [`BLOCK`].
    cur: Vec<[Id; 3]>,
    /// Reusable scratch for one encoded block.
    scratch: Vec<u8>,
    /// The block stream, written to a side file as blocks are sealed. `None` only after
    /// [`finish`](Self::finish) has taken it to close the write handle before the read.
    body: Option<std::io::BufWriter<std::fs::File>>,
    /// Path of the block side file (removed by [`finish`](Self::finish)).
    body_path: std::path::PathBuf,
    /// Running length of the block stream in bytes (the next block's byte offset).
    blocks_len: u64,
    /// Total rows pushed so far.
    len: u64,
}

#[cfg(feature = "mmap")]
impl CompressedPermWriter {
    /// Creates a writer that will produce the compressed perm at `out` in the block-stream
    /// [`Format`] the build's `emit_format` config gate selects (`SPQCPRM1` by default), staging
    /// the block stream in a sibling temp file `<out>.blocks` (same directory ⇒ same filesystem,
    /// so the final assembly copy never crosses devices). [FABLE-5] sq-7d3dj.32.2.7
    pub fn create(out: &std::path::Path) -> std::io::Result<Self> {
        Self::create_with(out, emit_format())
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — like [`create`](Self::create) but with an explicit emit
    /// [`Format`], bypassing the config gate. Used by the V2 migration differentials to pin a
    /// format without setting the process env / thread override.
    pub fn create_with(out: &std::path::Path, format: Format) -> std::io::Result<Self> {
        let mut body_path = out.as_os_str().to_owned();
        body_path.push(".blocks");
        let body_path = std::path::PathBuf::from(body_path);
        let body = std::io::BufWriter::new(std::fs::File::create(&body_path)?);
        Ok(CompressedPermWriter {
            format,
            dir: Vec::new(),
            cur: Vec::with_capacity(BLOCK),
            scratch: Vec::with_capacity(BLOCK * 6),
            body: Some(body),
            body_path,
            blocks_len: 0,
            len: 0,
        })
    }

    /// Appends one row. Rows MUST arrive in this permutation's sorted column order and be
    /// already deduplicated (the merge tail guarantees both); the delta encoder relies on
    /// `row >= prev` and strictly-increasing within an equal-prefix run.
    pub fn push(&mut self, row: [Id; 3]) -> std::io::Result<()> {
        self.cur.push(row);
        self.len += 1;
        if self.cur.len() == BLOCK {
            self.seal_block()?;
        }
        Ok(())
    }

    /// Seals the current full/partial block: records its directory entry then encodes it to
    /// the block side file. A no-op if the current block is empty.
    fn seal_block(&mut self) -> std::io::Result<()> {
        if self.cur.is_empty() {
            return Ok(());
        }
        // The directory byte-offset must fit u32, exactly as `CompressedPerm::encode`'s does
        // (the directory stores `u32` offsets). A single perm's block stream is bounded by
        // u32 by construction of the format; surface an error rather than silently truncate.
        let off = u32::try_from(self.blocks_len).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "compressed perm: block stream exceeds 4 GiB (u32 directory offset overflow)",
            )
        })?;
        self.dir.push((self.cur[0], off));
        self.scratch.clear();
        // [FABLE-5] sq-7d3dj.32.2.7 — the per-block encoder matches this writer's emit format,
        // so a `V1` writer streams byte-for-byte the shipped stream and a `V2` writer streams
        // the frame-of-reference stream (mirroring `encode`/`encode_v2` block-for-block).
        match self.format {
            Format::V1 => encode_block(&self.cur, &mut self.scratch),
            Format::V2 => encode_block_v2(&self.cur, &mut self.scratch),
        }
        let body = self.body.as_mut().expect("body present until finish() consumes the writer");
        std::io::Write::write_all(body, &self.scratch)?;
        self.blocks_len += self.scratch.len() as u64;
        self.cur.clear();
        Ok(())
    }

    /// Finishes the stream: seals any partial block, then writes the final compressed file to
    /// `out` (`magic[8] | header | directory | blocks`, magic per this writer's [`Format`]) and
    /// removes the block side file. For a non-empty perm the bytes are identical to
    /// `CompressedPerm::encode_emit(all_rows).write_to(out)` for the matching emit format.
    ///
    /// EMPTY-PERM POLICY: when no rows were pushed, `out` is written as a ZERO-byte file
    /// (NOT a bare 32-byte header), matching `TripleStore::save_compressed` which leaves an
    /// unbuilt permutation raw-empty so `open` skips it by size. This keeps the streaming
    /// build byte-identical to a raw build followed by `recompress`.
    pub fn finish(mut self, out: &std::path::Path) -> std::io::Result<()> {
        self.seal_block()?;
        // Flush and CLOSE the block write handle before re-opening it for reading (portable;
        // avoids a concurrent write+read handle to the same file). `take()` drops the inner
        // `File` here rather than at end of scope.
        let mut body = self.body.take().expect("body present until finish() consumes the writer");
        std::io::Write::flush(&mut body)?;
        drop(body);
        if self.len == 0 {
            // Unbuilt/empty perm: raw-empty file, like `save_compressed`'s `!rows.is_empty()`.
            std::fs::File::create(out)?;
            std::fs::remove_file(&self.body_path).ok();
            return Ok(());
        }
        // Re-open the staged block stream for the assembly copy.
        let mut body_rd = std::io::BufReader::new(std::fs::File::open(&self.body_path)?);
        let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
        {
            use std::io::Write;
            // [FABLE-5] sq-7d3dj.32.2.7 — magic matches this writer's emit format (V1 unchanged).
            let magic: &[u8; 8] = match self.format {
                Format::V1 => &FILE_MAGIC,
                Format::V2 => &FILE_MAGIC_V2,
            };
            w.write_all(magic)?;
            w.write_all(&self.len.to_le_bytes())?;
            w.write_all(&(self.dir.len() as u64).to_le_bytes())?;
            w.write_all(&self.blocks_len.to_le_bytes())?;
            for &(key, off) in &self.dir {
                for c in key {
                    w.write_all(&c.to_le_bytes())?;
                }
                w.write_all(&off.to_le_bytes())?;
            }
        }
        std::io::copy(&mut body_rd, &mut w)?;
        std::io::Write::flush(&mut w)?;
        drop(body_rd);
        std::fs::remove_file(&self.body_path).ok();
        Ok(())
    }
}

#[cfg(feature = "mmap")]
impl Drop for CompressedPermWriter {
    fn drop(&mut self) {
        // If `finish` was not called (e.g. a build error unwound past us), don't leak the
        // staged block side file. `finish` consumes `self`, so reaching `drop` with a path
        // still present means the writer was abandoned.
        std::fs::remove_file(&self.body_path).ok();
    }
}

/// [FABLE-5] sq-7d3dj.32.2.4 — SPIKE-ONLY per-field byte attribution over the
/// [`encode_block`] stream. This whole module is `#[cfg(test)]`: it adds ZERO bytes to
/// the shipped `SPQCPRM1` stream and does not touch the encode/decode hot path (the
/// `wasm_bundle_bytes` `feature_off_exact` floor is unmoved). It exists to adjudicate the
/// §4 root-cause hypotheses (H1/H2/H3) of `research/compressed-memory-profile.md` for why
/// the compressed store's B/triple grows 36.75 (1M) → 48.75 (10M) then plateaus at 48.75
/// (50M): it mirrors `encode_block` field-for-field and attributes each emitted varint's
/// byte length to the field class that produced it. All numbers this module prints are
/// NON-canonical work-box measurements (never a doc/test perf number).
#[cfg(test)]
mod byte_attribution {
    use super::{encode_block, Id, BLOCK};

    /// Byte length of the unsigned LEB128 varint that `put_varint` would emit for `x`
    /// (mirrors its `while x >= 0x80 { x >>= 7 }` loop exactly): `⌈bits/7⌉`, min 1.
    #[inline]
    fn varint_len(x: u64) -> usize {
        let mut x = x;
        let mut n = 1usize;
        while x >= 0x80 {
            x >>= 7;
            n += 1;
        }
        n
    }

    /// The disjoint field classes the block stream's bytes are attributed to. These are
    /// exactly the write sites in [`encode_block`]; every byte the encoder emits lands in
    /// exactly one class, so `total()` equals the real encoded block-stream length.
    #[derive(Clone, Copy, Default, Debug)]
    struct FieldBytes {
        /// The per-block `count` varint (one per block).
        count: u64,
        /// The block's FIRST row, written as three absolute varints (col0/col1/col2).
        first_row_abs: u64,
        /// col1+col2 written ABSOLUTE because the leading column changed (`d0 != 0`).
        reset_d0: u64,
        /// col2 written ABSOLUTE because the middle column changed (`d0 == 0 && d1 != 0`).
        reset_d1: u64,
        /// The `d0` (leading-column) delta varint on every non-first row.
        d0: u64,
        /// The `d1` (middle-column) delta varint, emitted only when `d0 == 0`.
        d1: u64,
        /// The `d2` (trailing-column) delta varint, emitted only when `d0 == 0 && d1 == 0`.
        d2: u64,
    }

    impl FieldBytes {
        fn total(&self) -> u64 {
            self.count + self.first_row_abs + self.reset_d0 + self.reset_d1 + self.d0 + self.d1 + self.d2
        }

        fn add(&mut self, o: &FieldBytes) {
            self.count += o.count;
            self.first_row_abs += o.first_row_abs;
            self.reset_d0 += o.reset_d0;
            self.reset_d1 += o.reset_d1;
            self.d0 += o.d0;
            self.d1 += o.d1;
            self.d2 += o.d2;
        }
    }

    /// Attributes the bytes of ONE block (`chunk`, sorted, 1..=`BLOCK` rows) to field
    /// classes, mirroring [`encode_block`] write-site for write-site. This is a pure
    /// re-derivation of the same varint lengths — no bytes are emitted — so its `total()`
    /// is asserted equal to the real `encode_block` output length in a self-check test.
    fn attribute_block(chunk: &[[Id; 3]], fb: &mut FieldBytes) {
        fb.count += varint_len(chunk.len() as u64) as u64;
        fb.first_row_abs += (varint_len(chunk[0][0] as u64)
            + varint_len(chunk[0][1] as u64)
            + varint_len(chunk[0][2] as u64)) as u64;
        for w in chunk.windows(2) {
            let (p, r) = (w[0], w[1]);
            let d0 = r[0] - p[0];
            fb.d0 += varint_len(d0 as u64) as u64;
            if d0 == 0 {
                let d1 = r[1] - p[1];
                fb.d1 += varint_len(d1 as u64) as u64;
                if d1 == 0 {
                    fb.d2 += varint_len((r[2] - p[2]) as u64) as u64; // strictly increasing
                } else {
                    fb.reset_d1 += varint_len(r[2] as u64) as u64; // col2 resets → absolute
                }
            } else {
                // cols 1,2 reset → absolute
                fb.reset_d0 += (varint_len(r[1] as u64) + varint_len(r[2] as u64)) as u64;
            }
        }
    }

    /// Attributes a whole permutation (sorted rows in this permutation's column order),
    /// blocking exactly as [`CompressedPerm::encode`] does (`rows.chunks(BLOCK)`), and
    /// adding the resident directory cost (16 B/block: one `([Id;3], u32)` per block).
    fn attribute_perm(rows: &[[Id; 3]]) -> (FieldBytes, u64) {
        attribute_perm_bs(rows, BLOCK)
    }

    /// [SONNET-4.6] sq-7d3dj.32.2 — as [`attribute_perm`] but at an ARBITRARY block size
    /// `bs` (rows per block), for the block-size sweep. Legitimate because the block stream
    /// is block-size agnostic: every block self-describes its row count with a leading
    /// `count` varint, so `rows.chunks(bs)` + the REAL [`encode_block`] is byte-for-byte
    /// what a `BLOCK = bs` build would emit. No encoder change is needed to measure the
    /// trade (and none is made — this module is `#[cfg(test)]`).
    fn attribute_perm_bs(rows: &[[Id; 3]], bs: usize) -> (FieldBytes, u64) {
        let mut fb = FieldBytes::default();
        let mut n_blocks = 0u64;
        for chunk in rows.chunks(bs) {
            attribute_block(chunk, &mut fb);
            n_blocks += 1;
        }
        let dir_bytes = n_blocks * 16;
        (fb, dir_bytes)
    }

    /// The byte classes that exist ONLY because the stream is blocked: the per-block
    /// `count` varint, the block's absolute first row, and the resident directory entry.
    /// Every other class is a row delta whose width is set by id density, not by framing.
    ///
    /// This is the quantity that bounds the block-size question. Growing `bs` drives the
    /// block count `B → 1`, so the *most* any block-size increase can ever save is this
    /// total (strictly less, in fact: each row promoted out of `first_row_abs` still has
    /// to be paid for as a delta). It is therefore a sound upper bound on the win, which
    /// is what [`block_size_cannot_flatten_the_scale_growth`] asserts against.
    fn block_overhead(fb: &FieldBytes, dir: u64) -> u64 {
        fb.count + fb.first_row_abs + dir
    }

    /// A deterministic xorshift PRNG (same family as the file's `sample`), seeded so runs
    /// are reproducible across hosts.
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }
        #[inline]
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        /// A Zipf-ish skewed draw in `1..=n`: bias toward small ids (frequent terms get
        /// small, insertion-ordered ids — the H3 mechanism). Squaring a uniform in [0,1)
        /// concentrates mass near 0; `1 +` maps to `1..=n`.
        #[inline]
        fn skewed(&mut self, n: u64) -> u64 {
            let u = (self.next() >> 11) as f64 / (1u64 << 53) as f64; // uniform [0,1)
            1 + ((u * u) * (n.saturating_sub(1)) as f64) as u64
        }
    }

    /// Synthesises `n_triples` canonical `[s, p, o]` triples over a term space of
    /// `n_terms` distinct ids, mimicking WatDiv skew: a small predicate vocabulary,
    /// skewed (Zipf-ish) subjects and objects so frequent terms take small ids — the
    /// insertion-order-gives-small-ids property H3 turns on. Deduplicated + returned
    /// UNSORTED (the caller re-sorts per permutation).
    fn synth_watdiv(n_triples: usize, n_terms: u64, seed: u64) -> Vec<[Id; 3]> {
        let mut rng = Rng::new(seed);
        // Reserve the low id band for predicates (WatDiv has ~85 predicates); subjects &
        // objects range across the whole term space but skewed toward small ids.
        let n_preds: u64 = 85;
        let mut set = std::collections::HashSet::with_capacity(n_triples);
        let mut v = Vec::with_capacity(n_triples);
        let mut guard = 0usize;
        while v.len() < n_triples && guard < n_triples * 4 {
            guard += 1;
            let s = rng.skewed(n_terms) as Id;
            let p = (1 + (rng.next() % n_preds)) as Id;
            let o = rng.skewed(n_terms) as Id;
            let t = [s, p, o];
            if set.insert(t) {
                v.push(t);
            }
        }
        v
    }

    /// Sorts a copy of `triples` into `perm`'s column order (the exact rows
    /// `from_triples_compressed` would hand `CompressedPerm::encode` for that permutation).
    fn perm_rows(triples: &[[Id; 3]], order: [usize; 3]) -> Vec<[Id; 3]> {
        let mut rows: Vec<[Id; 3]> =
            triples.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// The six permutation column orders (kept local so the spike does not depend on
    /// `store::Perm`, which is out of this module's scope).
    const PERM_ORDERS: [(&str, [usize; 3]); 6] = [
        ("SPO", [0, 1, 2]),
        ("SOP", [0, 2, 1]),
        ("PSO", [1, 0, 2]),
        ("POS", [1, 2, 0]),
        ("OSP", [2, 0, 1]),
        ("OPS", [2, 1, 0]),
    ];

    /// SELF-CHECK: the attribution's `total()` must equal the REAL `encode_block` output
    /// length for every block, byte-for-byte — otherwise the attribution table is a
    /// fiction. Runs across block boundaries and both reset shapes.
    #[test]
    fn attribution_total_equals_real_encoded_length() {
        for &n_terms in &[100_000u64, 1_000_000, 5_000_000] {
            let triples = synth_watdiv(20_000, n_terms, 0xA5A5);
            for (_, order) in PERM_ORDERS {
                let rows = perm_rows(&triples, order);
                // Real encoded block-stream length.
                let mut real = Vec::new();
                for chunk in rows.chunks(BLOCK) {
                    encode_block(chunk, &mut real);
                }
                // Attributed length.
                let (fb, _dir) = attribute_perm(&rows);
                assert_eq!(
                    fb.total(),
                    real.len() as u64,
                    "attribution total != real encoded length (n_terms={}, order={:?})",
                    n_terms,
                    order
                );
            }
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.4 — the MEASUREMENT. Prints a per-field, per-permutation
    /// byte-attribution table at three id-density regimes (100K / 1M / 5M distinct terms),
    /// each with the same triple:term ratio so the ONLY moving variable is term-space bits.
    /// Run with `cargo test -p sparq-core --lib byte_attribution -- --nocapture` to see the
    /// tables. All numbers are NON-canonical work-box measurements. `#[ignore]` so the
    /// default `cargo test` stays fast (the 5M-term regime allocates a few hundred MB); the
    /// self-check test above runs unconditionally and pins the attribution's correctness.
    #[test]
    #[ignore = "spike measurement: run explicitly with --ignored --nocapture (allocates ~hundreds of MB)"]
    fn measure_byte_attribution_across_id_density() {
        // Fixed triple:term ratio (~10 triples/term, roughly WatDiv's density) so the id
        // count grows with the term space and the regimes are comparable per-triple.
        let regimes: [(&str, usize, u64); 3] = [
            ("100K-term", 1_000_000, 100_000),
            ("1M-term", 10_000_000, 1_000_000),
            ("5M-term", 50_000_000, 5_000_000),
        ];

        println!("\n===== sq-7d3dj.32.2.4 per-field byte attribution (NON-canonical work-box) =====");
        for (label, want_triples, n_terms) in regimes {
            let triples = synth_watdiv(want_triples, n_terms, 0x5EED);
            let n = triples.len() as f64;
            let bits = (64 - (n_terms).leading_zeros()) as usize;
            println!(
                "\n--- regime {} : {} distinct triples, {} terms (~{} term-space bits) ---",
                label, triples.len(), n_terms, bits
            );
            println!(
                "{:<5} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9}",
                "perm", "count", "first", "reset_d0", "reset_d1", "d0", "d1", "d2", "dir", "blk_tot", "B/triple"
            );

            let mut agg = FieldBytes::default();
            let mut agg_dir = 0u64;
            let mut agg_rows = 0u64;
            for (name, order) in PERM_ORDERS {
                let rows = perm_rows(&triples, order);
                let (fb, dir) = attribute_perm(&rows);
                let blk_tot = fb.total();
                let bpt = (blk_tot + dir) as f64 / rows.len().max(1) as f64;
                println!(
                    "{:<5} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9.3}",
                    name, fb.count, fb.first_row_abs, fb.reset_d0, fb.reset_d1, fb.d0, fb.d1, fb.d2, dir, blk_tot, bpt
                );
                agg.add(&fb);
                agg_dir += dir;
                agg_rows += rows.len() as u64;
            }
            // Per-triple attribution summed across the six permutations (the store's
            // block-stream B/triple = sum over perms / #triples). This is the quantity the
            // §4 table (6.125 → 8.125 B/triple/perm) is about.
            let per_triple = |b: u64| b as f64 / n;
            println!(
                "sum/triple  count={:.3}  first={:.3}  reset_d0={:.3}  reset_d1={:.3}  d0={:.3}  d1={:.3}  d2={:.3}  dir={:.3}  ALL={:.3}",
                per_triple(agg.count),
                per_triple(agg.first_row_abs),
                per_triple(agg.reset_d0),
                per_triple(agg.reset_d1),
                per_triple(agg.d0),
                per_triple(agg.d1),
                per_triple(agg.d2),
                per_triple(agg_dir),
                per_triple(agg.total() + agg_dir),
            );
            let _ = agg_rows;
        }
        println!("\n===== end attribution — verdict adjudicated in the bead note =====\n");
    }

    /// A UNIFORM (non-skewed) id draw in `1..=n`, for the H3 discriminator: if the growth
    /// were purely LEB128 quantization of the term space (H1/H2, blind to id assignment),
    /// swapping skewed→uniform ids at the SAME term-space size would not change B/triple.
    /// If frequent-terms-get-small-ids (H3) is what holds the plateau, uniform ids should
    /// be MORE expensive (they spread mass across the whole bit width).
    fn synth_uniform(n_triples: usize, n_terms: u64, seed: u64) -> Vec<[Id; 3]> {
        let mut rng = Rng::new(seed);
        let n_preds: u64 = 85;
        let mut set = std::collections::HashSet::with_capacity(n_triples);
        let mut v = Vec::with_capacity(n_triples);
        let mut guard = 0usize;
        while v.len() < n_triples && guard < n_triples * 4 {
            guard += 1;
            let s = (1 + rng.next() % n_terms) as Id;
            let p = (1 + (rng.next() % n_preds)) as Id;
            let o = (1 + rng.next() % n_terms) as Id;
            let t = [s, p, o];
            if set.insert(t) {
                v.push(t);
            }
        }
        v
    }

    fn agg_per_triple(triples: &[[Id; 3]]) -> (FieldBytes, u64, u64) {
        let mut agg = FieldBytes::default();
        let mut agg_dir = 0u64;
        let mut rows_tot = 0u64;
        for (_, order) in PERM_ORDERS {
            let rows = perm_rows(triples, order);
            let (fb, dir) = attribute_perm(&rows);
            agg.add(&fb);
            agg_dir += dir;
            rows_tot += rows.len() as u64;
        }
        (agg, agg_dir, rows_tot)
    }

    /// [FABLE-5] sq-7d3dj.32.2.4 — H1/H2/H3 DISCRIMINATOR. Two controlled sweeps at a
    /// FIXED triple count so the only variable is (a) term-space bits and (b) skewed vs
    /// uniform id assignment. NON-canonical work-box. Run with `--ignored --nocapture`.
    ///
    /// Sweep A (bit-width, skewed): fixed 4M triples, term space 250K → 4M (18→22 bits).
    /// Isolates how B/triple moves with term-space bits when frequent terms keep small ids.
    ///
    /// Sweep B (skew vs uniform): fixed 4M triples, 4M terms, skewed vs uniform id draw.
    /// If uniform is materially costlier, the plateau is H3 (small-ids-for-frequent-terms),
    /// not pure H1/H2 bit-width quantization.
    #[test]
    #[ignore = "spike discriminator: run explicitly with --ignored --nocapture"]
    fn discriminate_h1_h2_h3() {
        const N: usize = 4_000_000;
        println!("\n===== sq-7d3dj.32.2.4 H1/H2/H3 discriminator (NON-canonical work-box) =====");

        println!("\n--- Sweep A: fixed {} triples, vary term-space bits (SKEWED ids) ---", N);
        println!("{:<10} {:>6} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>10}", "terms", "bits", "reset_d0", "reset_d1", "d0", "d1", "d2", "first", "B/triple");
        for &n_terms in &[250_000u64, 500_000, 1_000_000, 2_000_000, 4_000_000] {
            let triples = synth_watdiv(N, n_terms, 0xC0FFEE);
            let n = triples.len().max(1) as f64;
            let bits = 64 - n_terms.leading_zeros();
            let (agg, dir, _) = agg_per_triple(&triples);
            let pt = |b: u64| b as f64 / n;
            println!(
                "{:<10} {:>6} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>10.3}",
                n_terms, bits, pt(agg.reset_d0), pt(agg.reset_d1), pt(agg.d0), pt(agg.d1), pt(agg.d2), pt(agg.first_row_abs), pt(agg.total() + dir)
            );
        }

        println!("\n--- Sweep B: fixed {} triples, 4M terms, SKEWED vs UNIFORM ids ---", N);
        println!("{:<10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>10}", "assign", "reset_d0", "reset_d1", "d0", "d1", "d2", "first", "B/triple");
        for (label, triples) in [
            ("skewed", synth_watdiv(N, 4_000_000, 0xC0FFEE)),
            ("uniform", synth_uniform(N, 4_000_000, 0xC0FFEE)),
        ] {
            let n = triples.len().max(1) as f64;
            let (agg, dir, _) = agg_per_triple(&triples);
            let pt = |b: u64| b as f64 / n;
            println!(
                "{:<10} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>10.3}",
                label, pt(agg.reset_d0), pt(agg.reset_d1), pt(agg.d0), pt(agg.d1), pt(agg.d2), pt(agg.first_row_abs), pt(agg.total() + dir)
            );
        }
        println!("\n===== end discriminator =====\n");
    }

    /// Sums the per-field attribution over all six permutations at block size `bs`,
    /// returning `(fields, directory_bytes)` — the whole store's block-stream cost, which
    /// is what the §4 B/triple figures are about.
    fn agg_per_triple_bs(triples: &[[Id; 3]], bs: usize) -> (FieldBytes, u64) {
        let mut agg = FieldBytes::default();
        let mut agg_dir = 0u64;
        for (_, order) in PERM_ORDERS {
            let rows = perm_rows(triples, order);
            let (fb, dir) = attribute_perm_bs(&rows, bs);
            agg.add(&fb);
            agg_dir += dir;
        }
        (agg, agg_dir)
    }

    /// Store B/triple (all six permutations, block stream + directory) at block size `bs`,
    /// alongside the framing-only share that a block-size change can move.
    fn store_bpt_at(triples: &[[Id; 3]], bs: usize) -> (f64, f64) {
        let n = triples.len().max(1) as f64;
        let (agg, dir) = agg_per_triple_bs(triples, bs);
        ((agg.total() + dir) as f64 / n, block_overhead(&agg, dir) as f64 / n)
    }

    /// A corpus small enough for the default (non-`--ignored`) test run: 6 perms over
    /// 200K triples is ~1.2M attributed rows, well under a second.
    const SWEEP_N: usize = 200_000;

    /// SELF-CHECK at NON-default block sizes: the sweep is only meaningful if attribution
    /// stays byte-exact when the chunking changes, so re-run the `total() == real encoded
    /// length` identity at each block size the sweep uses (including sizes that do not
    /// divide the row count, so the short tail block is covered).
    #[test]
    fn attribution_total_exact_at_every_swept_block_size() {
        let triples = synth_watdiv(20_000, 500_000, 0xB10C);
        for bs in [7usize, 32, 128, 512, 4096] {
            for (name, order) in PERM_ORDERS {
                let rows = perm_rows(&triples, order);
                let mut real = Vec::new();
                for chunk in rows.chunks(bs) {
                    encode_block(chunk, &mut real);
                }
                let (fb, _dir) = attribute_perm_bs(&rows, bs);
                assert_eq!(
                    fb.total(),
                    real.len() as u64,
                    "attribution total != real encoded length (bs={}, perm={})",
                    bs,
                    name
                );
            }
        }
    }

    /// [SONNET-4.6] sq-7d3dj.32.2 — the block-size trade, ASSERTED (not merely printed).
    ///
    /// Bigger blocks amortise the framing cost (one directory entry + one `count` varint +
    /// one absolute first row per block) over more rows, so store B/triple falls
    /// monotonically with `bs`. This pins the *shape* of that curve so a future encoding
    /// change cannot silently invert it, and — with
    /// [`block_size_cannot_flatten_the_scale_growth`] — bounds how much is on the table.
    ///
    /// Deliberately NOT a proposal to raise `BLOCK`: the saving is bounded (see the sibling
    /// test) and it is paid for in *latency* — a point probe decodes a whole block, so
    /// doubling `bs` doubles the per-scan decode work. That trade is the query-latency
    /// harness's to measure (`scripts/bench/compressed-query-delta.sh`, sq-7d3dj.32.2.2),
    /// not this spike's to assert.
    #[test]
    fn store_bytes_per_triple_falls_monotonically_with_block_size() {
        let triples = synth_watdiv(SWEEP_N, 500_000, 0xB10C);
        let mut prev: Option<(usize, f64, f64)> = None;
        for bs in [32usize, 64, 128, 256, 512, 1024] {
            let (total, overhead) = store_bpt_at(&triples, bs);
            if let Some((pbs, ptotal, poverhead)) = prev {
                assert!(
                    total < ptotal,
                    "store B/triple did not fall from bs={} ({:.4}) to bs={} ({:.4})",
                    pbs, ptotal, bs, total
                );
                assert!(
                    overhead < poverhead,
                    "framing overhead did not fall from bs={} ({:.4}) to bs={} ({:.4})",
                    pbs, poverhead, bs, overhead
                );
            }
            prev = Some((bs, total, overhead));
        }
    }

    /// [SONNET-4.6] sq-7d3dj.32.2 — the VERDICT on the epic's open sub-question: *"whether
    /// a block-size ... tweak flattens"* the compressed store's growth with scale (36.75
    /// B/triple at 1M → 48.75 at 10M, `research/compressed-memory-profile.md` §4).
    ///
    /// **It cannot, and this test is the proof.** Framing cost is the ONLY thing a block
    /// size can move, and driving `bs → ∞` (block count → 1) is a strict upper bound on
    /// that win. So the test holds triple count fixed, moves only the term space — the
    /// variable §4's H1/H2 are about — and asserts:
    ///
    /// 1. the growth reproduces (denser id space ⇒ more B/triple), so we are measuring the
    ///    real phenomenon and not a degenerate corpus;
    /// 2. framing cost is **scale-invariant** — it barely moves between the two regimes,
    ///    because directory bytes are a flat 16/`bs` per row and the absolute first row
    ///    widens by at most a byte or two per column;
    /// 3. the entire framing budget is **smaller than the growth it would have to absorb**,
    ///    so even deleting blocking altogether leaves the growth substantially intact.
    ///
    /// Conclusion: the growth lives in the row-delta classes (`reset_d0`/`reset_d1`
    /// absolutes widening with the term space — §4's H1), which is exactly the class the
    /// adopted `SPQCPRM2` frame-of-reference encoding (sq-7d3dj.32.2.6/.7) attacks. A
    /// block-size change is the wrong lever, and is now measured rather than assumed.
    #[test]
    fn block_size_cannot_flatten_the_scale_growth() {
        // Same triple count, ~20x term space: the id-density axis in isolation.
        let sparse = synth_watdiv(SWEEP_N, 100_000, 0xB10C);
        let dense = synth_watdiv(SWEEP_N, 2_000_000, 0xB10C);
        assert_eq!(sparse.len(), dense.len(), "regimes must be equal-sized to compare B/triple");

        let (total_sparse, framing_sparse) = store_bpt_at(&sparse, BLOCK);
        let (total_dense, framing_dense) = store_bpt_at(&dense, BLOCK);
        let growth = total_dense - total_sparse;

        // (1) The phenomenon reproduces: a denser id space costs more per triple.
        assert!(
            growth > 1.0,
            "expected the id-density growth to reproduce, got {:.3} -> {:.3} B/triple",
            total_sparse, total_dense
        );

        // (2) Framing is scale-invariant: it is not what grows.
        assert!(
            (framing_dense - framing_sparse).abs() < 0.5,
            "framing cost should be scale-invariant, moved {:.3} -> {:.3} B/triple",
            framing_sparse, framing_dense
        );

        // (3) The upper bound on any block-size win is smaller than the growth itself, so
        //     no choice of block size flattens the curve.
        assert!(
            framing_dense < growth,
            "framing budget {:.3} B/triple is the MOST a block-size change can save; it \
             must be below the {:.3} B/triple growth for the verdict to hold",
            framing_dense, growth
        );
    }

    /// [SONNET-4.6] sq-7d3dj.32.2 — the block-size sweep TABLE at scale. Prints store
    /// B/triple and its framing share across block sizes at two id-density regimes. All
    /// numbers are NON-canonical work-box measurements (never a doc/test perf number); the
    /// asserting tests above carry the verdict. Run with
    /// `cargo test -p sparq-core --lib byte_attribution::sweep -- --ignored --nocapture`.
    #[test]
    #[ignore = "spike measurement: run explicitly with --ignored --nocapture (allocates ~hundreds of MB)"]
    fn sweep_block_size_across_id_density() {
        println!("\n===== sq-7d3dj.32.2 block-size sweep (NON-canonical work-box) =====");
        for (label, n_terms) in [("1M-term", 1_000_000u64), ("5M-term", 5_000_000)] {
            let triples = synth_watdiv(5_000_000, n_terms, 0xB10C);
            println!(
                "\n--- regime {} : {} distinct triples, {} terms ---",
                label,
                triples.len(),
                n_terms
            );
            println!("{:>6} {:>12} {:>12} {:>12}", "bs", "B/triple", "framing", "deltas");
            for bs in [32usize, 64, 128, 256, 512, 1024, 4096] {
                let (total, framing) = store_bpt_at(&triples, bs);
                println!("{:>6} {:>12.3} {:>12.3} {:>12.3}", bs, total, framing, total - framing);
            }
        }
        println!("\n===== end sweep — framing is the only class block size can move =====\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic pseudo-random sorted permutation with realistic structure:
    /// clustered subjects (long equal-col0 runs), a few predicates, sparse objects.
    fn sample(n: usize) -> Vec<[Id; 3]> {
        let mut v = Vec::with_capacity(n);
        let mut state = 0x9e3779b9u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..n {
            let s = 1 + (next() % (n as u32 / 8).max(1)); // clustered subjects
            let p = 1 + (next() % 20); // few predicates
            let o = 1 + (next() % 1_000_000); // sparse objects
            v.push([s, p, o]);
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    #[test]
    fn roundtrip_decode_all() {
        for &n in &[0usize, 1, 5, 127, 128, 129, 1000, 5000] {
            let rows = sample(n);
            let c = CompressedPerm::encode(&rows);
            assert_eq!(c.len(), rows.len());
            assert_eq!(c.decode_all(), rows, "decode_all mismatch at n={n}");
        }
    }

    #[test]
    fn range_matches_binary_search() {
        let rows = sample(5000);
        let c = CompressedPerm::encode(&rows);
        // Reference: a raw binary-search range over the sorted rows.
        let raw_range = |lo: [Id; 3], hi: [Id; 3]| -> Vec<[Id; 3]> {
            let s = rows.partition_point(|r| *r < lo);
            let e = rows.partition_point(|r| *r <= hi);
            rows[s..e].to_vec()
        };
        // Full scan, a bound-subject prefix, a bound subject+predicate prefix, an empty
        // range, and the boundaries.
        let cases: &[([Id; 3], [Id; 3])] = &[
            ([Id::MIN; 3], [Id::MAX; 3]),
            ([100, Id::MIN, Id::MIN], [100, Id::MAX, Id::MAX]),
            ([200, 5, Id::MIN], [200, 5, Id::MAX]),
            ([99999999, Id::MIN, Id::MIN], [99999999, Id::MAX, Id::MAX]),
            (rows[0], rows[0]),
            (rows[rows.len() / 2], rows[rows.len() / 2]),
            (rows[rows.len() - 1], rows[rows.len() - 1]),
        ];
        for &(lo, hi) in cases {
            assert_eq!(c.range(lo, hi), raw_range(lo, hi), "range mismatch for {lo:?}..={hi:?}");
        }
    }

    /// [OPUS-4.8] sq-bif — `count_range` is the planner's cheap cardinality estimate and must
    /// return EXACTLY `range(lo, hi).len()` without materialising the range. It only decodes
    /// the two boundary blocks and adds `BLOCK` for each full interior block
    /// (`first_count + (last - first - 1) * BLOCK + last_count`), so an off-by-one in that
    /// arithmetic — or a wrong interior-block count — yields a count that disagrees with the
    /// materialised range. This was previously only reached transitively through the planner;
    /// here we pin the invariant directly, across single-block, two-block and many-interior-
    /// block ranges, the empty / inverted range, and the exact boundary keys.
    #[test]
    fn count_range_equals_materialised_range_len() {
        let rows = sample(5000);
        let c = CompressedPerm::encode(&rows);
        assert!(rows.len() > 3 * BLOCK, "need several blocks to exercise the interior arithmetic");

        // Reference count: the length of the materialised range (itself proven correct above).
        let want = |lo: [Id; 3], hi: [Id; 3]| -> usize { c.range(lo, hi).len() };

        let cases: &[([Id; 3], [Id; 3])] = &[
            // Whole permutation: spans every block (first + many interior + last).
            ([Id::MIN; 3], [Id::MAX; 3]),
            // A bound-subject prefix and a bound subject+predicate prefix.
            ([100, Id::MIN, Id::MIN], [100, Id::MAX, Id::MAX]),
            ([200, 5, Id::MIN], [200, 5, Id::MAX]),
            // A subject that does not exist → empty count.
            ([99999999, Id::MIN, Id::MIN], [99999999, Id::MAX, Id::MAX]),
            // Exact single-row ranges at the start, middle, and end.
            (rows[0], rows[0]),
            (rows[rows.len() / 2], rows[rows.len() / 2]),
            (rows[rows.len() - 1], rows[rows.len() - 1]),
            // A wide interior span that crosses many full blocks (lo/hi land partway into
            // their boundary blocks, so first_count and last_count are both partial).
            (rows[BLOCK / 3], rows[rows.len() - BLOCK / 3]),
            // Two adjacent rows that straddle a block boundary.
            (rows[BLOCK - 1], rows[BLOCK]),
        ];
        for &(lo, hi) in cases {
            assert_eq!(c.count_range(lo, hi), want(lo, hi), "count_range != range().len() for {lo:?}..={hi:?}");
        }

        // An inverted range (lo > hi) and an empty permutation both count 0.
        assert_eq!(c.count_range([10, 0, 0], [1, 0, 0]), 0, "inverted range counts 0");
        let empty = CompressedPerm::encode(&[]);
        assert_eq!(empty.count_range([Id::MIN; 3], [Id::MAX; 3]), 0, "empty perm counts 0");

        // Every single-row range over the whole permutation counts exactly 1 (each row is
        // present once), which independently pins the boundary-block partition_point logic.
        for &r in &rows {
            assert_eq!(c.count_range(r, r), 1, "each present row counts exactly once: {r:?}");
        }
    }

    /// [OPUS-4.8] sq-vkz7 — for a NON-EMPTY perm the STREAMING [`CompressedPermWriter`] must
    /// produce a file BYTE-IDENTICAL to the in-RAM `encode(rows).write_to(..)`. Covers block
    /// boundaries (127/128/129), a partial last block, and a large run. The EMPTY case must
    /// instead produce a zero-byte file (the `save_compressed`/`recompress` empty-perm rule).
    #[cfg(feature = "mmap")]
    #[test]
    fn stream_writer_byte_identical_to_encode_write_to() {
        let tmp = std::env::temp_dir().join(format!("sq_vkz7_stream_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        for &n in &[0usize, 1, 5, 127, 128, 129, 256, 1000, 5000] {
            let rows = sample(n);

            // Streaming writer to a file.
            let out = tmp.join(format!("perm_{n}.bin"));
            let mut w = CompressedPermWriter::create(&out).unwrap();
            for &r in &rows {
                w.push(r).unwrap();
            }
            w.finish(&out).unwrap();
            let got = std::fs::read(&out).unwrap();

            // The block side file must be cleaned up by finish() in every case.
            let mut side = out.as_os_str().to_owned();
            side.push(".blocks");
            assert!(!std::path::Path::new(&side).exists(), "block side file leaked at n={n}");

            if rows.is_empty() {
                assert!(got.is_empty(), "empty perm must be a zero-byte file, got {} bytes", got.len());
                continue;
            }

            // Reference: in-RAM encode then write_to a byte buffer — must match exactly.
            let mut want = Vec::new();
            CompressedPerm::encode(&rows).write_to(&mut want).unwrap();
            assert_eq!(got, want, "stream vs encode+write_to byte mismatch at n={n}");

            // And it must round-trip back through from_mmap to the same rows.
            let f = std::fs::File::open(&out).unwrap();
            // SAFETY: we own and just wrote this file; nothing else mutates it during the test.
            let map = unsafe { memmap2::Mmap::map(&f) }.unwrap();
            let perm = CompressedPerm::from_mmap(map).unwrap();
            assert_eq!(perm.len(), rows.len(), "len mismatch at n={n}");
            assert_eq!(perm.decode_all(), rows, "decoded rows mismatch at n={n}");
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// [OPUS-4.8] sq-wihld — a HIGH-NDV leading column: many distinct subjects, several with
    /// runs long enough to straddle block boundaries (so a different bound subject's lookup
    /// lands in a block that does not contain it — the exact overlapping-block case a per-block
    /// Bloom filter prunes but the min/max zone map cannot). Sorted + deduplicated, like a real
    /// permutation column.
    #[cfg(feature = "block-bloom")]
    fn high_ndv_sample(n: usize) -> Vec<[Id; 3]> {
        let mut v = Vec::with_capacity(n);
        let mut state = 0x1234_5678u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..n {
            // Wide subject domain (high NDV) but a fraction of subjects repeat a few times,
            // producing runs that cross BLOCK boundaries → overlapping per-block [min,max].
            let s = 1 + (next() % (n as u32 / 2).max(1));
            let reps = 1 + (next() % 4); // 1..=4 rows per (s) on average
            for _ in 0..reps {
                let p = 1 + (next() % 8);
                let o = 1 + (next() % 5_000_000);
                v.push([s, p, o]);
            }
        }
        v.sort_unstable();
        v.dedup();
        v.truncate(n.max(1));
        v
    }

    /// [OPUS-4.8] sq-wihld (survey §A1) — LOAD-BEARING equivalence: with the `block-bloom`
    /// feature ON, `range` over an equality-bound LEADING column must return EXACTLY the rows
    /// a raw binary search returns — the Bloom skip never drops a matching row (zero false
    /// negatives by construction). This is the feature-on-vs-off result-equivalence contract:
    /// the raw binary search is the feature-OFF oracle (it has no Bloom path), so equality here
    /// proves the optimisation is correctness-neutral. We also confirm the density gate built a
    /// filter for this high-NDV column AND that the filter skips at least one candidate block
    /// across the keys — otherwise the test would pass trivially without exercising the skip.
    #[cfg(feature = "block-bloom")]
    #[test]
    fn bloom_range_equals_binary_search_and_skips_blocks() {
        let rows = high_ndv_sample(8000);
        assert!(rows.len() > 4 * BLOCK, "need many blocks to exercise overlap + skipping");
        let c = CompressedPerm::encode(&rows);

        // The density gate must admit this high-NDV leading column, or the skip path is never
        // reached and the test is vacuous.
        assert!(c.has_bloom(), "high-NDV leading column should get a Bloom directory");

        // Feature-OFF oracle: a raw binary-search range over the sorted rows (no Bloom).
        let raw_range = |lo: [Id; 3], hi: [Id; 3]| -> Vec<[Id; 3]> {
            let s = rows.partition_point(|r| *r < lo);
            let e = rows.partition_point(|r| *r <= hi);
            rows[s..e].to_vec()
        };

        // Every DISTINCT leading id present, plus a swathe of ABSENT ids (the false-positive /
        // definite-absent path). For each, the Bloom-enabled `range` must match the oracle.
        let mut present: Vec<Id> = rows.iter().map(|r| r[0]).collect();
        present.dedup();
        let max_present = *present.last().unwrap();

        let mut total_candidates = 0usize;
        let mut total_skipped = 0usize;
        for &k in &present {
            let lo = [k, Id::MIN, Id::MIN];
            let hi = [k, Id::MAX, Id::MAX];
            assert_eq!(c.range(lo, hi), raw_range(lo, hi), "present key {k} range mismatch");
            let (cand, skip) = c.bloom_skip_stats(k);
            total_candidates += cand;
            total_skipped += skip;
            // A present key must NEVER be skipped in the block(s) that hold it: the materialised
            // range is non-empty, so at least one candidate block survived the filter.
            assert!(cand > skip, "present key {k} had all candidate blocks skipped");
        }
        // Absent keys interleaved through and beyond the domain: results are empty, and these
        // are where the Bloom earns its keep (definite-absent → skip without decode).
        for k in (1..=max_present + 16).step_by(7) {
            let lo = [k, Id::MIN, Id::MIN];
            let hi = [k, Id::MAX, Id::MAX];
            assert_eq!(c.range(lo, hi), raw_range(lo, hi), "absent/odd key {k} range mismatch");
        }

        // The optimisation must do REAL work on this high-NDV column: across the present keys
        // at least one candidate block was Bloom-skipped (proves it is engaged, not a no-op).
        // This is a structural assertion, not a performance number.
        assert!(
            total_skipped > 0,
            "Bloom skipped no blocks ({total_skipped}/{total_candidates}) — optimisation never engaged"
        );
    }

    /// [OPUS-4.8] sq-wihld — the Bloom skip must be inert for NON-equality-bound shapes (a full
    /// scan and a multi-key range), and for ALL shapes it must agree with the raw binary search.
    /// Re-runs the existing `range` oracle cases under the feature so the feature-on path is
    /// proven byte-for-byte equivalent to the raw permutation across the same query shapes the
    /// plan-agreement tests rely on.
    #[cfg(feature = "block-bloom")]
    #[test]
    fn bloom_inert_for_non_equality_shapes() {
        let rows = high_ndv_sample(6000);
        let c = CompressedPerm::encode(&rows);
        let raw_range = |lo: [Id; 3], hi: [Id; 3]| -> Vec<[Id; 3]> {
            if lo > hi {
                return Vec::new(); // matches `range`'s inverted-range early-out
            }
            let s = rows.partition_point(|r| *r < lo);
            let e = rows.partition_point(|r| *r <= hi);
            rows[s..e].to_vec()
        };
        let mid = rows[rows.len() / 2][0];
        let cases: &[([Id; 3], [Id; 3])] = &[
            // Full scan (leading column NOT equality-bound).
            ([Id::MIN; 3], [Id::MAX; 3]),
            // A multi-key leading RANGE (lo[0] != hi[0]) — the Bloom path is bypassed.
            ([mid, Id::MIN, Id::MIN], [mid + 50, Id::MAX, Id::MAX]),
            // An empty / inverted range.
            ([Id::MAX, 0, 0], [Id::MIN, 0, 0]),
        ];
        for &(lo, hi) in cases {
            assert_eq!(c.range(lo, hi), raw_range(lo, hi), "non-equality shape {lo:?}..={hi:?}");
        }
    }

    /// [OPUS-4.8] sq-wihld — `decode_all` (whole-permutation iteration) is unaffected by the
    /// Bloom directory: it never consults the filter, so it must reproduce the rows exactly,
    /// feature on or off.
    #[cfg(feature = "block-bloom")]
    #[test]
    fn bloom_decode_all_roundtrips() {
        for &n in &[0usize, 1, 200, 2000, 8000] {
            let rows = high_ndv_sample(n);
            let c = CompressedPerm::encode(&rows);
            assert_eq!(c.decode_all(), rows, "decode_all mismatch at n={n}");
            assert_eq!(c.len(), rows.len(), "len mismatch at n={n}");
        }
    }

    // ===== [SONNET-4.6] sq-v8ixk — the `block-bloom` MEASUREMENT harness =====
    //
    // sq-wihld (PR #1252) landed the opt-in filter asserting NO performance number, because the
    // work box is not the canonical perf host. This harness is the instrument sq-0g6g runs, and
    // it also settles the HOST-INDEPENDENT half of the question anywhere it runs: the block-skip
    // rate, the false-positive rate and the filter's bytes are pure functions of the column's
    // distribution and the (BITS, HASHES) sizing — identical on any machine, for a given input.
    // Only the end-to-end latency table is host-sensitive, and it prints its own NON-canonical
    // label. No number produced here is asserted in any doc.
    //
    // The harness deliberately reports PRESENT-key and ABSENT-key point lookups separately: a
    // sorted column puts all rows for one leading id contiguously, so the zone map's candidate
    // window for a PRESENT id is (the blocks holding it) plus at most ONE leading block, whereas
    // for an ABSENT id the window is a single block the filter can drop entirely. Those two
    // shapes have completely different skip ceilings and averaging them would hide the answer.

    /// [SONNET-4.6] sq-v8ixk — a WatDiv-shaped skewed permutation column: `n_triples` triples
    /// over `n_terms` terms with a small predicate vocabulary and Zipf-ish subjects/objects
    /// (frequent terms take small ids), sorted + deduplicated into `order` — i.e. exactly the
    /// rows `from_triples_compressed` hands [`CompressedPerm::encode`] for that permutation.
    /// Deterministic (xorshift, fixed seed) so every host measures the identical column.
    ///
    /// `scatter` is the CONTROL for the sizing sweep's hash-quality question: with it off, term
    /// ids are DENSE (`1..=n_terms`) exactly as the dictionary assigns them; with it on, each id
    /// is multiplied by a large odd constant so the same number of distinct ids is spread across
    /// the whole 32-bit space. NDV, block count and clustering are unchanged — only the ids' low
    /// bits, which is what an FNV-1a-derived probe index reads. If a (bits, hashes) choice wins
    /// on dense ids but loses on scattered ones, its advantage was a hash artefact, not Bloom
    /// math, and must not be baked into the shipped constants.
    ///
    /// HONEST SCOPE: this is a synthetic stand-in with real-RDF *shape* (skew, small predicate
    /// vocabulary, high-NDV subject/object columns), NOT a Wikidata column. Substituting a real
    /// Wikidata permutation is the remaining gap on the canonical run.
    #[cfg(feature = "block-bloom")]
    fn bloom_perm_column(
        n_triples: usize,
        n_terms: u32,
        order: [usize; 3],
        seed: u64,
        scatter: bool,
    ) -> Vec<[Id; 3]> {
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        // Zipf-ish: squaring a uniform in [0,1) concentrates mass on small ids.
        let mut skewed = move |n: u32| -> Id {
            let u = (next() >> 11) as f64 / (1u64 << 53) as f64;
            1 + ((u * u) * n.saturating_sub(1) as f64) as Id
        };
        // A bijection on `Id` (odd multiplier ⇒ invertible mod 2^32), so scattering changes
        // WHICH ids are used, never how many distinct ones there are.
        let map = |id: Id| if scatter { id.wrapping_mul(2_654_435_761) } else { id };
        let mut v = Vec::with_capacity(n_triples);
        for _ in 0..n_triples {
            let s = map(skewed(n_terms));
            let p = 1 + (skewed(n_terms) % 85);
            let o = map(skewed(n_terms));
            let t = [s, p, o];
            v.push([t[order[0]], t[order[1]], t[order[2]]]);
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// [SONNET-4.6] sq-v8ixk — a PARAMETRIC re-implementation of the shipped per-block filter,
    /// so the sizing sweep can vary bits/hashes without touching the shipped constants. At
    /// `(BITS, HASHES)` it must answer IDENTICALLY to the real [`block_bloom::BlockBloomDir`] —
    /// pinned by `bloom_model_matches_shipped_filter`, without which the sweep would be fiction.
    #[cfg(feature = "block-bloom")]
    struct ModelBloom {
        bits: usize,
        hashes: u32,
        words_per_block: usize,
        words: Vec<u64>,
    }

    #[cfg(feature = "block-bloom")]
    impl ModelBloom {
        /// FNV-1a over the little-endian id bytes — the same seed hash the shipped filter uses.
        fn hash_id(id: Id) -> u64 {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in id.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            h
        }

        /// `bits` must be a multiple of 64 (whole u64 words per block), as the shipped layout is.
        fn build(blocks: &[&[[Id; 3]]], bits: usize, hashes: u32) -> Self {
            assert_eq!(bits % 64, 0, "model filter needs whole u64 words per block");
            let words_per_block = bits / 64;
            let mut words = vec![0u64; blocks.len() * words_per_block];
            for (b, chunk) in blocks.iter().enumerate() {
                let mut prev: Option<Id> = None;
                for r in chunk.iter() {
                    if prev != Some(r[0]) {
                        let h = Self::hash_id(r[0]);
                        let (h1, h2) = (h as usize, ((h >> 32) | 1) as usize);
                        let base = b * words_per_block;
                        for k in 0..hashes as usize {
                            let bit = (h1.wrapping_add(k.wrapping_mul(h2))) % bits;
                            words[base + bit / 64] |= 1u64 << (bit % 64);
                        }
                        prev = Some(r[0]);
                    }
                }
            }
            ModelBloom { bits, hashes, words_per_block, words }
        }

        fn maybe_contains(&self, b: usize, id: Id) -> bool {
            let h = Self::hash_id(id);
            let (h1, h2) = (h as usize, ((h >> 32) | 1) as usize);
            let base = b * self.words_per_block;
            for k in 0..self.hashes as usize {
                let bit = (h1.wrapping_add(k.wrapping_mul(h2))) % self.bits;
                if self.words[base + bit / 64] & (1u64 << (bit % 64)) == 0 {
                    return false;
                }
            }
            true
        }

        fn heap_bytes(&self) -> usize {
            self.words.len() * std::mem::size_of::<u64>()
        }
    }

    /// [SONNET-4.6] sq-v8ixk — the two probe sets a point-lookup measurement needs, capped at
    /// `cap` each: `(present, absent)` leading ids. PRESENT ids are sampled evenly across the
    /// column (a bound subject/object that exists); ABSENT ids are the immediate neighbours of
    /// present ids that are themselves not present (a bound id filtered out by an earlier
    /// pattern, or simply not in this permutation) — deriving them from the present ids rather
    /// than sweeping the id space keeps this O(n) whether the ids are dense or scattered.
    #[cfg(feature = "block-bloom")]
    fn bloom_probe_sets(rows: &[[Id; 3]], cap: usize) -> (Vec<Id>, Vec<Id>) {
        let mut present: Vec<Id> = rows.iter().map(|r| r[0]).collect();
        present.dedup();
        let present_set: std::collections::HashSet<Id> = present.iter().copied().collect();
        let sample: Vec<Id> = present.iter().copied().step_by(present.len() / cap + 1).collect();
        let absent: Vec<Id> = present
            .iter()
            .flat_map(|&k| [k.wrapping_add(1), k.wrapping_sub(1)])
            .filter(|k| *k != 0 && !present_set.contains(k))
            .take(cap)
            .collect();
        (sample, absent)
    }

    /// [SONNET-4.6] sq-v8ixk — the zone map's candidate window for an equality-bound leading id:
    /// `(first, last)` block indices the `range` loop would visit, derived from the per-block
    /// first-keys exactly as [`CompressedPerm::range`] derives them from `dir`.
    #[cfg(feature = "block-bloom")]
    fn bloom_candidate_window(firsts: &[[Id; 3]], key: Id) -> (usize, usize) {
        let lo = [key, Id::MIN, Id::MIN];
        let hi = [key, Id::MAX, Id::MAX];
        let first = firsts.partition_point(|k| *k <= lo).saturating_sub(1);
        let last = firsts.partition_point(|k| *k <= hi).saturating_sub(1).max(first);
        (first, last)
    }

    /// [SONNET-4.6] sq-v8ixk — `range` with the Bloom probe REMOVED: the feature-OFF loop,
    /// verbatim. The latency table's baseline arm, and (asserted at every probe) output-identical
    /// to `range`, so the two arms are timed on genuinely equal work.
    #[cfg(feature = "block-bloom")]
    fn range_without_bloom(c: &CompressedPerm, lo: [Id; 3], hi: [Id; 3]) -> Vec<[Id; 3]> {
        if c.dir.is_empty() || lo > hi {
            return Vec::new();
        }
        let first = c.dir.partition_point(|&(k, _)| k <= lo).saturating_sub(1);
        let last = c.dir.partition_point(|&(k, _)| k <= hi).saturating_sub(1).max(first);
        let mut decoded = Vec::with_capacity((last - first + 1) * BLOCK);
        for b in first..=last {
            c.decode_block_at(c.dir[b].1 as usize, &mut decoded);
        }
        let s = decoded.partition_point(|r| *r < lo);
        let e = decoded.partition_point(|r| *r <= hi);
        decoded.drain(..s);
        decoded.truncate(e - s);
        decoded
    }

    /// [SONNET-4.6] sq-v8ixk — SELF-CHECK (runs in the normal suite, not `#[ignore]`d): the
    /// parametric [`ModelBloom`] at the SHIPPED `(BITS, HASHES)` must answer identically to the
    /// real filter for every (block, probe) pair — present ids and absent ids alike. Without
    /// this the sizing sweep would be measuring a model that does not describe what ships.
    #[cfg(feature = "block-bloom")]
    #[test]
    fn bloom_model_matches_shipped_filter() {
        let rows = bloom_perm_column(20_000, 40_000, [0, 1, 2], 0x5EED, false);
        let chunks: Vec<&[[Id; 3]]> = rows.chunks(BLOCK).collect();
        assert!(chunks.len() > 4, "need several blocks to compare per-block filters");
        let shipped = block_bloom::BlockBloomDir::build(&chunks)
            .expect("high-NDV column should pass the density gate");
        let model = ModelBloom::build(&chunks, block_bloom::BITS, block_bloom::HASHES);
        assert_eq!(shipped.heap_bytes(), model.heap_bytes(), "model/shipped filter size differ");

        let max_id = rows.last().unwrap()[0];
        for b in 0..chunks.len() {
            for key in 1..=max_id + 8 {
                assert_eq!(
                    shipped.maybe_contains(b, key),
                    model.maybe_contains(b, key),
                    "model disagrees with shipped filter at block {} key {}",
                    b,
                    key
                );
            }
        }
    }

    /// [SONNET-4.6] sq-v8ixk — SELF-CHECK: the latency baseline [`range_without_bloom`] is the
    /// feature-OFF oracle, so it must agree with `range` on every shape the harness times.
    /// Pins the baseline arm honest independently of whether the measurement is ever run.
    #[cfg(feature = "block-bloom")]
    #[test]
    fn bloom_latency_baseline_matches_range() {
        let rows = bloom_perm_column(20_000, 40_000, [0, 1, 2], 0xBEEF, false);
        let c = CompressedPerm::encode(&rows);
        let max_id = rows.last().unwrap()[0];
        for key in (1..=max_id + 8).step_by(3) {
            let lo = [key, Id::MIN, Id::MIN];
            let hi = [key, Id::MAX, Id::MAX];
            assert_eq!(c.range(lo, hi), range_without_bloom(&c, lo, hi), "baseline differs at {}", key);
        }
    }

    /// [SONNET-4.6] sq-v8ixk — MEASUREMENT (i), (iii), (iv): per-column block-skip rate for
    /// present- and absent-key equality-bound point lookups, the filter's false-positive rate
    /// against its 256-bit/2-hash sizing, the density gate's admit/decline decision, and the
    /// filter's build time + resident bytes against the directory it rides beside. Closes with
    /// a (bits × hashes) sizing sweep so any retune of `BITS`/`HASHES` is evidence-led.
    ///
    /// Everything except the build-time column is host-INDEPENDENT (a pure function of the
    /// column and the sizing). Run with
    /// `cargo test -p sparq-core --features block-bloom --lib measure_bloom -- --ignored --nocapture`.
    #[cfg(feature = "block-bloom")]
    #[test]
    #[ignore = "spike measurement: run explicitly with --ignored --nocapture"]
    fn measure_bloom_skip_and_sizing() {
        const N: usize = 2_000_000;
        const TERMS: u32 = 1_000_000;
        // SPO/OSP are the high-NDV subject/object-leading columns the feature targets; POS is
        // the low-NDV predicate-leading column the density gate is supposed to DECLINE.
        // `SPO/scattered` is the hash-quality control (see `bloom_perm_column`): same NDV and
        // block count as `SPO`, different low id bits.
        let columns: [(&str, [usize; 3], bool); 4] = [
            ("SPO", [0, 1, 2], false),
            ("OSP", [2, 0, 1], false),
            ("POS", [1, 2, 0], false),
            ("SPO/scattered-ids", [0, 1, 2], true),
        ];

        println!("\n===== sq-v8ixk block-bloom skip/FP/sizing (build time NON-canonical work-box) =====");
        println!(
            "  shipped sizing: BITS={} HASHES={} density gate MIN_AVG_DISTINCT_PER_BLOCK={} (BLOCK={})",
            block_bloom::BITS,
            block_bloom::HASHES,
            block_bloom::MIN_AVG_DISTINCT_PER_BLOCK,
            BLOCK
        );

        for (name, order, scatter) in columns {
            let rows = bloom_perm_column(N, TERMS, order, 0x5EED, scatter);
            let chunks: Vec<&[[Id; 3]]> = rows.chunks(BLOCK).collect();
            let firsts: Vec<[Id; 3]> = chunks.iter().map(|c| c[0]).collect();

            // Density: distinct leading ids per block (what the gate thresholds on).
            let distinct: usize = chunks
                .iter()
                .map(|c| {
                    let mut prev = None;
                    c.iter().filter(|r| { let n = prev != Some(r[0]); prev = Some(r[0]); n }).count()
                })
                .sum();
            let avg_distinct = distinct as f64 / chunks.len() as f64;

            let t = std::time::Instant::now();
            let built = block_bloom::BlockBloomDir::build(&chunks);
            let build_time = t.elapsed();

            println!(
                "\n--- column {} : {} rows, {} blocks, avg distinct leading ids/block {:.1} ---",
                name,
                rows.len(),
                chunks.len(),
                avg_distinct
            );
            println!(
                "  density gate: {} (threshold {:.1})",
                if built.is_some() { "ADMIT" } else { "DECLINE" },
                block_bloom::MIN_AVG_DISTINCT_PER_BLOCK
            );
            let Some(shipped) = built else {
                println!("  (no filter built — nothing further to measure on this column)");
                continue;
            };
            let dir_bytes = chunks.len() * 16;
            println!(
                "  build time {:?} (work box) | filter {} B = {:.3} B/triple | directory {} B = {:.3} B/triple",
                build_time,
                shipped.heap_bytes(),
                shipped.heap_bytes() as f64 / rows.len() as f64,
                dir_bytes,
                dir_bytes as f64 / rows.len() as f64
            );

            // Probe sets: PRESENT leading ids (a bound subject/object that exists) and ABSENT
            // ones inside the same id domain (a bound id filtered out by an earlier pattern, or
            // simply not in this permutation) — the two shapes have different skip ceilings.
            let (sample, absent) = bloom_probe_sets(&rows, 20_000);

            for (label, probes) in [("present", &sample), ("absent", &absent)] {
                let (mut cand, mut skipped, mut fp, mut truly_absent_blocks) = (0usize, 0usize, 0usize, 0usize);
                for &key in probes.iter() {
                    let (first, last) = bloom_candidate_window(&firsts, key);
                    for (off, chunk) in chunks[first..=last].iter().enumerate() {
                        let b = first + off;
                        cand += 1;
                        let holds = chunk.binary_search_by(|r| r[0].cmp(&key)).is_ok();
                        let maybe = shipped.maybe_contains(b, key);
                        assert!(holds <= maybe, "FALSE NEGATIVE at block {} key {}", b, key);
                        if !maybe {
                            skipped += 1;
                        }
                        if !holds {
                            truly_absent_blocks += 1;
                            if maybe {
                                fp += 1;
                            }
                        }
                    }
                }
                let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
                println!(
                    "  {:<8} probes={:<7} candidate blocks/probe={:.2}  SKIP RATE={:.2}% ({}/{})  \
                     FP rate={:.2}% ({}/{} skippable)",
                    label,
                    probes.len(),
                    cand as f64 / probes.len().max(1) as f64,
                    pct(skipped, cand),
                    skipped,
                    cand,
                    pct(fp, truly_absent_blocks),
                    fp,
                    truly_absent_blocks
                );
            }

            // (iii) SIZING SWEEP — false-positive rate and bytes at other (bits, hashes). The
            // theoretical FP for n distinct keys in m bits with k probes is (1-e^(-kn/m))^k;
            // measured is what this column actually does.
            println!("  sizing sweep (measured FP on absent-key candidate blocks):");
            println!(
                "  {:>6} {:>7} {:>12} {:>12} {:>14}",
                "bits", "hashes", "B/triple", "FP measured", "FP theoretical"
            );
            for bits in [128usize, 256, 512, 1024] {
                for hashes in [1u32, 2, 3, 4] {
                    let m = ModelBloom::build(&chunks, bits, hashes);
                    let (mut fp, mut n_absent) = (0usize, 0usize);
                    for &key in &absent {
                        let (first, last) = bloom_candidate_window(&firsts, key);
                        for (off, chunk) in chunks[first..=last].iter().enumerate() {
                            let b = first + off;
                            if chunk.binary_search_by(|r| r[0].cmp(&key)).is_ok() {
                                continue;
                            }
                            n_absent += 1;
                            if m.maybe_contains(b, key) {
                                fp += 1;
                            }
                        }
                    }
                    let theory = (1.0 - (-(hashes as f64) * avg_distinct / bits as f64).exp())
                        .powi(hashes as i32);
                    println!(
                        "  {:>6} {:>7} {:>12.3} {:>11.2}% {:>13.2}%",
                        bits,
                        hashes,
                        m.heap_bytes() as f64 / rows.len() as f64,
                        if n_absent == 0 { 0.0 } else { 100.0 * fp as f64 / n_absent as f64 },
                        100.0 * theory
                    );
                }
            }
        }
        println!("\n===== end sq-v8ixk skip/FP/sizing — verdict adjudicated in the bead note =====\n");
    }

    /// [SONNET-4.6] sq-v8ixk — MEASUREMENT (ii): end-to-end `range` latency for selective
    /// equality-bound point lookups with the Bloom probe ON versus the feature-OFF loop
    /// ([`range_without_bloom`]), split by present- vs absent-key probes, plus the blocks each
    /// arm actually decodes. NATIVE-ONLY ([`std::time::Instant`] is absent on `wasm32`).
    ///
    /// PROTOCOL: a discarded warmup of both arms, then each arm timed twice in opposite orders
    /// (ON,OFF then OFF,ON) with `black_box` on both the probe key and the returned rows and a
    /// per-arm checksum asserted equal, so the timed work cannot be optimized away and the
    /// first-timed arm carries no systematic penalty.
    ///
    /// EVERY latency figure this prints is a NON-canonical work-box number on a SYNTHETIC column
    /// and must not be asserted anywhere; the canonical figures come from re-running this on the
    /// perf host against the intended real dataset (sq-0g6g).
    #[cfg(all(feature = "block-bloom", not(target_arch = "wasm32")))]
    #[test]
    #[ignore = "spike measurement: run explicitly with --ignored --nocapture"]
    fn measure_bloom_selective_scan_latency() {
        use std::time::Instant;
        const N: usize = 2_000_000;
        const TERMS: u32 = 1_000_000;
        let columns: [(&str, [usize; 3], bool); 3] =
            [("SPO", [0, 1, 2], false), ("OSP", [2, 0, 1], false), ("SPO/scattered-ids", [0, 1, 2], true)];

        println!("\n===== sq-v8ixk block-bloom selective-scan latency (NON-canonical work-box) =====");
        for (name, order, scatter) in columns {
            let rows = bloom_perm_column(N, TERMS, order, 0x5EED, scatter);
            let c = CompressedPerm::encode(&rows);
            if !c.has_bloom() {
                println!("--- column {}: density gate declined; no ON arm to time ---", name);
                continue;
            }
            let (hit, miss) = bloom_probe_sets(&rows, 50_000);

            println!("--- column {} : {} rows, {} blocks ---", name, rows.len(), c.dir.len());
            for (label, probes) in [("present", &hit), ("absent", &miss)] {
                // Equivalence first: the two arms must return the same rows, so the timing
                // below compares equal work rather than a shortcut.
                let (mut blocks_on, mut blocks_off) = (0usize, 0usize);
                for &key in probes.iter() {
                    let (lo, hi) = ([key, Id::MIN, Id::MIN], [key, Id::MAX, Id::MAX]);
                    assert_eq!(c.range(lo, hi), range_without_bloom(&c, lo, hi), "arm mismatch at {}", key);
                    let (cand, skip) = c.bloom_skip_stats(key);
                    blocks_off += cand;
                    blocks_on += cand - skip;
                }

                // Each arm keeps its OWN observable checksum, and both the probe key going in
                // and the row vector coming out pass through `black_box`, so an optimized build
                // cannot fold the key, elide the decode or drop the returned rows: the timed work
                // is the work the arm claims to do. Timing a whole probe sweep per call.
                let run_on = || {
                    let mut sum = 0usize;
                    let t = Instant::now();
                    for &key in probes.iter() {
                        let key = std::hint::black_box(key);
                        let got = c.range([key, Id::MIN, Id::MIN], [key, Id::MAX, Id::MAX]);
                        sum += std::hint::black_box(got).len();
                    }
                    (t.elapsed(), sum)
                };
                let run_off = || {
                    let mut sum = 0usize;
                    let t = Instant::now();
                    for &key in probes.iter() {
                        let key = std::hint::black_box(key);
                        let got = range_without_bloom(&c, [key, Id::MIN, Id::MIN], [key, Id::MAX, Id::MAX]);
                        sum += std::hint::black_box(got).len();
                    }
                    (t.elapsed(), sum)
                };

                // WARMUP (discarded): pays the cold-cache / first-touch cost once for BOTH arms,
                // so it is not charged to whichever arm happens to be timed first.
                let _ = std::hint::black_box(run_on());
                let _ = std::hint::black_box(run_off());

                // Then time each arm twice in OPPOSITE orders (ON,OFF then OFF,ON) and sum, so
                // any residual ordering bias falls on both arms equally instead of on the first.
                let (on1, sum_on1) = run_on();
                let (off1, sum_off1) = run_off();
                let (off2, sum_off2) = run_off();
                let (on2, sum_on2) = run_on();
                let (on, off) = (on1 + on2, off1 + off2);

                // Non-vacuity + equal-work: both arms must have had real candidate blocks, must
                // agree on the rows returned (a stronger, per-arm form of the equivalence assert
                // above — it is what keeps the checksums observable), and must be stable across
                // rounds. Rows returned are only expected to be POSITIVE on the PRESENT arm — an
                // absent-key lookup correctly returns nothing, which is that arm's entire point.
                assert!(blocks_off > 0, "{} arm visited no candidate blocks — vacuous", label);
                assert_eq!(sum_on1, sum_off1, "{} arms disagree on rows returned", label);
                assert_eq!((sum_on1, sum_off1), (sum_on2, sum_off2), "{} arms unstable", label);
                assert!(label != "present" || sum_on1 > 0, "present probes returned no rows — vacuous");

                let rounds = 2;
                let per =
                    |d: std::time::Duration| d.as_nanos() as f64 / (probes.len().max(1) * rounds) as f64;
                println!(
                    "  {:<8} probes={:<7} blocks decoded ON={} OFF={}  ns/lookup ON={:.0} OFF={:.0}  delta={:+.1}%",
                    label,
                    probes.len(),
                    blocks_on,
                    blocks_off,
                    per(on),
                    per(off),
                    100.0 * (per(on) - per(off)) / per(off)
                );
            }
        }
        println!("\n===== end sq-v8ixk latency — NON-canonical; canonical run belongs on sq-0g6g =====\n");
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — builds a `V2` [`CompressedPerm`] from `rows` using ONLY the
    /// `mmap`-gated primitives (`encode_block_v2`), so the V2 READER path is exercised even in the
    /// coverage build (which measures sparq-core under `mmap,dict-spill`, NOT `spqcprm2`). Mirrors
    /// `encode_v2` but is available without the emit feature.
    #[cfg(feature = "mmap")]
    fn build_v2_perm(rows: &[[Id; 3]]) -> CompressedPerm {
        let mut dir = Vec::with_capacity(rows.len() / BLOCK + 1);
        let mut blocks = Vec::with_capacity(rows.len() * 6);
        for chunk in rows.chunks(BLOCK) {
            dir.push((chunk[0], blocks.len() as u32));
            encode_block_v2(chunk, &mut blocks);
        }
        #[cfg(feature = "block-bloom")]
        let bloom = block_bloom::BlockBloomDir::build(&rows.chunks(BLOCK).collect::<Vec<_>>());
        CompressedPerm {
            dir,
            blocks: Blocks::Owned(blocks),
            len: rows.len(),
            #[cfg(feature = "block-bloom")]
            bloom,
            format: Format::V2,
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — the V2 READER ships with `mmap` (independent of the `spqcprm2`
    /// emit feature), so this test — gated on `mmap` ALONE — drives the frame-of-reference reader,
    /// the `SPQCPRM2` file magic (`file_magic` V2 arm), and the `from_mmap` auto-detect end-to-end.
    /// It guarantees those lines are COVERED in the per-crate coverage run (measured under
    /// `mmap,dict-spill`, not `spqcprm2`), so the migration cannot drag sparq-core below its floor.
    #[cfg(feature = "mmap")]
    #[test]
    fn v2_reader_ships_with_mmap_roundtrips() {
        for &n in &[1usize, 128, 129, 400, 3000] {
            let rows = sample(n);
            let c = build_v2_perm(&rows);
            assert_eq!(c.format, Format::V2);
            // In-RAM V2 decode (exercises decode_block_v2_at + get_zigzag_varint).
            assert_eq!(c.decode_all(), rows, "V2 in-RAM decode diverged at n={n}");
            // write_to picks the SPQCPRM2 magic (file_magic V2 arm) then from_mmap auto-detects it.
            let mut bytes = Vec::new();
            c.write_to(&mut bytes).unwrap();
            assert_eq!(&bytes[..8], &FILE_MAGIC_V2, "V2 write_to must emit SPQCPRM2 magic at n={n}");
            let dir = std::env::temp_dir().join(format!(
                "sparq-v2reader-{}-{}-{}",
                std::process::id(),
                n,
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("perm.bin");
            std::fs::write(&path, &bytes).unwrap();
            let file = std::fs::File::open(&path).unwrap();
            // SAFETY: we own and just wrote this file; nothing else mutates it during the test.
            let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
            let opened = CompressedPerm::from_mmap(map).unwrap();
            assert_eq!(opened.format, Format::V2, "from_mmap must auto-detect V2 at n={n}");
            assert_eq!(opened.decode_all(), rows, "V2 mmap decode diverged at n={n}");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    // ===== [FABLE-5] sq-7d3dj.32.2.6 — SPQCPRM2 frame-of-reference spike =====

    /// A sorted permutation whose col2 (object) CLUSTERS within each block — the shape the
    /// frame-of-reference col2-reset targets: many `reset_d1` rows (middle column advances, so
    /// col2 is written as an absolute in SPQCPRM1) whose objects sit near the block's first
    /// object. Long equal-subject runs with a moving predicate produce dense `reset_d1` rows.
    #[cfg(feature = "spqcprm2")]
    fn clustered_col2_sample(n: usize) -> Vec<[Id; 3]> {
        let mut v = Vec::with_capacity(n);
        let mut state = 0x2545_f491u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..n {
            // Few subjects (long equal-col0 runs), moderate predicate churn (drives reset_d1),
            // and objects drawn from a LOCAL window around a per-subject base so a block's col2
            // clusters — exactly where a frame offset beats an absolute id.
            let s = 1 + (next() % (n as u32 / 64).max(1));
            let base = 1 + (s.wrapping_mul(97) % 4_000_000);
            let p = 1 + (next() % 40);
            let o = base + (next() % 4096); // objects within a 4K window of the subject's base
            v.push([s, p, o]);
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — DIRECT unit test for `encode_v2`: the V2 in-RAM writer must
    /// round-trip to the exact input rows across block boundaries and both reset shapes, and its
    /// perm carries `Format::V2` (so it decodes through the frame-of-reference reader). This is
    /// the load-bearing correctness invariant of the spike (a lossy FoR would silently corrupt).
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn encode_v2_roundtrips_and_is_v2() {
        for &n in &[0usize, 1, 5, 127, 128, 129, 256, 1000, 5000] {
            let rows = clustered_col2_sample(n);
            let c = CompressedPerm::encode_v2(&rows);
            assert_eq!(c.len(), rows.len(), "v2 len mismatch at n={n}");
            assert_eq!(c.decode_all(), rows, "v2 decode_all mismatch at n={n}");
            assert_eq!(c.format, Format::V2, "encode_v2 must produce a V2 perm at n={n}");
        }
        // Also round-trip the existing clustered/sparse `sample` shape so the FoR is proven on
        // BOTH object distributions (clustered AND sparse), not just its favourable case.
        for &n in &[0usize, 1, 129, 5000] {
            let rows = sample(n);
            assert_eq!(CompressedPerm::encode_v2(&rows).decode_all(), rows, "v2 roundtrip on sample n={n}");
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — DIFFERENTIAL: SPQCPRM1 and SPQCPRM2 are two encodings of the
    /// SAME logical data. Decoding either must yield the IDENTICAL rows — the frame-of-reference
    /// changes only the col2-reset BYTES, never the decoded value. We also confirm the two
    /// encoders produce DIFFERENT block streams on data with `reset_d1` rows (so the spike is
    /// actually exercising the new path, not silently emitting the V1 stream).
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn spqcprm1_vs_spqcprm2_decode_identical() {
        for &n in &[0usize, 1, 129, 1000, 5000] {
            let rows = clustered_col2_sample(n);
            let v1 = CompressedPerm::encode(&rows);
            let v2 = CompressedPerm::encode_v2(&rows);
            assert_eq!(v1.decode_all(), v2.decode_all(), "v1/v2 decode divergence at n={n}");
            assert_eq!(v1.decode_all(), rows, "v1 decode != input at n={n}");
            assert_eq!(v2.decode_all(), rows, "v2 decode != input at n={n}");
        }
        // On a col2-clustered shape with many reset_d1 rows the two block streams MUST differ
        // (otherwise the FoR path is never taken and the differential is vacuous). We compare
        // the raw encoded bytes of a single well-populated block via the byte buffers.
        let rows = clustered_col2_sample(4000);
        let mut b1 = Vec::new();
        let mut b2 = Vec::new();
        for chunk in rows.chunks(BLOCK) {
            encode_block(chunk, &mut b1);
            encode_block_v2(chunk, &mut b2);
        }
        assert_ne!(b1, b2, "v1 and v2 block streams identical — the reset_d1 FoR path never fired");
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — a V2 perm's `range` must return EXACTLY the raw binary-search
    /// range, identical to the V1 `range_matches_binary_search` oracle. The frame-of-reference
    /// touches only the decode of col2-reset bytes, so random access is correctness-neutral.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn v2_range_matches_binary_search() {
        let rows = clustered_col2_sample(5000);
        let c = CompressedPerm::encode_v2(&rows);
        let raw_range = |lo: [Id; 3], hi: [Id; 3]| -> Vec<[Id; 3]> {
            let s = rows.partition_point(|r| *r < lo);
            let e = rows.partition_point(|r| *r <= hi);
            rows[s..e].to_vec()
        };
        let cases: &[([Id; 3], [Id; 3])] = &[
            ([Id::MIN; 3], [Id::MAX; 3]),
            ([rows[rows.len() / 4][0], Id::MIN, Id::MIN], [rows[rows.len() / 4][0], Id::MAX, Id::MAX]),
            ([99_999_999, Id::MIN, Id::MIN], [99_999_999, Id::MAX, Id::MAX]),
            (rows[0], rows[0]),
            (rows[rows.len() / 2], rows[rows.len() / 2]),
            (rows[rows.len() - 1], rows[rows.len() - 1]),
        ];
        for &(lo, hi) in cases {
            assert_eq!(c.range(lo, hi), raw_range(lo, hi), "v2 range mismatch for {lo:?}..={hi:?}");
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — DIRECT unit test for the zigzag varint round-trip
    /// (`put_zigzag_varint` / `get_zigzag_varint`): the signed frame offset must survive
    /// encode→decode across zero, both signs, and the extremes.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn zigzag_varint_roundtrips() {
        let cases = [0i64, 1, -1, 2, -2, 127, -128, 300, -300, i32::MAX as i64, i32::MIN as i64, i64::MAX, i64::MIN];
        for &x in &cases {
            let mut buf = Vec::new();
            put_zigzag_varint(&mut buf, x);
            let mut pos = 0;
            assert_eq!(get_zigzag_varint(&buf, &mut pos), x, "zigzag roundtrip failed for {x}");
            assert_eq!(pos, buf.len(), "zigzag reader did not consume all bytes for {x}");
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — DIRECT unit test that the reserved V2 version marker is the
    /// distinct 8-byte `SPQCPRM2` magic (never equal to the V1 magic), so a future migration can
    /// auto-detect it without a collision.
    #[cfg(all(feature = "spqcprm2", feature = "mmap"))]
    #[test]
    fn file_magic_v2_is_distinct() {
        assert_eq!(&FILE_MAGIC_V2, b"SPQCPRM2");
        assert_ne!(FILE_MAGIC_V2, FILE_MAGIC, "V2 magic must differ from V1");
    }

    // ===== [FABLE-5] sq-7d3dj.32.2.7 — the FORMAT MIGRATION (write + auto-detect V2) =====

    /// [FABLE-5] sq-7d3dj.32.2.7 — a unique scratch dir (no dev-dep tempfile), like the
    /// corruption oracle's helper. CI temp is ephemeral so we never remove it.
    #[cfg(feature = "spqcprm2")]
    fn scratch_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "sparq-spqcprm2-mig-{}-{}-{}",
            std::process::id(),
            n,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — the LOAD-BEARING migration invariant: a `V2` perm written via
    /// `write_to` carries the `SPQCPRM2` magic on disk, is AUTO-DETECTED by `from_mmap` (which
    /// picks the frame-of-reference reader), and decodes back to the EXACT input rows — including
    /// the full `range` random-access path. Proves the round-trip write→open→decode holds through
    /// the real mmap loader (not just the in-RAM encode/decode of the spike).
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn write_to_v2_roundtrips_through_from_mmap() {
        for &n in &[1usize, 127, 128, 129, 300, 5000] {
            let rows = clustered_col2_sample(n);
            let dir = scratch_dir();
            let path = dir.join("perm.bin");
            let c = CompressedPerm::encode_v2(&rows);
            assert_eq!(c.format, Format::V2);
            {
                let mut w = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
                c.write_to(&mut w).unwrap();
                std::io::Write::flush(&mut w).unwrap();
            }
            // On disk the file MUST start with the V2 magic, never the V1 magic.
            let head = std::fs::read(&path).unwrap();
            assert_eq!(&head[..8], &FILE_MAGIC_V2, "V2 write_to must emit the SPQCPRM2 magic (n={n})");
            assert_ne!(&head[..8], &FILE_MAGIC, "V2 file must not carry the V1 magic (n={n})");
            // Re-open through the real mmap loader; it must auto-detect V2 and decode identically.
            let file = std::fs::File::open(&path).unwrap();
            // SAFETY: we own and just wrote this file; nothing else mutates it during the test.
            let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
            let opened = CompressedPerm::from_mmap(map).unwrap();
            assert_eq!(opened.format, Format::V2, "from_mmap must auto-detect V2 (n={n})");
            assert_eq!(opened.decode_all(), rows, "V2 write→open→decode diverged (n={n})");
            // Full random-access parity too.
            assert_eq!(opened.range([Id::MIN; 3], [Id::MAX; 3]), rows, "V2 opened range diverged (n={n})");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — BACKWARD-COMPAT SOUNDNESS: an existing `SPQCPRM1` file on disk
    /// must keep decoding byte-identically forever, regardless of the emit config. We write a V1
    /// perm the ordinary way, re-open it through the loader, and require the decoded rows AND the
    /// on-disk magic to be exactly the shipped V1 form. Also asserts the V1 on-disk bytes are
    /// UNCHANGED by this migration (equal to `encode(rows).write_to`, the pre-migration writer).
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn v1_file_decodes_byte_identically_forever() {
        for &n in &[1usize, 129, 5000] {
            let rows = clustered_col2_sample(n);
            let dir = scratch_dir();
            let path = dir.join("perm.bin");
            let c = CompressedPerm::encode(&rows);
            assert_eq!(c.format, Format::V1);
            let mut bytes = Vec::new();
            c.write_to(&mut bytes).unwrap();
            assert_eq!(&bytes[..8], &FILE_MAGIC, "V1 write_to must emit the SPQCPRM1 magic (n={n})");
            std::fs::write(&path, &bytes).unwrap();
            let file = std::fs::File::open(&path).unwrap();
            // SAFETY: we own and just wrote this file; nothing else mutates it during the test.
            let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
            let opened = CompressedPerm::from_mmap(map).unwrap();
            assert_eq!(opened.format, Format::V1, "a SPQCPRM1 file must open as V1 (n={n})");
            assert_eq!(opened.decode_all(), rows, "V1 decode diverged (n={n})");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — the streaming `CompressedPermWriter` in V2 mode must produce a
    /// file BYTE-IDENTICAL to `encode_v2(rows).write_to`, mirroring the V1 byte-identity guarantee
    /// (`stream_writer_byte_identical_to_encode_write_to`). This is the invariant that lets the
    /// external build emit V2 straight from the merge tail with no recompress pass.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn streaming_writer_v2_byte_identical_to_encode_v2_write_to() {
        for &n in &[1usize, 127, 128, 129, 500, 5000] {
            let rows = clustered_col2_sample(n);
            let dir = scratch_dir();
            let streamed = dir.join("streamed.bin");
            // Stream the rows through the V2 writer.
            let mut w = CompressedPermWriter::create_with(&streamed, Format::V2).unwrap();
            for &row in &rows {
                w.push(row).unwrap();
            }
            w.finish(&streamed).unwrap();
            // In-RAM V2 reference bytes.
            let mut want = Vec::new();
            CompressedPerm::encode_v2(&rows).write_to(&mut want).unwrap();
            let got = std::fs::read(&streamed).unwrap();
            assert_eq!(got, want, "streamed V2 bytes differ from encode_v2().write_to (n={n})");
            assert_eq!(&got[..8], &FILE_MAGIC_V2, "streamed V2 magic wrong (n={n})");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — the emit-format CONFIG GATE. `encode_emit` writes SPQCPRM1 by
    /// default (the shipped behaviour — V2 is NOT defaulted on), and SPQCPRM2 only inside a
    /// `with_emit_format(V2)` scope. Proves the gate actually flips the format and restores it.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn emit_format_config_gate_selects_v2() {
        let rows = clustered_col2_sample(500);
        // DEFAULT: no override, no env ⇒ V1.
        assert_eq!(CompressedPerm::encode_emit(&rows).format, Format::V1, "default emit must be V1");
        // Opt in on this thread ⇒ V2, then restore.
        with_emit_format(EmitFormat::V2, || {
            assert_eq!(CompressedPerm::encode_emit(&rows).format, Format::V2, "gate did not select V2");
            assert_eq!(emit_format(), Format::V2);
        });
        // Restored after the scope.
        assert_eq!(CompressedPerm::encode_emit(&rows).format, Format::V1, "emit override leaked past scope");
        // The V2-emitted perm still round-trips to the exact rows.
        with_emit_format(EmitFormat::V2, || {
            assert_eq!(CompressedPerm::encode_emit(&rows).decode_all(), rows, "V2 emit lost data");
        });
    }

    /// [FABLE-5] sq-7d3dj.32.2.7 — MUTATION WITNESS on the magic auto-detect: corrupting the
    /// 8-byte file magic to anything that is neither `SPQCPRM1` nor `SPQCPRM2` must be a LOUD,
    /// CLEAN error from `from_mmap` (never a silent misdecode). Also proves a V1 file whose magic
    /// is flipped to the V2 magic does NOT silently decode as V2 to the SAME rows — the reader
    /// applies the wrong (frame-of-reference) interpretation, so a magic swap changes the decoded
    /// bytes (evidence the magic genuinely selects the reader), but it is still memory-safe. The
    /// on-disk store is a TRUSTED asset (no per-record checksum, threat-model B5): the guarantee
    /// is a clean Err on an UNKNOWN magic + never UB, not integrity of a same-magic content flip.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn corrupt_magic_is_loud_error_never_misdecode() {
        let rows = clustered_col2_sample(600);
        let dir = scratch_dir();
        let path = dir.join("perm.bin");
        let mut bytes = Vec::new();
        CompressedPerm::encode(&rows).write_to(&mut bytes).unwrap();
        assert_eq!(&bytes[..8], &FILE_MAGIC);

        // (a) An UNKNOWN magic — every single-byte flip of the 8 magic bytes that lands on
        // neither known marker must be a clean Err (loud rejection), never a decode.
        for i in 0..8 {
            for xor in [0x01u8, 0x80, 0xFF] {
                let mut m = bytes.clone();
                m[i] ^= xor;
                if m[..8] == FILE_MAGIC || m[..8] == FILE_MAGIC_V2 {
                    continue; // a flip that happens to hit the other valid magic — covered in (b)
                }
                std::fs::write(&path, &m).unwrap();
                let file = std::fs::File::open(&path).unwrap();
                // SAFETY: we own and just wrote this file; nothing else mutates it during the test.
                let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
                let res = CompressedPerm::from_mmap(map);
                assert!(res.is_err(), "corrupt magic @byte{i}^{xor:#x} silently accepted (must be a loud Err)");
            }
        }

        // (b) Swapping a V1 file's magic to the V2 magic must NOT silently decode to the same
        // rows: the V2 reader applies the frame-of-reference inverse to a stream the V1 encoder
        // wrote absolute, so at least one decoded object differs. This is the witness that the
        // magic genuinely picks the reader (a silent misdecode would be the dangerous case). It
        // must still be memory-safe (no panic / OOB) — `from_mmap`'s validation is format-agnostic.
        let mut swapped = bytes.clone();
        swapped[..8].copy_from_slice(&FILE_MAGIC_V2);
        std::fs::write(&path, &swapped).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        // SAFETY: we own and just wrote this file; nothing else mutates it during the test.
        let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
        // Whether validation accepts it or not, it must never panic; if it opens, the decode
        // must DIFFER from the true rows (the magic swap really changed the interpretation) as
        // long as the data actually contains reset_d1 rows (clustered_col2_sample does).
        if let Ok(opened) = CompressedPerm::from_mmap(map) {
            assert_eq!(opened.format, Format::V2, "swapped magic must open as V2");
            let decoded = opened.decode_all();
            assert_ne!(decoded, rows, "a V1 stream read as V2 must NOT decode to the same rows (reset_d1 present)");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [FABLE-5] sq-7d3dj.32.2.6 — the MEASUREMENT: block-stream B/triple for SPQCPRM1 vs
    /// SPQCPRM2 at 1M/10M-row scale, summed over the six permutations (the store's real cost).
    /// All numbers are NON-canonical work-box measurements (never a doc/test perf number).
    /// `#[ignore]` so default `cargo test` stays fast; run with
    /// `cargo test -p sparq-core --features spqcprm2 --lib v2_measure -- --ignored --nocapture`.
    #[cfg(feature = "spqcprm2")]
    #[test]
    #[ignore = "spike measurement: run explicitly with --ignored --nocapture (allocates hundreds of MB)"]
    fn v2_measure_bytes_per_triple() {
        let orders: [(&str, [usize; 3]); 6] = [
            ("SPO", [0, 1, 2]), ("SOP", [0, 2, 1]), ("PSO", [1, 0, 2]),
            ("POS", [1, 2, 0]), ("OSP", [2, 0, 1]), ("OPS", [2, 1, 0]),
        ];
        println!("\n===== sq-7d3dj.32.2.6 SPQCPRM1 vs SPQCPRM2 B/triple (NON-canonical work-box) =====");
        for &n in &[1_000_000usize, 10_000_000] {
            let triples = clustered_col2_sample(n);
            let n_tri = triples.len().max(1) as f64;
            let (mut v1_bytes, mut v2_bytes) = (0u64, 0u64);
            for (_, order) in orders {
                let mut rows: Vec<[Id; 3]> =
                    triples.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
                rows.sort_unstable();
                rows.dedup();
                for chunk in rows.chunks(BLOCK) {
                    let mut b1 = Vec::new();
                    let mut b2 = Vec::new();
                    encode_block(chunk, &mut b1);
                    encode_block_v2(chunk, &mut b2);
                    v1_bytes += b1.len() as u64;
                    v2_bytes += b2.len() as u64;
                }
            }
            let v1_bpt = v1_bytes as f64 / n_tri;
            let v2_bpt = v2_bytes as f64 / n_tri;
            let delta_pct = (v2_bpt - v1_bpt) / v1_bpt * 100.0;
            println!(
                "n={:>10} distinct={:>10}  SPQCPRM1={:>7.3} B/tri  SPQCPRM2={:>7.3} B/tri  delta={:+.2}%",
                n, triples.len(), v1_bpt, v2_bpt, delta_pct
            );
        }
        println!("===== end measurement — verdict in the bead note / PR body =====\n");
    }

    // ===== [OPUS-5] sq-sh7be — the V2-DEFAULT FLIP DECISION (see the emit-gate comment) =====

    /// A sorted permutation whose col2 (object) is drawn from a WIDE domain — the ADVERSE shape
    /// for the frame-of-reference, and the complement of `clustered_col2_sample`. Subjects
    /// repeat (long equal-col0 runs) and predicates churn, so `reset_d1` rows are dense, but the
    /// objects are spread far enough that a block's `|r[2] - first_col2|` is comparable to `r[2]`
    /// itself — and zigzag doubles that magnitude, so the frame offset costs MORE varint bytes
    /// than the absolute id V1 writes. Deterministic (fixed-seed xorshift), like every other
    /// sampler here, so the byte counts it produces are reproducible on any host.
    #[cfg(feature = "spqcprm2")]
    fn wide_col2_sample(n: usize) -> Vec<[Id; 3]> {
        let mut v = Vec::with_capacity(n);
        let mut state = 0x51ed_2701u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..n {
            let s = 1 + (next() % (n as u32 / 32).max(1)); // long equal-subject runs
            let p = 1 + (next() % 30); // predicate churn → dense reset_d1 rows
            let o = 1 + (next() % 40_000_000); // objects spread across a wide domain
            v.push([s, p, o]);
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Total encoded block-stream bytes for `triples` under BOTH formats, summed over all six
    /// permutations (the store's real cost), plus the `reset_d1` row count. Encoded length is a
    /// pure function of the rows, so this is exact and host-independent — no timing involved.
    #[cfg(feature = "spqcprm2")]
    fn v1_v2_stream_bytes(triples: &[[Id; 3]]) -> (u64, u64, u64) {
        const ORDERS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        let (mut v1, mut v2, mut resets) = (0u64, 0u64, 0u64);
        for order in ORDERS {
            let mut rows: Vec<[Id; 3]> =
                triples.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
            rows.sort_unstable();
            rows.dedup();
            for chunk in rows.chunks(BLOCK) {
                let (mut b1, mut b2) = (Vec::new(), Vec::new());
                encode_block(chunk, &mut b1);
                encode_block_v2(chunk, &mut b2);
                v1 += b1.len() as u64;
                v2 += b2.len() as u64;
                // A `reset_d1` row: leading column held, middle column moved → col2 is re-based.
                resets += chunk.windows(2).filter(|w| w[1][0] == w[0][0] && w[1][1] != w[0][1]).count() as u64;
            }
        }
        (v1, v2, resets)
    }

    /// [OPUS-5] sq-sh7be — the DECISION EVIDENCE for the V2-default flip, and the guard that
    /// keeps it honest. Unlike `v2_measure_bytes_per_triple` (`#[ignore]`d, and measured only on
    /// the shape V2 was designed to win on) this runs in the ordinary suite and measures BOTH
    /// sides, because "is V2 smaller?" has no single answer:
    ///
    ///   * col2-CLUSTERED → V2's stream is strictly SMALLER (the frame offset beats the absolute);
    ///   * WIDE col2      → V2's stream is strictly LARGER (zigzag doubles the offset magnitude).
    ///
    /// Both directions are exact integer byte comparisons over deterministic samplers, so this is
    /// reproducible on any host and needs no quiet box — which is precisely why the flip decision
    /// did not have to wait for one. The conclusion it pins is that V2 is a shape-dependent trade,
    /// so it stays OPT-IN; if either direction ever stops holding, this test goes RED and the
    /// decision must be re-taken rather than silently inherited.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn v2_size_delta_is_shape_dependent_not_a_uniform_win() {
        // Favourable shape: the frame-of-reference must actually pay off where it was aimed.
        let clustered = clustered_col2_sample(50_000);
        let (v1, v2, resets) = v1_v2_stream_bytes(&clustered);
        assert!(resets > 0, "clustered sample has no reset_d1 rows — the comparison would be vacuous");
        assert!(
            v2 < v1,
            "SPQCPRM2 must be SMALLER than SPQCPRM1 on a col2-clustered corpus (v1={}, v2={}, reset_d1={})",
            v1, v2, resets
        );

        // Adverse shape: the same encoder must be shown to LOSE, which is what blocks the flip.
        let wide = wide_col2_sample(50_000);
        let (v1, v2, resets) = v1_v2_stream_bytes(&wide);
        assert!(resets > 0, "wide sample has no reset_d1 rows — the comparison would be vacuous");
        assert!(
            v2 > v1,
            "SPQCPRM2 is expected to be LARGER than SPQCPRM1 on a wide-object corpus; if this no \
             longer holds, the sq-sh7be V2-default decision must be re-taken (v1={}, v2={}, reset_d1={})",
            v1, v2, resets
        );
    }

    /// [OPUS-5] sq-sh7be — the ENCODER-DIVERGENCE anchor. `encode_block_v2` is documented as
    /// byte-for-byte identical to `encode_block` except at `reset_d1` rows; this asserts that
    /// documented claim directly on a block stream that contains NO `reset_d1` row (one subject,
    /// one predicate, strictly increasing objects — every row takes the `d0 == 0 && d1 == 0`
    /// arm), and checks the converse is non-vacuous on a `reset_d1`-bearing stream.
    ///
    /// Scope, deliberately: this observes EMITTED BYTES only. It does NOT bound decode cost — a
    /// V2 perm is still dispatched to `decode_block_v2_at`, which captures the frame origin once
    /// per block whether or not the block holds a `reset_d1` row, so byte equality does not imply
    /// equal decode work. Any runtime claim about V2 needs a decode benchmark, not this test.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn v2_stream_is_byte_identical_when_no_reset_d1() {
        // Spans several blocks so block re-entry (where V2 captures its frame origin) is covered.
        let rows: Vec<[Id; 3]> = (0..5_000u32).map(|i| [1, 1, 1 + i * 7]).collect();
        let (mut b1, mut b2) = (Vec::new(), Vec::new());
        for chunk in rows.chunks(BLOCK) {
            encode_block(chunk, &mut b1);
            encode_block_v2(chunk, &mut b2);
        }
        assert!(rows.len() > BLOCK, "sample must span multiple blocks");
        assert_eq!(b1, b2, "with no reset_d1 row the V2 stream must be byte-identical to V1");
        // And the identity is not vacuous — the same encoders DO diverge once reset_d1 rows exist.
        let (mut c1, mut c2) = (Vec::new(), Vec::new());
        for chunk in clustered_col2_sample(2_000).chunks(BLOCK) {
            encode_block(chunk, &mut c1);
            encode_block_v2(chunk, &mut c2);
        }
        assert_ne!(c1, c2, "encoders must diverge on a reset_d1-bearing stream (else the test is vacuous)");
    }
}
