# sparq feature-showcase website — design (bead sq-4r4b)

> Status: DESIGN / deep-research-first (design-for-review). This document specifies a
> comprehensive, **live-interactive** feature-demonstration website for sparq, the honest
> per-surface live-vs-fallback feasibility, the three flagship demos in detail, the
> Next.js + Pages hosting plan, and a buildable bead graph. **It does not build the site** —
> that is the work the child beads track. Authored by Claude Opus 4.8 while Fable was
> unavailable — flagged `[OPUS-4.8]` for later re-review (esp. the ZK/MPC honesty claims).

Non-canonical timing: any latency figure here is an order-of-magnitude estimate from public
anchors or non-canonical local runs, never a measured-and-baked number.

---

## 0. North star + the one hard constraint

The user wants **live-interactive everywhere** — attempt in-browser execution for everything,
including ZK proof generation and an MPC simulation — and **explicitly accepts honest
fallbacks** where live is infeasible. So the design's job is to be *honest about the seam*:
for each surface, classify it as one of:

- **(a)** live via the **existing `sparq-wasm`** (Store load/query/ask/construct/update over
  turtle/ntriples/nquads/trig — confirmed surface, `crates/sparq-wasm/src/lib.rs`);
- **(b)** live via **NEW wasm bindings** we'd add (a new feature compiled into a *second*,
  larger wasm bundle — never the lean default bundle);
- **(c)** live via a **3rd-party WASM** (bb.js / barretenberg for ZK proving);
- **(d)** an **in-browser SIMULATION** (MPC — a faithful JS re-implementation of the protocol
  shape, clearly labelled as illustration, not the hardened crate);
- **(e)** **hosted backend** (a deployed `sparq-server`) — for surfaces whose code is
  native-only and impractical to bring to wasm; or **infeasible-live → guided walkthrough
  with real captured I/O** as the honest fallback.

**The hard architectural fact** discovered in research and worth stating up front: today only
the **core parser + triplestore + SPARQL engine + the four text formats** are in the shipped
wasm bundle. **Full-text, vector, SHACL, inference/reasoning, GeoSPARQL, GenAI/NLQ, MPC, ZK,
and the http-server/python/CLI hosts are NOT in wasm.** That is a deliberate "lean default
bundle" choice (`sparq-wasm` depends only on `sparq-core` + `sparq-engine`). So "live
everywhere" is achieved by a **three-tier live strategy**, not by one magic wasm blob:

1. **Tier-1 live** — the lean wasm bundle already shipped (`@sparq-org/sparq`, ~886 KB).
2. **Tier-2 live** — *new optional wasm bundles* we build for the surfaces that are pure-Rust
   and wasm-portable but simply not in the default bundle (SHACL, reasoning, RSP, full-text;
   each a separate `wasm-pack` artifact, lazy-loaded only on that demo page).
