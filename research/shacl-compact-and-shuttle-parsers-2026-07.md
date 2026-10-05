# Native SHACL Compact Syntax (+ extended) via rdf-shuttle-generated parsers — design record

> 🤖 SPARQ agent (architect synthesis, 2026-07-11). Epic: **sq-tonhr**.
> Sources: study fan-out over jeswr/rdf-shuttle, jeswr/shaclcjs, jeswr/shaclc-1.2, and the
> in-repo parser/SHACL estate; live-verified facts are dated. Perf figures cite their
> canonical records (`research/gap-parse-2026-07.md`, `research/custom-parsers-baseline.md`,
> upstream `bench/README.md`) — do not re-quote from here.

## 1. Problem and maintainer intent

The maintainer wants (verbatim intent, 2026-07):

1. **Native SHACL Compact Syntax (SHACL-CS) in sparq**, including the **extended** syntax
   from jeswr/shaclcjs (turtle-style shape annotations, `% … %` property escapes, trailing
   turtle statements, `a` keyword).
2. A **spec** for the extended compact syntax in the house UPD/ReSpec format (sq-rvgr2
   factory), destined for jeswr/solid-specs (repo does not exist yet — verified
   2026-07-11; land in-repo meanwhile).
3. Implement **SHACL Compact Syntax 1.2** (jeswr/shaclc-1.2 — RDF 1.2 triple terms,
   reifiers, dir-lang tags; `sh:TripleTerm`, `sh:reifierShape`, `sh:reificationRequired`).
4. **Describe** the extended compact syntax as a **Shuttle grammar** (rdf-shuttle
   meta-language).
5. Use rdf-shuttle's **Rust parser/serializer generator** for the SHACL-CS
   parser+serializer, and **generalize** it to sparq's other RDF syntax parsers
   (Turtle/N-Triples/TriG/N3).
6. **Hard constraint:** do NOT degrade performance or correctness of the existing
   parsers — ideally improve them.

### Reality corrections the plan is built on (all verified in the study)

- **The Shuttle Rust generator does not exist.** Zero `.rs`/`Cargo.toml` across every
  branch and PR of jeswr/rdf-shuttle. Rust is roadmap only (spec §12 `--emit rust`,
  "later waves", sequenced *last* in upstream issue #4, after the v0.2 rewrite and the
  TriG/N3/NQ grammar wave). The advertised "shared IR" is also unbuilt — gen-js compiles
  grammar AST straight to JS strings. Requirement 5 therefore starts with **building the
  Rust backend**, not consuming one.
- **Only one full grammar exists upstream:** `grammars/turtle12.shuttle` (+ a prose-only
  turtle11 profile). NT/NQ/TriG/N3 are lattice *designs*; SHACL-CS is a worked-example
  excerpt in `examples/shacl-compact.md`. Four of the five "existing grammars" in the
  program premise must be authored.
- **The v0.1/v0.2 inversion:** upstream's v0.2 spec (RDF-native grammar graphs, SHACL
  well-formedness) supersedes the v0.1 `.shuttle` surface as normative, but only v0.1 is
  implemented. We target **v0.1 now** and accept bounded rework risk (§6, open question Q2).
