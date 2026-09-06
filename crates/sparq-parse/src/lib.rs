// [OPUS-4.8] sq-ieqz: rustdoc front page IS the crate-local README
// (`crates/sparq-parse/README.md`), so the doc surface and the README never drift.
// The README carries the format rationale (multi-member gzip / multi-frame zstd),
// the no-wasm-dep guard, and the runnable API-sketch doctest below.
#![doc = include_str!("../README.md")]
// [OPUS-5] sq-98w7z.2: the gzip-backend verdict lives HERE rather than in the README
// because that README is a gate-capped internal stub (`check-readme-template.py`), and
// the template rule's own guidance is to put verbose detail in rustdoc. Appending a
// `#![doc]` block keeps it on the same rustdoc front page as the README.
#![doc = r#"
## Gzip encoder backend

| feature | backend | notes |
|---|---|---|
| *(default)* | `miniz_oxide` | pure Rust; keeps the better ratio at `-1` |
| `zlib-rs` | zlib-rs | pure Rust; much faster encoder at `-6` |
| `zlib-ng` | zlib-ng (C) | native only — safe here because this crate never enters the wasm graph |

flate2 resolves these by **precedence, not by the order they are listed**: a C
backend (`zlib-ng`) outranks `zlib-rs`, which outranks `miniz_oxide`. Enabling
`zlib-ng` and `zlib-rs` together therefore silently yields `zlib-ng`, and
`--all-features` resolves to `zlib-ng`. A backend has to be exercised on its own
(`--features zlib-rs`); an `--all-features` run does **not** cover it.

Measured verdict (`sq-98w7z.2`, A/B over the [`CompressedSink`] and
[`SingleMemberGzipSink`] gzip paths — figures recorded in the bead, not restated
here, per the repository's no-baked-benchmark-numbers rule):

- At **`-6`** `zlib-rs` encodes substantially faster for a negligible ratio cost.
  This is the level the D4 measurement found could not keep up with the result
  serializer, so it is the level worth switching.
- At **`-1`** it also encodes faster but gives up noticeably more ratio, and the
  loss grows the more repetitive the payload — a trade, not a free win.

Both backends hold the format invariants these sinks rest on: byte-valid RFC 1952
output, `Z_FULL_FLUSH` segments that concatenate into a single member, and a
correct combined CRC32 — note that `flate2::Crc` (including `combine`) also
switches implementation with the feature. Checked by round-tripping each
backend's output through flate2's strict single-member `GzDecoder`, plus direct
assertions on the fixed RFC 1952 header bytes. Be precise about what that buys:
because flate2 picks decoder and CRC by the same backend precedence, the
round-trip runs under the backend it is testing, so it is a self-consistency
check, **not** an implementation-independent interoperability check. Validating
the wire against a separate gzip implementation is not yet covered.
"#]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex};

use zstd::dict::{DecoderDictionary, EncoderDictionary};

/// A zstd dictionary prepared (digested) once for a fixed compression level, so
/// per-response compression does not re-digest the dictionary bytes. Cheap to
/// share across threads behind the `Arc` in [`Codec::ZstdDict`].
pub struct PreparedDict {
    encoder: EncoderDictionary<'static>,
    decoder: DecoderDictionary<'static>,
    raw: Vec<u8>,
    level: i32,
}

impl PreparedDict {
    /// Digests `dict_bytes` (e.g. the output of [`train_dictionary`]) for
    /// compression at `level` and for decompression.
    pub fn new(dict_bytes: Vec<u8>, level: i32) -> Self {
        PreparedDict {
            encoder: EncoderDictionary::copy(&dict_bytes, level),
            decoder: DecoderDictionary::copy(&dict_bytes),
            raw: dict_bytes,
            level,
        }
    }

    /// The raw dictionary bytes (what a `/dictionary` endpoint would serve).
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The compression level the encoder side was prepared for.
    pub fn level(&self) -> i32 {
        self.level
    }

    /// The prepared decoder side (for `zstd::bulk::Decompressor::with_prepared_dictionary`).
    pub fn decoder(&self) -> &DecoderDictionary<'static> {
        &self.decoder
    }
}

/// Trains a zstd dictionary from sample messages (wraps `zstd::dict::from_samples`).
/// `max_size` is the dictionary size cap in bytes (the addendum's vocabulary
/// dictionaries are small: 16–112 KiB).
pub fn train_dictionary<S: AsRef<[u8]>>(samples: &[S], max_size: usize) -> io::Result<Vec<u8>> {
    zstd::dict::from_samples(samples, max_size)
}

/// Which compressed format each pushed chunk becomes one member/frame of.
#[derive(Clone)]
pub enum Codec {
    /// One RFC 1952 gzip member per chunk (flate2). Level 0–9; 6 is the gzip CLI
    /// default, 1 the speed end. Concatenated members are a valid gzip stream.
    Gzip { level: u32 },
    /// One zstd frame per chunk. Concatenated frames are a valid zstd stream.
    Zstd { level: i32 },
    /// One zstd frame per chunk, compressed with a prepared dictionary (the
    /// small-response path: the dictionary supplies the JSON/vocabulary context
    /// a tiny message cannot carry itself). Decoding requires the same dictionary.
    ZstdDict(Arc<PreparedDict>),
}

impl Codec {
    /// Human-readable tag used by benches/reports.
    pub fn name(&self) -> String {
        match self {
            Codec::Gzip { level } => format!("gzip -{level}"),
            Codec::Zstd { level } => format!("zstd -{level}"),
            Codec::ZstdDict(d) => format!("zstd -{} +dict({}B)", d.level, d.raw.len()),
        }
    }
}

/// Compresses one chunk into one self-contained gzip member / zstd frame.
pub fn compress_member(codec: &Codec, chunk: &[u8]) -> io::Result<Vec<u8>> {
    match codec {
        Codec::Gzip { level } => {
            let mut enc = flate2::write::GzEncoder::new(
                Vec::with_capacity(chunk.len() / 4 + 64),
                flate2::Compression::new(*level),
            );
            enc.write_all(chunk)?;
            enc.finish()
        }
        Codec::Zstd { level } => zstd::bulk::compress(chunk, *level),
        Codec::ZstdDict(d) => {
            let mut c = zstd::bulk::Compressor::with_prepared_dictionary(&d.encoder)?;
            c.compress(chunk)
        }
    }
}

