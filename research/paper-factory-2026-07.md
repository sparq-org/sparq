# Paper factory 2026-07 — verified evidence provenance + canonical-timing binding

[FABLE-5] FRONT decomposition of the remaining gap in epic **sq-gum8** (academic paper factory:
auto-generated, live-updating, PDF-exportable papers hosted on the site). Design-only record —
the implementation is cut into the disjoint child beads in §6. Supersedes nothing: it BUILDS ON
`research/paper-factory-design.md` (the phase-3 design, now implemented) and
`research/paper-factory-honesty-gate-coverage.md`, and corrects the epic's implicit premise.

## 1. Corrected premise — the factory is already built

The epic reads as "build a paper factory". Verified against the actual tree, nearly all of it
exists and is wired end-to-end:

| Vision component | Status | Where (verified) |
| --- | --- | --- |
| Paper registry + site hosting + PDF download | **Done** | `site/src/data/papers.ts` (8 papers), `site/src/app/papers/{page,layout,[slug]/page}.tsx`, `site/public/papers/<slug>.pdf` |
| Typst → PDF + semantic-HTML single-source build | **Done** | `site/scripts/build-papers.mjs` — the SAME evidence JSON is injected into both artifacts via `--input data=` |
| Evidence layer + headline gate | **Done** | `site/src/data/paper-evidence.json` (57 records, `environment: canonical\|indicative`), `site/papers/_lib/bench.typ` `headline()` / `ev()` / `provenance()` |
| Phrase-level honesty gates at the build boundary | **Done** | `scripts/check-no-perf-numbers.py` (accessor-aware `.typ` scan) + `scripts/check-privacy-claims.sh` + shared `scripts/honesty-phrases.json`, re-run fail-closed inside `build-papers.mjs`; negative fixtures (sq-ddc0) |
| Live-update deploy trigger | **Done** | `.github/workflows/pages.yml` triggers on pushes to `main` AND `benchmark-data` (sq-gdhy); `prebuild` re-syncs data + rebuilds every paper |
| Page-level provenance stamp | **Done** | `site/src/components/papers/paper-provenance` + `[slug]/page.tsx` (commit + generatedAt) |
| Repeatable process / skill / venue calibration | **Done** | `skills/academic-paper/SKILL.md`, `research/paper-contributions-inventory.md`, `research/papers-venue-audit.md` (sq-gum8.2 ✓); rewrite program in flight (sq-gum8.3 ◐); zkSPARQL submission pack in flight (sq-gum8.5 ◐, maintainer-arm) |

So this record decomposes only what is genuinely missing. Two gaps remain, one load-bearing.

## 2. The load-bearing gap — asserted provenance vs verified provenance

Every evidence record carries a `source` field ("every number must trace to a real
test/dataset"), but `source` is **free text**. The build gate (`runHonestyGate()` in
`build-papers.mjs`) checks only that the field is *present*. Nothing verifies that:

1. the cited file / test fn / const still **exists** (a rename or deletion silently orphans the
   record), or
2. the record's `value` still **equals** what the committed source asserts.

Values were hand-transcribed from test assertions, ratchet consts, and bench artifacts.
Concrete drift scenario: the W3C conformance ratchet in
`crates/sparq-conformance/src/scoreboard.rs::SUITES` rises; CI enforces the new floor; the
paper-evidence record keeps the old count; every deploy faithfully republishes the **stale**
number. The live-update property currently holds for the *plumbing* (rebuild-on-data-change)
but not for the *values* papers actually headline — the exact failure mode the epic's
honesty mandate exists to prevent.

Source classes across the 57 records (measured): 36 point at Rust tests/consts under
`crates/` (no machine-readable mirror exists for the scoreboard consts —
`render_scoreboard()` emits markdown only), 15 at `bench/` artifacts (machine-readable JSON,
just unbound), the rest at `research/` audit records and `compliance/` evidence docs.

**Second gap (capability, not soundness):** canonical measurement envelopes from the dedicated
quiet EC2 protocol exist and are committed (`bench/canonical-competitor-results/<date>/*.json`,
each self-describing: `"canonical": true`, full git commit, host/dataset/methodology note), but
the evidence schema has no class for them. Papers therefore cannot cite ANY measured wall-clock
result — even provenance-complete canonical ones — leaving e.g. the engine-systems paper's
evaluation section weaker than the data the project already has. (Work-box timings are
non-canonical and must stay excluded; that posture is correct and is preserved below.)

## 3. Options and decisions

### 3.1 Making the number→source trace *verified* (the fix for §2)

- **(a) Derive everything** — regenerate every evidence value from its source at build time.
  Strongest, but impossible for values whose source of truth is a Rust const/test assertion
  without first exporting those to a machine-readable artifact; a big-bang prerequisite.
