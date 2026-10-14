//! sq-oy1f.26 — the W3C JSON-LD 1.1 `flatten` lane ratchet floor
//! (lib-side single source; imported by the runner + scoreboard + ci grep).

/// sq-oy1f.26 — `flatten` pass floor over the `flatten` category of
/// `w3c/json-ld-api`, under the NATIVE DOCUMENT-LEVEL oracle
/// (`sparq_jsonld::flatten()` + `json_ld_equal`). RATCHET: may only RISE.
///
/// ## Oracle re-pin (side-by-side, per the sq-oy1f honesty rule)
///
/// The lane MOVED from the RDF-writer round-trip oracle to the native Flattening
/// Algorithm (JSON-LD 1.1 API §7.1, bead sq-oy1f.26): `flatten()` expands the input,
/// generates the node map (§7.2), folds named graphs under `@graph`, sorts by `@id`,
/// and drops empty nodes, and the result is deep-compared to the suite's NORMATIVE
/// expected document via `json_ld_equal` (object key order insignificant; array order
/// significant only inside `@list` and a `@json` payload; integers exact, non-integral as f64) — the SAME
/// comparator the `expand` lane uses (sq-kk1mq). This is a document-level oracle, not
/// an RDF round-trip, so it detects JSON-LD structure the RDF oracle could not.
///
/// * **Old oracle (RDF round-trip, `graph_to_jsonld(Flattened)`): 50 pass / 0 fail / 8 skip.**
/// * **New oracle (native `flatten()`): MEASURED 53 pass / 0 fail / 5 skip** at the
///   pinned suite revision, ON THE MERGED TREE.
///
/// ## sq-oy1f.26 rebase note — native oracle now BEATS the RDF-writer oracle
///
/// When this branch was first cut the native lane measured 46 pass / 7 fail (below the
/// old writer oracle's 50), because the 7 fails were ALL inherited gaps of the native
/// `expand()` it composes over (flatten = expand ∘ node-map ∘ fold), NOT flatten-algorithm
/// bugs. Those exact gaps were the ones `sq-oy1f.37` fixed on `main` (value-object `@type`
/// collapse + empty-array-property retention, raising the expand floor 240 → 259):
///
/// * `#t0002/#t0013/#t0028/#t0036` — the old `invalid typed value` errors (fixed by the
///   `@type`-collapse expand fix).
/// * `#t0004/#t0015/#t0016` — the dropped empty-array properties `"…set1": []` (fixed by
///   the empty-array-property-retention expand fix).
///
/// Merging `main` (sq-oy1f.37) into this branch flips all 7 inherited fails to passes, so
/// the native flatten oracle now MEASURES 53 pass / 0 fail — ABOVE the old writer oracle's
/// 50. The oracle re-pin is therefore a net RISE (50 → 53), taking the union: the native
/// document-level oracle AND the sq-oy1f.37 expand raises it composes over.
///
/// The 5 SKIP are the documented buckets: 1 NegativeEvaluationTest, the JSON-LD-1.0-only
/// positives, and 1 compaction case (`#t0044`, which supplies a `context` member —
/// post-flatten compaction is the separate document-level Compaction bead sq-oy1f.27).
///
/// RATCHET after this re-pin: rise-only.
pub const FLOOR: usize = 53;