/// Serial (compress on `push`, caller's thread) or parallel (compress on the
/// rayon pool, members still emitted in push order).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Serial,
    Parallel,
}

struct Shared {
    /// member index → compression result, for members finished but not yet drained.
    ready: Mutex<BTreeMap<usize, io::Result<Vec<u8>>>>,
    cv: Condvar,
}

// [OPUS-4.8] True when this code is executing inside a rayon worker thread. A
// `finish` that blocks on a Condvar for `rayon::spawn` jobs can deadlock here
// (the worker is stuck waiting for work it could be running), and on a
// single-thread pool there is no other worker to make progress. Both sinks use
// this to decide when to compress on the calling thread instead of detaching.
fn parallel_would_starve() -> bool {
    rayon::current_num_threads() <= 1 || rayon::current_thread_index().is_some()
}

/// Feed serialized chunks in, drain compressed members/frames out **in order**.
///
/// Each `push` becomes exactly one gzip member / zstd frame; the concatenation
/// of the drained members (in emission order) is a valid gzip/zstd stream whose
/// decompression is byte-identical to the concatenation of the pushed chunks.
///
/// In [`Mode::Parallel`] the chunks compress concurrently on the rayon pool;
/// [`try_drain`](CompressedSink::try_drain) hands back the contiguous prefix
/// that is ready (so the caller can put early members on the wire while later
/// ones are still compressing) and [`finish`](CompressedSink::finish) waits for
/// the rest. Compression errors surface on drain, at the failed member's
/// position.
pub struct CompressedSink {
    codec: Codec,
    mode: Mode,
    shared: Arc<Shared>,
    next_in: usize,
    next_out: usize,
}

impl CompressedSink {
    pub fn new(codec: Codec, mode: Mode) -> Self {
        CompressedSink {
            codec,
            mode,
            shared: Arc::new(Shared {
                ready: Mutex::new(BTreeMap::new()),
                cv: Condvar::new(),
            }),
            next_in: 0,
            next_out: 0,
        }
    }

    /// Number of chunks pushed so far.
    pub fn pushed(&self) -> usize {
        self.next_in
    }

    /// Submits one serialized chunk; it becomes member/frame number
    /// [`pushed`](CompressedSink::pushed)` - 1` of the output stream.
    pub fn push(&mut self, chunk: &[u8]) {
        let idx = self.next_in;
        self.next_in += 1;
        // [OPUS-4.8] Parallel mode falls back to compressing on the calling
        // thread when the rayon pool has a single worker or when we are already
        // running inside a rayon worker: in either case detaching the job and
        // blocking on the Condvar in `finish` could deadlock (the only worker —
        // or this worker — would be parked waiting for the very job it should
        // run). Serial compression here cannot starve.
        if self.mode == Mode::Serial || parallel_would_starve() {
            let out = compress_member(&self.codec, chunk);
            self.shared.ready.lock().unwrap().insert(idx, out);
        } else {
            // The chunk is copied so the compression task is 'static; a 64 KiB
            // memcpy is noise next to compressing those bytes.
            let owned = chunk.to_vec();
            let codec = self.codec.clone();
            let shared = Arc::clone(&self.shared);
            rayon::spawn(move || {
                let out = compress_member(&codec, &owned);
                shared.ready.lock().unwrap().insert(idx, out);
                shared.cv.notify_all();
            });
        }
    }

    /// Non-blocking: removes and returns the contiguous run of finished members
    /// starting at the emission cursor. An `Err` is a compression failure of the
    /// member at the cursor (the stream is unusable past that point).
    pub fn try_drain(&mut self) -> io::Result<Vec<Vec<u8>>> {
        let mut ready = self.shared.ready.lock().unwrap();
        let mut out = Vec::new();
        while let Some(r) = ready.remove(&self.next_out) {
            self.next_out += 1;
            out.push(r?);
        }
        Ok(out)
    }

    /// Waits for every outstanding member and returns the remainder, in order.
    pub fn finish(mut self) -> io::Result<Vec<Vec<u8>>> {
        let total = self.next_in;
        let mut out = Vec::with_capacity(total - self.next_out);
        // [OPUS-4.8] When `finish` is called from inside a rayon worker we must
        // not park the worker on the Condvar: that worker may be the only one
        // that can run the pending compression jobs, so a plain `cv.wait` would
        // deadlock. Instead, hand control back to rayon (`yield_now`) so this
        // worker drains the spawned jobs itself, then re-check. Off the pool we
        // keep the cheaper blocking wait. `push` already guarantees no jobs were
        // detached on a single-thread pool, so progress is always possible here.
        let on_pool = rayon::current_thread_index().is_some();
        let mut ready = self.shared.ready.lock().unwrap();
        while self.next_out < total {
            match ready.remove(&self.next_out) {
                Some(r) => {
                    self.next_out += 1;
                    out.push(r?);
                }
                None if on_pool => {
                    drop(ready);
                    rayon::yield_now();
                    ready = self.shared.ready.lock().unwrap();
                }
                None => ready = self.shared.cv.wait(ready).unwrap(),
            }
        }
        Ok(out)
    }
}

/// One-shot convenience: compress a batch of chunks into ordered members/frames.
/// [`Mode::Parallel`] uses the rayon pool; order is preserved by construction.
pub fn compress_chunks<S: AsRef<[u8]> + Sync>(
    chunks: &[S],
    codec: &Codec,
    mode: Mode,
) -> io::Result<Vec<Vec<u8>>> {
    match mode {
        Mode::Serial => chunks
            .iter()
            .map(|c| compress_member(codec, c.as_ref()))
            .collect(),
        Mode::Parallel => {
            use rayon::prelude::*;
            chunks
                .par_iter()
                .map(|c| compress_member(codec, c.as_ref()))
                .collect()
        }
    }
}

// ---------------------------------------------------------------------------
// Browser-safe parallel gzip: ONE member, independently-deflated segments
// ---------------------------------------------------------------------------

