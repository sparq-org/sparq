//! [FABLE-5] sq-oy1f.40 — the W3C JSON-LD 1.1 `expand` lane ratchet floor
//! (relocated from `tests/jsonld_suite.rs` to this lib-side single source).

/// [SONNET-4.6] sq-kk1mq (oracle-correction re-baseline; supersedes the old
/// sq-oy1f RDF-equivalence oracle) — `expand` pass floor over the
/// `expand` category of `w3c/json-ld-api`, now measured with the NATIVE
/// DOCUMENT-LEVEL oracle: `sparq_jsonld::expand(input, opts, &NoopLoader)`
/// produces a `Json` value; that value is deep-compared against the suite's
/// expected expanded document using the runner's `json_ld_equal` comparator
/// (object key order insignificant; array order SIGNIFICANT only inside `@list`
/// values, insignificant elsewhere; numeric equality as f64).
///
/// ## Oracle-correction rationale
///
/// The OLD oracle (sq-oy1f, merged before this bead) compared the RDF dataset
/// produced by the engine's JSON-LD WRITER (`graph_to_jsonld(Expanded)`) against
/// the RDF produced by re-parsing the suite's expected document. That oracle
/// measured the writer's RDF fidelity, not the expansion algorithm. A case could
/// "pass" by coincidence (writer happened to produce equivalent RDF) even when the
/// expansion JSON shape differed from the spec, or fail spuriously when the input
/// had no RDF projection but the expander output was correct. The new oracle
/// measures JSON-LD data-model (semantic) equivalence — order-insensitive outside
/// `@list` per bead sq-kk1mq — NOT structural identity with the reference output.
///
/// NOTE: ~18 of the 240 passes are semantically-equal-but-reordered vs. the W3C
/// reference (@type order #tpr30, @id-map #tm001, @index #tpi06, multi-value
/// #tn004/#t0030 families); a strict order-sensitive harness would fail these 18
/// (strict-ordered count = 222).  The comparator is intentionally order-insensitive
/// per the JSON-LD data model (elements outside `@list` are a set).
///
/// This is an HONEST REBASE, not a ratchet weakening: the new floor may be lower
/// than 247 (old oracle measured 247/385) because some old passes were oracle
/// artefacts (the writer round-tripped to equivalent RDF while the JSON was wrong).
/// The gain is precision: passes now mean the expander produced a semantically
/// correct expanded document (data-model equivalence, not byte-identical structure).
/// See `run_expand_native` for the skip buckets and the PR body for the full
/// old-vs-new breakdown (bead sq-kk1mq).
///
/// ## What changed from the old oracle
///
/// * OLD (sq-oy1f, EXPAND floor = 247): RDF-equivalence —
///   `reparse_rdf(write_expanded(ingest_rdf(input))) ≡ reparse_rdf(expected)`.
///   Forwarded NO options; skipped empty-RDF inputs, 1.0-mode cases.
/// * NEW (sq-kk1mq, EXPAND floor = measured): document-level JSON comparison —
///   `json_ld_equal(expand(input, opts), expected_json)`. Forwards base,
///   expandContext, processingMode from the manifest; attempts all positive tests
///   regardless of RDF projection emptiness.
///
/// ## Honest SKIP buckets (recorded, not passed, not failed)
///
/// * `requires` optional-feature cases — out of the gated surface (same as before).
/// * NegativeEvaluationTests — sparq's expander raises some spec errors but
///   error-code completeness is unverified; deferred to a child bead of sq-oy1f.
///   SKIP (honest), never a counted pass.
/// * Remote `input` URLs — no network (SKIP).
/// * No `expect` file — nothing to compare (SKIP).
///
/// ## RATCHET: may only RISE
///
/// This floor is the MEASURED pass count with the new oracle at the pinned
/// suite revision. It is NOT aspirational. Future rises land when the native
/// expander fixes known divergences (tracked as children of sq-oy1f).
/// MEASURED 240/385 at the pinned revision under the new document-level oracle
/// (sq-kk1mq): 240 pass / 36 fail / 109 skip.  Breakdown vs. old oracle (247/10/128):
///   - 7 fewer passes (247→240): NET of 20 old-pass→new-fail flips (old oracle over-passed;
///     writer happened to produce equivalent RDF while JSON structure differed from spec),
///     offset by 13 recoveries: 8 old-fail→new-pass via oracle precision (new oracle
///     correctly passes cases the old oracle spuriously failed), plus 5 old-skip→new-pass
///     via options forwarding (processingMode/expandContext now forwarded; previously skipped)
///   - 26 more fails (10→36): honest divergences the new oracle reveals (JSON-level
///     mismatches invisible to the old RDF oracle, options-driven, or @direction shapes)
///   - 19 fewer skips (128→109): the old 1.0-mode and empty-RDF skip buckets are gone
///     (the new oracle forwards processingMode and doesn't require a non-empty RDF
///     projection); the 109 that remain are the NegativeEvaluationTests (deferred)
///
/// ## [FABLE-5] sq-oy1f.37 — expand() correctness raise 240 → 259
///
/// Three spec-faithful expander fixes (`crates/sparq-jsonld/src/expand.rs`) flip 19
/// previously-failing positive cases to pass (measured 259 pass / 17 fail / 109 skip):
///
/// * **value-object `@type` collapse** — a value object's `@type` is a SINGLE value in
///   the JSON-LD data model; the general keyword path arrayifies it, so cleanup now
///   collapses a single-element `@type` array back to a scalar. This cleared the false
///   `invalid typed value` errors on positive cases (W3C expand/0002, 0013, 0014, 0028,
///   0036, 0046, 0077, c020, js15/16/19/20, tn02).
/// * **empty-array-property retention** — the forward-property add now uses the
///   `addValue` `asArray = true` rule (per the reference `_addValue(..., propertyIsArray
///   true)`), so a `@set`/`@list`/plain-array term whose value expands to `[]` RETAINS
///   the property as an empty array instead of dropping it (W3C expand/0004, 0015, 0016,
///   plus the flatten cases that inherit `expand()`).
/// * **free-floating value/list drop** — the null/`@graph` active-property drop (step 19)
///   now reaches value objects (any map with `@value`) and list objects, not just
///   empty/`@id`-only node objects (W3C expand/0045, 0046).
///
/// The 17 remaining fails are 7 remote-`@context` positives (need an `FsLoader` — no
/// network under `NoopLoader`) and 10 deeper divergences deferred to follow-up beads
/// (`@id: null` retention t0122, relative-IRI-with-colon t0109, rdf-star reverse
/// `@index` t0131, `invalid IRI mapping` / `invalid scoped context` shapes).
///
/// ## [SONNET-4.6] sq-oy1f.45 — expand() correctness raise 259 → 276
///
/// Six additional spec-faithful fixes flip 17 previously-failing positive cases to pass
/// (measured 276 pass / 0 fail / 109 skip):
///
/// * **FsLoader wiring** — the expand test harness now uses `FsLoader` (maps the W3C
///   suite URL prefix to the local fixture directory) instead of `NoopLoader`, so
///   `@context` / `@import` relative-URL references in test inputs are resolved from
///   the checked-out files. Fixes t0126, t0127, t0128, tc031, tso08, tso09, tso11,
///   tc034, tso05, tso06 (10 cases).
/// * **`@id: null` retention** — `@id` with a string value that expands to nothing
///   (keyword-form strings that don't map to an IRI) now emits `"@id": null` per
///   §5.1.2 step 13.4.1, instead of omitting the property. Fixes t0122.
/// * **Relative-IRI-with-colon** — `expand_iri` (step 6) now validates the prefix
///   against RFC 3986 scheme rules (`is_valid_scheme`) before treating a colon-
///   containing term as a compact or absolute IRI. Prefixes starting with `#`, `?`,
///   etc. fall through to vocab/base resolution. Fixes t0109.
/// * **`@nest` property-scoped context** — step 14 now applies any property-scoped
///   `@context` declared on a `@nest`-aliased term (using `propagate = true` so the
///   context persists into child node objects). Fixes tc037 and tc038.
/// * **`@reverse` + `@index`** — `create_reverse_definition` now processes `@index`
///   before storing the definition, matching the behaviour of `finish_definition`
///   for forward terms with `@container: [@reverse, @index]`. Fixes t0131.
/// * **Invalid IRI mapping in 1.0 mode** — the round-trip check in
///   `create_term_definition` (§4.2.2 step 14.3.3) is now gated on JSON-LD 1.1
///   mode and skipped when the expanded value is itself a keyword (e.g. `@type`),
///   matching the spec's 1.1-only semantics. Fixes t0026 and t0071.
///
/// ## [OPUS-5] sq-gzsky — the NEGATIVE lane lands; 276 → 381 (skips 109 → 0)
///
/// The 109-case skip bucket above WAS the entire expand gap
/// (`research/gap-jsonld-conformance-2026-07.md` §6, lever sq-oy1f.31). It is now closed:
/// the runner RUNS every `NegativeEvaluationTest` — a case passes iff `expand()` raises
/// EXACTLY the manifest's `expectErrorCode` (a WRONG code is a FAIL, never a pass), the
/// same oracle shape the `frame` lane has used since sq-oy1f.29. Wiring alone measured
/// 371/14/0; seven spec-faithful `sparq-jsonld` fixes took it to **381 pass / 4 fail /
/// 0 skip** of 385:
///
/// * **`@included` arrayification** (§5.1.2 step 13.4.13) — a DROPPED expansion (`None`)
///   is not an empty array; arrayifying it yields one non-node element, so
///   `@included: "string"` / `{"@value": …}` / `{"@list": […]}` raise
///   `invalid @included value` instead of vacuously succeeding (in07, in08, in09).
/// * **`@type` + `@direction`** (step 15.1) — a value object "must not contain an `@type`
///   entry if it contains either `@language` or `@direction`"; only the `@language` half
///   was checked (di09).
/// * **datatype-IRI validation** (step 15.4) — `is_absolute_iri` accepted any
///   scheme-prefixed string, so `"http://example.com/baz z"` (a SPACE) passed; it now
///   admits only the RFC 3987 `unreserved`/`gen-delims`/`sub-delims`/`ucschar`/`iprivate`
///   code points and requires every `%` to open a well-formed `pct-encoded` triplet (0123).
///   It validates the IRI *code-point* grammar, not the full structural grammar — see
///   `sparq_jsonld`'s `has_only_iri_chars` for the stated scope.
/// * **blank-node datatype** (step 15.4) — `@type: "_:dt"` is not an IRI, so it is an
///   `invalid typed value` outside frame expansion (er40).
/// * **term round-trip vs a keyword** (§4.2.2 step 14.3.3) — the check was skipped when the
///   IRI mapping was a keyword, which is exactly the case 1.1 forbids; the 1.0/1.1 split is
///   the `ProcessingMode::JsonLd10` gate alone, so the 1.0 twin `#t0026` still passes (er43).
/// * **`@container` array in 1.0 mode** (§4.2.2 step 21.2) — a container value that "is
///   otherwise not a string" is an `invalid container mapping` under
///   `processingMode: json-ld-1.0`, so `["@set"]` no longer normalises to a legal `@set`
///   (es01; the same fix takes compact `#tep12`).
/// * **relative `@vocab` in 1.0 mode** (§4.1.2 step 5.8) — resolving a relative reference
///   (including `""`) against `@base`/the current `@vocab` is the 1.1 relaxation; in 1.0
///   it is an `invalid vocab mapping` (0115, 0116). The 1.1 positive `#t0112` is unaffected.
///
/// ### The 4 remaining FAILS — honest, itemised, NOT skipped
///
/// All four expect a JSON-LD **1.0** error code that the 1.1 REC REMOVED from the
/// `JsonLdErrorCode` registry, so `sparq_jsonld::JsonLdErrorCode` (a deliberately CLOSED
/// mirror of that registry) cannot name them:
///
/// * `#ter02`, `#ter03` — `recursive context inclusion`. 1.1 replaces the cyclic-remote-
///   context error with `context overflow` on a processor-defined recursion limit
///   (§4.1.2 step 5.2.3); sparq's loader resolves the cycle rather than raising either.
/// * `#ter24`, `#ter32` — `list of lists`. 1.1 ALLOWS a list of lists outright — the code
///   is absent from the registry and the shape is legal — so a REC-conformant 1.1
///   processor expanding in 1.0 mode has nothing to raise.
///
/// They are recorded as FAILS rather than absorbed into a skip bucket so they stay visible
/// in the runner's failure listing (the `#t0038` precedent narrowly pins one exact id; a
/// blanket `specVersion: json-ld-1.0` skip was rejected in review and is not reintroduced
/// here). Deciding whether to model the retired 1.0 codes is follow-up work, not a floor
/// concern: the floor moves with the PASS count.
pub const FLOOR: usize = 381;
