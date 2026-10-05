# sparq GUI — operational workbench (Tauri 2 desktop + hosted web)

A cross-platform **operational workbench** for the sparq RDF + SPARQL engine, per the design
record [`research/gui-design.md`](../research/gui-design.md) §A (beads `sq-ixc3.8` / `sq-ixc3.9`,
epic `sq-ixc3`). It is a **DISTINCT app, not the marketing site in a window**: a dense IDE-style
workbench (left rail · thin top bar · IDE tab strip · status bar) over a live engine, where the
showcase capabilities are TOOLS you run, not pages that describe them.

> **Foundation shell.** The distinct frontend + app shell stands up a working Query tool over the
> live in-tab engine, the Cmd-K palette (`sq-ixc3.10`), the multi-view query workbench
> (`sq-ixc3.12`), surfaces-as-tools (`sq-ixc3.11`), and the **Import drawer** (`sq-ixc3.13` — real
> disk/URL/paste ingest via the native loader); the remaining TOOLS open as honest stubs the later
> phases fill. The full per-platform desktop `cargo tauri build`
> needs the webview **system libraries** (webkit2gtk on Linux, WebView2 on Windows, WKWebView on
> macOS) validated in CI ([`.github/workflows/gui.yml`](../.github/workflows/gui.yml)), not
> necessarily on every dev box.

## Two builds, one frontend

The operational frontend (`gui/app/`) static-exports (`output: export`, backend-free) for BOTH
targets, selected by the single `NEXT_PUBLIC_BASE_PATH` switch (the same switch the site + the
`@sparq/client` wasm loader key off, kept in lockstep):

| Target | Command (`gui/app`) | `basePath` | Where it runs |
|---|---|---|---|
| **Tauri desktop** | `npm run build:tauri` | `""` (root-relative) | embedded in the desktop webview (serves from the `tauri://` root) |
| **Hosted "Try the GUI live" web** | `npm run build:web` | `/sparq/app` (default; overridable) | a static page hosted at the site's **App** destination so the live-GUI link has a real target (beads `sq-wt4s3` + `sq-vnd0i`; `/try` stays the lightweight REPL) |

The desktop bundle is wired via `gui/src-tauri/tauri.conf.json` — `frontendDist: "../app/out"`
and `beforeBuildCommand: cd ../app && npm run build:tauri`. **`frontendDist` is no longer
`site/out`** (the core of `sq-ixc3.8`): the GUI never embeds the marketing site.

## Why Tauri 2

The design weighed Tauri 2 vs egui/iced vs Electron/PWA and chose **Tauri 2** because it (1)
reuses the existing React/Tailwind stack + the WASM engine client, (2) unlocks the **full native
engine** on desktop (rayon, mmap, `save`/`open`, no ~2 GiB tab ceiling) via a direct Rust link,
(3) closes the static-site live-backend gap, and (4) is the only credible mobile path here. See
`research/gui-design.md` §1 for the full comparison and honest caveats.

## Layout

```
gui/
  README.md
  app/                  # NEW: the DISTINCT operational frontend (Next.js, static-exported)
    next.config.ts      #   env-switched basePath: build:tauri (root) / build:web (sub-path)
    src/
      app/              #   single route — the workbench shell (no /about, /benchmarks, …)
      components/
        workbench/      #   the shell: left-rail · top-bar · tab-strip · status-bar +
                        #     query-workbench (the working Query tool) + tool-stub (honest stubs)
        ui/             #   reused design-system primitives (button, badge) — sibling of the site
      data/tools.ts     #   the TOOLS taxonomy (each a VERB with an honesty-tier dot)
      lib/engine-context.tsx  # the ONE live wasm store + measured-latency query path
  e2e/                  # tauri-driver (WebDriver) e2e — launches the shell, runs a query, asserts
  src-tauri/
    Cargo.toml          # standalone crate root (own [workspace]); links sparq-engine + sparq-core
    tauri.conf.json     # window + bundle + frontendDist (= ../app/out)
    build.rs            # tauri_build::build()
    capabilities/       # least-privilege window capability (core + dialog:open)
    src/
      main.rs           # desktop binary -> sparq_gui_lib::run()
      lib.rs            # Tauri builder: manages EngineState, registers the command handlers
      engine.rs         # the DIRECT native engine link: load/query/.../ask + the Import drawer's
                        #   native loader (load_path/load_text: compressed + native-only HDT)
```

## The app shell (sq-ixc3.9)