3. **Tier-3 live** — a small **hosted `sparq-server`** for surfaces that pull native-only
   stacks (GeoSPARQL's georust, on-disk vector indexes, the NLQ LLM loop) where a wasm rebuild
   is uneconomic; plus **bb.js** (tier-c) for ZK and an **in-tab simulation** (tier-d) for MPC.

Every page that is *not* tier-1/2/3-live degrades to a **rich guided walkthrough with real
captured I/O** (the exact inputs + real outputs recorded from the native engine), so the page
is always honest and always shows real sparq output — it just replays it.

---

## 1. Look & feel — design language (sibling of solid-pod-manager)

Extracted verbatim from `jeswr/solid-pod-manager` so the showcase reads as a sibling app.

**Stack (exact versions to match):** Next.js **15.5.19**, React/React-DOM **^19**,
Tailwind **v4** (`@tailwindcss/postcss ^4`, **theme-in-CSS**, *no* `tailwind.config.ts`),
`next-themes` **^0.4.6**, the **unified `radix-ui` ^1.5.0** package (not split `@radix-ui/*`),
`class-variance-authority ^0.7.1`, `tailwind-merge ^3.6.0`, `clsx ^2.1.1`,
`lucide-react ^1.17.0`, `sonner ^2.0.7` (toasts), `tw-animate-css ^1.4.0`.

**shadcn (`components.json`):** style **`radix-nova`**, baseColor `neutral` (overridden in
CSS), `cssVariables: true`, iconLibrary `lucide`, aliases `@/components`, `@/lib/utils`,
`@/components/ui`, `@/hooks`. CSS entry `src/app/globals.css`.

**Palette — privacy-first teal/cyan in OKLCH (hue ~195–235), NOT default grey.** Reuse the
exact tokens; the load-bearing ones:

| token | light | dark |
|---|---|---|
| `--background` | `oklch(0.992 0.003 210)` | `oklch(0.17 0.018 235)` |
| `--foreground` | `oklch(0.21 0.022 235)` | `oklch(0.96 0.006 210)` |
| `--primary` | `oklch(0.52 0.094 205)` (deep teal) | `oklch(0.7 0.1 200)` |
| `--primary-foreground` | `oklch(0.99 0.005 200)` | `oklch(0.17 0.03 235)` |
| `--card` | `oklch(1 0 0)` | `oklch(0.21 0.02 233)` |
| `--muted` | `oklch(0.965 0.008 215)` | `oklch(0.26 0.02 232)` |
| `--accent` | `oklch(0.94 0.03 195)` | `oklch(0.32 0.045 205)` |
| `--border` | `oklch(0.91 0.012 220)` | `oklch(1 0 0 / 10%)` |
| `--ring` | `oklch(0.52 0.094 205)` | `oklch(0.7 0.1 200)` |
| `--destructive` | `oklch(0.55 0.2 27)` | `oklch(0.7 0.18 22)` |

Plus the **non-stock status tokens** to reuse for demo verdicts: `--success`
(`oklch(0.55 0.11 155)` light / `0.72 0.12 155` dark), `--warning`
(`oklch(0.7 0.14 75)` / `0.8 0.13 80`), and five `--chart-1..5` category hues for the
surface taxonomy. Sidebar gets its own `--sidebar*` token family.

**Radius:** `--radius: 0.7rem`; derived `sm=*.6 md=*.8 lg=1× xl=*1.4 2xl=*1.8 4xl=*2.6`.
Buttons `rounded-lg`, cards `rounded-xl`, badges `rounded-4xl` (full pill).

**Fonts:** **Inter** via `next/font/google` → `--font-sans` / `--font-heading`;
`--font-mono: var(--font-geist-mono)`. Body `font-feature-settings: "cv11","ss01"`,
antialiased; headings `font-semibold tracking-tight text-balance`. Utilities `.tabular`
(tabular-nums — for triple counts / gate counts) and `.measure` (max-width 65ch).

**Component conventions (copy them):** compact sizing — `Button` default `h-8` (cva + radix
`Slot`, `focus-visible:ring-3 ring-ring/50`, `active:translate-y-px`, tinted-not-solid
`destructive`); `Card` uses `ring-1 ring-foreground/10` instead of a border, `rounded-xl`,
footer `border-t bg-muted/50`; `Badge` `rounded-4xl h-5 text-xs`. `data-slot` attrs throughout,
`cn()` from `@/lib/utils`.

**Shell:** persistent **`w-64` left sidebar** (`border-r bg-sidebar`, 64px brand header,
scrollable nav) + sticky **`h-16` header** (`bg-background/90 backdrop-blur`, mobile hamburger →
left `Sheet` drawer `w-72`, right = ThemeToggle + (optional) AccountMenu) + main `flex-1`
content centered at **`mx-auto w-full max-w-6xl`** (72rem) + mobile `BottomNav`. Providers:
`ThemeProvider` (next-themes `attribute="class" defaultTheme="system" enableSystem
disableTransitionOnChange`) → `TooltipProvider` → (`SessionProvider` only on the Solid demo
route) → `AppShell`. `Toaster` (sonner `richColors`) at root. Viewport themeColor light
`#f7fbfc` / dark `#13181c`.

**next.config:** **`output: "export"`** (static `out/`), no `basePath` by default — but see §5
(Pages serves under `/sparq/`, so we will set `basePath: '/sparq'` + `assetPrefix`),
`trailingSlash: false`, the webpack `extensionAlias` `.js`→`.ts/.tsx` rule.

### solid-ai-coding skills — which applies where

Consumed by build agents via `npx skills add jeswr/solid-ai-coding -a <agent>` (the
`setup-bob.js` path; `install-skills.sh` is deprecated). Mapping:

| skill | applies to |
|---|---|
| **`solid-app-shell`** | the site shell + every demo's *result panel* — its optimistic, non-blocking update pattern + the corner `aria-live` status pill ("Proving…/Verifying…/Done") is exactly the UX for long-running demos (ZK proof, wasm load). |
| **`accessible-html-links`** | **whole-site lint gate** — every nav element is a real `<a href>` (it navigates), `rel="noopener noreferrer"` on external (GitHub, paper) links, descriptive link text. A review checklist for every PR. |
| **`solid-reactive-authentication`** | **the Solid user/app-pair demo** only — the real Solid login (`ReactiveFetchManager` + DPoP, patched global fetch, `<authorization-code-flow>` element, `/callback.html`) for the "log in as userA in appX" flow. |
| **`solid-client-id`** | **the Solid demo's deploy/identity** — ship a static `clientid.jsonld` whose URL === `client_id` so the consent screen shows the showcase's name (mirrors pod-manager's own `/clientid.jsonld`; for `output:export` ship a real `.jsonld` file with `application/ld+json`). |
| `solid-fetch-rdf`, `solid-object`, `solid-type-index`, `solid-offline`, `solid-server-matrix`, `solid-notifications` | optional, only if the Solid demo reads a *live* third-party Pod rather than a bundled fixture Pod (recommended default = bundled fixture; live-Pod is a stretch). |
| `solid-test-infrastructure` | the Playwright config for the Solid demo's e2e (authenticated fixture without driving the popup). |

The look-and-feel rules from solid-ai-coding's external skills (`responsive-design`,
`web-typography`, `color-mode-and-theme`, `web-design-guidelines`, `semantic-html`) are the
QA bar for the whole site.

---

## 2. Site IA / sitemap

Root site at `https://sparq.jeswr.org/`. Sidebar nav grouped by the surface taxonomy
(`skills/SKILL.md` is the canonical taxonomy source). Each group is a sidebar section; each
surface is a page; the three flagships are top-billed "Showcase" pages.

```text
/                         Landing / overview — what sparq is, the live REPL teaser,
                          three flagship cards, the surface grid, "all runs in your tab"
/showcase/zk-car-hire     ★ Flagship 1 — ZK cross-credential car-hire
/showcase/mpc-100k        ★ Flagship 2 — MPC £100k secure threshold
/showcase/solid-pairs     ★ Flagship 3 — Solid (user,app)-pair result sets

Core engine
  /try                    The live SPARQL REPL (the shared engine component)
  /surface/sparql         SPARQL 1.1/1.2 — SELECT/ASK/CONSTRUCT/UPDATE, paths, RDF 1.2 triple terms
                          (EXPLAIN / EXPLAIN ANALYZE are now wasm exports — `Store.explain` /
                           `Store.explainAnalyze` over `sparq_engine::explain*` (sq-ncvq.14 / #269),
                           so plan introspection is in-tab live in the lean default bundle)
  /surface/data-formats   Turtle / N-Triples / N-Quads / TriG + compressed ingest
  /surface/javascript-wasm  The @sparq-org/sparq browser/Node API (streaming, match, applyDelta)

Reasoning & validation
  /surface/inference      RDFS / OWL 2 RL / N3 closure + proof trees (why())
  /surface/shacl          SHACL Core + SHACL-SPARQL validation report

Search & retrieval
  /surface/full-text      BM25 text: magic predicates
  /surface/vector         Embedding store + cosine top-k (HNSW/DiskANN) + hybrid fusion
  /surface/genai          schema-card / VoID introspection + NL→SPARQL loop

Spatial & streaming
  /surface/geosparql      geof: functions + R-tree GeoIndex (map overlay)
  /surface/streaming-rsp  RSP-QL windows (sliding/tumbling, R/I/DSTREAM)

Privacy (ZK + MPC)
  /surface/zk             ZK query proofs (commitments, BGP+FILTER, issuer attest, revocation)
  /surface/mpc            Federated SPARQL across distrusting holders (Shamir, threshold)

Serving & hosts
  /surface/http-server    SPARQL 1.1 Protocol endpoint, GSP, /metrics, WS/SSE subscriptions
  /surface/cli            sparq-cli (query / reason / build / query-mmap)
  /surface/python         sparq pyo3 bindings

About
  /about                  Architecture, the honest "what runs where" matrix, links to paper/research
  (external) /dev/        the existing benchmark dashboard (co-hosted, see §5)
```

Each `/surface/*` page has a consistent layout: a one-line capability statement, a **live/
fallback badge** (a `Badge` coloured by tier — teal "Live in your tab", amber "Hosted",
slate "Walkthrough"), the interactive widget (or the captured-I/O replay), the example data +
query prefilled, a "how this works" disclosure, and a "real sparq, no mocks" honesty note that
states exactly which tier is running.

---

## 3. Per-surface live-execution strategy (HONEST)

Legend: **(a)** existing wasm · **(b)** new wasm bundle · **(c)** bb.js · **(d)** in-tab sim ·
**(e)** hosted server / captured-I/O walkthrough.

| Surface | Verdict | How / what runs | Honest caveat |
|---|---|---|---|
| **sparql-query** | **(a) LIVE** | The shipped wasm Store: SELECT/ASK/CONSTRUCT/UPDATE, BGP+WCOJ, FILTER, OPTIONAL, UNION, MINUS, BIND, VALUES, aggregates, paths, sub-SELECT, RDF 1.2 triple terms. | REGEX/REPLACE are compiled out of the lean bundle; QueryBudget deadline is native-only (row-cap only in wasm). |
| **data-formats** | **(a) LIVE** | `load`/`loadDataset` over the 4 text formats; gzip/zstd-compressed RDF ingest via `SparqStore.fromCompressed()` (JS-side decompress → `fromString`). (`loadCompressed` is unrelated — it stores the *index* block-compressed, not a gzip/zstd input decoder.) | HDT, mmap/external-memory, parallel fast paths are native-only → walkthrough for those. |
| **javascript-wasm** | **(a) LIVE** | This *is* the engine; the page documents the `@sparq-org/sparq` API (streaming cursors, `match()`, `count`, `applyDelta`) by calling it. | — |
| **inference** | **(b) NEW wasm bundle** | `sparq-reason` is pure-Rust forward-chaining; a `sparq-reason-wasm` bundle (RDFS/OWL2RL/N3 closure + `why()` proof tree) is portable. Pulls `regex` → larger bundle, lazy-loaded on this page only. | If the regex/rule-engine wasm size is unacceptable, fall back to **(e)** hosted/walkthrough. Confirm wasm-portability as a build spike. |
| **shacl** | **(b) NEW wasm bundle** | `sparq-shacl` is pure-Rust over `sparq-engine` (already wasm-portable); a `sparq-shacl-wasm` bundle validates data+shapes → W3C report in-tab. | SHACL-SPARQL constraints need the engine's REGEX (compiled out of lean bundle) — include it in this bundle or note the gap. |
| **streaming-rsp** | **(b) NEW wasm bundle** | `sparq-rsp` is documented "wasm-safe" (pure, no clock/async) — a small `sparq-rsp-wasm` bundle can run windows live with a UI-driven logical clock. | Not in any shipped bundle today; needs the build-change spike to confirm. |
| **full-text** | **(b) NEW wasm bundle** *or* **(e)** | `sparq-text` BM25 index is pure-Rust; a `sparq-text-wasm` could run small indexes in-tab. | Index build memory on big corpora is the risk; demo uses a tiny corpus. If the `text:` SPARQL hook can't be wired into the lean engine, fall back to hosted/walkthrough. |
| **geosparql** | **(e) HOSTED** | `sparq-geo` pulls the **georust** stack (proj/geos-like deps) — uneconomic to wasm-port. Run live against a hosted `sparq-server --features geo` endpoint; render results on a Leaflet/MapLibre map. | If no endpoint is deployed, **captured-I/O walkthrough** with the map overlay (still visually compelling). |
| **vector** | **(e) HOSTED / walkthrough** | `sparq-vectors` uses `mmap`/`std::fs` on-disk indexes — native-only. Hosted endpoint for live top-k, or captured-I/O. | Embedding generation needs a model; demo uses the deterministic `HashEmbedder` so it's reproducible. |
| **genai** | **(e) HOSTED + REPLAY** | `sparq-introspect` schema-card/VoID could be a **(b)** wasm bundle (pure over the scan API); the **NL→SPARQL loop** needs an LLM → hosted endpoint or a **recorded replay fixture** (`olympics_replay.json`) played in-tab. | Never call a live LLM from the static site (no key); use replay, or a hosted NLQ endpoint behind a rate limit. |
| **http-server** | **(e) LIVE-REMOTE** | A browser *can* hit a deployed `sparq-server` live: a query box that POSTs to the SPARQL Protocol endpoint, a GSP read/write panel, and a **live WebSocket/SSE subscription** demo (push an Update on the server → watch the subscription fire in the tab). | Requires a hosted endpoint; otherwise captured curl/WS-frame walkthrough. |
| **cli** | **(e) WALKTHROUGH** | Different host (terminal) — an asciinema-style captured-stdout replay of `query`/`reason`/`build`/`query-mmap`. | Inherently not browser-live. |
| **python** | **(e) WALKTHROUGH** (Pyodide = stretch) | Different host (interpreter) — captured REPL I/O. A future **Pyodide** build could run the pyo3 surface in-tab, but that's a large stretch; default = walkthrough. | — |
| **mpc** | **(d) IN-TAB SIMULATION** | `sparq-mpc` is deliberately native-only (not in wasm graph, uses `std::net`/`std::process`). Ship a **faithful JS re-implementation** of additive/Shamir sharing for the sum-threshold, purely for visualization, mirroring `sparq-mpc`'s `run_secure`. | Labelled "faithful illustration of the protocol the native crate runs, not the hardened crate in your browser." The collaborative-ZK-proof layer is a *stub* in the crate — do not claim it. |
| **zk** | **(c) bb.js LIVE** | Prove the existing Noir circuits **in-browser via barretenberg `bb.js` (UltraHonk WASM)**. Verdict below. | Single-threaded on Pages by default; COEP service-worker shim unlocks threads. Honesty: "research-grade verifier, sound as landed, pending Fable re-review." |

### 3.1 The ZK in-browser proving verdict (the load-bearing determination)

**Verdict: in-browser proving via bb.js is FEASIBLE for this demo; a hosted prover is an
optional UX upgrade, not a requirement.**

- **Stack:** Noir (`nargo` 1.0-beta.21) → ACIR → Barretenberg **UltraHonk** (`bb` nightly).
  Today proving is **subprocess-only** in `crates/sparq-zk-compose`; the one engineering item
  is wiring the proving step to the **bb.js WASM** path for the browser. The manifest/verify
  data model is already pure Rust/serde and portable.
- **Gate counts vs the ceiling:** bb.js WASM has a practical/device-dependent ceiling around
  **~2^19 (≈524,288) gates** (the repo elsewhere frames the browser limit as ~2^19–2^20
  constraints, not a strict spec cap — see `research/zkp-performance-landscape.md`). The
  entire sparq circuit family is **5,991–34,821 gates** (measured `bb gates` snapshot in
  `crates/sparq-zk-compose/tests/gate_count_snapshot.json`): `scan_k2_n64_r8` 34,821 (largest,
  ~15× under the ceiling), `filter_int/f64_d*` 17,416, `hidden_issuer_d4` 16,932, `holder_pok`
  10,334, `join_eq_na16_nb16` 7,025, `scan_k1_n16_r4` 5,991, `revoke_unset_d10` 899. A full
  car-hire manifest (2 scans + 1 filter + 1 issuer + 1 join ≈ 60–90k gates as *separate*
  sub-proofs, each <35k) never approaches the ceiling.
- **Timing (order-of-magnitude, non-canonical):** public anchor — Hyli proved a ~2M-constraint
  p256 circuit (~60× our largest) in **<3 s on an M1, multithreaded UltraHonk**; noir-benchmarks
  show ~505k gates at ~150 s single-thread → ~15.7 s at 64 threads (near-linear). Extrapolating
  to ~35k gates: **multithreaded ≈ low single-digit seconds per sub-proof; single-threaded
  ≈ ~10 s for the big scan, ~3–5 s for the smaller members.** Tolerable for a one-off live demo
  either way.
- **The COOP/COEP / threads constraint (deployment caveat):** bb.js multithreading needs
  `SharedArrayBuffer`, which the browser only enables under cross-origin isolation
  (`COOP: same-origin` + `COEP: require-corp` response headers). **GitHub Pages cannot set those
  headers**, so bb.js falls back to **single-threaded** there. Two paths: **(a)** ship
  single-threaded bb.js — works on Pages, no infra, seconds-to-tens-of-seconds per proof; or
  **(b)** a **`coi-serviceworker` COEP shim** that injects the headers client-side to re-enable
  multithreading on a static host. **Recommendation: single-threaded for the first build (zero
  infra, honest "this is proving in your tab right now"), with the COEP shim as a fast-follow
  UX upgrade.** Hosted prover microservice = optional offload only.
- **Soundness honesty (mandatory wording):** two audits exist. `research/zk-soundness-audit.md`
  found the *pre-remediation* v1 verifier BROKEN (6 critical). `research/zk-verifier-reaudit.md`
  finds the verifier **SOUND as landed** — all 12 findings closed with code evidence + forge
  tests — *under its stated threat model* (relying party supplies external trust anchors). So
  the demo MAY claim "the relying party cryptographically verifies the result, attestation,
  freshness and non-revocation against its own anchors," but MUST carry: **research-grade / v1
  / pending Fable ZK re-review**; the forge tests are `#[ignore]`d (closure partly by
  code-reading); and privacy deferrals (HolderPoP clear tier not yet credential-bound;
  status-list IRI/version disclosed → linkability). Do **not** claim full unlinkability by
  default.

---

## 4. The three flagship demos in detail

### 4.1 ★ ZK cross-credential car-hire — `/showcase/zk-car-hire`

**Narrative.** "Prove you may hire a car without showing your documents." The renter holds two
W3C Verifiable Credentials — a **gov-ID** credential and a **DVLA driving-licence** credential.
The car-hire desk (the verifier) must learn only: *(1) the holder is ≥ 25, (2) the holder has a
valid, non-revoked licence, (3) both credentials belong to the same person* — and nothing else
(not the date of birth, not the licence number, not the holder's identity).

**Circuit mapping (all circuits exist + are gate-counted today):**
- Two **`scan`** proofs — one BGP scan per credential graph (the gov-ID graph, the DVLA graph).
- **`filter_int_d*`** — the **age ≥ 25** predicate as a hidden-operand integer FILTER
  (`op=Ge, bound=25, expected=true`), bound to the gov-ID scan's age slot.
- **`hidden_issuer_d4`** ×2 — proves each credential was signed by an issuer in the desk's
  trusted set (gov, DVLA) *without revealing which key* (or clear-key attestation if issuer
  privacy isn't wanted).
- **`revoke_unset_d10`** — proves the licence is not revoked at a *hidden* status-list index.
- **`join_eq_na16_nb16`** — the headline primitive: proves the two credentials share the same
  holder term **without disclosing the holder** (only a per-presentation hiding commitment to
  the join value is public).
- **`holder_pok`** (recommended) — proves the presenter possesses the bound holder key.

**Live mechanism (tier-c).** On "Generate proof," the page loads bb.js (UltraHonk WASM) and
proves the manifest sub-proofs in-tab, showing a per-circuit progress list with the `aria-live`
"Proving…" pill (solid-app-shell pattern); then "Verify" runs the verifier (also wasm) against
the desk's trust anchors and flips a teal **ELIGIBLE** verdict. A **What the desk sees / What
stays private** two-column panel (reuse `--success`/`--muted` tokens) makes the disclosure
explicit:

| Desk sees (public) | Stays private |
|---|---|
| predicate text (age ≥ 25; "has a licence"; a holder-join exists) | exact **date of birth / age** (only "≥ 25 true" leaks) |
| graph commitments `C(G)`; both issuer attestations valid vs trusted set | the **holder's identity** (join value hidden behind a hiding commitment) |
| freshness nonce; non-revocation result | the **licence number** + other undisclosed slots; which status-list index |
| *that* a join exists, its slot positions + cardinality | which issuer signed (if hidden-issuer path) |

**Honesty caveats shown in-page:** join slot positions + cardinality are public (the query
reveals them); status-list IRI/version are disclosed (linkability); research-grade verifier,
pending Fable re-review. **Build gaps to flag** (so the demo is end-to-end): wire bb.js proving;
confirm the verifier-side `bind_joins` stage for the `k=2` join is plumbed through
`verify_manifest`; date-of-birth→age derivation is *not* an existing circuit (credential must
carry an integer age claim, or pre-computed "≥25" boolean) — state this honestly.

**Fallback if bb.js wiring slips:** a captured pre-generated `ProofManifest` + a live in-tab
**verify** (verify is cheap, the prove is the expensive part) — still a real cryptographic check
in the tab, with the prove step shown as a recorded animation.

### 4.2 ★ MPC £100k secure threshold — `/showcase/mpc-100k`

**Narrative.** Four flatmates want to know if their **combined** income clears a £100k mortgage
threshold — *without anyone revealing their salary, and without even revealing the exact total*.
Only the **verdict bit** (`sum ≥ £100k`) is opened. (Mirrors the crate's tested
`four_flatmates_hundred_k_verdict` path: 30k/28k/26k/24k = 108k → true.)

**Live mechanism (tier-d, in-tab simulation).** `sparq-mpc` is deliberately native-only (uses
`std::net`/`std::process`, excluded from the wasm graph), and bringing it to wasm to run a
*visualization* is uneconomic. So ship a **faithful, ~few-dozen-line JS re-implementation** of
Shamir/additive secret-sharing for the sum-threshold, mirroring the crate's `run_secure`
(zero-round local addition over shares). It is **labelled honestly**: "a faithful illustration
of the protocol the native `sparq-mpc` crate runs — not the hardened crate executing in your
browser." The collaborative-ZK-proof-of-correctness layer is a *stub* in the crate today, so the
demo must **not** claim a proof of correctness.

**What is visualized (one page, four party panels, default N=4):**
- Each party's **private value** with a "stays on this device" badge.
- The **N×N shares matrix**: party *i* splits its value into N shares; the diagonal/own column
  = "kept," off-diagonal cells = "sent to peer *j*" (this is *what crosses the wire*). Show that
  any single column (≤ t shares) is uniform-random → reveals nothing.
- Each party's **received view** (the column it gets) — visibly independent of any secret.
- The **local sum** per party → the combine step → the secret-shared total.
- The **opened bit only**: the final reveal shows just `total ≥ £100k = true/false`; the exact
  total is shown **struck-through / redacted** to emphasize it is the value that is *not*
  revealed. A "what the bank learns (one bit) vs what it does NOT learn (the salaries + the
  exact sum)" contrast panel is the money shot.

**Fallback:** none needed — the simulation *is* the in-browser live experience. (A "view the
real native run" link can show captured `run_federated` console output for provenance.)

### 4.3 ★ Solid (user, app)-pair result sets — `/showcase/solid-pairs`

**Narrative.** One Pod of data. The **same query** returns **different result sets** depending
on *who is asking from which app* — `(userA, appX)` vs `(userB, appX)` vs `(userA, appY)` — as
enforced by the Pod's WAC/ACP access control. The session key is exactly the **(agent, client)
pair** (`sparq-solid`'s `Session { agent, client }`).

**Live mechanism (tier-a — existing wasm, NO new bindings).** The key structural fact:
`sparq-solid`'s enforcement reduces to a **`FROM NAMED <authorized-graphs>` dataset
restriction** (its `query_as_rewrite` path) — and the **existing wasm Store already runs that**
via `loadDataset` (preserves named graphs) + standard SPARQL `FROM NAMED`. So:
1. Load the fixture Pod (one named graph per document) into the wasm Store via `loadDataset`.
2. For each `(user, app)` pair, obtain its authorized named-graph set. **Cleanest:** precompute
   the three sets natively with `sparq-solid` at build time (or ship the materialized
   `<urn:sparq:auth>` graph and run the `accessible(...)` decision as a SPARQL query in-tab —
   the auth view is itself queryable triples).
3. Run the **same** user query, rewritten with the pair's `FROM NAMED` set (the crate's
   `rewrite_for` transform) → three different result sets, **live in-tab**.

The page shows a `(WebID, app-origin)` session selector, one shared query box, **three
side-by-side result panels**, an "authorized graphs for this session" panel making the
`FROM NAMED` set explicit, and the **fail-closed** property (anonymous/unknown session → empty,
indistinguishable from absent). Optionally wire the **real Solid login** here via
`solid-reactive-authentication` + `solid-client-id` (log in as userA, switch apps) — the auth
flow is live even though the access-control decision is precomputed.

**Honest note:** the WAC/ACP *materialization* engine (`sparq-solid` + the N3 reasoner) runs
**natively/at build time**, not in the browser; the in-tab part is the SPARQL `FROM NAMED`
enforcement (which is the real engine doing the real restriction). A "purity upgrade" — a
`PodStore` wasm binding running materialization in-tab — is a **later** option (a tier-b
bundle), explicitly **not** a prerequisite.

---

## 5. Stack + hosting

- **Build:** Next.js 15.5 / React 19 / Tailwind v4 / shadcn (`radix-nova`) / `next-themes`,
  matching §1, **statically exported** (`output: 'export'`) to `out/`.
- **Pages base path:** the repo's Pages site is served at **`https://sparq.jeswr.org/`**
  (project page, not a user page), so set **`basePath: '/sparq'`** + matching `assetPrefix` and
  load all wasm/bb.js assets through the base path. (Confirmed: `gh api repos/sparq-org/sparq/pages`
  → `html_url: https://sparq.jeswr.org/`.)
- **Co-hosting with `/dev/bench`:** today Pages is served from the **`benchmark-data` branch**
  at `/` (`build_type: legacy`), whose tree is `index.html` + `dev/` — i.e. the benchmark
  dashboard lives at **`/sparq/dev/`**. The showcase becomes the **new root site**; we must
  **preserve `/dev/`**. Two clean options:
  - **(preferred) Switch Pages to GitHub Actions deployment** (`actions/deploy-pages`): a
    workflow builds the Next.js `out/`, then **overlays the existing `dev/` tree** (fetched from
    the `benchmark-data` branch, or kept as a build input) into `out/dev/` before upload, so one
    artifact serves both the showcase root and the unchanged bench dashboard. Add `out/.nojekyll`.
  - **(lower-effort) Keep legacy branch publishing:** build `out/` and commit it (plus the
    preserved `dev/`) to the `benchmark-data` branch. Workable but couples site source to the
    data branch — prefer the Actions path.
- **Deploy workflow (SHA-pinned actions, least-privilege):** new `.github/workflows/pages.yml`
  on push to `main` — checkout, setup-node 22, build the wasm bundles (`wasm-pack`, reuse the
  `js.yml` pattern) + any tier-b bundles, `npm run build` (Next export), fetch/overlay `dev/`,
  `actions/upload-pages-artifact` → `actions/deploy-pages`. Pin every action by commit SHA
  (match the repo's existing pinning convention) and grant `pages: write`/`id-token: write`
  only on the deploy job; everything else `contents: read`. Gate it through the `ci-summary`
  aggregator + branch protection like the other workflows.
- **WASM asset loading:** the lean `sparq_wasm.js`+`.wasm` (~886 KB) loaded on `/try` and the
  core surfaces; each **tier-b** bundle (`sparq-shacl-wasm`, `sparq-reason-wasm`, etc.)
  **lazy-loaded only on its page** (`next/dynamic`, client-only) to keep the landing page light;
  **bb.js** (`@aztec/bb.js`) lazy-loaded only on `/showcase/zk-car-hire` + `/surface/zk`
  (it is large) — with the optional `coi-serviceworker` registered there for threads. The MPC
  sim is plain JS (no wasm). Solid demo loads the lean bundle + the auth libs.
- **Hosted backend (tier-e, optional):** if/when GeoSPARQL/vector/NLQ/http-server go live, a
  single small `sparq-server --features geo` instance behind CORS; the static site calls it.
  Until then those pages are captured-I/O walkthroughs. (No backend is required to ship the
  site — the three flagships are tier-c/d/a.)

---

## 6. Build bead graph

Children of `sq-4r4b` (foundation → parallel demos + new-wasm bundles + flagships → deploy).
Created with `bd create` (see §7 for actual ids assigned at creation). Dependency intent:

```text
F1 site-shell+theme  (foundation — port the pod-manager design language, shell, routing, REPL component)
   ├─ depends: nothing
   │
   ├─► P-* per-surface demo pages (parallel, each depends only on F1):
   │     P-sparql (a)        P-formats (a)       P-jswasm (a)
   │     P-httpserver (e)    P-cli (e)           P-python (e)
   │     P-geo (e)           P-vector (e)        P-genai (e)
   │
   ├─► W-* new tier-b wasm bundle beads (parallel, depend on F1; each unblocks its page):
   │     W-shacl  ──► P-shacl
   │     W-reason ──► P-inference
   │     W-rsp    ──► P-streaming
   │     W-text   ──► P-fulltext   (W-* may be spikes first: confirm wasm-portability)
   │
   ├─► FL1 zk-car-hire   depends: F1 + WZK (bb.js proving wiring bead)
   │     WZK  bb.js in-browser UltraHonk proving wiring (+ optional coi-serviceworker)  [also unblocks P-zk]
   │     (build gaps: confirm bind_joins k=2 verifier stage; note DOB→age not a circuit)
   │
   ├─► FL2 mpc-100k      depends: F1 only  (in-tab JS sim — no wasm/crate dep)
   │
   ├─► FL3 solid-pairs   depends: F1 + DSOLID (build-time authorized-graph-set precompute)
   │     DSOLID  precompute (userA/userB × appX/appY) authorized FROM NAMED sets via sparq-solid
   │     (+ optional solid-reactive-auth login wiring)
   │
   └─► D1 pages-deploy-workflow  depends: F1 (+ ideally the demo pages exist)
         Actions Pages deploy that overlays the existing /dev/ bench dashboard; SHA-pinned.
```

Sizing: each bead is one context-independent deliverable a single agent can finish (one page,
one wasm bundle, one workflow). The `W-*` bundles are gated by a cheap **portability spike**
each (confirm the crate compiles to `wasm32-unknown-unknown` without `std::fs`/threads) before
the full bundle work — if a spike fails, that surface drops to tier-e (hosted/walkthrough) and
its `P-*` page builds as a walkthrough instead.

---

## 7. Honest summary verdict (per surface)

- **Live in your tab, today, zero new code:** sparql-query, data-formats, javascript-wasm,
  **Solid pairs** (FROM NAMED on the existing wasm Store), **MPC £100k** (JS simulation).
- **Live in your tab with a new wasm bundle (portability spike first):** SHACL, inference,
  streaming-RSP, full-text. Risk: bundle size / the engine REGEX gap.
- **Live in your tab via 3rd-party WASM:** **ZK proving via bb.js** — feasible, circuits 15×
  under the gate ceiling, single-threaded works on Pages (COEP shim unlocks threads), verifier
  **sound as landed** but **research-grade / pending Fable re-review**.
- **Hosted server or captured-I/O walkthrough (honest fallback):** GeoSPARQL (georust),
  vector (on-disk indexes), GenAI NLQ (LLM), http-server (live-remote subscriptions possible),
  CLI + Python (different hosts).

This satisfies "live everywhere" as far as it is honestly achievable: three tiers of real
in-tab execution cover the majority of surfaces and all three flagships, and every remaining
page shows **real sparq output** (captured from the native engine) rather than a mock.
