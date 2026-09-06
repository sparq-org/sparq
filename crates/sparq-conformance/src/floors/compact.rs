//! [FABLE-5] sq-oy1f.40 — the W3C JSON-LD 1.1 `compact` lane ratchet floor
//! (relocated from `tests/jsonld_suite.rs` to this lib-side single source).

/// [FABLE-5] sq-oy1f.27 — `compact` pass floor, RE-PINNED for the ORACLE
/// CORRECTION to the native document-level Compaction Algorithm
/// (`sparq_jsonld::compact::compact()`), compared against the suite's
/// NORMATIVE EXPECTED document via `json_ld_equal` — the same oracle shape as
/// the `expand` (sq-kk1mq) and `flatten` (sq-oy1f.26) lanes. RATCHET: may only
/// RISE from here.
///
/// ## Side-by-side re-pin (old oracle vs new oracle, same pinned suite rev)
///
/// * OLD (sq-3uos5/sq-oy1f.16, floor **186**): the engine's RDF-first writer
///   (`graph_to_jsonld_compact`) + an **oxjsonld self-reparse round-trip**
///   (`reparse(compact(D, ctx)) ≡ D`) — measured 186 pass / 35 fail / 25 skip.
///   That oracle measured RDF losslessness of the writer, NOT the Compaction
///   Algorithm: it never compared against the suite's expected document, could
///   pass shapes a strict third-party processor reads differently (the
///   sq-oy1f.10 data-loss class), and had to SKIP every case whose features the
///   writer never emits (scoped/typed contexts, `@nest`, `@index`/`@id` maps,
///   non-inline contexts, empty-RDF inputs).
/// * NEW (sq-oy1f.27, floor **228**): the spec Compaction Algorithm over the
///   natively expanded document, deep-compared to the W3C EXPECTED document —
///   measured **228 pass / 0 fail / 18 skip** of 246 (sq-uzdw7 re-pin; the
///   sq-oy1f.27 measurement was 228/1/17 with `#t0038` a documented below-floor
///   FAIL, since reclassified — see below; the pass count and floor are
///   unchanged). The 42-case rise over the old floor is real algorithm coverage
///   (scoped/typed contexts, `@nest`, container maps, `@graph` containers,
///   options forwarding), not an oracle relaxation: the new oracle is strictly
///   harder (document equivalence vs self-reparse). Passes also hold under
///   STRICT order-sensitive JSON equality (strict-ordered count = 228 — no pass
///   relies on the comparator's outside-`@list` array order-insensitivity).
///
/// ## The 18 honest SKIPs (0 fails at the pin)
///
/// * `#t0038` (`specVersion: json-ld-1.0`, the ONLY 1.0-only positive at the
///   pin) expects 1.0-era compact-IRI creation through an EXPANDED (map-valued)
///   term definition (`body:/format`). A REC-conformant 1.1 processor never
///   sets the prefix flag on an expanded term — in either processing mode
///   (`compact/p001` pins exactly that for 1.0 mode, so "fixing" `#t0038`
///   would break `#tp001`) — so this expectation is unreachable; jsonld.js and
///   pyld make the same trade, and the suite's own `vocab.jsonld` defines
///   `specVersion` as "the JSON-LD version to which the test applies".
///   DECISION (bead sq-uzdw7): reclassified from a documented below-floor FAIL
///   to an honest SKIP, NARROWLY PINNED to this exact manifest id (never a
///   blanket `specVersion: json-ld-1.0` match): the runner's
///   `is_pinned_1_0_only_skip` matches `#t0038` alone, its
///   `t0038_skip_is_narrowly_scoped` test enforces that scope, the
///   `processingMode: json-ld-1.0` cases of the 1.1 suite still RUN, and any
///   FUTURE 1.0-only positive added at a suite-pin bump RUNS (and fails the
///   scope test) rather than being silently skipped.
/// * 17 SKIPs = the NegativeEvaluationTests: compaction raises `invalid @nest
///   value`, but error-code completeness across context processing, expansion,
///   and compaction is unverified; the negative ratchet lanes are bead
///   sq-oy1f.31. SKIP (honest), never a counted pass.
///
/// ## [OPUS-5] sq-gzsky — the NEGATIVE lane lands; 228 → 243 (skips 18 → 1)
///
/// The 17-case negative skip bucket above is closed (lever sq-oy1f.31 in
/// `research/gap-jsonld-conformance-2026-07.md` §6). The runner RUNS every
/// `NegativeEvaluationTest`: a case passes iff `compact()` — expansion + context
/// processing + compaction — raises EXACTLY the manifest's `expectErrorCode`, with a WRONG
/// code a FAIL rather than a pass. Measured **243 pass / 2 fail / 1 skip** of 246. Wiring
/// alone measured 242/3/1; `#tep12` came from the `@container`-array-in-1.0 fix landed for
/// the expand lane (§4.2.2 step 21.2 — see the expand floor doc for the full fix list).
/// The one remaining SKIP is `#t0038`, unchanged and still narrowly pinned.
///
/// ### The 2 remaining FAILS — honest, itemised, NOT skipped
///
/// * `#te001` — expects `compaction to list of lists`, a JSON-LD **1.0** error code the 1.1
///   REC REMOVED from the `JsonLdErrorCode` registry (1.1 allows a list of lists), so the
///   closed `sparq_jsonld::JsonLdErrorCode` mirror cannot name it. Same class as expand
///   `#ter24`/`#ter32`.
/// * `#te002` — expects `IRI confused with prefix`. This one is a REAL 1.1 gap, not a
///   retired code: IRI Compaction step 8 aborts when the scheme of an IRI with no `//`
///   authority matches a `@prefix`-flagged term (`tag:champin.net,2019:prop` against a
///   `tag` prefix term). `context::inverse::compact_iri` is infallible (`-> String`), so
///   raising it needs a fallible signature threaded through the compaction walk — a
///   scoped follow-up, deliberately not folded into this floor rise.
pub const FLOOR: usize = 243;
