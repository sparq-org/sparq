# zkSPARQL fragment extension: the greatest reasonable OWA-conforming, federation-free SPARQL subset

<!-- [OPUS-4.8] sq-5reoy (#1599): the in-tree `zk/ieee754` and `zk/xpath` Noir trees were externalized to the `sparq-org/noir_IEEE754` (v0.10.0) and `sparq-org/noir_XPath` (v0.2.0) face repos and REMOVED from this repo. Child bead `sq-3kd2g.4` (estate gap-fill) and the `SPARQL_COVERAGE.md` reference now target the `sparq-org/noir_XPath` face repo rather than an in-tree `zk/xpath/` path. Any `zk/xpath/…` / `zk/ieee754/…` path below is a HISTORICAL in-tree reference — the live source is the corresponding face repo. -->

<!-- [FABLE-5] Design record for epic sq-3kd2g / GitHub issue #1591 (maintainer directive
     2026-07-05). Decomposition-only: this record defines the extended fragment and cuts
     the implementation into disjoint child beads; it changes no code. -->

**Status:** decomposition design (proposed; nothing below is implemented unless explicitly
marked as such). **Epic:** `sq-3kd2g` / [#1591]. **Spec under amendment:** the zkSPARQL
Unofficial Proposal Draft (PR #1509, bead `sq-vvu9d`) — this record designs against its
technical content and does **not** edit it while its language pass + review are in flight.

**Honesty framing (load-bearing).** Everything here inherits the project's standing ZK
soundness posture: the v1 verifier stack is *internally re-audited* but **external
accredited-cryptographer sign-off is pending** (`sq-qhy4`); nothing in this program may be
described as a proven cryptographic guarantee. Every child bead's invariant is phrased as
"matches the spec / fail-closed / re-audited-pending-external", never "sound". All
soundness-sensitive fragments below are tiered `opus` and flagged **maintainer-arm**.

## 1. The problem and the maintainer directive

The provable fragment today is deliberately tiny. Verified against the actual code
(`crates/sparq-zk/src/verify.rs`, `crates/sparq-zk-compose`, `zk/compose/`):

- **Accepted:** `SELECT`/`ASK` over BGP scans (per-graph commitment recompute, row
  soundness, per-scan completeness in-circuit), datatype-bucketed value `FILTER` lanes
  (`filter_int` / `filter_f64` / `filter_signed_int` / `filter_decimal`, plus the
  off-by-default `dual-leaf` value-dictionary lanes), single equality `Join` across hidden
  credentials (`join_eq`), and the membership-indifferent modifiers
  (projection / `DISTINCT` / `REDUCED` / `LIMIT`-`OFFSET`).
- **Rejected fail-closed** (`VerifyError::UnsupportedFragment`): property paths,
  `OPTIONAL`, `UNION`, `MINUS`, `GRAPH`, `VALUES`, `BIND`, `ORDER BY`, aggregates,
  subqueries, `SERVICE`, `EXISTS`/`NOT EXISTS`, `CONSTRUCT`, `DESCRIBE`.

The maintainer directive ([#1591], verbatim in the issue): extend the fragment — in both
the spec/paper and the implementation — to "the greatest reasonable subset of SPARQL
(probably anything that conforms to the open world assumption, and does not require sparql
federation)", explicitly including property paths and most filter expressions.
Plus: a **pure single-prover zk-architecture paper** with performance + security analysis
(verified today: it does not exist — `cozk-witness-validation.typ` is the
collaborative-path negative result, `verifiable-fed-sparql.typ` is the SoK).

A useful pre-existing map: `bench/zk-compose/sparql_feature_catalog.json` (sq-1s2.1.2)
already catalogues 26 feature queries as covered / partial / gap. Caveat verified while
grounding this record: its "covered" rows for path sequence/inverse (Q13/Q14) mean *the
desugared BGP form compiles to existing circuits* — the path **syntax** is still rejected
by the verifier gate. The catalog measures circuit-primitive coverage, not accepted query
syntax; the fragment gate is the authority on what a verifier accepts.

## 2. The fragment principle: what "OWA-conforming" buys us

The proved correctness property (spec §7.2, `bind_query_correctness`) is **result
membership**: a manifest disclosing a solution mapping μ (or claiming one exists) for
pattern P over committed graphs G1…Gn is correct iff μ ∈ eval(P). The extension principle
is: admit exactly the constructs for which result membership is

- **(P1) monotone** — a membership witness remains valid when the world contains more
  data. This is the precise content of "conforms to the open world assumption": within one
  committed graph, scan completeness *is* proved in-circuit, but composed-pattern
  completeness is not, and the deployment model (a holder presents a *subset* of their
  credentials) is intrinsically open-world. Non-monotone constructs (closed-world
  negation, completeness-dependent semantics) would let additional undisclosed data
  falsify a "proved" answer — they are excluded on semantics, not on circuit cost.
- **(P2) federation-free** — evaluation ranges only over the graphs committed to the
  prover; no `SERVICE`.
- **(P3) privacy-model compatible** — nothing that forces disclosing what the trust model
  hides (named-graph attribution ⇒ no `GRAPH`).
- **(P4) circuit-realizable with honest bounds** — unbounded recursion (path closure) is
  admitted only under an explicitly disclosed bound, and the *proved statement is the
  bounded one* (§4). No construct is admitted whose circuit statement is weaker than what
  the manifest implies to a consumer.

This is the maximal-monotone-fragment position: SPARQL's monotone core is
BGP + `FILTER` (error-as-unsatisfied) + `Join` + `UNION` + property paths + `VALUES` +
deterministic `BIND` + subqueries composed of the same, under set semantics. That is what
we admit, phased.

## 3. The extended fragment: feature table

Every SPARQL 1.1/1.2 construct, with disposition and reason. "IN (today)" =
implemented-and-verified on `main`; "IN (phase N)" = designed here, to be implemented by
the child beads of §7; "DEFERRED" = admissible in principle, explicitly out of this
program with re-entry criteria; "OUT" = excluded with the reason stated.

### 3.1 Query forms

| Construct | Disposition | Reason |
|---|---|---|
| `SELECT` | IN (today) | Membership property defined over solution mappings. |
| `ASK` | IN (today) | Non-emptiness of eval(P); monotone. |
| `CONSTRUCT` | OUT | The result form is a graph template instantiation, outside the membership property; a consumer can instantiate templates client-side from a proved mapping. Monotone, so re-entry is possible, but it adds no proof value. |
| `DESCRIBE` | OUT | Implementation-defined result; nothing well-defined to prove. |

### 3.2 Graph-pattern operators

| Construct | Disposition | Reason |
|---|---|---|
| BGP | IN (today) | Scan circuits: row soundness + per-scan completeness in-circuit. |
| `Join` | IN (today) | `join_eq` hidden equality join; cross-graph blank-node exclusion (Q6 guard) retained unchanged. |
| `FILTER` | IN (today: 4 numeric lanes) → IN (phases 2–3: expression fragment of §5) | Monotone given SPARQL's error-as-unsatisfied semantics. |
| `UNION` | IN (phase 2) | Monotone (eval is set union of branch evals). Circuit semantics: **per-solution branch attribution** — the manifest discloses which branch each disclosed solution witnesses; the verifier re-derives the branch pattern lists from the query text and checks the branch's sub-proofs. No new circuit primitive needed. |
| `OPTIONAL` (`LeftJoin`) | OUT | Non-monotone: a solution leaving the optional side unbound asserts *no compatible extension exists* — a closed-world claim. The bound case is semantically plain `Join`, so admitting `OPTIONAL` adds no sound expressiveness. Query authors rewrite to `Join`, or to `UNION` of explicit cases. |
| `MINUS` | OUT | Closed-world set difference; non-monotone. |
| `FILTER NOT EXISTS` | OUT | Closed-world negation; non-monotone. |
| `FILTER EXISTS` (positive) | DEFERRED | Monotone (extra data only adds matches). Deferred because SPARQL 1.1 `EXISTS` substitution semantics is famously under-specified (the SPARQL-EXISTS errata; SPARQL 1.2 clarifies); admit later against the 1.2 semantics as an extra bound scan sub-proof. Re-entry: after phase 2 lands and the 1.2 semantics is pinned in the spec amendment. |
| `GRAPH` | OUT | Contradicts graph-set privacy (plan §2.4): naming graphs discloses the hidden attribution the model exists to protect. A privacy-model exclusion, not an OWA one. |
| `SERVICE` | OUT | Federation excluded by directive (P2). |
| `VALUES` | IN (phase 2) | Inline data is monotone; the rows are public constants in the query text. Composition-level check (the verifier re-derives the rows and checks disclosed solutions against them); `UNDEF` cells are wildcards; no new circuit. |
| `BIND` (`Extend`) | IN (phase 3) | Deterministic expression over the §5 estate; monotone (adds a derived binding). Same expression evaluator as `FILTER`, output bound into the disclosed solution. Non-deterministic builtins (`RAND`, `UUID`, `STRUUID`, `BNODE`, `NOW`) stay out (§5). |
| Subquery (nested `SELECT`) | IN (phase 3) | Monotone when composed of in-fragment operators; inner projection is membership-compatible. Subquery + aggregates remains OUT (below). |
| Property paths | IN (phases 1–2, bounded semantics) | §4 — first-class soundness requirement. |
| Aggregation (`GROUP BY`, `HAVING`, `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `GROUP_CONCAT`, `SAMPLE`) | OUT | An aggregate value is a completeness claim over the *whole* pattern ("exactly these solutions and no more") — closed-world at pattern level. Composed-pattern completeness is not proved today (only per-BGP-scan completeness is), and the earlier plan also flags double-count forgeability across credentials. Re-entry criteria: a composed completeness obligation (a separate future program, not this epic). |
| `ORDER BY` | OUT (may re-enter as accept-and-strip) | Ordering is membership-*indifferent* — accepting and ignoring it would be membership-correct — but a consumer reading "ORDER BY … LIMIT 1" naturally infers a proved top-1 claim the circuit does not make. Excluded to avoid implying an unverified guarantee; may re-enter only with an explicit "order not proved" manifest flag. |
| `DISTINCT` / `REDUCED` / `LIMIT` / `OFFSET` / projection | IN (today) | Membership-indifferent modifiers. |

### 3.3 SPARQL 1.2 specifics

| Construct | Disposition | Reason |
|---|---|---|
| Triple terms / reification (`<<…>>`) | OUT (encoding gap) | Monotone, but the committed leaf encoding (`sparq-zk::encode`) has no triple-term lane. Re-entry belongs to the field-native encoding overhaul (`sq-1s2.1`), not this program. |
| 1.2 builtins (`LANGDIR`, `hasLANG`, `hasLANGDIR`, `STRLANGDIR`, `isTRIPLE`, `TRIPLE`, `SUBJECT`, `PREDICATE`, `OBJECT`) | OUT (same encoding gap) | Term-encoding lanes first; cross-ref `sq-1s2.1`. |
| 1.2 `EXISTS` semantics clarifications | Adopted where relevant | Governs the eventual positive-`EXISTS` re-entry (§3.2). |

## 4. Property paths: bounded-depth proof semantics (first-class soundness requirement)

Path operators are admitted with the following table. The non-recursive operators are
*rewrites* into the existing fragment; the recursive ones get a new circuit family with an
explicitly bounded statement.

| Path form | Disposition | Circuit semantics |
|---|---|---|
| `iri` (PredicatePath) | IN (phase 1) | Identical to a triple pattern. |
| `^p` (inverse) | IN (phase 1) | Rewrite: swap subject/object. Composes under all other forms. |
| `p1/p2` (sequence) | IN (phase 1) | Rewrite to a BGP with a fresh non-projected intermediate variable — exactly the SPARQL 1.1 translation. Existing scan + join circuits carry it. |
| `p1\|p2` (alternative) | IN (phase 2) | Desugars to `UNION`; lands with `UNION` branch attribution. |
| `p?` (zero-or-one) | IN (phase 2) | Disjunction of the zero-length case (term equality, see below) and a single step. |
| `p+` (one-or-more) | IN (phase 2, **bounded**) | `path_reach` circuit family, statement below. |
| `p*` (zero-or-more) | IN (phase 2, **bounded**) | `path_reach` with the zero-length case. |
| `!(p1\|…\|pn)` (negated property set, incl. inverse members) | DEFERRED | Monotone — it asserts the *existence* of a committed triple whose predicate differs from each listed IRI (per-triple inequality against public constants; inequality of salted term encodings implies term inequality, a re-audited-pending-external argument that must be stated in the spec amendment). Deferred to a follow-up bead after the `path_reach` family lands, to keep phase 2 crisp. |

**The bounded-depth statement (normative for the spec amendment).** For `p+`/`p*` the
circuit proves, and the manifest MUST be read as claiming, exactly:

> There exists a chain of committed triples `(t_1, …, t_ℓ)` with `1 ≤ ℓ ≤ k`
> (`0 ≤ ℓ ≤ k` for `*`), each `t_i` a member of a committed graph in the disclosed
> attribution set with predicate `p`, chained object-to-subject, connecting `μ(s)` to
> `μ(o)` — where **`k` is a public input disclosed in the manifest**.

Soundness requirements, first-class (each becomes a spec MUST and a verifier check):

1. **`k` is public and surfaced.** Proofs at different `k` are *different statements*; a
   verifier MUST expose `k` to the consumer and MUST reject a manifest whose claimed path
   depth exceeds the circuit member's bound.
2. **Existence only — never absence.** A bounded path proof is monotone: it never asserts
   that longer paths do not exist, nor that the reachable set is complete. Failure to
   produce a proof at depth `k` proves *nothing*.
3. **One-directional SPARQL equivalence.** eval_k(P) ⊆ eval(P): every bounded witness is a
   genuine SPARQL `p+`/`p*` solution (membership is preserved); completeness holds only up
   to `k`. If any walk connects two nodes, a simple path of length ≤ |nodes(G)| does, so
   `k ≥` the committed union's node count restores per-pair completeness — worth stating,
   never assumed.
4. **Padding soundness.** Unused steps (ℓ < k) must be constrained to contribute nothing —
   padding rows are a classic forgery surface and get dedicated forge-negative tests
   (each step must either be a proven committed-row membership or a constrained
   pass-through that preserves the chain endpoint).
5. **Zero-length case (`p*`, `p?`).** The zero-length path holds when `μ(s) = μ(o)` *and*
   the term occurs in the committed union (matching the SPARQL term-universe semantics of
   zero-length paths); the circuit requires an occurrence witness, not bare equality.
6. **Cycles.** SPARQL 1.1 path semantics is existence-based set semantics (no duplicate
   counting), so cycles are harmless for membership; the chain is not required to be
   simple.

**Circuit-family shape (chosen option).** `path_reach_d{k}` members with unrolled `k`
steps over the (k, n, r)-style lattice already used by `scan_*`/`filter_*_d*` — each step
re-uses the scan row-membership relation, plus chain constraints. Rejected alternatives:
recursion/folding (Ultra-in-Ultra — same cost grounds the earlier plan rejected recursive
aggregation on; revisit under `sq-1s2.5`'s configurable circuit-builders) and an
unconstrained-closure witness table (buys nothing for existence proofs).

## 5. Filter expressions: estate coverage and gaps

"Most filter expressions" decomposes into (a) the **function estate** — largely already
built in `zk/xpath` (noir_XPath) + `zk/ieee754` (noir_IEEE754), verified against
`zk/xpath/SPARQL_COVERAGE.md` and the sources — and (b) the **composition layer**, which
today binds only four single-comparison numeric lanes and is the real gap.

### 5.1 Coverage table (SPARQL 1.1 §17.4 + operators → estate)

| Expression class | Estate status (verified) | Fragment disposition |
|---|---|---|
| Logical `&&`, `\|\|`, `!` | `logical_and`/`logical_or` implemented; `!` trivial | IN (phase 3) — requires the EBV + error lane (§5.2). |
| Numeric compare `=` `!=` `<` `<=` `>` `>=` | Integers implemented + circuit lanes today; doubles via noir_IEEE754; decimal fixed-point lane | IN (today for the 4 lanes; phase 3 for expression positions). |
| String compare / equality | Equality via committed-leaf identity; `compare` utilities in estate | IN (phase 3); locale collation OUT (codepoint order only, stated). |
| `dateTime`/`date`/`time`/duration compare + arithmetic | Implemented (`datetime_*`, `duration_*`) | IN (phase 3). |
| `sameTerm` | Leaf equality — the primitive `join_eq` already proves | IN (phase 3). |
| RDFterm-equal (`=` on terms) | Leaf equality + value-lane equality for literals | IN (phase 3), with the dual-leaf INV-VL caveat carried over (spec §17.2). |
| `IN` / `NOT IN` (constant lists) | Disjunction/conjunction of (in)equalities vs public constants | IN (phase 3). Note `NOT IN` is a *value* inequality against listed constants — not closed-world negation — hence monotone. |
| `BOUND` | — | IN (phase 3, statically): with `OPTIONAL` out, boundness is decidable from the query text alone; no circuit. |
| `IF` / `COALESCE` | — | IN (phase 3) — needs the error lane. |
| Term accessors `isIRI` `isBlank` `isLiteral` `isNumeric` `datatype` `lang` `str` | NOT in noir_XPath (out of its scope by design) | IN (phase 3) via new term-accessor circuits over the leaf encoding's type/datatype/lang lanes — **depends on the value/type-lane encoding** (`sq-j506` / `sq-1s2.1`). |
| Constructors `IRI` `STRDT` `STRLANG` | — | DEFERRED (deterministic, admissible; needs encoding-side term construction). |
| `BNODE` `UUID` `STRUUID` `RAND` `NOW` | — | OUT — non-deterministic; nothing provable. (`NOW` pattern: a verifier-supplied public "as-of" input, as the revocation snapshot already does.) |
| Strings `STRLEN` `SUBSTR` `UCASE` `LCASE` `STRSTARTS` `STRENDS` `CONTAINS` `STRBEFORE` `STRAFTER` `ENCODE_FOR_URI` `CONCAT` | Implemented (byte-array tuple representation; `SUBSTR` byte-position caveat tracked `sq-hjvte`) | IN (phase 3). |
| `REGEX` / `REPLACE` | Bounded subset only (literal / anchored / char-class, `sq-y73`) | IN (phase 3) **for the bounded subset only**, stated as such in the spec; full `fn:matches` stays a documented gap. |
| `langMatches` | Gap | IN (phase 3) after estate gap-fill (child bead). |
| Numeric fns `abs` `round` `ceil` `floor` | Integers done; float lanes via noir_IEEE754 | IN (phase 3). |
| Arithmetic `+ - * /` | Integers done; `/` yields decimal-as-double (documented approximation); doubles via noir_IEEE754 | IN (phase 3), approximation caveat carried into the spec. |
| Date component fns (`YEAR` … `TZ`) | All but `TZ` implemented | IN (phase 3); `TZ` after estate gap-fill. |
| Hash fns `MD5` `SHA1` `SHA256` `SHA384` `SHA512` | Gap (hex-output formatting exists: `bytes_to_lower_hex`) | IN (phase 3) after estate gap-fill (Noir stdlib digests + hex). |
| Aggregate fns in expressions | n/a | OUT (aggregation OUT, §3.2). |

### 5.2 The composition layer (the real work): expression-tree manifests

Today a `FILTER` is provable only if it is a *single* datatype-bucketed comparison bound
slot-wise to a scan row. Extending to expressions requires a representation the verifier
can re-derive and bind. Options considered:

- **A. Generic expression-VM circuit** — one interpreter circuit executing an encoded
  expression. Rejected: an in-circuit interpreter is the highest-risk shape (a single
  under-constrained opcode breaks every expression), and it abandons the fixed named
  circuit family the spec's verification model depends on.
- **B. Per-query circuit synthesis at prove time** (compile the expression to a bespoke
  Noir main). Rejected for this program: the verifier would need a circuit-identity ↔
  expression binding (verification-key provenance), a much bigger trust-model change —
  exactly the ground `sq-1s2.5` (configurable circuit-builders) is chartered to explore.
- **C. Composable expression-node circuits (CHOSEN).** Extend the fixed family with
  expression-node members (one per operator class, datatype-bucketed like today's lanes).
  Each node sub-proof discloses operand/result commitments; the existing binding-edge
  mechanism (`bind_*`) chains node results leaf-to-root and roots the leaves in scan-row
  slots. The verifier re-derives the expression tree from the query text (extending the
  existing `FilterCmp` extraction) and rejects any manifest whose declared tree differs —
  fail-closed, same discipline as today. Cost: one sub-proof per node (honest overhead,
  measured not asserted); optimization later via `sq-1s2.5`.

**Error semantics.** SPARQL expression errors map to *not satisfied* (spec §7.2 already
pins this). In-circuit this needs a three-valued lane: every node carries
`(value, is_error)`; comparisons/functions propagate `is_error` per SPARQL/XPath rules;
`&&`/`\|\|`/`IF`/`COALESCE` implement the three-valued EBV table; the root verdict is
`true ∧ ¬error`. This lane is part of the term-accessor bead (it defines the node calling
convention) and is a stated verification obligation in the spec amendment.

## 6. The pure single-prover zk-architecture paper

Verified gap: no paper describes the single-prover architecture itself.
`cozk-witness-validation.typ` (collaborative-path negative result) and
`verifiable-fed-sparql.typ` (SoK) cite pieces of it, but the system paper — commitment
scheme, circuit family, manifest composition, verifier re-derivation, the fragment of this
record, measured performance, and a research-grade security analysis — does not exist.
One child bead creates it under the paper factory's honesty discipline: `sq-qhy4`
research-grade caveats mandatory (internally re-audited, external audit pending;
explicitly NOT a proven-soundness claim), no unmeasured performance numbers (all
measurements via the canonical `bench/zk-compose` protocol: `bb gates -s ultra_honk`
gate counts joined from the regression-gated snapshot, prove/verify wall-clock from the
family cost-curve harness), and the `.typ` honesty gates
(`scripts/check-privacy-claims.sh`, `scripts/check-no-perf-numbers.py`) as the mechanical
floor.

## 7. Decomposition: phased plan and child beads

All beads are parented under `sq-3kd2g`; overlaps with the ZK build-out epic family are
cross-linked to `sq-1s2.x` in each description. Disjointness: no two beads touch the same
file; where the same crate/file is genuinely involved, beads are sequenced with `bd dep`
edges and are NOT parallel. All ZK-soundness-sensitive beads are `opus`-tier and
maintainer-armed.

**Phase 1 — independently dispatchable now (disjoint surfaces):**

| Bead | Surface | Tier | What |
|---|---|---|---|
| `sq-3kd2g.1` (paper) | `site/papers/` (+ registry) | opus | §6 pure zk-architecture paper. |
| `sq-3kd2g.2` (path circuits) | `zk/compose/` | opus | `path_reach_d{k}` family + padding forge-negatives (§4). |
| `sq-3kd2g.3` (verifier gate wave 1) | `crates/sparq-zk/src/verify.rs` | opus | Accept the monotone rewrites (path forms incl. bounded `*`/`+` markers, `UNION`, `VALUES`, subquery), everything else still fail-closed. |
| `sq-3kd2g.4` (estate gap-fill) | `zk/xpath/` | sonnet | `langMatches`, hash digests, `TZ()` (§5.1 gaps). |

**Phase 1b — gated on the #1509 review cycle completing (external event, not a bd dep):**

| Bead | Surface | Tier | What |
|---|---|---|---|
| `sq-3kd2g.5` (spec amendment) | `site/specs/zksparql.typ` | opus | Follow-up PR extending §7 with the feature table (§3), bounded-depth path semantics as normative MUSTs (§4), and the expression fragment + error lane (§5). |

**Phase 2 — composition (dep-gated):**

| Bead | Surface | Tier | Deps | What |
|---|---|---|---|---|
| `sq-3kd2g.6` (compose paths/UNION/VALUES) | `crates/sparq-zk-compose` | opus | `sq-3kd2g.2`, `sq-3kd2g.3` | Manifest schema (`PathReach{k}`, branch attribution), builder, fail-closed verifier dispatch. |
| `sq-3kd2g.7` (term-accessor circuits) | `zk/compose/` | opus | `sq-3kd2g.2` (file adjacency), `sq-j506` (value/type lane) | §5.1 term accessors + §5.2 error-lane calling convention. Cross-link `sq-1s2.1`. |
| `sq-3kd2g.8` (query-side expression extraction) | `crates/sparq-zk/src/verify.rs` area | opus | `sq-3kd2g.3` (same file) | `FilterCmp` → full expression trees, fail-closed on unsupported forms. |

**Phase 3 — expressions end-to-end + hardening:**

| Bead | Surface | Tier | Deps | What |
|---|---|---|---|---|
| `sq-3kd2g.9` (expression compiler) | `crates/sparq-zk-compose` | opus | `sq-3kd2g.6`, `sq-3kd2g.7`, `sq-3kd2g.8` | §5.2 Option C: expression-tree manifests, node binding, verifier tree re-derivation. Re-runs the `sq-1s2.6` adversarial-review checklist over the new edge kinds. |
| `sq-3kd2g.10` (differential fuzzer extension) | `crates/sparq-zk-compose/tests/` | sonnet | `sq-3kd2g.9` | Extend the prove→verify→cleartext-oracle fuzzer + forge-negative suites to paths/`UNION`/expressions. |
| `sq-3kd2g.11` (bench + catalog refresh) | `bench/zk-compose/` | haiku | `sq-3kd2g.6` | Gate-count snapshot + `sparql_feature_catalog.json` regeneration for the new members; gaps stay `null`. |

Deliberately NOT decomposed here (stated so the boundary is auditable): positive
`FILTER EXISTS` (re-entry criteria §3.2), negated property sets (§4), `ORDER BY`
accept-and-strip (§3.2), aggregation/completeness (a future program), triple-term
encoding (belongs to `sq-1s2.1`), and per-query circuit synthesis (belongs to
`sq-1s2.5`).

## 8. Risks and honesty flags

- **The bounded-`k` statement is the program's biggest consumer-honesty risk**: a
  verifier UI that renders "path exists" without `k` misrepresents the claim. Requirement
  1 of §4 makes surfacing `k` normative; the spec-amendment bead owns the wording.
- **Everything here is unaudited-externally** (`sq-qhy4` pending). New circuit families
  (path_reach, expression nodes) *expand* the internally-re-audited surface; each impl
  bead's acceptance includes forge-negative tests, and the fuzzer bead extends the
  differential oracle — necessary, not sufficient, and labelled as such.
- **Expression-node composition inherits the `bind_*` edge mechanism**, which was itself
  the subject of an adversarial review (`sq-1s2.6`). The expression-compiler bead must
  re-run that review's checklist over the new edge kinds before arm (maintainer-armed).
- **Catalog vs gate drift**: §1's Q13/Q14 finding shows coverage claims can silently
  diverge from accepted syntax; the bench/catalog bead re-grounds the catalog on the
  post-extension gate.

[#1591]: https://github.com/sparq-org/sparq/issues/1591
