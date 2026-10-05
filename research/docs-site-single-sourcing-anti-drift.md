<!-- [OPUS-5] sq-w9sr — docs-site single-sourcing + anti-drift design record. -->

# Docs site: single-sourcing, build-time injection, anti-drift (sq-w9sr)

> 🤖 **SPARQ agent** design record. Design + measured inventory only. The one code
> change shipped alongside it is named in §8; everything else here is specification
> for follow-up beads, most of which land in `ci`-area files this record does not
> touch. Predecessors: sq-9jw5 (docs overhaul), sq-5fd1 (doc-lint CI),
> sq-p26u (`include_str!` / docs.rs). Companion runbook:
> [`docs/pages-cutover-runbook.md`](../docs/pages-cutover-runbook.md).
> **Post-record status:** The separate sq-iigf Pages source-mode cutover is complete
> on the repo side: the producer workflow exists. The Pages service setting is reported
> as flipped but is not verified here; see the linked runbook's Provenance section.
> The guide's Pages mount discussed in §8 remains open: `docs.yml` still has no deploy
> job, and `pages.yml` still records the guide-at-root versus `/guide/` decision as open.

## 1. The requirement, restated

The bead asks for a Pages-hosted docs site with **no content duplicated across
locations**: where the same content must appear twice, it is **copied at docs-build
time** rather than maintained twice; CI **detects** duplication; code examples are
**actual testable code** injected into the docs; the bulk of the reference material is
**generated from inline rustdoc**. Everything is in service of one property — a docs
site that cannot silently drift from the code and the canonical prose.

## 2. What has already landed

The single-sourcing *mechanism* exists and is wired to a build gate. It is worth
stating plainly so follow-up work does not re-derive it:

| Piece | Where | Bead |
|---|---|---|
| mdBook scaffold (`book.toml`, `SUMMARY.md`, 3 content pages) | `book/` | sq-h0tr |
| Guide pages single-source their prose via `{{#include}}` (3 content pages, 7 anchored includes, plus the tested-example embed below) | `book/src/**` | sq-im8u |
| `ANCHOR` regions in the canonical sources | `README.md`, `skills/{zk-query-proofs,mpc}/SKILL.md` | sq-im8u, sq-g4h0c |
| Repo-relative → mount-portable link rewriting at build time | `scripts/mdbook-rewrite-links.py` (mdBook preprocessor) | sq-g4h0c |
| Tested-example embedding (`{{#rustdoc_include}}` of a compiled example) | `book/src/getting-started/install.md` ← `crates/sparq-engine/examples/quickstart.rs` | sq-384j |
| Pinned reproducible build + broken-include detection + `mdbook test` | `.github/workflows/docs.yml` | sq-h0tr |
| Crate README injected into rustdoc (`#![doc = include_str!("../README.md")]`) | 30 of the 67 workspace crates | sq-p26u |
| Pages cutover sequence + rollback | `docs/pages-cutover-runbook.md` | sq-iigf |

