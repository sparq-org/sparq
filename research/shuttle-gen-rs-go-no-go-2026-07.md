# Shuttle gen-rs: build + GO/NO-GO evaluation vs oxttl (sq-tonhr.1)

> 🤖 SPARQ agent (Fable), 2026-07-11. Bead **sq-tonhr.1** of epic sq-tonhr
> (design record: `research/shacl-compact-and-shuttle-parsers-2026-07.md`).
> This is the canonical measurement record for the gen-rs go/no-go gate;
> cite it rather than re-quoting numbers.

## What was built (upstream, jeswr/rdf-shuttle)

The Shuttle **Rust backend did not exist** (verified in the design record);
it does now: `packages/gen-rs` beside gen-js, consuming the same v0.1
grammar AST (gen-js's `meta.js` front end + shared token analysis), emitting
**one dependency-free `.rs` module per grammar** — streaming parser, chunked
push parser (bounded memory, mid-token suspension, per-statement rollback),
and serializer. Upstream commits `763182c` (backend + conformance identity)
and `f157d31` (interning reversal + bench harness), both on main per that
repo's convention.

Artifact contract, all **verified green** on the generated `turtle12.rs`:

- single file, **zero dependencies** (std-only), **no `unsafe`**
  (`#![forbid(unsafe_code)]`-compatible; the harness compiles it under
  `forbid`);
- **MSRV 1.87**: build + `clippy --all-targets -- -D warnings` green on
  1.87.0 and stable 1.97 (a small documented allow-list of *style* lints in
  the artifact header — `non_snake_case`, `manual_range_contains`, etc.;
  correctness lints all on); `RUSTDOCFLAGS="-D warnings" cargo doc` green;
- **conformance identity to gen-js**: all 22 upstream oracle pairs produce
  **byte-identical** canonical dumps in **5 modes** (parse `.nt`, parse
  `.ttl`, plain round trip, abbreviated round trip, 7-byte-chunk push
  parse), *including byte-identical serializer output* — deterministic
  blank-node allocation on both backends turns cross-backend conformance
  into a plain `diff`, no isomorphism needed (`packages/gen-rs/test/conformance.sh`);
- the identity harness is **mutation-checked non-vacuous** (a seeded
  fresh-bnode-counter offset turns the diff red);
- negative cases return the grammar's stable codes
  (`UNDECLARED_PREFIX` / `UNEXPECTED_TOKEN` / `LEX`).

Documented divergences from the JS artifact (in the artifact header +
upstream README): UTF-8 byte spans (vs UTF-16 units), and lone-surrogate
`\u` escapes are an `INVALID_CODEPOINT` error (JS strings hold lone
surrogates; Rust `String` cannot).

## Bench setup (honesty notes first)

- **Box**: the shared EC2 work box (8 cores, otherwise idle, load < 0.6)
  — per `feedback-ec2-benchmarks` these absolutes are **non-canonical**;
  the *ratios* are the finding, they are decisive (≥ 1.4×) and were stable
  across repeated runs. `packages/gen-rs/harness bench <file>` reproduces
  the same-binary comparison anywhere.
- **Same binary, same allocator, same term-materialization level**: gen-rs
  emits owned `Rc`-backed terms via a callback; oxttl 0.2.3 (`rdf-12`
  feature, pinned `=0.2.3`) yields owned oxrdf terms from `for_slice`;
  both sides `black_box` every term. This is the apples-to-apples
  "text → owned terms, 1 thread" comparison the gate asked for.
- **Corpora** (the canonical `bench/parse` generators):
  `parse-baseline gen 320000` → `synthetic.ttl` (60 MB, pname-heavy
  Turtle) + `synthetic.nt` (170 MB, repeated-IRI N-Triples), and
  `gen_highcard.py 2000000` → `highcard.nt` (192 MB, unique subject +
  unique literal per line — the dict-heavy worst case). Median of 5 runs.
- gen-rs parses `&str`; oxttl parses `&[u8]`. Feeding gen-rs from raw bytes
  adds one `str::from_utf8` pass, measured at **0.047 ns/byte** (21 GB/s)
  on the largest corpus — < 1.1% of gen-rs's cost, immaterial.

## Results (this box, 2026-07-11; median of 5; higher MB/s = better)

| corpus | gen-rs one-shot | gen-rs push 64 KiB | oxttl Turtle | oxttl NT | gen-rs vs **best** oxttl |
|---|---|---|---|---|---|
| synthetic.ttl (Turtle) | **59.3 MB/s** | 58.8 | 29.1 | n/a | **2.04×** |
| synthetic.nt | **155.2 MB/s** | 148.5 | 68.4 | 98.7 | **1.57×** |
| highcard.nt | **218.4 MB/s** | 208.6 | 105.1 | 155.7 | **1.40×** |

Per-dimension reading:

- **Turtle throughput**: gen-rs ≈ 2× oxttl's TurtleParser.
- **N-Triples through the *full Turtle grammar***: gen-rs beats even
  oxttl's *specialized* NTriplesParser by 1.4–1.6×.
- **Streaming/push overhead**: 1–5% vs one-shot (statement-rollback push
  mode with mid-token suspension) — the property `load_reader_parallel`
  integration needs.
- **Conformance at that speed**: identity to gen-js (which is 92/92 on the
  upstream suite); the W3C-suite ratchets are sq-tonhr.2's gate, not yet
  wired.

### The design finding that produced the margin (measured reversal)

The first cut faithfully mirrored gen-js's term-interning (NamedNode map +
two-level pname cache). Measured, **both caches are net losses in Rust on
every corpus profile tried** — including the repeated-term profiles they
exist for: per-term *hashing* costs more than per-occurrence `Rc`
*allocation* (the reverse of the JS trade-off, where object allocation +
GC pressure dominate). Removing them took synthetic.ttl 36→59 MB/s,
synthetic.nt 93→155, highcard.nt 87→218, with conformance identity
unchanged. Consumers that want shared terms intern downstream (exactly
what a sparq dictionary sink on the emit callback would do).