/// Compresses `chunk` as a standalone raw-deflate segment: a fresh deflate
/// stream (no shared dictionary with neighbours) ended by `Z_FULL_FLUSH` — so
/// segments produced independently CONCATENATE into one valid deflate stream —
/// or by `Finish` (the stream's final block) when `finish` is set.
fn deflate_segment(level: u32, chunk: &[u8], finish: bool) -> io::Result<Vec<u8>> {
    use flate2::{Compress, Compression, FlushCompress, Status};
    let mut c = Compress::new(Compression::new(level), false);
    let mut out: Vec<u8> = Vec::with_capacity(chunk.len() / 4 + 128);
    // Phase 1: feed all input.
    while (c.total_in() as usize) < chunk.len() {
        if out.capacity() == out.len() {
            out.reserve(16 * 1024);
        }
        c.compress_vec(
            &chunk[c.total_in() as usize..],
            &mut out,
            FlushCompress::None,
        )?;
    }
    // Phase 2: flush. Per the zlib contract a flush is complete once deflate
    // returns with output space still available; until then keep growing.
    let flush = if finish {
        FlushCompress::Finish
    } else {
        FlushCompress::Full
    };
    loop {
        if out.capacity() == out.len() {
            out.reserve(16 * 1024);
        }
        let status = c.compress_vec(&[], &mut out, flush)?;
        if finish {
            if matches!(status, Status::StreamEnd) {
                break;
            }
        } else if out.len() < out.capacity() {
            break;
        }
    }
    Ok(out)
}

const GZIP_HEADER: [u8; 10] = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]; // deflate, no flags, no mtime, OS=unknown

/// Parallel gzip that every gzip consumer decodes — including the ones that
/// stop at the first member.
///
/// Measured reality (see the D4 research note): Chromium, Firefox, WebKit and
/// curl all silently TRUNCATE a multi-member `Content-Encoding: gzip` body to
/// its first member, so [`Codec::Gzip`] member-per-chunk streams are wrong for
/// browser-facing responses. This sink instead emits ONE gzip member (pigz's
/// technique): each pushed chunk becomes an independent raw-deflate segment
/// ended by a full flush (byte-aligned, dictionary reset, not the final block),
/// compressed in parallel; the drained pieces are the gzip header, then the
/// segments in order, then a final empty `Finish` segment + the trailer with
/// the combined CRC32 (`flate2::Crc::combine`) and total length.
///
/// Same contract as [`CompressedSink`]: push chunks, drain wire pieces in
/// order, concatenation decodes (with plain single-member decoders) to the
/// concatenation of the chunks. The per-chunk dictionary reset costs the same
/// ratio loss as multi-member framing (measured negligible at the engine's
/// chunk sizes).
pub struct SingleMemberGzipSink {
    level: u32,
    mode: Mode,
    shared: Arc<GzShared>,
    next_in: usize,
    next_out: usize,
    /// Running CRC/length over the chunks of drained segments, combined in order.
    crc: flate2::Crc,
    header_sent: bool,
}

/// One decompressed-and-CRCed gzip member, keyed by member index in `GzShared::ready`.
type GzMember = io::Result<(Vec<u8>, flate2::Crc)>;

struct GzShared {
    ready: Mutex<BTreeMap<usize, GzMember>>,
    cv: Condvar,
}

impl SingleMemberGzipSink {
    pub fn new(level: u32, mode: Mode) -> Self {
        SingleMemberGzipSink {
            level,
            mode,
            shared: Arc::new(GzShared {
                ready: Mutex::new(BTreeMap::new()),
                cv: Condvar::new(),
            }),
            next_in: 0,
            next_out: 0,
            crc: flate2::Crc::new(),
            header_sent: false,
        }
    }

    /// Number of chunks pushed so far.
    pub fn pushed(&self) -> usize {
        self.next_in
    }

    pub fn push(&mut self, chunk: &[u8]) {
        let idx = self.next_in;
        self.next_in += 1;
        let level = self.level;
        let work = move |chunk: &[u8]| {
            let mut crc = flate2::Crc::new();
            crc.update(chunk);
            deflate_segment(level, chunk, false).map(|seg| (seg, crc))
        };
        // [OPUS-4.8] Same starvation guard as CompressedSink::push: never detach
        // a job when the rayon pool has one worker or when we are already on a
        // rayon worker, since `finish` would otherwise block waiting for a job
        // that cannot run. Serial compression on the calling thread is safe.
        if self.mode == Mode::Serial || parallel_would_starve() {
            let out = work(chunk);
            self.shared.ready.lock().unwrap().insert(idx, out);
        } else {
            let owned = chunk.to_vec();
            let shared = Arc::clone(&self.shared);
            rayon::spawn(move || {
                let out = work(&owned);
                shared.ready.lock().unwrap().insert(idx, out);
                shared.cv.notify_all();
            });
        }
    }

    fn take(
        &mut self,
        piece: io::Result<(Vec<u8>, flate2::Crc)>,
        out: &mut Vec<Vec<u8>>,
    ) -> io::Result<()> {
        // [OPUS-4.8] Advance the emission cursor BEFORE propagating a compression
        // error. The caller (`try_drain`/`finish`) has already removed this index
        // from the ready map; if we returned `Err` without advancing `next_out`,
        // the cursor would still point at the consumed-and-gone index and a later
        // `finish` would wait forever for it. A compression error is terminal for
        // the stream anyway (documented), so surfacing it once at its position
        // and leaving the cursor past it is the consistent behaviour — matching
        // CompressedSink::try_drain.
        let (mut seg, crc) = match piece {
            Ok(v) => v,
            Err(e) => {
                self.next_out += 1;
                return Err(e);
            }
        };
        self.next_out += 1;
        self.crc.combine(&crc);
        if !self.header_sent {
            self.header_sent = true;
            let mut first = GZIP_HEADER.to_vec();
            first.append(&mut seg);
            out.push(first);
        } else {
            out.push(seg);
        }
        Ok(())
    }

    /// Non-blocking: the contiguous run of finished wire pieces (the first one
    /// carries the gzip header).
    pub fn try_drain(&mut self) -> io::Result<Vec<Vec<u8>>> {
        let shared = Arc::clone(&self.shared);
        let mut ready = shared.ready.lock().unwrap();
        let mut out = Vec::new();
        while let Some(p) = ready.remove(&self.next_out) {
            self.take(p, &mut out)?;
        }
        Ok(out)
    }

