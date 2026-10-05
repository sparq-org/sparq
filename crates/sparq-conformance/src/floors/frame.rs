//! [FABLE-5] sq-oy1f.40 — the W3C JSON-LD 1.1 `frame` lane ratchet floor
//! (relocated from `tests/jsonld_suite.rs` to this lib-side single source).

/// [FABLE-5] sq-oy1f.29 — `frame` pass floor over the SEPARATE `w3c/json-ld-framing`
/// suite (`scripts/fetch-jsonld-framing-tests.sh`), for the NATIVE document-level
/// Framing Algorithm (`sparq_jsonld::frame::frame()` — expand → frameExpansion →
/// node map + `@merged` → frame matching → prune → compact against the frame's
/// `@context`). RATCHET: may only RISE. This is the MEASURED pass count at the
/// pinned framing-suite revision.
///
/// ## Oracle (re-pinned side-by-side, sq-oy1f.29)
///
/// * **Old (RDF-first framer, `graph_to_jsonld_framed`): 61/92** under the
///   RDF-answer-equivalence oracle (`reparse(frame(D, F)) ≡ reparse(expected)`), with
///   3 SKIPs (the suite's NegativeEvaluationTests — that framer was TOTAL and never
///   raised the spec's frame-validation errors) and 28 honest divergences
///   (value-pattern matching over `@value` alternative arrays, `@explicit`/`@default`
///   fill, named-graph `@graph` framing shapes, `@list`/`@set` re-emit, blank-node
///   `@embed` table edges).
/// * **New (native pipeline, THIS floor): 92/92** under the STRONGER normative
///   document oracle — the framed output is deep-compared to the suite's expected
///   document with `json_ld_equal` (object key order insignificant; array order
///   significant only inside `@list`), and the 3 NegativeEvaluationTests are RUN
///   (pass iff `frame()` raises exactly the manifest's `expectErrorCode`:
///   `invalid frame`, `invalid @embed value`), not skipped.
///
/// All 28 old-framer divergences are resolved and the negatives are modelled, so the
/// lane holds a full score with ZERO skips at the pinned revision. Documented
/// behavioural fallbacks that do NOT currently cost a case: `@embed: @link` is
/// treated as `@once` (design record §11 — no output-tree object identity), and
/// `@embed: @last` uses an embed-then-demote post-pass equivalent to the reference
/// processors' remove-embed.
///
/// ## [OPUS-5] sq-gzsky — this floor does NOT rise, and that is the honest answer
///
/// Bead sq-gzsky asked to raise `floors::{expand,frame,compact}::FLOOR` off the
/// sq-hmd7l.22 finding, which recorded frame at **66.3%**. That cell was already STALE
/// when the bead was written: sq-oy1f.29 moved this lane to the native framer and
/// `research/gap-jsonld-conformance-2026-07.md` §3 corrected it to 100.0% (the correction
/// predates the bead). The lane is at the pinned suite's CEILING — 92 pass of 92 entries,
/// 0 fail, 0 skip, negatives already RUN — so there is no headroom to ratchet into and
/// raising the const to anything above 92 would be an ASPIRATIONAL floor, not a measured
/// one. The expand and compact halves of sq-gzsky did rise (276 → 381, 228 → 243); this
/// one stays put until the framing-suite pin bumps and adds entries.
pub const FLOOR: usize = 92;