### Triangulation vs the sparq incumbent (same box, `parse-baseline bench-nt`, synthetic.nt)

| stack | threads | MB/s |
|---|---|---|
| oxttl NT parse-only | 1 | 99 |
| **gen-rs turtle12 (owned terms, full Turtle grammar)** | 1 | **155** |
| incumbent custom NT **parse+intern** (direct-Dict `[Id;3]`) | 1 | 171 |
| incumbent custom NT parse+intern | 8 | 918 |

The oxttl rows cross-validate the two harnesses (99 vs 98.7). Single-thread,
gen-rs is within ~10% of the incumbent's parse+intern *while doing a
different job* (owned-term emission vs direct-Dict interning, full Turtle
grammar vs NT-only). The incumbent's decisive advantage is 8-thread chunk
parallelism — which the design record assigns to sparq's layer regardless
of parser, and the push/span contract supports.

## Verdict: **GO** for hot-syntax generalization

The gate question was "can Shuttle-generated Rust reach oxttl-class
performance at conformance?" The measured answer is that it *exceeds*
oxttl-class: ≥ 1.4× the best oxttl parser on every corpus profile tried,
at cross-backend conformance identity, in a zero-dep, no-unsafe, MSRV-1.87,
clippy-clean single file. Phase-3 beads (sq-tonhr.8 NT/NQ, then .9 Turtle
1.2/TriG) should proceed, **with the no-regression strategy unchanged**
(opt-in features, set-identity ratchets first — sq-tonhr.2 — bench gates,
per-syntax default-flip verdicts under sq-tonhr.11).

Caveats that keep this honest (none block the GO):

1. Absolutes are work-box numbers; the canonical bench/parse rows for any
   default-flip verdict must come from the quiet-EC2 procedure. The ≥1.4×
   margins dwarf box noise (repeated runs varied by low single digits).
2. Corpus coverage is the two canonical generators; a real-data slice
   (e.g. the Wikidata truthy slice) should be added when sq-tonhr.2 wires
   the differential+bench harness. The three profiles already span the
   repetition-cardinality axis in both directions with the same outcome.
3. W3C-suite conformance (beyond the 22-pair identity + gen-js's 92/92) is
   deliberately a separate gate (sq-tonhr.2 ratchets) before any sparq
   wiring; equality-of-outcome-set vs oxttl remains the bar.
4. Backend gaps that gate the *next* grammars, not this verdict:
   `emit @graph` (needed for N-Quads/TriG) is not yet compiled; the
   FIRST-set bitmask supports ≤ 128 token kinds (array fallback needed
   beyond); `@maxdepth` is a fixed constant. Tracked as beads under
   sq-tonhr. **Superseded 2026-07-27 (sq-uyney) — see
   [`shuttle-gen-rs-backend-gaps-2026-07.md`](./shuttle-gen-rs-backend-gaps-2026-07.md):**
   the mask fallback and grammar-driven `@maxdepth` are both closed
   upstream (verified + tested at `89bbe6d`, and output-neutral for
   sparq's vendored SHACL-CS artifacts); `emit @graph` remains open and
   is **not** a gen-rs-specific gap — gen-js refuses it identically, and
   the declared `emits` header is read by neither backend.
5. Single-thread comparison by design; parallelism composes at sparq's
   chunk layer and is measured at that integration (sq-tonhr.8), where the
   span-emission `(kind,start,end)` contract exists precisely so the
   intern-direct shim doesn't forfeit the incumbent's direct-Dict edge.

## Reproduction

```sh
git clone https://github.com/jeswr/rdf-shuttle && cd rdf-shuttle/packages/gen-rs
./test/conformance.sh                       # identity gate (node + cargo >= 1.87)
(cd harness && cargo build --release)
harness/target/release/shuttle-rs-harness bench <corpus.ttl|.nt> [iters]
# corpora: sparq bench/parse `parse-baseline gen 320000 …` + `gen_highcard.py 2000000`
```