    /// Waits for the outstanding segments, then emits the final piece: an empty
    /// `Finish` segment closing the deflate stream + the 8-byte gzip trailer.
    pub fn finish(mut self) -> io::Result<Vec<Vec<u8>>> {
        let total = self.next_in;
        let shared = Arc::clone(&self.shared);
        let mut out = Vec::with_capacity(total - self.next_out + 1);
        {
            // [OPUS-4.8] See CompressedSink::finish: yield to rayon (rather than
            // parking the worker on the Condvar) when called from inside a rayon
            // worker, so this worker can drain its own pending segment jobs and
            // cannot deadlock.
            let on_pool = rayon::current_thread_index().is_some();
            let mut ready = shared.ready.lock().unwrap();
            while self.next_out < total {
                match ready.remove(&self.next_out) {
                    Some(p) => self.take(p, &mut out)?,
                    None if on_pool => {
                        drop(ready);
                        rayon::yield_now();
                        ready = shared.ready.lock().unwrap();
                    }
                    None => ready = shared.cv.wait(ready).unwrap(),
                }
            }
        }
        let mut last = deflate_segment(self.level, &[], true)?;
        if !self.header_sent {
            let mut first = GZIP_HEADER.to_vec();
            first.append(&mut last);
            last = first;
        }
        last.extend_from_slice(&self.crc.sum().to_le_bytes());
        last.extend_from_slice(&self.crc.amount().to_le_bytes());
        out.push(last);
        Ok(out)
    }
}

/// One-shot convenience over [`SingleMemberGzipSink`]: ordered wire pieces
/// whose concatenation is ONE browser-decodable gzip member.
pub fn gzip_single_member<S: AsRef<[u8]>>(
    chunks: &[S],
    level: u32,
    mode: Mode,
) -> io::Result<Vec<Vec<u8>>> {
    let mut sink = SingleMemberGzipSink::new(level, mode);
    for c in chunks {
        sink.push(c.as_ref());
    }
    sink.finish()
}

/// Decodes a concatenation of gzip members as one stream (what `gzip -d`,
/// Python `gzip` and Node `zlib.gunzip` do). Note: flate2's plain `GzDecoder` —
/// like browsers, curl and `DecompressionStream('gzip')` — stops after the
/// FIRST member; multi-member streams need `MultiGzDecoder` (see the D4
/// research note's consumer matrix; for browser-facing gzip use
/// [`SingleMemberGzipSink`] instead).
pub fn decode_gzip_concat(data: &[u8]) -> io::Result<Vec<u8>> {
    // [GPT-5.6] The zero-chunk encoder emits no gzip member.
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(data).read_to_end(&mut out)?;
    Ok(out)
}

/// Decodes a concatenation of zstd frames as one stream (the zstd stream decoder
/// continues across frame boundaries by default, like the `zstd` CLI).
pub fn decode_zstd_concat(data: &[u8]) -> io::Result<Vec<u8>> {
    zstd::stream::decode_all(data)
}