- **sparq already has a standard-SCS parser**: `crates/sparq-shacl/src/scs.rs` (1620
  lines, opt-in `scs` feature, hand-rolled recursive descent over W3C SHACLC.g4,
  32/32 vendored shacl12-cs valid fixtures round-trip). The genuinely new SHACL-CS work
  is: the **extended** syntax, the **1.2 surface**, a **Rust serializer** (none exists —
  display ships client-side in the site, #860), and the single-source grammar discipline.
- **What works upstream today:** the gen-js backend. `turtle12.shuttle` → one
  dependency-free ES module (parser + push parser + serializer); 92/92 conformance
  assertions verified live this session; measured ~2× n3@1.26.0 like-for-like (JS-vs-JS,
  shared 2-core box — says nothing about Rust).
- **jeswr/shaclc-1.2** is a 1-commit README scaffold, **no LICENSE** (blocks vendoring
  until fixed), no spec/ content. **jeswr/solid-specs** does not exist.
- **shaclcjs strict-mode leak (measured):** with `extendedSyntax:false` the `% … %`
  property escape and trailing-turtle statements are still accepted — only the `;`
  annotations and `a` keyword are guarded. A conformant strict parser must reject them;
  concrete first upstream fix (sq-tonhr.5) — **filed as jeswr/shaclcjs#199, open and
  awaiting maintainer review**; leak and fix re-verified 2026-07-27, see
  [`docs/upstream-proposals.md`](../docs/upstream-proposals.md) § D.

## 2. The Shuttle value proposition

One deterministic L-attributed translation grammar with relational semantics yields, from
a single source:

- a **streaming single-pass parser** (LL(k≤2) spec / LL(1) implemented; quads emitted at
  earliest groundness; O(depth) memory; chunked push parsing with mid-token suspension);
- a **serializer** — the backward "lens" reading of the same productions, with
  guard-free fallbacks guaranteeing print totality (law L3), and — crucially for
  SHACL-CS — a *residual-based expressibility decision procedure* (print fails with a
  non-empty residual ⟺ the graph is not compact-expressible; exactly shaclc-write's
  "unserializable quads" mode, but derived rather than hand-maintained);
- a **generated conformance suite** (coverage-directed positive pairs + provably-negative
  LL-table mutants, W3C manifest.ttl format) — *unimplemented upstream today*; the 22
  oracle pairs are hand-authored.

Parser/serializer consistency is a compile-time theorem (round-trip laws L1/L2), so the
writer cannot drift from the parser — a class of bug sparq currently guards against only
with hand-written proptest round-trips. Precedent that the approach finds real bugs:
coverage-directed sampling of the annotation production exposed three genuine N3.js RDF
1.2 pending-reifier conformance bugs.

## 3. Honest feasibility verdict (phased confidence)

**Can Shuttle-generated Rust match/beat oxttl on perf AND conformance?** Split the claim:

| Claim | Confidence | Basis |
|---|---|---|
| Generated Rust SHACL-CS parser+serializer, conformance-correct, adequate perf | **HIGH** | Small whole-document syntax; 44+12+70-file fixture corpus exists; shapes graphs are tiny so throughput is not a gate; gen-js proves the grammar semantics end-to-end first |
| Generated Rust **conformance parity** with oxttl on Turtle/TriG/NT/NQ | **MEDIUM-HIGH** | Mechanical set-identity ratchet exists (native_ttl_ratchet pattern); risk is byte-exactness minutiae (oxiri IRI resolution, verbatim lexical forms, lang-tag casing, bnode scoping) — the generated runtime must delegate to the same iso primitives, which the formalism's closed iso library is designed for |
| Generated Rust **throughput ≥ oxttl** (and ≥ incumbent nt.rs) | **LOW-MEDIUM — unevidenced** | The only perf data is JS-vs-N3.js. Generated recursive descent typically loses to hand-specialized code; sparq's NT advantage comes from direct-Dict interning + chunk parallelism, both outside Shuttle's model. The 2026-06 baseline verdict (research/custom-parsers-baseline.md) already REJECTED new custom serial parsers as poor ROI. Must be **proven at gate G1**, not assumed |
| Generated parser fixes the open Turtle 1T gap vs serd (sq-hmd7l.27) | **HYPOTHESIS ONLY** | The generator controls the emission contract, so span-level `(kind,start,end)` emission into an intern shim *could* reach native-ttl-class perf; measure, never presume |

**Program posture:** SHACL-CS proceeds regardless of the perf verdict (its gates are
conformance + round-trip). The **generalization to hot syntaxes is conditional** on the
gen-rs go/no-go gate. If generated Rust cannot reach parity, Shuttle's standing value to
sparq is still real: generated conformance/negative corpora, the serializer-completeness
mechanism, a machine-readable pinned RDF 1.2 semantics, and single-source grammars for
low-traffic syntaxes (SHACL-CS, N3) sparq does not want to hand-maintain — and the
verdict is recorded honestly (gap + root cause + plan), per the dominance-mandate rules.

## 4. The no-regression strategy (the crux, requirement 6)

Never replace a parser blind. Every Shuttle-generated parser:

1. **Lands opt-in.** New default-OFF cargo features: `shuttle-parsers` (sparq-core),
   `shacl-compact` (sparq-shacl). Defaults, dispatch, and the parallel ingest paths are
   untouched. Feature-OFF wasm bytes must not move (known drift trap). sparq-core/engine
   stay lean per the opt-in architecture rule.
2. **Is differential-tested against the incumbent** — identical input → identical quad
   set — across the **full W3C suites + the existing fuzz corpus + the nightly Oxigraph
   differential lanes**. The established template is
   `crates/sparq-conformance/tests/native_ttl_ratchet.rs`: the per-test PASS/FAIL
   **outcome set must be IDENTICAL** to the incumbent (not merely "passes"), in both
   feature states. Gap to close first: **no W3C syntax ratchet exists today for NT, NQ,
   or TriG** — those suites get wired with pinned floors *before* any candidate parser
   exists (sq-tonhr.2), and the differential harness is **mutation-tested for
   non-vacuity** (a seeded divergent parser must be caught).
3. **Is benchmark-gated**: bench/parse competitor rows vs the incumbent (and external
   reference parsers) on the canonical corpus, canonical runs on quiet EC2 per
   feedback-ec2-benchmarks; the `parse_ns_per_byte` CI floor stays watched as an advisory
   timing signal (tracked/warned, non-blocking — it is the sole `mode: noise` metric in
   `bench/perf-baseline.json`; every `mode: auto` metric there hard-fails
   `scripts/perf-gate.py`).
4. **Default flips per-syntax, only on a documented verdict** (sq-tonhr.11): conformance
   set-identity evidence + bench ≥ incumbent on BOTH 1T and 16T, each flip its own PR,
   incumbent kept reachable behind a fallback feature for ≥ one release, maintainer
   steering issue per proceed-and-document. A **no-flip is a first-class outcome** with
   gap + root cause + plan.

Byte-exactness invariants any candidate must reproduce (ratchet-enforced): IRI
resolution through oxiri's RFC-3987 semantics; literal lexical forms verbatim (0.9 stays
`"0.9"^^xsd:decimal`); lang tags ASCII-lowercased; labeled bnodes unify via merge,
anonymous bnodes doc-scoped fresh.