The build gate is real but narrow: mdBook **exits 0 on a broken `{{#include}}`**
(rust-lang/mdBook#1094), so `docs.yml` greps the build log for `[ERROR]`, and its path
filter deliberately lists every *embedded source* (`README.md`, the two `SKILL.md`s, the
example, the preprocessor) so an anchor rename in a file outside `book/` still re-runs
the lane.

**What does not exist yet:** any duplication *detection*; any injection mechanism for
the renderers mdBook cannot reach (crate READMEs, `skills/`); a published home for the
guide.

## 3. Measured duplication baseline

Method (reproducible, ad-hoc — not a committed script): strip HTML comments, split every
markdown file on blank lines, normalise whitespace + case, discard blocks under a
prose-length floor, then compare blocks (a) for exact equality and (b) by 8-gram Jaccard
similarity. Two passes: whole repo (exact) and the published doc surface — root
`README.md`, `docs/**`, `book/**`, `skills/**/SKILL.md`, `crates/*/README.md` — for
near-duplicates.

**Exact duplication across the whole tree is low** — a dozen repeated blocks, and the
bulk of them are the deliberately-replicated agent-contract paragraphs in
`.claude/agents/*.md` and `AGENTS-worker-core.md` ↔ `AGENTS.md`, which are outside the
published doc surface.

**Near-duplication on the published surface is where the drift risk actually lives.**
The pairs the scan surfaces, in descending similarity:

| Pair | Kind | Fixable with existing machinery? |
|---|---|---|
| `README.md` ↔ `crates/sparq-cli/README.md` (quickstart command block) | near-verbatim prose/commands | **No** — a crate README cannot use mdBook syntax (§5) |
| `skills/shacl-validation/SKILL.md` ↔ `crates/sparq-shacl/README.md` | duplicated **code example** | **No** — needs a tested example + injection (§6) |
| `crates/sparq-{text,shacl,reason}-wasm/README.md` (mutual) | duplicated "how-to" link stanza | **No** — same as above |
| `skills/streaming-rsp/SKILL.md` ↔ `crates/sparq-rsp/README.md` | duplicated code example | **No** — §6 |
| `skills/prov-lineage/SKILL.md` ↔ `crates/sparq-prov/README.md` | duplicated code example | **No** — §6 |
| `README.md` ↔ `book/src/introduction.md` (experimental-status caveat) | hand-copied honesty caveat | **Yes** — plain `{{#include}}`; done in this PR (§8) |

Two conclusions follow, and they shape the rest of this design:

1. **The mdBook `{{#include}}` mechanism has already done its job.** Every remaining
   duplicate involves at least one file mdBook does not render. Extending the guide
   alone cannot reduce this set further.
2. **The dominant duplicate kind is a code example**, copied between a `SKILL.md` and the
   crate `README.md` that documents the same surface. That is precisely the class the
   bead wants replaced by injected, compiled example files.

## 4. The duplication-detection gate (specification)

A follow-up bead in `ci` area should add `scripts/check-doc-duplication.py`, wired as a
step in `docs-quality.yml`. Specification:

**Surface.** Root `*.md`, `docs/**`, `book/src/**`, `skills/**/*.md`,
`crates/*/README.md`. Deliberately excludes `research/**` (design records legitimately
restate context, and the markdownlint config already treats `research/` as a separate
tier), `.claude/**` and `AGENTS-worker-core.md` (the agent-contract replication is
intentional and is governed by the contract's own single-source rule), and any
`vendor/` subtree.

**Normalisation.** Strip HTML comments and YAML frontmatter; split on blank lines;
collapse whitespace; lowercase; **strip markdown link *targets* while keeping link
text**. That last step is load-bearing: the same paragraph copied between two mounts
usually differs *only* in whether its links are repo-relative or absolute, which is
exactly the difference the `link-fixup` preprocessor exists to erase. A comparator that
keeps targets scores such a pair as merely "similar" and lets it through.

**Two tiers, matching the repo's advisory→gating probation convention:**

- **Exact tier — GATING from day one.** An identical normalised block appearing in two
  or more files in the surface fails the build. Deterministic; the only false positives
  are genuine boilerplate, handled by the escape hatch below.
- **Near tier — ADVISORY first.** 8-gram Jaccard above a threshold is *reported*, not
  failed, until the backlog in §3 is cleared; it is promoted to gating by deleting its
  entry from `.github/advisory-registry.json`, the same one-line flip the E2E lanes use.
  Landing it gating on day one would mean either failing `main` immediately or picking a
  threshold high enough to be decorative.

**Escape hatch, following the house pattern.** An inline
`<!-- doc-duplication-allow: <why> -->` marker on the block (mirroring
`terminology-allow` / `privacy-claims-allow`), plus a JSON allowlist of file pairs each
carrying a `why` (mirroring `banned-terminology.json`'s `exemptPaths` discipline).
Broad path exclusions to make a violation pass defeat the gate; fix the wording or
inject the block instead.

**Both-direction self-test.** Per the repo's standing rule for every gate: hermetic
fixtures proving the checker *fires* on a planted duplicate **and** stays quiet on
distinct prose, run in the same HARD job so a coverage regression fails `ci-summary`.

**The property that makes this gate cheap.** Injected content is *not physically present
twice in the source tree* — a `{{#include}}`d region exists once, in the canonical file.
So the scanner sees only hand-copies, and the gate's finding count is a direct measure
of un-single-sourced content. It needs no knowledge of the injection mechanism.

## 5. Injection for the renderers mdBook cannot reach

A crate `README.md` is consumed by three renderers — GitHub, crates.io, and rustdoc via
`#![doc = include_str!("../README.md")]` — none of which understand `{{#include}}`.
Preprocessing is therefore the wrong shape; the content must be **materialised into the
file and checked**:

```text
<!-- BEGIN-INJECT: README.md#quickstart-cli -->
…generated copy…
<!-- END-INJECT -->
```

with a generator that rewrites the region and a `--check` mode that fails CI when the
committed region differs from the source — the standard "generated file is up to date"
gate. Properties: the file stays valid, self-contained markdown for every renderer; the
injected region is reviewable in the diff; and the duplication gate of §4 skips
`BEGIN-INJECT`/`END-INJECT` regions, so injected content is not reported as a duplicate
while a hand-copy of the same text still is.

This is the mechanism that closes the `README.md` ↔ `crates/sparq-cli/README.md` and the
three-way `*-wasm` README duplications in §3.

## 6. Code examples as tested code

The `{{#rustdoc_include <example>:<anchor>}}` pattern is proven in
`book/src/getting-started/install.md`: the snippet is an `ANCHOR` region of
`crates/sparq-engine/examples/quickstart.rs`, compiled and run by
`cargo test -p sparq-engine --examples`, so it cannot drift from the public API. The
fence is `rust,ignore` on purpose — the snippet references workspace crates a bare
rustdoc invocation cannot resolve, so `cargo test` is the compile-and-run gate rather
than `mdbook test`.

Generalising it: for each duplicated code example in §3, move the block into
`crates/<crate>/examples/<topic>.rs` with `ANCHOR` regions, and inject it into **both**
consumers — the guide via `{{#rustdoc_include}}`, the crate README and `SKILL.md` via
the §5 mechanism. One compiled file resolves both halves of each pair at once. 38 crates
already carry an `examples/` directory, so the placement convention exists.

A gate should then require that a fenced ```rust block on the published surface is either
an injected region or carries an explicit marker justifying why it is illustrative-only.
That is the enforcement half of "code examples should mostly be ACTUAL testable code";
without it the pattern is a convention that erodes.

## 7. Rustdoc as the generated bulk, and the one open decision

**Rustdoc coverage is the cheap win.** 30 of 67 crates pull their README into their crate
docs via `#![doc = include_str!("../README.md")]`. Making that near-universal for
publishable crates — enforced the way `gate-new-crate.py` enforces other per-crate
structure — is a small, mechanical bead that turns each crate README into
simultaneously the GitHub landing page and the rustdoc front page, with no second copy.

**Recommendation: link docs.rs rather than self-hosting rustdoc.** `sparq-engine`
already declares `all-features = true` + `--cfg docsrs` for docs.rs. Publishing a second
`cargo doc --workspace` tree onto Pages would add a large artifact to every deploy and,
more importantly, a *second* rendering of the same rustdoc that can disagree with
docs.rs about which features were enabled. The guide should link out; the API reference
has a canonical home already.

**The one genuinely blocked item is the Pages mount**, and it is a product decision, not
a settings flip. GitHub Pages exposes exactly one deploy slot, `pages.yml` owns `/`
(the Next.js showcase, with the benchmark dashboards overlaid under `dev/bench*`), and
`docs.yml` therefore ships **no** deploy job — a second `deploy-pages` would race it
last-writer-wins. The options:

- **(a) Guide at a `/guide/` sub-path, assembled into the *existing* `pages.yml`
  artifact — recommended.** One producer, one deploy slot, no race, no live URL moves.
  `pages.yml`'s build job already assembles multiple sources into one artifact (it
  overlays the `dev/bench*` dashboards from the `benchmark-data` branch and smoke-checks
  the result), so the guide is one more overlay directory plus a build step. `docs.yml`
  keeps its build-validate role for PRs.
- **(b) Guide at the root, showcase relocated.** Rejected: breaks live URLs, and the
  showcase is the current front door.
- **(c) Two deploy workflows.** Rejected: documented race, already ruled out in both
  workflow headers.

Under (a) the `link-fixup` preprocessor keeps working unchanged — it rewrites
repo-relative links to absolute GitHub URLs, which are mount-independent, so the guide
is portable across whatever sub-path is chosen. Implementation is `ci`-area
(`pages.yml` + `docs.yml`) and out of scope for this record's PR.

## 8. What this PR changes

One duplication from §3 is fixable with the machinery already in the tree, and it is the
one that matters most for honesty: the **experimental-status caveat** was hand-copied
from `README.md` into `book/src/introduction.md`, with the links swapped and one clause
dropped. A required honesty caveat is the last thing that should be maintained twice.
This PR anchors it in the README and `{{#include}}`s it into the guide, so the two can
no longer disagree, and corrects the wrapper comment that claimed no README prose was
duplicated — it was.

One link inside the newly-anchored region is written as an absolute URL rather than
repo-relative: `scripts/mdbook-rewrite-links.py` only rewrites root-level markdown files
whose name begins with an uppercase letter, so a lowercase root-level target would
survive the preprocessor and 404 under the guide mount. Widening that pattern is a
one-line change in a `ci`-area file and is left to the follow-up.

**Update — that follow-up has landed (issue #5021).** `_ROOT_MD` now matches root-level
markdown of ANY case, the README link named above is back to its repo-relative form, and
the preprocessor carries a hermetic `--self-test` (run in `docs-quality.yml`'s HARD
quick-gates job on every PR) whose lowercase-root-doc case reds if the pattern is
narrowed again.

## 9. Gate-coverage gaps found while writing this

Recorded here because they are adjacent to the anti-drift mandate and are not otherwise
tracked:

- **`book/**` is outside the markdownlint HARD globs and outside the lychee
  internal-links file list**, though it *is* inside the terminology gate's surface. A
  broken relative link or a structural markdown defect authored in `book/src/` is
  currently caught by neither. It should be added to both.
- **`{{#include}}` and `{{#rustdoc_include}}` targets are not link-checked.** `mdbook
  build` resolves them, and `docs.yml` catches a failure via the `[ERROR]` log grep, but
  only for the files in that lane's path filter — which is hand-maintained. Deriving the
  filter from the includes actually present in `book/src/` would make it self-updating.

## 10. Follow-up beads this record specifies

1. `ci`: `scripts/check-doc-duplication.py` + `docs-quality.yml` wiring + both-direction
   self-test (§4). Exact tier gating, near tier advisory.
2. `ci`: the `BEGIN-INJECT`/`END-INJECT` generator with `--check` (§5).
3. `docs`: migrate the §3 duplicated code examples into `crates/*/examples/*.rs` with
   `ANCHOR` regions and inject them into both consumers (§6).
4. `ci`: require `#![doc = include_str!("../README.md")]` for publishable crates (§7).
5. `ci`/product: land option (a) — assemble the guide into `pages.yml`'s artifact under
   a `/guide/` sub-path (§7); this is what actually publishes the site.
6. `ci`: add `book/**` to the markdownlint globs and the lychee file list; derive
   `docs.yml`'s path filter from the includes present in `book/src/` (§9).