- **(b) Verify only** — keep hand-entered values, add a checker that the value matches the
  source. Works everywhere but leaves JSON-sourced values needlessly hand-maintained.
- **(c) Tiered bindings** — **chosen.** Each record gains a machine-checkable `binding`:
  - `json-pointer` — `{file, pointer}` into a committed JSON artifact; the verifier asserts
    the recorded value **equals** the pointed value (derivation-strength; covers all `bench/`
    sources today and the scoreboard class once §3.1.1 lands).
  - `rust-anchor` — `{file, anchor}` (a test-fn / const name); the verifier asserts the anchor
    exists in the file AND the recorded value's literal appears within the anchor's window.
    *Honest limitation:* literal-adjacency can in principle false-pass if the same literal
    occurs nearby for another reason — strictly weaker than `json-pointer`, still strictly
    stronger than today's unchecked free text, and the highest-value class (scoreboard floors)
    is moved OFF this tier by §3.1.1.
  - `doc-anchor` — `{file, quote}` for records sourced from `research/` / `compliance/` audit
    records; the verifier asserts the exact quote is present (existence-strength only).

  **Fail-closed + migration ratchet:** any `environment: canonical` record without a passing
  binding fails the paper build unless listed in a shrink-only allowlist
  (`scripts/paper-evidence-binding-allowlist.json`); the verifier fails if the allowlist ever
  grows. This lands the gate immediately without a big-bang migration, and the migration bead
  drives the allowlist to empty.

#### 3.1.1 Machine-readable scoreboard export

The single biggest `rust-anchor` class (conformance/ratchet floors) becomes `json-pointer`
strength by exporting the `SUITES` consts to a committed
`bench/conformance-scoreboard.generated.json`, drift-guarded by a test in `sparq-conformance`
that regenerates and byte-compares (the same committed-generated-artifact pattern the repo
already uses, e.g. `served-conformance.generated.json` via
`src/bin/served-conformance-report.rs`).

### 3.2 Measured timings in papers

- **(a) Status quo** — no timings in papers, ever. Safest; leaves evaluation sections
  hollow for systems venues, while provenance-complete canonical envelopes sit unused.
- **(b) Indicative callouts** — already allowed via `ev()` with explicit labelling; not a
  headline mechanism and must not become one (work-box numbers are non-canonical).
- **(c) A third evidence class `canonical-timing`** — **chosen.** Bound EXCLUSIVELY to
  committed envelopes under `bench/canonical-competitor-results/**` that self-declare
  `"canonical": true` (the dedicated quiet-EC2 protocol). Values are **auto-derived** by a new
  sync script (never hand-typed), and papers can render them ONLY through a new
  `headline_timing()` accessor (new `site/papers/_lib/timing.typ`) that unconditionally emits
  the provenance footnote (host class, git commit, date, dataset/query) alongside the value —
  there is deliberately no raw-value accessor for this class. The sync refuses any envelope
  not declaring `canonical: true`, so work-box numbers cannot enter by construction.

This is a *policy extension* (papers may now headline measured timings under strict
provenance). Per the standing proceed-and-document rule it is decided here and flagged to the
maintainer via a GitHub issue rather than stalling; the fragment stays `opus`-tier.

### 3.3 Templating / PDF / hosting / live trigger

Already decided and implemented (Typst CLI at build time; native HTML export replaced the
originally-designed browser-side typst.ts; `pages.yml` two-branch trigger). No redesign.
Explicitly rejected here: adding a second templating stack (LaTeX/Pandoc) — the Typst pipeline
is proven across 8 papers and both artifact forms.

## 4. End-state honesty invariant (what the decomposition buys)

After the beads land, every number in a published paper sits on this verified chain:

committed measurement (test assertion / ratchet const / canonical envelope)
→ machine-verified `binding` (fail-closed at build; allowlist empty)
→ evidence record (`canonical` / `canonical-timing` / `indicative`)
→ accessor-only rendering (`headline()` / `headline_timing()` / labelled `ev()`)
→ phrase + number gates (`check-no-perf-numbers.py`, `check-privacy-claims.sh`)
→ PDF + HTML artifacts (same injected data, provenance-stamped).

A paper cannot claim a result the data does not support because (i) a number not in the
evidence file cannot render (the accessor panics the compile), (ii) an evidence value that no
longer matches its committed source fails the build, (iii) a timing without a canonical
envelope + forced provenance footnote cannot render, and (iv) forbidden claims fail the phrase
scan. **Honest scope:** these gates are mechanical; a *semantic* overclaim (a true number
framed misleadingly) remains the human claims↔evidence review in
`skills/academic-paper/SKILL.md` (per sq-dxi3 — do not oversell the gates). Nothing here
changes the ZK/MPC posture: the external cryptographer audit is pending (sq-qhy4), MPC is
semi-honest-only, C-family papers stay `wip-arxiv`, and evidence records citing the ZK audit
records remain scoped as internally re-audited with external sign-off pending.