## 5. Generator integration (no build-time JS in sparq)

- **Build gen-rs upstream** in jeswr/rdf-shuttle (`packages/gen-rs` beside gen-js),
  consuming the same v0.1 grammar AST from meta.js. Porting surface, by analogy from the
  JS backend: lexer-gen (813 lines) + parser-gen (427) + serializer-gen (344) + clausec
  (303) + the 264-line runtime iso library (RFC3986 resolve, escape families, langCanon).
  Upstream conventions apply there: commit straight to main (no PRs), never write
  "RDF-star", pin bench baselines. This de-facto defines the "shared IR" — coordinate
  with the maintainer's v0.2 direction rather than forking it.
- **sparq consumes checked-in generated Rust**: one `.rs` file per grammar with a
  DO-NOT-EDIT header + source-grammar hash + pinned rdf-shuttle commit;
  `scripts/regen-shuttle-parsers.sh` regenerates; a **CI drift-check lane** regenerates
  and byte-diffs. Node exists only in that lane — **no build.rs codegen, no npm in the
  cargo build graph**. The checked-in artifact is reviewable in-PR (supply-chain posture:
  the generator is a dev-time external tool at a pinned commit; generated code is not a
  dependency, so no cargo-vet entry; zero new runtime deps is a hard requirement of the
  generated-code contract).
- **Generated-code contract**: single file, zero runtime deps,
  `#![forbid(unsafe_code)]`-compatible, MSRV 1.87, clean under workspace clippy
  `-D warnings` + rustdoc gates, streaming push API (per-block with incomplete-statement
  carry) so it can sit on `load_reader_parallel`'s per-chunk inner loop; emission as
  `(kind,start,end)` spans so an intern shim can write `[Id;3]` directly into
  `sparq_core::dict::Dict`. Chunk-parallelism and cross-chunk bnode/prefix scope remain
  sparq's layer.
- **Licences:** rdf-shuttle MIT, shaclcjs MIT (fixtures vendorable), sparq MIT —
  maintainer-owned on both sides, no friction. shaclc-1.2 has **no LICENSE**; fix
  upstream before vendoring anything from it (sq-tonhr.5) — MIT LICENSE filed as
  jeswr/shaclc-1.2#3, **still open as of 2026-07-27**, so the no-vendoring
  constraint still holds. Conformance pairs filed alongside it as
  jeswr/shaclc-1.2#4; both tracked in
  [`docs/upstream-proposals.md`](../docs/upstream-proposals.md) § D.

## 6. SHACL-CS specifics

**One grammar, `shaclc12ext.shuttle`** (authored upstream, sq-tonhr.4), three layers:

1. **Standard SHACL-CS** (CG spec surface — directives, shape/shapeClass, `->`
   targetClass, full param vocabulary, nodeKinds, `[min..max]`, full path algebra, `!`/`|`,
   `@refs`, nested bodies, value arrays).
2. **RDF 1.2 / SHACL 1.2 delta** (shaclc-1.2 scope): triple terms in value positions,
   dir-lang literals, surface syntax for `sh:nodeKind sh:TripleTerm`, `sh:reifierShape`,
   `sh:reificationRequired`. The w3c shacl12-cs ED has **no** RDF 1.2 surface yet — this
   syntax must be *invented*, coordinated with jeswr/shaclc-1.2 (spec-flux risk: RDF 1.2
   Turtle is still a WD; pin the WD date in the grammar `spec-ref`).
3. **The shaclcjs extensions** as a distinct extended layer, so **strict mode provably
   rejects all four** (fixing the shaclcjs enforcement leak by construction). v0.1 lacks
   `@profile` (upstream NOTE 3) — either a small upstream meta-language extension or two
   grammar variants sharing productions; document the choice.

Emission invariants to reproduce exactly (from the jison actions + W3C mapping):
`<base> a owl:Ontology` at EOF, `sh:NodeShape` typing, `rdfs:Class` for shapeClass,
`sh:minCount` omitted when 0, the xsd-datatype-whitelist oracle (`sh:datatype` vs
`sh:class` — Shuttle's `oracle` verb models this directly), exact `sh:or`/`sh:not`/path
RDF-list construction. Grammar semantics are **de-risked on the working gen-js backend
first** against the shaclcjs fixtures (44 valid + 12 extended), independent of gen-rs.

**Wiring (sq-tonhr.6):** generated Rust parser+serializer behind `shacl-compact` in
sparq-shacl (default OFF). Parse → `Vec<oxrdf::Triple>`/`Graph` mirroring the `scs` API;
whole-document parse is fine (shapes graphs are small). The serializer is sparq's first
Rust-side SHACL-CS writer, with the typed "not compact-expressible" residual verdict.
**Coexists with the hand-rolled `scs` feature initially**, differential scs-vs-generated
on every shared fixture; supersession is an explicit maintainer question (Q6). Round-trip
gates: parse∘print graph-isomorphic on the 44+12 corpus, print output re-parsed by
`shaclc-parse@2.0.0` to isomorphic graphs, cross-checked against shaclc-write's 70-file
suite. wasm exposure follows the `scs` forwarding pattern (sq-quly), zero default-bundle
bytes. Hygiene first: the existing `scs` feature has **no gating feature-matrix leg**
(allowlisted gap) — close it (sq-tonhr.3) before adding the sibling leg.

## 7. Spec plan (requirement 2) + improvement thesis

**Spec** (sq-tonhr.7): `site/specs/shacl-compact-extended.typ` in the house Typst UPD
factory (spec-head/sotd helpers, registry entry in `site/src/data/specs.ts`, status
`unofficial`, PDF+HTML via build-specs.mjs, honesty gates `check-no-perf-numbers.py
--enforce` + `check-privacy-claims.sh`), grounded by a recon doc in `research/specs/`
per sq-rvgr2 rules (every normative statement traces to implemented behaviour or a
labelled proposal; Security Considerations + implementation-status sections mandatory;
Fable soundness review pre-publication). Normative core = the Shuttle grammar + emission
mapping. **Destination:** jeswr/solid-specs pending → land in-repo now, contribute when
created; also contribute spec tests upstream to jeswr/shaclc-1.2 per standing
rule #1546. Note: house Typst is ReSpec-*look*; genuine ReSpec/Bikeshed conversion for
upstream is unbudgeted and flagged (Q4).

**Improvement thesis — measurable, not assumed** (sq-tonhr.12): the claim "Shuttle's
generated suite + provable round-trip catch corner cases oxttl misses" has one real
precedent (3 N3.js pending-reifier bugs) and one blocker (generate mode is unimplemented
upstream). Measurement: build/interim-script generate mode, harvest positive + negative
corpora into the existing differential/fuzz lanes, and **count divergences** found in
oxttl/sparq/Oxigraph, filing each. An honest null result is recorded as such — it would
falsify the thesis for mature syntaxes and narrow the value case to SHACL-CS/N3/1.2
freshness.

## 8. Phasing and bead map (epic sq-tonhr)

Risky foundations first; nothing default-facing until proven.

| Phase | Bead | Surface | Tier | Deps |
|---|---|---|---|---|
| 0a | sq-tonhr.1 — build gen-rs upstream + **go/no-go** eval vs oxttl (22-pair conformance identity to gen-js; honest bench; GO/NO-GO for generalization) | upstream + research/ | fable | — |
| 0b | sq-tonhr.2 — differential+bench harness + **W3C NT/NQ/TriG syntax ratchets** (mutation-tested non-vacuous) | sparq-conformance, bench/parse | opus | — |
| 0c | sq-tonhr.4 — author `shaclc12ext.shuttle` (std + 1.2 + extended), validated via **gen-js** vs shaclcjs fixtures | upstream grammars | fable | — |
| 0d | sq-tonhr.3 — gating feature-matrix leg for existing `scs` (allowlist gap) | .github fragment | haiku | — |
| 0e | sq-tonhr.5 — upstream hygiene: shaclcjs strict-leak fix + tests; shaclc-1.2 LICENSE + pairs | upstream | sonnet | — |
| **G1** | **decision gate**: gen-rs verdict shapes Phase 3 scope; SHACL-CS proceeds regardless | | | |
| 1 | sq-tonhr.6 — generated SHACL-CS parser+serializer behind `shacl-compact` + regen script + CI drift check | sparq-shacl | fable | .1 .3 .4 |
| 2 | sq-tonhr.7 — Extended SHACL-CS spec (house UPD + recon; upstream when destinations exist) | site/specs, research/specs | fable | .4 .6 |
| 3a | sq-tonhr.8 — NT/NQ grammars → generated parsers behind `shuttle-parsers` (differential + bench vs nt.rs/oxttl) | sparq-core opt-in | fable | .1 .2 |
| 3b | sq-tonhr.9 — Turtle 1.2 + TriG generated parsers (set-identity vs oxttl 313-suite + trig; bench vs oxttl/native-ttl/serd row) | sparq-core opt-in | fable | .2 .8 |
| 3c | sq-tonhr.10 — N3 grammar (LL(2) open question; counterexample = valid outcome) + parser vs sparq-reason incumbent (1464+4div floor) | sparq-reason opt-in | fable | .9 |
| 3d | sq-tonhr.11 — **per-syntax default-flip verdicts** (flip only on ≥ both axes; fallback feature; steering issue; adjudicate three-Turtle-impl question) | defaults + research/ | opus | .8 .9 |
| 4 | sq-tonhr.12 — generate-mode harvest: measure the improvement thesis (divergence count or honest null) | upstream + fuzz lanes | fable | .1 .2 |

Beads are disjoint by crate/surface; same-crate work is serialized via deps (.3→.6 share
the feature-matrix fragment; .8→.9 share sparq-core; .2→.12 share the lanes).

## 9. Open questions for the maintainer

1. **End-state for the default Turtle parser**: replace oxttl per-syntax when proven, or
   permanent coexist? If shuttle-turtle wins its gate, does **native-ttl retire** (three
   Turtle impls otherwise)?
2. **gen-rs home + surface**: upstream `packages/gen-rs` targeting the **v0.1** front-end
   now (rework risk when v0.2 lands) — acceptable, or wait for the v0.2 RDF-native
   modules? Upstream issue #4 sequences Rust last; we'd be reordering the roadmap.
3. **shaclc-1.2**: add MIT LICENSE; should sparq's grammar + 1.2 fixture pairs become the
   repo's reference `spec/` content (it currently promises a "reference implementation"
   that doesn't exist)?
4. **solid-specs timing** and format: is house Typst-UPD acceptable upstream, or is
   genuine ReSpec/Bikeshed required (conversion unbudgeted)?
5. **Strictness semantics**: confirm strict mode must reject `% … %` escapes and trailing
   turtle (i.e. the shaclcjs behaviour is a bug to fix upstream, not a compat surface to
   mirror).
6. **Does `shacl-compact` supersede the existing `scs` feature** once proven a superset,
   or do both stay indefinitely?
7. **RDF 1.2 SHACL-CS surface syntax** must be invented ahead of the w3c ED — appetite
   for sparq/shaclc-1.2 defining it unilaterally (WG-note coordination)?
8. **Case-sensitivity**: shaclcjs lexes keywords case-insensitively, deviating from the
   case-sensitive W3C grammar — which behaviour is normative for the extended spec?