`gui/app/src/components/workbench/` is the operational shell from `gui-design.md` §A.2:

- **Left rail** (`w-56`): a workspace switcher (the persistent model is `sq-atb0`; foundation =
  default), a **datasets tree of the live store** (default + named graphs with per-graph counts)
  with an **Imports** subgroup (the active workspace's `WorkspaceSourceMeta` — file + re-fetchable
  URL sources) and a working `+ Import` entry point (opens the Import drawer), and the **TOOLS**
  list — each a VERB opened as a tab with an honesty-tier dot, never a page describing the feature.
- **Top bar** (`h-10`): a LOCAL⇄ENDPOINT target switch, the store size, a ⌘K hint, a theme toggle,
  and an engine status LED.
- **IDE tab strip** + a full-bleed work area (default = Query).
- **Status bar** (`h-7`): the **measured `performance.now()` latency** of the last run, the row
  count, the target, the persistence backend, the live store size, an unintrusive ingest meter
  (`sq-vw3ax`, #820), and a **disk gauge**. The disk gauge shows the **OS-reported** on-disk size
  of the app-data `workspaces/` dir in the desktop shell (a recursive native `stat()` via the
  `disk_usage` command, `sq-cno90`) and falls back to the **snapshot-bytes estimate** on the web
  target — always labelled which of the two it is (`disk` vs `≈disk`), never a fabricated figure.

## The direct native engine link (desktop)

`src-tauri/src/engine.rs` is the concrete proof of the design's embedding decision: a native
`sparq_core::Graph` behind a `Mutex`, with Tauri commands that mirror the wasm `Store` surface
(`load` / `query` / `query_quads` / `update_in_place` / `explain` / `explain_analyze` / `count` /
`ask` / `store_size`) but backed by the full native store. Its `#[cfg(test)]` tests exercise the
engine calls directly. The foundation frontend runs the in-tab WASM engine in BOTH targets for
**query** (the honest, working-today path). `src-tauri/src/disk.rs` adds the `disk_usage` command
(`sq-cno90`): a recursive `stat()` of `$APPLOCALDATA/workspaces` that returns its real on-disk byte
total for the status bar's disk gauge (least-privilege — confined in Rust to that subtree, follows
no symlink out of it; the byte-summing walk is unit-tested directly).

<!-- [GPT-5.6] sq-n18o5 — browser URL imports now share the compressed-file decoder. -->
For **ingest**, the Import drawer (`sq-ixc3.13`) calls the engine's **native loader** IPC
(`load_path` / `load_text`) when running in the desktop shell: a disk file — including
**compressed** (`.gz` / `.bz2` / `.zst`) streams and **native-only HDT** (`.hdt` / `.hdt.gz`,
behind the crate's opt-in `hdt` feature) — is decoded by the native engine (threads, no ~2 GiB
wasm-tab ceiling) and handed back as N-Quads for the in-tab store to merge (named-graph-
preserving). On the hosted web target, the drawer parses paste, file uploads, and URL responses in
the in-tab WASM engine; compressed uploads and URL responses (`.gz` / `.zip` / `.zst` / `.bz2`)
are decoded in the browser before parsing, while HDT remains native-only. A successful import
records a `WorkspaceSourceMeta` + a workspace snapshot (the `sq-atb0` save/open cache). The full
on-disk workspace persistence path activates once the shell grants the `fs` capability
(`sq-ixc3.6`).

For **federation** (`sq-ixc3.14`), a SERVICE-bearing SELECT/ASK in the Query tool routes to the
native `query_service` command (behind the crate's opt-in `federation` feature, which forwards to
`sparq-engine/service`): the live store's N-Quads snapshot is evaluated natively, joining remote
SPARQL endpoints under the engine's **strict fail-closed egress allowlist** — a SERVICE clause may
dial ONLY the per-workspace `Federation` control's entries (host / host:port / `*.suffix`, the
same grammar as sparq-server's `--service-allow`); everything else, including public hosts, is
refused pre-HTTP. The browser build labels SERVICE **native-only** (CORS) instead of pretending.

