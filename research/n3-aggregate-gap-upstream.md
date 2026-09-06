# Upstream w3c-cg/N3: no *dedicated* aggregate primitive (verification + draft issue)

**Bead:** sq-6tykl.3.4 (from sq-6tykl.3 / issue #1993) · **Status:** gap **NARROWED** — what is confirmed
is the absence of a *dedicated* aggregate primitive, **not** an N3 expressivity limit; issue drafted,
**NOT yet filed** upstream — awaiting @jeswr review per the upstream-contribution protocol
(`AGENTS.md` § *Upstream contributions*) · **Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-27

Companion record: `gpt56-decomp-rules-substrate-2026-07.md` § 4 (the gap analysis this verifies).

## Why this record exists

Maintainer direction in issue #1993: N3 must be expressive enough to be a **superset** of RIF and the
RDFox rule syntax; any construct N3 cannot express → a clear, concise issue on the N3 CG repo stating
what is missing and that it was discovered implementing RIF/RDFox rules support. § 4 of the
decomposition record flagged exactly one candidate gap — RDFox's
`AGGREGATE { … } ON ?g BIND SUM(?v) AS ?x` — and cut this bead to verify it against the *current*
spec before anything is said upstream.

## Venue correction (found while verifying)

The CG has **renamed**: `w3c/N3` → **`w3c-cg/N3`**, and the spec host moved
`w3c.github.io/N3/spec/` → **`w3c-cg.github.io/N3/spec/`** (the old URL serves a `<meta refresh>`
redirect; the old GitHub org path resolves only through GitHub's rename redirect). The repo is live —
50 open issues, last push 2026-07-13. **Any upstream issue goes to `w3c-cg/N3`, not `w3c/N3`.**

## Verification (2026-07-27, against the live documents)

| document | version / status | `aggregat*` hits |
|---|---|---|
| [Notation3 Language](https://w3c-cg.github.io/N3/spec/) | W3C N3 Community Group draft | **0** |
| [Notation3 Builtin Functions](https://w3c-cg.github.io/n3Builtins/) | **Final Community Group Report, 3 June 2026** | **0** |

**Builtin inventory of the Final CG Report — PARTIAL, counts unreliable.** This inventory was built
by a *namespace-prefix scan* of the report text, which over-counts: it matches references in prose,
examples and schema fragments, not only definitions. It is recorded here for orientation, **not** as
a verified count, and it should not be cited as one.

- `log:` — the scan reported 22; the report in fact defines **18**: `collectAllIn`, `conclusion`,
  `conjunction`, `content`, `dtlit`, `equalTo`, `forAllIn`, `includes`, `langlit`, `notEqualTo`,
  `notIncludes`, `outputString`, `parsedAsN3`, `rawType`, `semantics`, `semanticsOrError`, `skolem`,
  `uri`. (Corrected during review of this record; the 18 terms are listed so the correction is
  itself checkable.)
- `list:` — scan reported 9: `append`, `first`, `last`, `in`, `iterate`, `length`, `member`,
  `memberAt`, `remove`. Names listed so they can be checked; the count is **not** re-derived.
- `math:` (25?), `string:` (16?), `time:` (7?), `crypto:` (1?) — prefix-scan figures, **not**
  re-derived, and given the `log:` error they are probably all high. `math:` at least includes
  `sum`, `product`, `quotient`, `difference`, `remainder`, `rounded`, `absoluteValue`,
  `exponentiation`, `negation`, the comparisons and the trig family.

None of this record's conclusions depends on any of these totals — only on the *absence* of specific
terms (`math:min`/`math:max`/`list:min`/`list:max`), established by targeted term search rather than
by counting.

### Confirmed: no *dedicated* aggregate primitive (composition still works)

There is **no aggregate construct** in the grammar and **no aggregate builtin family**. That is a
claim about *primitives*, not about expressivity: aggregates are reachable in N3, by composing the
list-valued findall idiom the gap analysis named with the arithmetic/list builtins —

```n3
( ?v { …clause… } ?list ) log:collectAllIn ?scope .
```

with the published schema `( $s.1- $s.2+ $s.3- )+ log:collectAllIn $o?` where `$s.2 : log:Formula`
and `$s.3 : rdf:List` — i.e. every such composition routes through an inherently **formula-valued
and list-valued** term.

### Two corrections to the prior gap analysis (both *narrow* the claim)

1. **Grouping is NOT missing.** § 4 implied the idiom gives no equivalent of RDFox's `ON ?g`. It
   does: variables bound in the enclosing rule body act as the group key, so
   `AGGREGATE … ON ?hero` maps onto `?hero` being bound outside the `collectAllIn` triple (this is
   precisely the spec's own `:defeatedEnemies` example). This should not be claimed upstream.
2. **MIN/MAX cost more than SUM/COUNT/AVG.** § 4 said the idiom "gives no direct
   SUM/MIN/MAX/AVG". More precisely, *through the list idiom* — every row is **expressible**; what
   differs is how much machinery each one needs:

   | aggregate | dedicated builtin? | how it is reached |
   |---|---|---|
   | SUM | yes — `math:sum` | schema `( $s.i+ )+ math:sum $o-`, "the sum of the numbers given in the subject list" |
   | COUNT | yes — `list:length` | direct |
   | AVG | no | composed, 3 steps: `math:sum` + `list:length` + `math:quotient` |
   | **MIN / MAX** | **no** — no `math:min`/`math:max`/`list:min`/`list:max` in the registry | expressible, but only by hand-rolled recursion or a `list:member` + negation-as-failure ("no member is greater") encoding |

   So the honest claim is **not** "N3 cannot aggregate" — it demonstrably can, by composition. It
   is: there is *no dedicated aggregate primitive*; every route is unavoidably formula-/list-valued;
   and MIN/MAX additionally need recursion or negation-as-failure where SUM/COUNT need one builtin.

### Prior art upstream — already surfaced once, closed unmerged, undiscussed

**[w3c-cg/N3 PR #119](https://github.com/w3c-cg/N3/pull/119) — "add aggregation operations"**
(@domel, opened 2023-01-16, **closed 2023-03-09 without merge, zero comments**). It added
`list:min`, `list:max`, `list:sum`, `list:avg`, `list:median`, `list:mode`, `list:variance`,
`list:stddev` to `ns/list.n3` (+64 lines, term status `unstable`). The diff also has Turtle syntax
errors — every term after `list:min` is missing its `a` (`list:max rdf:Property;`) — which may be
all that happened to it.

Full-text search of `w3c-cg/N3` issues **and** bodies for `aggregate` / `aggregation` / `list:sum` /
`list:avg` returns **only** PR #119. So: the request has been *made* upstream once, as a bare
vocabulary patch with no motivation and no discussion, and was dropped. **This is a dedupe-relevant
fact, and it shapes the draft below** — the issue is framed as reviving #119 *with* the motivation
and the use case it lacked, not as a fresh discovery.

### Why the composition route is unusable *in sparq's chosen profile* (the sparq-side point)

sparq's compiled/stratified N3 subset (`crates/sparq-reason/src/n3/compiled.rs`, module docs)
excludes, as a *loud compile error*: "list builtins/generators, `math:`/`time:` builtins, … formula-
or list-valued facts". The `collectAllIn` + `math:sum` idiom needs all three. sparq's full text
engine does implement `log:collectAllIn`/`log:forAllIn`
(`crates/sparq-reason/tests/n3_collect_stratified.rs`), so this is neither a sparq capability gap nor
an N3 expressivity gap. It is a **profile** incompatibility: every available N3 spelling of an
aggregate forces you out of the first-order, stratifiable fragment that our compiled Datalog-style
evaluator (and RDFox's own `AGGREGATE`) lives in — a fragment *we* chose to restrict to, so the cost
is ours to justify, not N3's to answer for.

## Draft issue — w3c-cg/N3 (NOT FILED; @jeswr review gate)

> **Title: No dedicated aggregate primitive in N3 (reviving #119 with motivation): RDFox-style
> `AGGREGATE … ON … BIND SUM(?v) AS ?x` is reachable only by composition through
> `log:collectAllIn`**
>
> > 🤖 This issue was written by an autonomous agent (a SPARQ agent) operating on @jeswr's behalf,
> > and is posted with his review. It is a question/gap report, not a defect report.
>
> **Why this is being raised.** We are implementing a rules substrate that treats N3 as the common
> surface for a native Datalog dialect and for RIF, on the working assumption that N3 is a superset
> of both, plus of the RDFox rule syntax. Checking that assumption construct-by-construct, RIF-Core
> came out fully expressible (frames/membership/subclass lower to triples; the RIF builtins lower to
> `math:`/`string:`/`list:`), and RDFox's negation maps onto `log:notIncludes`. Exactly one construct
> has no *dedicated* counterpart and is reachable only by composition: **aggregation**. To be clear
> up front: we are not claiming N3 cannot express these aggregates — it can, and we show how below.
>
> **The construct.** RDFox (and SPARQL, and RIF-BLD's usual extensions) have an aggregate
> *expression* that binds a scalar computed over a group:
>
> ```text
> [?dept, :headcount, ?n] :- AGGREGATE( [?p, :worksIn, ?dept] ON ?dept BIND COUNT(?p) AS ?n ) .
> [?dept, :payroll,   ?t] :- AGGREGATE( [?p, :worksIn, ?dept], [?p, :salary, ?s] ON ?dept BIND SUM(?s) AS ?t ) .
> [?dept, :topSalary, ?m] :- AGGREGATE( [?p, :worksIn, ?dept], [?p, :salary, ?s] ON ?dept BIND MAX(?s) AS ?m ) .
> ```
>
> **What N3 has today.** As far as we can tell from the [Notation3 Language
> spec](https://w3c-cg.github.io/N3/spec/) and the [Notation3 Builtin Functions Final Community Group
> Report (3 June 2026)](https://w3c-cg.github.io/n3Builtins/), neither document contains the word
> "aggregate", and there is no aggregate builtin family. Aggregation is instead *composed* out of the
> list-valued findall plus the arithmetic/list builtins:
>
> ```n3
> { ?dept a :Dept .
>   ( ?s { ?p :worksIn ?dept . ?p :salary ?s } ?salaries ) log:collectAllIn _:t .
>   ?salaries math:sum ?total .
>   ?salaries list:length ?n .
> } => { ?dept :payroll ?total ; :headcount ?n } .
> ```
>
> To be clear about what *does* work, so this is not overstated: **grouping is fine** — `?dept`,
> bound in the enclosing body, is the group key, so `ON ?dept` is covered. **SUM** (`math:sum` over
> the collected list) and **COUNT** (`list:length`) are reachable, and **AVG** is reachable in three
> steps (`math:sum` + `list:length` + `math:quotient`).
>
> **Two things that cost us.** Neither is an expressivity claim.
>
> 1. **MIN and MAX have no builtin at all.** The registry has no `math:min`/`math:max` and no
>    `list:min`/`list:max`, so unlike SUM and COUNT the `MAX(?s)` rule above cannot be written with a
>    builtin: it has to be *encoded*, either as hand-rolled recursion over the collected list or as a
>    `list:member` + `log:notIncludes` "no member is greater" idiom. Both work; they are just
>    markedly more machinery than the one-triple SUM. That asymmetry (SUM/COUNT builtin, MIN/MAX
>    hand-encoded) looks more like an omission than a design decision, which is really the narrow
>    question here.
> 2. **Every route to aggregation is formula-and-list-valued.** `log:collectAllIn` takes a
>    `log:Formula` and yields an `rdf:List`. For a full N3 engine that is no obstacle — and our own
>    full engine implements it. But an implementation that compiles a stratifiable, first-order
>    subset of N3 down to a Datalog-style evaluator (which is the point of the exercise for us, and
>    is the fragment RDFox's `AGGREGATE` itself inhabits) typically excludes formula-valued and
>    list-valued terms from that subset precisely to keep it first-order. Under that restriction none
>    of the encodings above is admissible. We want to be exact about what that does and does not
>    show: it is a limitation of **the profile we chose**, not of N3 — N3 expresses these aggregates
>    fine, just never without a higher-order term (and, for MIN/MAX, without recursion or
>    negation-as-failure on top).
>
> **Prior art.** [#119](https://github.com/w3c-cg/N3/pull/119) proposed `list:min`/`max`/`sum`/`avg`/
> `median`/`mode`/`variance`/`stddev` in January 2023 and was closed unmerged with no discussion. We
> suspect it lapsed because it was a bare vocabulary patch with no stated use case (and the Turtle in
> it is malformed — every term after `list:min` is missing its `a`). This issue is an attempt to
> supply the missing motivation rather than to open a new front.
>
> **Questions for the CG.**
>
> 1. Is the absence of `math:min`/`math:max` (or `list:min`/`list:max`) intentional, or simply an
>    omission that #119 tried and failed to fix? If the latter, would a corrected version of #119 —
>    at minimum `min`/`max`, matching `math:sum`'s existing list-valued schema — be welcome?
> 2. Is `log:collectAllIn` + list builtins considered *the* intended and sufficient way to aggregate
>    in N3, with no aggregate expression construct planned? A sentence to that effect in the spec
>    would settle it for implementers either way.
> 3. Has the CG considered a first-order aggregate construct (a term-level `?x` bound to
>    `SUM/MIN/MAX/AVG/COUNT` over a group, without routing through a formula and a list)? We are not
>    proposing syntax — we are asking whether the higher-order-only status is deliberate, since it is
>    the one place where an N3 profile restricted to a compiled first-order fragment stops being a
>    superset of RDFox's rule language.

## Notes for @jeswr (review gate — nothing filed)

- **Venue is `w3c-cg/N3`** (renamed from `w3c/N3`), issue tracker open, 50 open issues.
- The draft deliberately **concedes** grouping, SUM, COUNT, AVG *and* the recursion / `list:member`
  + NAF encodings of MIN/MAX rather than claiming a blanket "N3 cannot aggregate". What is verified
  is much narrower than § 4 assumed: **no dedicated aggregate primitive** (and no `min`/`max`
  builtin), plus the fact that every existing encoding is formula-/list-valued and therefore outside
  *sparq's own* compiled first-order profile. No claim of N3 non-expressivity is made or supported
  anywhere in this record — a construct reachable by composition or recursion is still expressible,
  and overstating this would be wrong and would read badly upstream.
- It does **not** claim sparq is correct or that N3 should change; questions 1–3 are questions.
- Per `AGENTS.md` § *Upstream contributions*, if this is filed it carries the 🤖 agent self-id (in
  the draft above) and should not be marked ready for CG action without your say-so.
- Follow-up beads once upstream answers: (a) if `min`/`max` land, extend the RIF/N3 builtin lowering
  table; (b) if the CG confirms the list idiom is the intended route, record that in
  `gpt56-decomp-rules-substrate-2026-07.md` § 4 — the "N3 is not a superset" reading is already
  retired here in favour of a documented, deliberate profile restriction.

## Verification method (reproducible)

Live documents fetched 2026-07-27: `https://w3c-cg.github.io/N3/spec/` and
`https://w3c-cg.github.io/n3Builtins/` (tags stripped, case-insensitive term search); upstream prior art via
the GitHub search API over `repo:w3c-cg/N3` titles **and** bodies for `aggregate`, `aggregation`,
`list:sum`, `list:avg`; PR #119 state, diff and comment count via the REST API.

**Known method defect (do not repeat).** The builtin inventory was extracted by a *namespace-prefix
scan* of the report text. That over-counts: it picks up references in prose, examples and schema
fragments, not just definitions — it reported `log:` at 22 where the report defines 18. Every count
in the inventory above is therefore flagged unverified, and the inventory must be re-derived by
enumerating the report's **definition headings** before it is cited anywhere. The record's
conclusions do not rest on it.