## 5. Decomposition constraints observed

- **Disjointness:** no two beads below share a file. Where the same file is genuinely needed
  (`paper-evidence.json`, `build-papers.mjs`) the beads are sequenced with `bd dep` edges and
  marked non-parallel. The `site/` surface additionally serialises to ≤ 1 in-flight bead
  (house conflict-partition), which the dep chain encodes.
- **In-flight coordination:** sq-gum8.5 owns `site/specs/zksparql.typ` + `bench/zk-compose/**`
  (untouched here); sq-gum8.3 owns paper prose — the flagship bead (F5) depends on F4 and
  carries an explicit pickup check that no open rewrite-program PR touches
  `sparq-engine-systems.typ`; open PR #1875 touches a different `sparq-conformance` file than
  F2's scope.
- **Cheapest-sound-tier:** the two honesty-gate design fragments are `opus`; the mechanical
  export and the flagship prose are `sonnet`; the record migration is `haiku`.

## 6. Child beads (the phased plan)

| # | Bead | Surface | Tier | One-line scope | Depends on |
| --- | --- | --- | --- | --- | --- |
| F1 | sq-gum8.13 | `scripts/` + paper build | opus | Evidence-binding schema + `scripts/verify-paper-evidence.py` fail-closed verifier (self-test fixtures, shrink-only allowlist), wired into `build-papers.mjs` + CI | — |
| F2 | sq-gum8.14 | `crates/sparq-conformance` | sonnet | Scoreboard JSON export (`bench/conformance-scoreboard.generated.json`) + regenerate/byte-compare drift-guard test | — |
| F3 | sq-gum8.15 | `site/src/data` | haiku | Migrate all 57 records to verified bindings; allowlist → empty | F1, F2 |
| F4 | sq-gum8.16 | `site/` papers lib + sync | opus | `canonical-timing` class: `sync-canonical-timing.mjs` auto-derivation from `canonical: true` envelopes + `timing.typ` provenance-forced `headline_timing()` | F1, F3 (site serialisation) |
| F5 | sq-gum8.17 | `site/papers/sparq-engine-systems.typ` | sonnet | Flagship consumer: live canonical-timing evaluation section, accessor-only numbers | F4 (+ sq-gum8.3 coordination) |

**AS-BUILT (F4 / sq-gum8.16, [OPUS-5]).** Landed as designed, with three deviations worth
recording:

1. **No `--input timing=`.** `timing.typ` reads the derived JSON from the Typst project root
   instead of taking it as an injected argument. Same anti-drift property (both the PDF and the
   HTML compile from the same committed file in the same build) without the OS single-argument
   size ceiling a growing envelope set would eventually hit.
2. **Provenance is inline + block, not a `footnote()`.** Typst's native HTML export is
   experimental and drops layout-only constructs, so a real footnote would render in the PDF and
   vanish from the in-site page — i.e. the provenance would be droppable, which is exactly what
   the design forbids. `headline_timing()` therefore emits a compact inline tag (aggregate · host
   class · commit) and `timing_provenance()` the full block form; both survive both exports.
3. **Two admission rules the design did not anticipate, both found by reading the real
   envelopes.** (a) *COUNT BEFORE TIME* — the envelopes themselves say "this is why COUNT is
   checked before trusting any timing", so a row is admitted only if that engine's own count
   matches the suite's committed `expected-rows`; this is what keeps a comparator that returned
   0 rows on SP2Bench q08/q12b out of the class. (b) An envelope carrying a free-text
   `disqualified_note` is refused *whole* — the note scopes a disqualification in prose a machine
   cannot bound.

Also deliberately NOT built: any ratio / speed-up accessor (a cross-engine ratio is a claim, not
a derivation), and the HTTP / materialization / HDT envelope families, which are recognised and
skipped with a recorded reason rather than guessed at. F5 (the first consuming paper) remains
open, so no published paper headlines a measured timing yet.

Each bead's full spec (files, invariant, acceptance test) lives on the bead itself; the
invariants in one line: **F1** fail-closed value↔source verification; **F2**
result-equivalence between the Rust consts and the JSON mirror; **F3** zero unverified
canonical records; **F4** timing numbers only from committed canonical envelopes, always with
provenance, work-box numbers barred by construction; **F5** zero inline numeric literals in
the paper (accessor-only), claims scoped to exactly what the cited envelopes measured.

---

> 🤖 SPARQ agent [FABLE-5] — FRONT decomposition for epic sq-gum8. Design-only; no
> implementation in this PR.
