# Shuttle print mode: generalizing the residual-consumption printer past the shaclc skeleton (sq-e7hba)

> 🤖 SPARQ agent [OPUS-5], 2026-07-28. Bead **sq-e7hba** (issue #3030), child of the
> Shuttle epic **sq-tonhr** (design record:
> [`shacl-compact-and-shuttle-parsers-2026-07.md`](./shacl-compact-and-shuttle-parsers-2026-07.md)).
> Increments jeswr/rdf-shuttle#5 (sq-tonhr.4), which landed the SHACL-CS residual printer in
> both backends. Sibling record for the other open upstream gap:
> [`shuttle-gen-rs-backend-gaps-2026-07.md`](./shuttle-gen-rs-backend-gaps-2026-07.md).
>
> **Verdict: the gap is real, larger than "one backend pass", and is NOT a pure backend
> change.** Generalizing needs a compiler pass that does not exist (mode analysis over `emit`
> clauses) *and* a small additive grammar-surface delta, because the current printer's output
> depends on choices the grammar does not state anywhere. Byte-identical regeneration of the
> SHACL-CS printer is achievable but only if that delta is landed first and the shaclc grammar
> is annotated to re-derive the skeleton it currently gets for free.

## Why this record exists

The bead asks for the generic backward interpretation the print-mode spec already promises:
compile each production's `emit` clauses to residual quad-pattern matchers, `fresh` to linear
blank matches, `thread` to match-driven iteration, so *any* grammar with graph-typed
productions gets a derived printer plus an expressibility verdict. The upstream generator's own
header comment already concedes the gap in the same words the bead uses, so the useful
contribution is not restating it — it is (a) measuring exactly how deep the coupling runs,
(b) naming the compiler pass that is missing, and (c) surfacing the part the bead's framing does
not anticipate: **the shaclc printer's byte output is not fully determined by the shaclc
grammar**, so "regenerates with zero behavior change" is a constraint on the *surface delta*,
not just on the backend.

The work lives in the upstream repository `jeswr/rdf-shuttle` (`packages/gen-js`,
`packages/gen-rs`, and the shared front end); sparq only vendors the *generated* artifacts. As
with sq-uyney, this bead's sparq-side deliverable is the verification and the design; the code
change belongs in a draft PR on that repository (see *What is deliberately not done here*).

## Verification pin and method

Everything below was run against a fresh clone at **`89bbe6d`**
(`89bbe6d2ee38b4ec3379f39f0c8bd25894f8efac`, *"meta + both backends: grammar-driven
`@maxdepth(N)` production annotation"*) — the same head the sibling record verified. It is not
the `b7801c5` pinned in `crates/sparq-shaclc/src/provenance.rs`, but the vendored grammar and
both vendored artifacts are unchanged between them (shown below), so it is a sound baseline.

- **Baseline suite, measured, not quoted:** regenerating the three gen-js artifacts and running
  `node --test test/` gives **362 pass / 0 fail** — the number the bead's acceptance criterion
  names, confirmed at this head rather than carried forward. The print direction is **172** of
  those subtests: **118** SHACL-CS `parse∘print∘parse ≅ parse` (strict × extended × the four
  fixture directories), **10** named residual/oracle/`print{}`-inversion verdicts, and **44**
  turtle `write∘parse` round-trips belonging to the *other* serializer. The rest are parse,
  push-parse and negative cases.
- **sparq non-regression baseline:** the vendored grammar is byte-identical to upstream's
  (`sha256 3e6bceb2…`, matching `GRAMMAR_SHA256`), and re-running the two `gen-rs` CLI
  invocations from `scripts/regen-shuttle-parsers.sh` at this head reproduces
  `crates/sparq-shaclc/src/raw/shaclc12.rs` and `shaclc12ext.rs` **byte-identically**. That is
  the diff any generalization must keep empty.

## How deep the coupling actually runs

The generator's honest-derivation note claims the *data* is derived and only the control
skeleton is built in. That claim holds, and the split is sharper than the note suggests.

**Genuinely grammar-agnostic today** (would survive generalization unchanged): the residual
store and its transaction discipline — `all` / `used` / `bySubj` / `bRef`, `singleRef(t)`,
`on(t)`, `free(i, txn)`, `commit(txn)` — set-semantics load with duplicate collapse, the
`ShuttleResidualError` type carrying the unconsumed quads, and the `writeQuads` /
`createWriter` API. This is the part spec §8 describes and the part a generic compiler should
keep verbatim.

**Keyed to shaclc's production shapes:**

| coupling | measured at `89bbe6d` |
|---|---|
| grammar production names hard-required by `packages/gen-js/src/residual-serializer-gen.js` | **33** via `need(g, '…')` (which throws `production '<name>' missing` on drift), plus **3** optional probes via `prodByName.get` (`pcSection`, `ttlStatement`, `tripleTerm`) that double as the profile-layer flags |
| production-shaped consuming functions emitted into the JS printer | **16** (`readList`, `ttText`, `valueText`, `pathText`, `propertyAtomText`, `propertyNotText`, `orChainText`, `propertyOrText`, `nodeValueText`, `nodeNotText`, `nodeOrText`, `extObjectText`, `extPredicateGroups`, `bodyText`, `constraintLines`, `propertyShapeText`) |
| the same skeleton re-authored in `packages/gen-rs/src/residual-serializer-gen.js` | **17** mirrored `*_text` / `*_lines` methods |
| shape assertions beyond name lookup | e.g. `'residual serializer: sh:or predicate not found in nodeOrEmit'`, `'sh:node / sh:nodeKind pairs not found in propertyAtom'` — the extractor asserts *where inside a production* a given curie sits |
| the document driver | inline in `printWithResidual`: iterate the residual for `rdf:type sh:NodeShape` quads, then `shapeClass`, then targets, then constraints, then the two extended fallback layers |

Two consequences the bead's framing should absorb. First, this is **double maintenance already**:
the same backward reading exists twice, hand-written, in two languages, and byte-identity between
them is held by a conformance script rather than by construction — which is precisely the drift
argument the project makes against hand-written parser/writer pairs, reproduced one level up.
Second, the extractor's failure mode is loud (a throw at generation time), which is the right
choice and should be preserved: a generic path must not silently degrade a grammar it cannot
invert into a printer that quietly under-consumes.

### The finding that changes the plan: the grammar under-states the skeleton

`grammars/shaclc12ext.shuttle` carries **9** `@prefer` annotations — and every one of them is on
a *term-level* prioritized group (`NumericLiteral | BooleanLiteral | RDFLiteral`, the four string
forms, `IRIREF` vs abbreviated). **No graph-typed production carries `@prefer` or `@when` at
all.** Meanwhile `grep -n prefer packages/gen-js/src/residual-serializer-gen.js` matches only
two English words in comments: the residual backend **never reads the `@prefer` annotations that
are there**, and re-implements that term-level ordering by hand as `iriText` / `litText`, while
`packages/gen-js/src/serializer-gen.js` — the other, turtle-spine serializer — *does* consume
`@prefer` / `@when` generically for exactly those groups. (The residual backend's single
annotation-adjacent read is `pcEmits[0].when !== null`, which inspects an **emit clause's** guard
to learn that `sh:minCount` is conditional — a clause field, not an alternative annotation.)

So the v0.1 backend already contains one half of the generic machinery (annotation-driven,
guard-gated alternative selection, at term level, forward-rendered) and one half of the other
(residual + transaction machinery, at graph level, hand-skeletoned), and the two halves have
never been joined. That is the actual shape of this bead.

It also means the order the current printer emits shapes in, the decision to anchor iteration on
the `rdf:type sh:NodeShape` quad, the precedence of the property-shape reading over the extended
annotation fallback, and the try-nested-then-refuse discipline are **facts about the skeleton, not
about the grammar**. A generic compiler cannot derive them from a grammar that does not state
them. Zero-behaviour-change therefore has a precondition: state them.

## Design: the generic backward reading

Spec §8 already fixes the semantics — *"each template consumes exactly one matching quad; Skolem
sites are linear blank-node matches; repetitions become match-driven iteration; alternatives are
tried in `shtl:prefer` order subject to `shtl:printGuard` guards, ending at the guard-free
fallback"*, with **print succeeds iff the residual empties** and L3 totality. As with the quad
gap, this is not a new design; the v0.1 surface is behind the spec on a specific, nameable point.

### 1. The missing pass: mode analysis over `emit` clauses

Each `emit s p o [@g] [when c]` becomes a quad pattern whose four slots are each one of: a ground
term (curie/IRI — a literal match constraint), a production **parameter** (bound by the caller —
an *input* slot), a **synthesized** variable from a nonterminal binding (`p=nodeNot`, then
`fst(p)` / `snd(p)` — an *output* slot, recovered by matching), a `fresh` (an existential — a
linear blank match), or an **environment-derived** expression (`resolve(env.base, "")` — see §6).

Deciding which is which, per clause, in dependency order, is a **binding-time / mode analysis**:
the classic mode inference of logic-programming compilers, applied to the emission relation. It
does not exist in either backend today — the hand-written skeleton *is* the answer to it,
precomputed by a human for one grammar. This pass is the load-bearing deliverable of the bead;
everything else below is mechanical once modes are known.

Two obligations fall out of it and should be *generation-time errors with a counterexample*,
never runtime surprises:

- **Recoverability.** Every output slot of a production must be recoverable from the quads that
  production consumes. A production emitting a value that appears in no consumed slot cannot be
  inverted — diagnose it, name the clause.
- **Fresh-node linearity.** A `fresh` site's blank node must be referenced exactly once among the
  quads its production consumes (`singleRef(t)`, §2). This is a property of a single production's
  own existentials, and it is what makes the `thread` walk of §3 deterministic.
- **Emission overlap.** Two emit sites — in different productions, or in different clauses of one —
  whose patterns can produce the *same* quad are a **separate and harder** problem, and `@prefer`
  does not solve it. RDF dataset set semantics collapse the duplicate on load, so the residual
  carries **no provenance** recording which site or sites emitted it: the single surviving quad may
  have to discharge one emit site, or discharge several jointly. `@prefer` only orders competing
  interpretations of one consumption, so it cannot express "both sites were required"; and choosing
  the preferred derivation does not by itself establish that the choice replays to the same graph
  or satisfies the rest of the surrounding production. An overlap is admissible only if:
  - the overlapping alternatives are **provably mutually exclusive** — the §5 disjointness check,
    applied to emit patterns rather than oracle branches — so at most one site can have fired and
    consumption is determined; or
  - the overlap is **witness-distinguishable** — the two sites' patterns can never be instantiated
    to the *same* quad, so a firing that required both necessarily leaves behind a quad the one-site
    reading does not produce — **and** the chosen reading is discharged by **reconstruction plus
    full forward replay of the whole derivation instance**: recover the bindings, re-run the forward
    semantics of every production in the reconstructed derivation, discharge every *mandatory*
    `emit` clause of each — including the clauses reached through the caller context, not only the
    production that happens to own the matched quad — and require the replay to produce exactly the
    set of quads the reading consumed, no more and no fewer.

  **Replay alone is not a discharge, and the identical-quad case is why.** When two jointly-required
  sites emit the byte-identical quad, RDF set semantics erase the multiplicity that replay would
  have to observe: a reading that fires only one of them replays to the very same dataset, so
  "reproduce every quad this production emits" passes. Reproducing the chosen production's quads
  validates *that production locally*; it witnesses neither the site that was elided nor the caller
  context that may have made the elided site mandatory — so the one-site reading can print text that
  is not in the grammar's language even though the residual empties. Such an overlap is admissible
  only if the required-site provenance is reconstructible some other way (a co-emitted quad the two
  sites do *not* share, or a `fresh` node whose §1 linearity condition forces both firings). Absent
  that, and for every other overlap that is neither provably exclusive nor witness-distinguishable,
  the answer is a **generation-time rejection naming both clauses** — the conservative default, not
  a runtime check. The compiler must not fall back on the transaction machinery's incidental
  behaviour (whichever matcher runs first wins), and must not treat a bare `@prefer` as a discharge.

### 2. `emit` → residual matcher; `fresh` → linear blank match

With modes assigned, a clause compiles to a search over `on(subjectTerm)` (the existing
subject index) filtered by `free(i, txn)` and the slot constraints, adding the matched index to
`txn` and binding output slots. `fresh` slots additionally require `termType === 'BlankNode'`
and `singleRef(t)` — the linearity condition the current code already applies by hand for
property-shape blanks and `sh:not` blanks. Both predicates exist verbatim in the generic half of
the runtime; the generalization is that the *compiler* emits the calls instead of a human.

### 3. `thread` → match-driven chain walk

`thread prev : T = init in ( … )*` compiles backward to: bind `prev := init`; repeatedly search
for the unique free quad set matching the loop body's linking pattern with the threaded variable
in its bound position; consume, rebind `prev`, iterate; terminate on the post-loop clause's
pattern (`emit prev rdf:rest rdf:nil`). Determinism comes from linearity — each intermediate node
is a `fresh` blank with object-position refcount 1 — which is exactly why the fresh-node linearity
obligation above must be checked and not assumed. This subsumes both the `rdf:rest` walk the
shaclc printer open-codes as `readList`/`orChainText` and Turtle's collection spine.

### 4. `when` + `print { … ?? d }` → guard consistency and suppression inversion

A `when c` guard makes emission conditional, so backward it does two things, and the second is
the one that is easy to get wrong:

- a matched quad may be consumed only if `c` **holds** under the recovered bindings; and
- a value that would make `c` **false** must be *refused*, not silently printed — because the
  parse side would not re-emit it, and printing it anyway is round-trip drift dressed up as
  success. The shaclc case is `sh:minCount 0`: the parse-side guard is `when int(mn) > 0`, so an
  explicit zero goes to the residual. The generic rule is "reconstruct, re-evaluate the guard
  forward, refuse on false", and the `print { mn = lookup(ps, sh:minCount) ?? 0 }` block supplies
  the default for the *suppressed* case. The existing extractor already reads that block's
  defaults; what it cannot do generically today is derive the refusal from an arbitrary `c`.

Guards over closed comparisons (`==`, `!=`, `<`, `>` against literals and curies) invert
mechanically. Guards calling a user predicate do not, and belong to the fixed print-guard
vocabulary spec §8 describes — which is where `shtl:requiresIndex` earns its keep: a guard that
needs non-local evidence declares the index it needs, so the batch printer can answer it and the
stream printer can fall back to L3 rather than guess.

### 5. `oracle` → forward discharge, with a disjointness obligation

Oracles must never run backward — the current implementation is right, and the reason generalizes
cleanly. `oracle f(a) then C₁ else C₂` produces two emission shapes; backward, the *matched quad*
selects the branch (in shaclc, `sh:datatype` vs `sh:class` on the predicate), and the oracle is
then re-evaluated **forward as a check**. Two obligations:

- the two branches' patterns must be **provably disjoint** — otherwise a graph matches both and
  the printer's choice is not determined by the grammar. Check at generation time; this is
  decidable for the ground-slot patterns the surface admits.
- the forward check must be applied after reconstruction, so a value outside the registry cannot
  print in a form that would re-parse into the other branch. (`sh:class xsd:string` must not print
  as a bare IRI.) This is the property the current printer holds by hand and the generic path must
  hold by construction.

### 6. Environment-derived slots are solved, not inverted

`resolve(env.base, "")` is not injective, so no inversion exists. The current printer already
does the right thing and the mechanism should be named and generalized rather than reinvented:
**candidate generation plus forward replay.** It scans the residual for quads matching the
ground part of the pattern (`? rdf:type owl:Ontology`), takes the subject as the candidate base
— preferring `options.baseIRI` when a candidate matches it, because a `BASE` directive in the
source overrides any parse-time base — commits it, and prints the directive; absence of any
candidate is the `missing` verdict rather than a residual.

Generically: an `env`-writing clause (`env.base := …`, `env.prefixes := bind(…)`) is **not**
inverted. The printer emits directives from a policy, then *replays the forward semantics* and
requires that the replayed environment reproduces the emitted quads. This keeps `resolve` and
`bind` in the forward direction only — the same discipline as the oracle rule — and it is the
honest reason a fully-derived printer still needs a policy layer.

### 7. Alternatives, anchors, and the document driver

Alternatives compile to: try in `@prefer` order, each inside a nested transaction, roll back on
failure, end at the guard-free alternative (L3). The extended profile's fallback layers already
behave exactly this way — and profile filtering removes alternatives, so a strict build's
"residualize by construction" property is preserved for free.

The hand-written document walk generalizes to **anchor selection**: for a repetition over
graph-typed alternatives, the compiler picks, per alternative, the emit clause whose pattern is
most selective under the recovered modes (in shaclc: `emit n rdf:type sh:NodeShape`) and drives
iteration over the residual quads matching it, in residual load order. That default reproduces
the current "shapes in input order of their typing quads" behaviour without special-casing it —
but only because *residual load order* is chosen as the policy. Say so explicitly in the policy
module; do not leave it implicit a second time.

## The surface delta this requires (additive, v0.1)

Because §7 and the `@prefer` finding above show the grammar does not state what the skeleton
assumes, the backend change must be preceded by a small, additive delta — each item maps onto
vocabulary spec §4/§8 already reserves, so this is v0.1 catching up to v0.2, not new invention:

1. **`@prefer` / `@when` admitted on graph-typed alternatives**, not just term-level prioritized
   groups. Parsing already supports both annotations on any alternative (`meta.js` reads them
   uniformly); only the backends' consumption is term-level. Low cost, unlocks §7.
2. **An anchor annotation** (spec-side `shtl:printAnchor`, or an `@anchor` marker on one `emit`
   clause) for the cases where selectivity is a tie or where the intended anchor is not the most
   selective pattern. Optional wherever mode analysis is decisive.
3. **A declared print policy** for the residues that are genuinely not grammar facts — iteration
   order and layout. v0.2 already models this as `shtl:PrintPolicy` modules that *refine, never
   redefine*; v0.1 needs only the one default ("residual load order") to be written down.

Landing 1–3 first, then annotating `shaclc12ext.shuttle` so the annotations reproduce the
existing skeleton, is what makes "zero behaviour change" a checkable claim instead of a hope.
This is also why the bead's own hedge — *"likely alongside the v0.2 RDF-native grammar
modules or gen-rs"* — is the right instinct: the delta is the v0.2 vocabulary arriving early.

## What cannot be derived (honest limits)

- **Layout and ordering are policy, not grammar.** Indentation, blank-line placement, directive
  grouping, prefix mining. Spec §8 says so; a derived printer that pretends otherwise will
  produce a byte diff on the first grammar it meets.
- **Byte-level round-tripping is explicitly not promised** by the spec (lexical forms preserved,
  layout not). The acceptance criterion here is stricter than the spec's law — it is a
  *regression* criterion on one grammar, and should be stated as such.
- **Guards needing non-local evidence** (`listShaped`, `freshSingleUse`, `reifiesQuadPresent`)
  are conservatively false in the v0.1 stream window. A batch residual printer *can* answer them,
  and the generic path should — but that is a behaviour *change* for the turtle-spine serializer
  and must not be smuggled into this bead.
- **Ambiguity is not always decidable.** The disjointness and fresh-node linearity checks above are
  decidable for the ground-slot patterns the v0.1 surface admits; they are not a general decision
  procedure, and emission overlap is not resolvable by annotation at all — only by proved
  exclusivity, or by whole-derivation replay of an overlap whose sites are witness-distinguishable.
  An overlap whose sites can produce the **identical** quad is not resolvable by replay either,
  because set semantics erase the multiplicity replay would have to observe. Where a check cannot be
  discharged — including every jointly-required overlap replay cannot rule out — the compiler must
  fail and name the clauses, rather than defaulting to source order, or to a `@prefer`, and hoping.
- **Performance is unmeasured and unclaimed.** A compiled matcher chain doing nested transaction
  rollback over a subject index has a different cost profile from the hand-written skeleton's
  early returns. sparq vendors this generated code into a benchmarked crate, so the generic path
  must be measured on the canonical box against the current artifact before it is adopted here.
  This record makes **no** performance claim in either direction.
- **Generate mode stays unimplemented upstream**, so a new grammar's conformance pairs are
  hand-authored — which caps how cheaply a third grammar can be added and is the main cost driver
  in the acceptance plan below.

The compensating benefit is that all of these become **generation-time diagnostics with
counterexamples**: today a grammar that cannot be inverted is discovered by a human failing to
write its skeleton; afterwards it is a compiler error naming the clause. That is the same
"expressibility verdict, moved left" argument the residual printer makes for graphs, applied to
grammars.

## Acceptance, phased

The bead's criterion — *"the shaclc printer regenerates from the generic path with zero behavior
change (362/362 incl. parse-print-parse iso + residual verdicts), plus at least one non-shaclc
non-turtle-spine grammar prints"* — is the right gate. Phasing it:

1. **Surface delta** (annotations 1–3 above) + annotate `shaclc12ext.shuttle`. Gate: both
   backends still generate **byte-identical** artifacts, since nothing consumes the annotations
   yet. This is the cheapest possible checkpoint and it isolates the delta from the rewrite.
2. **Mode analysis + matcher compilation in gen-js only**, behind a flag, with the hand-written
   skeleton retained as the default. Gate: the generic path's output is byte-identical to the
   skeleton's on all four shaclc fixture directories, and the suite stays at **362/362**.
3. **Third grammar.** Gate: it prints, and its verdicts are right — including the negative ones,
   so the overlap cases below are rejected at generation time, or replay-validated when printing,
   rather than printed.
4. **gen-rs mirror**, then delete both skeletons. Gate: cross-backend identity
   (`packages/gen-rs/test/conformance.sh`) plus byte-identical regeneration of sparq's two
   vendored artifacts. Only after step 4 is the double-maintenance actually retired — a generic
   path that ships beside the skeleton has made the problem worse, not better.

**On the third grammar.** Within RDF, most syntaxes are either turtle-spine (Turtle, TriG,
N-Triples, N-Quads, N3) or SHACL-C-like, so this criterion is harder to satisfy honestly than it
looks, and two obvious candidates are blocked or oversized:

- **TriG / N-Quads are not available** as the third grammar until `emit @graph` lands — that is
  the open gap of the sibling record, and it is a hard dependency, not a preference.
- **ShExC** is genuinely graph-typed and non-turtle-spine, but ShExC → ShExR is a large grammar
  and would dominate the bead.

The honest split is therefore a **gate** and a **demonstration**. The gate is a small
purpose-built grammar whose graph-typed productions are structurally unlike shaclc and which
exercises the full clause vocabulary the design touches — `fresh`, `thread`, a `when`-guarded
emit with a `print { … ?? d }` default, an `oracle` with disjoint branches, and a nested optional
— with hand-authored conformance pairs. It must also carry **negative** cases for the §1
obligations: a production with a non-recoverable output slot, and two productions that emit the
same quad, in both variants — one where the two are provably mutually exclusive (must compile, and
the derivation it picks must be replay-validated) and one where forward execution requires *both*
emit sites *and the two patterns instantiate to the **identical** quad*. That second variant must be
a generation-time rejection naming both clauses: not a `@prefer`-resolved pick, not a silent
first-matcher-wins consumption, and — the point of choosing an identical-quad example — **not
admitted by replay**, since a one-site reading of it replays to the same dataset and so passes any
replay check. Only the §1 witness-distinguishability precondition rejects it, which is exactly the
guard this case exists to keep red. Its value is *coverage of the inversion*,
and it should be described that way rather than dressed up as a syntax anyone wants. The
demonstration is a
real syntax, and the best-sized real candidate is an **OWL 2 Manchester Syntax class-frame
subset** (`Class: X SubClassOf: A and (p some B)`): graph-typed via the OWL-2-mapping-to-RDF
graphs, structurally a keyword-sectioned frame rather than either spine, and it naturally
stresses `fresh` plus `thread` list chains (`owl:intersectionOf`) and a bare-IRI-vs-class-
expression discrimination. It is a proposal here, not a commitment — sizing it is part of step 3.

## Non-regression for sparq (measured, not assumed)

`crates/sparq-shaclc` vendors generated artifacts and pins provenance, so the entire risk surface
is a byte diff in `src/raw/`. At `89bbe6d`, before any of this work, that diff is empty (verified
above), and `tests/provenance_drift.rs` keeps the vendored grammar and its hash from drifting
apart independently. Consequences for the bead map:

- Nothing in sparq is **blocked** by this. The SHACL-CS parser and printer already ship; this is
  a maintainability and reach change upstream.
- The sparq-side follow-up is mechanical and belongs *after* step 4: re-run
  `scripts/regen-shuttle-parsers.sh`, expect an empty diff, and bump `RDF_SHUTTLE_COMMIT`. If the
  diff is **not** empty, that is the regression signal, and the right response is to hold the
  bump — not to re-baseline the artifacts.
- Because the vendored artifact sits in a benchmarked crate, the perf measurement named under
  *honest limits* is a precondition of the bump, not a nice-to-have.

## What is deliberately not done here

No upstream patch was written, and no upstream behaviour was changed. The change belongs in
`jeswr/rdf-shuttle`, and this lane can neither push to that repository nor open a pull request on
it; a patch that exists only as text inside a sparq markdown file would be unverifiable by any
gate on either side. Per `AGENTS.md` § *Upstream contributions*, it lands as a **draft** PR on
that repository, self-identified as agent-authored, with @jeswr as the review gate — captured as
a follow-up rather than guessed at here.

Also deliberately out of scope: implementing the batch answers for the non-local print guards
(a behaviour change to the turtle-spine serializer), and any change to `crates/sparq-shaclc`
(there is nothing to change until step 4 lands upstream).

## Reproduction

```sh
git clone https://github.com/jeswr/rdf-shuttle && cd rdf-shuttle
git rev-parse HEAD                                    # expect 89bbe6d…

# baseline suite (the 362 the acceptance criterion names)
cd packages/gen-js
node src/cli.js ../../grammars/turtle12.shuttle   -o generated/turtle12.js
node src/cli.js ../../grammars/shaclc12ext.shuttle -o generated/shaclc12.js    --profile rdf12
node src/cli.js ../../grammars/shaclc12ext.shuttle -o generated/shaclc12ext.js --profile rdf12,ext
node --test test/ | grep '^ok ' > /tmp/ok.txt
wc -l < /tmp/ok.txt                                    # 362
grep -c 'parse∘print∘parse'      /tmp/ok.txt           # 118
grep -c 'write∘parse round-trips' /tmp/ok.txt          # 44 (turtle-spine serializer)
grep -i print /tmp/ok.txt | grep -vc 'parse∘print∘parse'   # 10 named verdicts

# the coupling, counted
grep -o "need(g, '[a-zA-Z]*')" src/residual-serializer-gen.js | sort -u | wc -l   # 33
grep -n  "prefer" src/residual-serializer-gen.js                                  # comments only
grep -c  "@prefer" ../../grammars/shaclc12ext.shuttle                             # 9, all term-level

# sparq non-regression: regenerate per scripts/regen-shuttle-parsers.sh and diff
# against crates/sparq-shaclc/src/raw/{shaclc12,shaclc12ext}.rs — expect no diff.
```