For **usage control** (`sq-ixc3.15`), the **Policies (ODRL) tool** runs its whole round-trip in
one native command (`odrl_preview`, behind the crate's opt-in `odrl` feature, pulling the
research-track `sparq-policy` evaluator + `sparq-solid` enforcement store): author/validate a
Turtle ODRL policy, evaluate a (party, action, target) request — decision + matched rules + unmet
constraints — then run the SAME SPARQL query ungated AND per requester through `PodStore`'s
**fail-closed** per-session named-graph gating (the one-shot `odrl-bridge` materialization path;
the `*_conditional` variants with the bare-assignee widening hazard, bead `sq-9n1q4`, are not
wired). A malformed policy materializes **nothing** — deny-everything with the parser's verbatim
reason — and an `odrl:prohibition` visibly flips a previously visible named graph to hidden in
that requester's pane. The browser build labels the tool **native-only** (the ODRL stack is not
in the wasm bundle) instead of pretending.

## File ingest library (`lib/file-ingest.ts`, sq-vnh1v)

The **file ingest library** is a shared zero-server multi-file upload harness for the RDF import (sq-eydh9), SHACL shapes (sq-txrui), and N3 rules (sq-glo5r) surfaces. It operates on a single `IngestResult` contract: every file is `accepted[]` (name, text, bytes) or `rejected[]` (name, reason) — **no silent drops**. 

**Entry points** — `pickTextFiles(opts)` opens a file picker using File System Access `showOpenFilePicker` where available, falling back to `<input type="file" multiple>` for browser parity (the floor); `readDroppedFiles(dataTransfer, opts)` extracts files from a drop event (must be called synchronously from the drop handler). Both work in the static `/app` export with no server and no Tauri global.

**UI layer** — `<Dropzone>` (standalone dashed panel + keyboard-accessible button), `<DropTarget>` (wraps children as a drop surface with a hover overlay), and `useFileDrop` (raw drag/drop wiring for custom affordances). All three consume the same `IngestResult` contract, so callers surface accepted and rejected files identically.

## Inference (per-workspace entailment) — `sq-tp1m`, `sq-glo5r`

The **Inference tool** (and a compact selector in the Query action row) applies an **RDFS / OWL 2
RL / N3** entailment regime to queries over the live store, **per workspace**. It is real
forward-chaining, not a mock: a non-`off` mode lazily loads the tier-b **W-reason** wasm bundle
(`crates/sparq-reason-wasm`, the same reasoner the site's `/surface/inference` page runs),
materialises the deductive **closure** over the whole dataset (named graphs folded into the
default graph), and runs **read** queries against that closure so an *entailed* triple can match.
It is a **query-time** regime — it never mutates the persisted store; an `UPDATE` always targets
the asserted data and invalidates the cached closure. The chosen mode is **persisted on the
workspace** (`Workspace.inference`) and restored on reopen.

The W-reason bundle is **optional at build time**: `app/scripts/sync-wasm.mjs` copies it into
`public/wasm/reason/` when present and warns-and-skips otherwise, so the frontend build never
hard-fails on it. The CI `gui-app` job builds it (`js`: `npm run build:reason-wasm`) so the
exported artifact ships live inference; a build without it degrades **honestly** — the tool shows
a "reasoner unavailable" state, and a query issued while a non-`off` mode is active then fails with
a clear message (turn inference **Off** to query the asserted data, or rebuild the bundle) rather
than silently returning un-reasoned results.

**N3 mode** carries a persisted rules document list on each workspace. The Inference tool can
author rules inline or bulk-add `.n3` / `.ttl` files, and each document can be enabled, disabled,
or removed independently. At query time the engine combines the enabled rules with an N-Triples
snapshot of the asserted store and runs `reasonN3`; derived ground triples are loaded into the
query-time closure. [SONNET-4.6] This is the honest live-wasm boundary: formula and quoted-graph
conclusions are not inserted because the in-tab store represents ground triples only.

## Shared TS client

The frontend consumes the framework-agnostic `@sparq/client`
([`packages/sparq-client`](../packages/sparq-client)) for the wasm `Store` type surface, the
loaders, and the result-shaping helpers — the **same package the site uses** — so the GUI is a
**zero-new-copy** consumer, not a third hand-redeclaration of the engine's TS surface.

## Honesty

No performance number is baked in — the status bar shows the **measured** latency of the query you
just ran, labelled as such (this work box / CI runner is non-canonical). The ZK/MPC tools are
**research-grade and not externally audited** (the v1 ZK verifier is internally re-audited only,
external accredited-cryptographer sign-off is pending; `sparq-mpc` is honest-majority semi-honest
only, and the site's MPC demo is an in-tab JS simulation, not live MPC) — the tool stubs carry
those caveats verbatim and never present either as a settled cryptographic guarantee. A
`walkthrough`/`soon` tool is never silently dressed up as `live`.