/// [`decode_zstd_concat`] for dictionary-compressed frames: the dictionary is
/// loaded once and applies to every frame in the stream.
pub fn decode_zstd_concat_with_dict(data: &[u8], dict: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    zstd::stream::read::Decoder::with_dictionary(data, dict)?.read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chunks shaped like the engine's SPARQL-JSON output: a head, then row
    /// fragments with repeated structure but distinct values (so members differ
    /// and ordering bugs cannot cancel out).
    fn json_chunks(n: usize) -> Vec<Vec<u8>> {
        let mut chunks =
            vec![br#"{"head":{"vars":["s","p","o"]},"results":{"bindings":["#.to_vec()];
        for i in 0..n {
            let mut c = Vec::new();
            for j in 0..200 {
                c.extend_from_slice(
                    format!(
                        r#"{{"s":{{"type":"uri","value":"http://www.wikidata.org/entity/Q{i}_{j}"}},"p":{{"type":"uri","value":"http://www.wikidata.org/prop/direct/P{j}"}},"o":{{"type":"literal","value":"row {i} col {j}"}}}},"#
                    )
                    .as_bytes(),
                );
            }
            chunks.push(c);
        }
        chunks.push(b"]}}".to_vec());
        chunks
    }

    fn concat(chunks: &[Vec<u8>]) -> Vec<u8> {
        chunks.iter().flat_map(|c| c.iter().copied()).collect()
    }

    fn run_sink(chunks: &[Vec<u8>], codec: Codec, mode: Mode) -> Vec<Vec<u8>> {
        let mut sink = CompressedSink::new(codec, mode);
        let mut members = Vec::new();
        for c in chunks {
            sink.push(c);
            members.extend(sink.try_drain().unwrap());
        }
        members.extend(sink.finish().unwrap());
        members
    }

    #[test]
    fn multi_member_gzip_roundtrips() {
        let chunks = json_chunks(20);
        for mode in [Mode::Serial, Mode::Parallel] {
            let members = run_sink(&chunks, Codec::Gzip { level: 6 }, mode);
            assert_eq!(
                members.len(),
                chunks.len(),
                "one member per chunk ({mode:?})"
            );
            assert_eq!(
                decode_gzip_concat(&concat(&members)).unwrap(),
                concat(&chunks),
                "{mode:?}"
            );
        }
    }

    // [GPT-5.6] sq-ue37p: decoding a zero-chunk gzip stream is the empty identity.
    #[test]
    fn decode_gzip_concat_accepts_empty_input() {
        assert_eq!(decode_gzip_concat(&[]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn multi_frame_zstd_roundtrips() {
        let chunks = json_chunks(20);
        for mode in [Mode::Serial, Mode::Parallel] {
            let members = run_sink(&chunks, Codec::Zstd { level: 3 }, mode);
            assert_eq!(
                members.len(),
                chunks.len(),
                "one frame per chunk ({mode:?})"
            );
            assert_eq!(
                decode_zstd_concat(&concat(&members)).unwrap(),
                concat(&chunks),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn parallel_members_equal_serial_members() {
        // Same codec + same input must give byte-identical members regardless of
        // mode — this pins both determinism and the in-order emission contract.
        let chunks = json_chunks(32);
        for codec in [Codec::Gzip { level: 1 }, Codec::Zstd { level: 1 }] {
            let serial = run_sink(&chunks, codec.clone(), Mode::Serial);
            let parallel = run_sink(&chunks, codec.clone(), Mode::Parallel);
            assert_eq!(serial, parallel, "{}", codec.name());
        }
    }

    #[test]
    fn compress_chunks_matches_sink() {
        let chunks = json_chunks(8);
        let codec = Codec::Zstd { level: 3 };
        let batch = compress_chunks(&chunks, &codec, Mode::Parallel).unwrap();
        assert_eq!(batch, run_sink(&chunks, codec, Mode::Serial));
    }

    #[test]
    fn empty_chunks_and_empty_stream() {
        // An empty chunk is a legal (if pointless) member; an empty sink yields
        // an empty stream.
        let chunks = vec![b"".to_vec(), b"x".to_vec(), b"".to_vec()];
        let members = run_sink(&chunks, Codec::Gzip { level: 6 }, Mode::Parallel);
        assert_eq!(decode_gzip_concat(&concat(&members)).unwrap(), b"x");
        let none = CompressedSink::new(Codec::Zstd { level: 3 }, Mode::Parallel)
            .finish()
            .unwrap();
        assert!(none.is_empty());
    }

    fn small_samples(n: usize) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| {
                format!(
                    r#"{{"head":{{"vars":["p","o"]}},"results":{{"bindings":[{{"p":{{"type":"uri","value":"http://www.wikidata.org/prop/direct/P{}"}},"o":{{"type":"literal","value":"point response {i}"}}}}]}}}}"#,
                    i % 50
                )
                .into_bytes()
            })
            .collect()
    }

    #[test]
    fn dictionary_frames_decode_with_the_dictionary() {
        let samples = small_samples(300);
        let dict = train_dictionary(&samples, 16 * 1024).unwrap();
        let prepared = Arc::new(PreparedDict::new(dict.clone(), 3));
        let codec = Codec::ZstdDict(Arc::clone(&prepared));

        // Held-out small responses (not in the training set).
        let responses = small_samples(320).split_off(300);
        let frames = compress_chunks(&responses, &codec, Mode::Parallel).unwrap();
        let stream = concat(&frames);

        // Decodes with the dictionary — as one multi-frame stream and per frame.
        assert_eq!(
            decode_zstd_concat_with_dict(&stream, &dict).unwrap(),
            concat(&responses)
        );
        let mut d = zstd::bulk::Decompressor::with_prepared_dictionary(prepared.decoder()).unwrap();
        for (frame, want) in frames.iter().zip(&responses) {
            assert_eq!(&d.decompress(frame, want.len() + 1).unwrap(), want);
        }

        // And does NOT decode without it (proves the dictionary is load-bearing).
        assert!(
            decode_zstd_concat(&stream).is_err(),
            "dict frames must not decode dict-less"
        );

        // Dictionary actually helps on these sizes: strictly smaller than plain zstd.
        let plain = compress_chunks(&responses, &Codec::Zstd { level: 3 }, Mode::Serial).unwrap();
        let (with, without): (usize, usize) = (
            frames.iter().map(Vec::len).sum(),
            plain.iter().map(Vec::len).sum(),
        );
        assert!(
            with < without,
            "dict {with} B should beat plain {without} B on small responses"
        );
    }

    /// Decodes with flate2's SINGLE-member `GzDecoder` — the strict stand-in for
    /// the consumers that truncate multi-member streams (browsers, curl,
    /// `DecompressionStream`): if this decodes fully, those do too.
    fn decode_single_member(data: &[u8]) -> Vec<u8> {
        let mut d = flate2::read::GzDecoder::new(data);
        let mut out = Vec::new();
        d.read_to_end(&mut out).unwrap();
        // The whole input must be consumed by the ONE member (no ignored tail).
        let mut rest = Vec::new();
        d.into_inner().read_to_end(&mut rest).unwrap();
        assert!(
            rest.is_empty(),
            "single-member stream must leave no trailing bytes"
        );
        out
    }

    #[test]
    fn single_member_gzip_roundtrips_with_a_single_member_decoder() {
        let chunks = json_chunks(20);
        for mode in [Mode::Serial, Mode::Parallel] {
            for level in [1, 6] {
                let mut sink = SingleMemberGzipSink::new(level, mode);
                let mut pieces = Vec::new();
                for c in &chunks {
                    sink.push(c);
                    pieces.extend(sink.try_drain().unwrap());
                }
                pieces.extend(sink.finish().unwrap());
                assert_eq!(
                    pieces.len(),
                    chunks.len() + 1,
                    "chunks + trailer piece ({mode:?})"
                );
                let wire = concat(&pieces);
                assert_eq!(
                    decode_single_member(&wire),
                    concat(&chunks),
                    "level {level} {mode:?}"
                );
                // MultiGzDecoder agrees (it sees one member).
                assert_eq!(decode_gzip_concat(&wire).unwrap(), concat(&chunks));
            }
        }
    }

    #[test]
    fn single_member_gzip_parallel_equals_serial() {
        let chunks = json_chunks(32);
        let serial = gzip_single_member(&chunks, 6, Mode::Serial).unwrap();
        let parallel = gzip_single_member(&chunks, 6, Mode::Parallel).unwrap();
        assert_eq!(serial, parallel);
    }

    #[test]
    fn single_member_gzip_empty_and_empty_chunks() {
        // No pushes: a valid empty gzip member.
        let pieces = SingleMemberGzipSink::new(6, Mode::Parallel)
            .finish()
            .unwrap();
        assert_eq!(decode_single_member(&concat(&pieces)), b"");
        // Empty chunks interleaved.
        let chunks = vec![b"".to_vec(), b"abc".to_vec(), b"".to_vec(), b"def".to_vec()];
        let pieces = gzip_single_member(&chunks, 1, Mode::Parallel).unwrap();
        assert_eq!(decode_single_member(&concat(&pieces)), b"abcdef");
    }

    #[test]
    fn drain_overlaps_production() {
        // try_drain hands back a contiguous prefix while later chunks are still
        // being pushed — the property the streaming server path relies on.
        let chunks = json_chunks(64);
        let mut sink = CompressedSink::new(Codec::Zstd { level: 1 }, Mode::Parallel);
        let mut members = Vec::new();
        let mut drained_early = false;
        for c in &chunks {
            sink.push(c);
            let got = sink.try_drain().unwrap();
            drained_early |= !got.is_empty() && sink.pushed() < chunks.len();
            members.extend(got);
        }
        members.extend(sink.finish().unwrap());
        assert_eq!(members.len(), chunks.len());
        assert_eq!(
            decode_zstd_concat(&concat(&members)).unwrap(),
            concat(&chunks)
        );
        // Not asserted as a hard invariant (scheduling), but record the intent:
        // on any real machine some members are ready before the last push.
        if !drained_early {
            eprintln!("note: no member drained before the final push (scheduling fluke)");
        }
    }

    // [OPUS-4.8] Regression for reviews 1913 & 1921: Mode::Parallel must not
    // deadlock on a single-thread rayon pool. With the old detached-spawn +
    // Condvar `finish`, the lone worker has no one to run the spawned jobs, so
    // `finish` would block forever. We run inside a 1-thread pool with a watchdog
    // thread so a regression fails the test (panic) instead of hanging CI.
    #[test]
    fn parallel_sinks_do_not_deadlock_on_single_thread_pool() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let chunks = json_chunks(16);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let done = Arc::new(AtomicBool::new(false));

        // Watchdog: if `pool.install` has not finished in 30 s it has deadlocked.
        let wd = Arc::clone(&done);
        let watchdog = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while !wd.load(Ordering::Acquire) {
                assert!(
                    start.elapsed() < std::time::Duration::from_secs(30),
                    "parallel sink deadlocked on a single-thread rayon pool (1913/1921 regression)"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });

        pool.install(|| {
            // CompressedSink (gzip + zstd) round-trips inside the constrained pool.
            let members = run_sink(&chunks, Codec::Gzip { level: 6 }, Mode::Parallel);
            assert_eq!(members.len(), chunks.len());
            assert_eq!(
                decode_gzip_concat(&concat(&members)).unwrap(),
                concat(&chunks)
            );

            let members = run_sink(&chunks, Codec::Zstd { level: 1 }, Mode::Parallel);
            assert_eq!(
                decode_zstd_concat(&concat(&members)).unwrap(),
                concat(&chunks)
            );

            // SingleMemberGzipSink round-trips inside the constrained pool.
            let pieces = gzip_single_member(&chunks, 6, Mode::Parallel).unwrap();
            assert_eq!(decode_single_member(&concat(&pieces)), concat(&chunks));
        });

        done.store(true, Ordering::Release);
        watchdog.join().unwrap();
    }

    // [OPUS-4.8] Regression for reviews 1913 & 1921: a `finish` called from
    // *inside* a rayon worker (nested execution) must not park that worker on
    // the Condvar while its own compression jobs are still pending — that is a
    // self-starvation deadlock. The yield-on-pool path must make progress.
    #[test]
    fn parallel_sink_finish_inside_rayon_worker_does_not_deadlock() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let chunks = json_chunks(24);
        let done = Arc::new(AtomicBool::new(false));
        let wd = Arc::clone(&done);
        let watchdog = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while !wd.load(Ordering::Acquire) {
                assert!(
                    start.elapsed() < std::time::Duration::from_secs(30),
                    "parallel sink finish deadlocked inside a rayon worker (1913/1921 regression)"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });

        // Default global pool, but the whole push/drain/finish lifecycle runs on
        // a worker via rayon::scope, exercising the nested-execution guard.
        rayon::scope(|s| {
            s.spawn(|_| {
                let members = run_sink(&chunks, Codec::Zstd { level: 1 }, Mode::Parallel);
                assert_eq!(
                    decode_zstd_concat(&concat(&members)).unwrap(),
                    concat(&chunks)
                );
                let pieces = gzip_single_member(&chunks, 6, Mode::Parallel).unwrap();
                assert_eq!(decode_single_member(&concat(&pieces)), concat(&chunks));
            });
        });

        done.store(true, Ordering::Release);
        watchdog.join().unwrap();
    }

    // [OPUS-4.8] Regression for review 1921 (Low): when a member fails to
    // compress, SingleMemberGzipSink::take must advance the emission cursor past
    // the consumed-and-removed index so a subsequent drain/finish cannot wait
    // forever for an index that no longer exists in the ready map.
    #[test]
    fn single_member_gzip_error_advances_cursor() {
        let mut sink = SingleMemberGzipSink::new(6, Mode::Serial);
        sink.push(b"alpha");
        sink.push(b"beta");
        let next_out_before = sink.next_out;
        // Inject a compression failure at the cursor position directly into the
        // ready map (mirrors a real deflate_segment error), then drain.
        sink.shared
            .ready
            .lock()
            .unwrap()
            .insert(next_out_before, Err(io::Error::other("boom")));
        let err = sink.try_drain().unwrap_err();
        assert_eq!(err.to_string(), "boom");
        // Cursor advanced past the failed (and already-removed) index; the entry
        // is gone, so a re-check must not block on it.
        assert_eq!(
            sink.next_out,
            next_out_before + 1,
            "cursor must advance past the error"
        );
    }

    // [OPUS-4.8] sq-qcnn.15: DIRECT tests for PreparedDict accessors — kills the
    // raw()/level()/decoder() whole-fn mutants that the indirect dictionary test
    // cannot reach.  Assertions compare EXACT returned values, not just is_ok().
    #[test]
    fn prepared_dict_accessors_return_exact_values() {
        let samples = small_samples(300);
        let dict_bytes = train_dictionary(&samples, 16 * 1024).unwrap();
        let level: i32 = 5;
        let prepared = PreparedDict::new(dict_bytes.clone(), level);

        // raw() must return the exact bytes used to create it — kills
        // `Vec::leak(Vec::new())`, `Vec::leak(vec![0])`, `Vec::leak(vec![1])`.
        assert_eq!(
            prepared.raw(),
            dict_bytes.as_slice(),
            "PreparedDict::raw() must equal the original dict bytes"
        );

        // level() must return the exact level — kills level->0, level->1, level->-1.
        assert_eq!(
            prepared.level(),
            level,
            "PreparedDict::level() must equal the construction level"
        );

        // decoder() must be the real decoder — verify by decompressing a frame that
        // was compressed with the same dict.  A wrong decoder would fail here.
        let frame = {
            let mut c =
                zstd::bulk::Compressor::with_prepared_dictionary(&prepared.encoder).unwrap();
            c.compress(b"hello dict").unwrap()
        };
        let mut d =
            zstd::bulk::Decompressor::with_prepared_dictionary(prepared.decoder()).unwrap();
        let plain = d.decompress(&frame, 64).unwrap();
        assert_eq!(
            plain,
            b"hello dict",
            "decoder() must be the real prepared decoder"
        );
    }

    // [OPUS-4.8] sq-qcnn.15: DIRECT tests for Codec::name — kills String::new() and
    // "xyzzy" whole-fn mutants.  Each arm format is pinned to its exact human-readable
    // string (used in bench/report output so the exact format matters).
    #[test]
    fn codec_name_exact_strings() {
        assert_eq!(
            Codec::Gzip { level: 6 }.name(),
            "gzip -6",
            "Gzip level 6 name"
        );
        assert_eq!(
            Codec::Gzip { level: 1 }.name(),
            "gzip -1",
            "Gzip level 1 name"
        );
        assert_eq!(
            Codec::Zstd { level: 3 }.name(),
            "zstd -3",
            "Zstd level 3 name"
        );
        assert_eq!(
            Codec::Zstd { level: -7 }.name(),
            "zstd --7",
            "Zstd negative level name"
        );
        let samples = small_samples(100);
        let dict = train_dictionary(&samples, 8 * 1024).unwrap();
        let dict_len = dict.len();
        let prepared = Arc::new(PreparedDict::new(dict, 3));
        let name = Codec::ZstdDict(prepared).name();
        // Pattern: "zstd -3 +dict(NB)" where N is the actual dict length.
        assert!(
            name.starts_with("zstd -3 +dict("),
            "ZstdDict name must start with 'zstd -3 +dict(', got: {}",
            name
        );
        assert!(
            name.ends_with("B)"),
            "ZstdDict name must end with 'B)', got: {}",
            name
        );
        assert!(
            name.contains(&format!("{}B)", dict_len)),
            "ZstdDict name must contain dict size, got: {}",
            name
        );
    }

    // [OPUS-4.8] sq-qcnn.15: DIRECT tests for pushed() on both sinks — kills the
    // `pushed -> 0` and `pushed -> 1` whole-fn mutants that the other roundtrip
    // tests never assert on (they care about member/piece counts, not the push
    // counter itself).
    #[test]
    fn compressed_sink_pushed_tracks_count() {
        let codec = Codec::Zstd { level: 1 };
        for mode in [Mode::Serial, Mode::Parallel] {
            let mut sink = CompressedSink::new(codec.clone(), mode);
            assert_eq!(sink.pushed(), 0, "zero before any push ({:?})", mode);
            sink.push(b"a");
            assert_eq!(sink.pushed(), 1, "one after first push ({:?})", mode);
            sink.push(b"bb");
            assert_eq!(sink.pushed(), 2, "two after second push ({:?})", mode);
            sink.push(b"ccc");
            assert_eq!(sink.pushed(), 3, "three after third push ({:?})", mode);
            // finish consumes the sink; count was 3 for all those members.
            let members = sink.finish().unwrap();
            assert_eq!(members.len(), 3, "three members drained ({:?})", mode);
        }
    }

    // [OPUS-4.8] sq-qcnn.15: DIRECT test for SingleMemberGzipSink::pushed() —
    // kills the `pushed -> 0` and `pushed -> 1` whole-fn mutants.
    #[test]
    fn single_member_gzip_sink_pushed_tracks_count() {
        for mode in [Mode::Serial, Mode::Parallel] {
            let mut sink = SingleMemberGzipSink::new(6, mode);
            assert_eq!(sink.pushed(), 0, "zero before any push ({:?})", mode);
            sink.push(b"x");
            assert_eq!(sink.pushed(), 1, "one after first push ({:?})", mode);
            sink.push(b"yy");
            assert_eq!(sink.pushed(), 2, "two after second push ({:?})", mode);
            sink.push(b"zzz");
            assert_eq!(sink.pushed(), 3, "three after third push ({:?})", mode);
            let _pieces = sink.finish().unwrap();
        }
    }

    // [OPUS-4.8] sq-qcnn.15: chunk-boundary correctness — verifies that each
    // compressed member/frame decodes to EXACTLY the corresponding input chunk
    // (not just that the concatenation round-trips).  This kills index-offset
    // and cursor arithmetic mutants that could swap members while preserving the
    // overall byte sequence.
    #[test]
    fn each_member_decodes_to_its_own_chunk() {
        let chunks = json_chunks(10);
        // Gzip multi-member: each member decompresses to its chunk.
        for mode in [Mode::Serial, Mode::Parallel] {
            let members = run_sink(&chunks, Codec::Gzip { level: 6 }, mode);
            assert_eq!(
                members.len(),
                chunks.len(),
                "member count ({:?})",
                mode
            );
            for (i, (member, chunk)) in members.iter().zip(&chunks).enumerate() {
                let decoded = decode_gzip_concat(member).unwrap();
                assert_eq!(
                    decoded, *chunk,
                    "gzip member {} must decode to its chunk ({:?})",
                    i, mode
                );
            }
        }
        // Zstd multi-frame: each frame decompresses to its chunk.
        for mode in [Mode::Serial, Mode::Parallel] {
            let members = run_sink(&chunks, Codec::Zstd { level: 3 }, mode);
            assert_eq!(
                members.len(),
                chunks.len(),
                "frame count ({:?})",
                mode
            );
            for (i, (frame, chunk)) in members.iter().zip(&chunks).enumerate() {
                let decoded = decode_zstd_concat(frame).unwrap();
                assert_eq!(
                    decoded, *chunk,
                    "zstd frame {} must decode to its chunk ({:?})",
                    i, mode
                );
            }
        }
    }

    // [OPUS-4.8] sq-qcnn.15: truncated / corrupt stream errors are reported, not
    // silently swallowed — verifies that the decode functions surface io::Error
    // on malformed input rather than returning Ok with garbage.
    #[test]
    fn decode_errors_on_truncated_streams() {
        // gzip concat: truncated magic
        assert!(
            decode_gzip_concat(b"\x1f\x8b").is_err(),
            "truncated gzip header must error"
        );
        // gzip concat: completely wrong bytes
        assert!(
            decode_gzip_concat(b"this is not gzip").is_err(),
            "non-gzip bytes must error"
        );

        // zstd concat: truncated magic
        assert!(
            decode_zstd_concat(b"\x28\xb5\x2f").is_err(),
            "truncated zstd frame must error"
        );
        // zstd concat: wrong bytes
        assert!(
            decode_zstd_concat(b"not zstd at all").is_err(),
            "non-zstd bytes must error"
        );

        // zstd with dict: truncated
        assert!(
            decode_zstd_concat_with_dict(b"\x28\xb5\x2f", b"dictbytes").is_err(),
            "truncated zstd-dict frame must error"
        );
    }

    // [OPUS-4.8] sq-qcnn.15: byte-equality between parallel and serial for
    // SingleMemberGzipSink — each OUTPUT PIECE must be byte-identical, not just
    // the concatenated wire stream.  Kills piece-order / header-flag mutants.
    #[test]
    fn single_member_gzip_pieces_byte_equal_serial_vs_parallel() {
        let chunks = json_chunks(8);
        for level in [1u32, 6] {
            let serial_pieces = {
                let mut sink = SingleMemberGzipSink::new(level, Mode::Serial);
                for c in &chunks {
                    sink.push(c);
                }
                sink.finish().unwrap()
            };
            let parallel_pieces = {
                let mut sink = SingleMemberGzipSink::new(level, Mode::Parallel);
                for c in &chunks {
                    sink.push(c);
                }
                sink.finish().unwrap()
            };
            assert_eq!(
                serial_pieces.len(),
                parallel_pieces.len(),
                "piece count must match for level {}",
                level
            );
            for (i, (s, p)) in serial_pieces.iter().zip(&parallel_pieces).enumerate() {
                assert_eq!(
                    s, p,
                    "piece {} must be byte-identical (serial vs parallel, level {})",
                    i, level
                );
            }
        }
    }

    // [OPUS-4.8] sq-qcnn.15: verify the GZIP_HEADER constant bytes — the first
    // piece from SingleMemberGzipSink must start with the RFC 1952 ID bytes
    // 0x1f 0x8b and have the right CM/OS fields.
    #[test]
    fn single_member_gzip_wire_starts_with_rfc1952_header() {
        let pieces = gzip_single_member(&[b"hello".as_ref()], 6, Mode::Serial).unwrap();
        let wire: Vec<u8> = pieces.iter().flat_map(|p| p.iter().copied()).collect();
        // RFC 1952 §2.3.1: ID1=0x1f ID2=0x8b CM=8 (deflate).
        assert_eq!(wire[0], 0x1f, "ID1 must be 0x1f");
        assert_eq!(wire[1], 0x8b, "ID2 must be 0x8b");
        assert_eq!(wire[2], 8, "CM must be 8 (deflate)");
        // OS byte (index 9) in our fixed header is 0xff (unknown) — pin it.
        assert_eq!(wire[9], 0xff, "OS byte must be 0xff (unknown)");
        // Wire must be decodable by a single-member decoder.
        assert_eq!(decode_single_member(&wire), b"hello");
    }

    // [OPUS-4.8] sq-qcnn.15: try_drain must return the IMMEDIATELY READY prefix —
    // kills `CompressedSink::try_drain -> Ok(vec![])` (the whole-fn survivor that
    // "works" by deferring everything to finish).  In Serial mode every push
    // immediately inserts into the ready map, so try_drain must hand back that
    // member without waiting for finish.
    #[test]
    fn compressed_sink_try_drain_returns_ready_members_immediately() {
        let mut sink = CompressedSink::new(Codec::Zstd { level: 1 }, Mode::Serial);
        // Push one chunk; in Serial mode the member is ready immediately.
        sink.push(b"first chunk");
        let first_drain = sink.try_drain().unwrap();
        assert_eq!(
            first_drain.len(),
            1,
            "try_drain must return the 1 ready member after 1 serial push"
        );
        assert_eq!(
            decode_zstd_concat(&first_drain[0]).unwrap(),
            b"first chunk",
            "drained member must decompress to the pushed chunk"
        );

        // Push a second chunk; try_drain returns it too.
        sink.push(b"second chunk");
        let second_drain = sink.try_drain().unwrap();
        assert_eq!(
            second_drain.len(),
            1,
            "try_drain must return the 2nd ready member"
        );
        assert_eq!(
            decode_zstd_concat(&second_drain[0]).unwrap(),
            b"second chunk",
            "second drained member must decompress correctly"
        );

        // finish with no remaining members (all already drained).
        let tail = sink.finish().unwrap();
        assert!(
            tail.is_empty(),
            "finish must be empty when all members were already drained"
        );
    }

    // [OPUS-4.8] sq-qcnn.15: train_dictionary must return non-trivial bytes —
    // kills Ok(vec![]) / Ok(vec![0]) / Ok(vec![1]) mutations by checking both
    // that the output is non-trivial AND that it actually aids compression.
    #[test]
    fn train_dictionary_returns_non_empty_useful_bytes() {
        let samples = small_samples(300);
        let dict = train_dictionary(&samples, 16 * 1024).unwrap();
        assert!(
            dict.len() > 4,
            "trained dictionary must be non-trivial, got {} bytes",
            dict.len()
        );
        // The dict bytes must start with the zstd magic (0xEC30A437 LE) — a real
        // dict; Ok(vec![0]) or Ok(vec![1]) would not.
        let magic = u32::from_le_bytes([dict[0], dict[1], dict[2], dict[3]]);
        assert_eq!(
            magic, 0xEC30A437,
            "dict must start with zstd dictionary magic 0xEC30A437, got 0x{:08X}",
            magic
        );
    }
}
