# oxttl prefixed-name expansion — measured verdict (2026-07-28)

> 🤖 SPARQ agent [OPUS-5] — bead **sq-98w7z.3** ("Upstream oxttl low-copy prefixed-name
> expansion"), epic **sq-98w7z**, parent row 3 of
> `research/dependency-bottleneck-analysis-2026-07.md`. Harness:
> `bench/parse/oxttl-prefix-alloc/`. **No sparq crate code was changed and no `oxttl`
> source is vendored into this repository.**

## Verdict

**Measured-yes on the win; no upstream PR — the fix is already upstream.**

The bead exists to upstream a low-copy prefixed-name expansion to
`oxigraph/oxigraph`, because `oxttl` 0.2.3 allocates a fresh `String` per prefixed
name. That is real and still true of the released crate sparq pins. But it is
**already fixed on oxigraph `main`**, in commit
[`f5383d8`](https://github.com/oxigraph/oxigraph/commit/f5383d8) (committed
2026-05-31, *"oxrdf: introduce our own OxString type"*), which `git tag --contains`
places in **no released tag** — the newest `oxttl` release on the crates.io index
is still 0.2.3. Drafting the PR this bead asks for would re-submit work that landed
two months ago.

What the measurement did find is worth more than the bead expected:

- Upstream `main` parses this Turtle corpus **1.38× faster** than the 0.2.3 release
  sparq pins (1.531 s → 1.112 s, min-of-5, box below), byte-identical output.
- Only about a third of that is the allocation fix. The **larger** share is
  [`924d0b1`](https://github.com/oxigraph/oxigraph/commit/924d0b1) (2026-07-22,
  *"oxttl: avoid moving the parser objects for every token"*), which is not an
  allocation change at all — see the attribution table.
- So the actionable item is **not** an upstream patch. It is a **dependency
  decision**: sparq is holding a Turtle parser that is materially slower than the
  same project's current tree, on the exact axis (`sq-wrn61`, D14c) that is a P1
  dominance gap. That is a version-pin question, and it is captured as follow-up
  work rather than actioned here (this bead's file scope is the harness only).

**Disposition:** close sq-98w7z.3 as **measured-no on the PR deliverable /
measured-yes on the profile**, superseded by the follow-ups in §7. It joins
`sq-98w7z.8` (vendored spargebra) as an *upstream-release-lag* item, not an
*upstream-contribution* item.

## 1. Dep-gate check

The bead is dep-gated: *"start only after sq-jocpn's design record lands and align
with its findings (its option 3 IS this bead)"*.

`sq-jocpn` has landed — commit `6ec1a2da`, *"perf(sparq-core): native byte-level
Turtle tokenizer/parser (opt-in `native-ttl`)"* (#1882), documented at
`skills/data-formats/SKILL.md`. It chose the **roll-our-own** option, not
stay-on-oxttl. That does not retire this bead, for the reason row 3 already gives:
the wasm build and the TriG / N-Quads paths stay on `oxttl` regardless. The gate is
satisfied and the bead's premise survives sq-jocpn. It does not survive contact
with oxigraph `main`.

## 2. The bottleneck, as it actually is

`oxttl` 0.2.3, `src/lexer.rs`, `resolve_local_name` — reached once per prefixed
name from nine call sites in `src/terse.rs` (nine on `main` too):

```rust
let iri = format!("{start}{local}");
```

The token itself is already zero-copy (`prefix: &'a str`, `local: Cow<'a, str>`),
so this `format!` is the whole cost. It is worse than one allocation, which is
worth stating precisely because it is *measurable*, not folklore — a standalone
probe under a counting allocator, on the exact operand shape
(`"http://www.wikidata.org/entity/"` + `"Q1326"`):

| expression | allocs | reallocs | final capacity | len |
|---|---|---|---|---|
| `format!("{start}{local}")` | 1 | **1** | 62 | 36 |
| `String::with_capacity(a+b)` + two `push_str` | 1 | **0** | 36 | 36 |

A format string that *begins* with an argument gets no useful capacity estimate,
so `{start}` allocates and `{local}` then reallocates and memcpys `start` again —
and the amortised regrowth overshoots to 62 bytes for a 36-byte IRI. So the
released crate pays, per prefixed name, one extra allocator round-trip, one extra
copy, and ~1.7× the memory. That is what the realloc column in §4 is counting:
3 813 289 reallocations against 1 921 676 triples is very close to one per
prefixed name.

On `main` the same line reads:

```rust
let iri = OxString::concat([start.as_str(), local]);
```

`OxString` (introduced by `f5383d8`) is a compact `Arc<str>`/`Cow<'static, str>`
fusion; `concat` sizes the single allocation exactly, so the realloc disappears,
and the type is threaded through the whole token enum (`IriRef`, `String`,
`Variable`), which is why the allocation saving is much larger than the
prefixed-name path alone.

## 3. Method

`bench/parse/oxttl-prefix-alloc/` compiles **one** source file (`src/main.rs`) twice
— once against `oxttl =0.2.3` from crates.io, once against `oxigraph.git` at a
pinned rev — via two thin wrapper packages that share it through `[[bin]] path`.
The measured code is therefore byte-identical between legs and the only variable is
which `oxttl` the linker resolved. `./ab.sh --attribute <sha>` mints an extra leg at
any commit, which is how §5 was produced. Each leg is buildable under `mimalloc`
(the production allocator), `count-alloc` (a counting shim over the system
allocator), and `rdf-12` (the feature set the sparq workspace enables) — ten
builds in all for §4, every one of them warning-free.

**Invariant evidence.** Every run prints an order-sensitive FNV-1a digest over the
raw component strings of every parsed triple (`as_str()` / `value()` /
`datatype()` / `language()` — deliberately *not* any `Display` impl, so a
serializer change between revisions cannot masquerade as a parse divergence),
plus the triple count. **All 14 builds exercised for this record — the two legs ×
{default, mimalloc, count-alloc, rdf-12, rdf-12+count-alloc}, plus the four §5
attribution revisions — returned digest `0x96c046175f277757` and 1 921 676
triples.** That is the bead's "parse is byte/count-identical" invariant, discharged
as a differential.

**The digest was mutation-tested, not assumed.** On a 3 MB prefix of the corpus
(97 027 triples) the released leg digests `0xd610…d3f9`, and the upstream leg
returns the same value. Two deliberate corruptions:

| mutation | triples | digest |
|---|---|---|
| none | 97 027 | `0xd6106107b9b0d3f9` |
| `@prefix wd:` retargeted `entity/` → `ENTITY/` | 97 027 | `0xbe5f9ae0c75ff339` |
| one local name of ~97 k changed (`wd:Q1326` → `wd:Q1327`) | 97 027 | `0xa0c8d0909ccf39b2` |

Both mutations change the digest and **neither changes the triple count** — so the
count on its own is a vacuous guard for this bead, and the digest is the one doing
the work. The first mutation is the important one: it perturbs exactly what
`resolve_local_name` computes, which is the function the two legs implement
differently.

**What was NOT run: W3C TurtleTests.** The bead's invariant clause asks for the
W3C suite green *under the patched oxttl*. No `oxttl` patch was authored — the
change under test is upstream's own, carried by upstream's CI on those commits —
so there is no sparq-authored parser change for the suite to cover. The
differential digest above is the evidence actually available here, and it is
weaker than a conformance run: it proves the two revisions agree **on this
corpus**, not that either is conformant.

## 4. Results

**Box: NON-CANONICAL.** 4-core Intel Xeon Platinum 8573C, 15 GB, Linux
6.17.0-1020-azure, `rustc 1.88.0` — *not* the canonical `c6i.4xlarge` bench
instance, and *not* the repo's pinned 1.97.1 toolchain (the sandbox cannot install
it). Timings are directional only and must not be published or compared against
any canonical row. Corpus: `gen`-produced deterministic stand-in with the
`wd:`/`wdt:` shape of the sq-wrn61 slice, 60 000 047 bytes / 1 921 676 triples —
**not** the real `wikidata-slice.ttl`, which is gitignored and was not available
here. min-of-5.

| oxttl | allocator | s (min) | MB/s | Mtriples/s |
|---|---|---|---|---|
| 0.2.3 (release, what sparq pins) | system | 1.531 | 39 | 1.26 |
| main `9af7d59` | system | 1.112 | 54 | 1.73 |
| 0.2.3 (release) | mimalloc | 1.490 | 40 | 1.29 |
| main `9af7d59` | mimalloc | 1.122 | 53 | 1.71 |
| 0.2.3 (release), `rdf-12` | system | 1.531 | 39 | 1.25 |
| main `9af7d59`, `rdf-12` | system | 1.119 | 54 | 1.72 |

→ **1.38× under system malloc, 1.33× under mimalloc** (the allocator
`sparq-cli ingest` ships with — quoted so an allocation win is not over-claimed
against the allocator production actually uses), and **1.37× with `rdf-12`**, which
is the feature set the sparq workspace actually enables
(`oxttl = { version = "0.2", features = ["rdf-12"] }`). The allocation counts below
are identical in the `rdf-12` and default builds.

Allocation counts come from no clock, so unlike the timings they do not move with
box speed or load: repeated runs of the *same binary* over the same corpus return
the same counts. They are **not** box-independent facts, though — the shim counts
every allocation the whole binary makes, so toolchain, target, dependency
resolution and feature set all feed them. The counts below are measurements of one
configuration: `rustc 1.88.0`, `x86_64-unknown-linux-gnu`, each leg's `oxttl`/`oxrdf`
pin as declared in its wrapper manifest (`=0.2.3` from crates.io / git `9af7d59`)
with the rest of the graph resolved fresh (the harness lockfiles are deliberately
uncommitted, so transitive revisions can drift between runs), `count-alloc` the only
feature enabled, on the corpus described above. Re-derive them under any other
configuration rather than carrying these numbers across:

| oxttl | allocs/parse | reallocs/parse | allocs/triple | bytes/triple |
|---|---|---|---|---|
| 0.2.3 (release) | 10 554 933 | 3 813 289 | 5.49 | 247.3 |
| main `9af7d59` | 4 464 454 | **4** | 2.32 | 105.2 |

−57.7% allocations, and reallocation is eliminated outright (3.8 M → 4) — the
direct signature of `format!`'s zero-capacity start being replaced by an
exact-size `concat`.

## 5. Attribution — which commit actually bought it

Each row is a separate build of the same harness source against that commit.

| oxttl rev | committed | commit | s (min) | Δ vs row above |
|---|---|---|---|---|
| `aaa1d5f` | 2026-05-27 | parent of `f5383d8` | 1.527 | — |
| `f5383d8` | 2026-05-31 | *oxrdf: introduce our own OxString type* | 1.401 | **−8.3%** |
| `d14ac0b` | 2026-07-20 | *Bump OxIRI* — parent of `924d0b1` | 1.394 | −0.5% |
| `924d0b1` | 2026-07-22 | *oxttl: avoid moving the parser objects for every token* | 1.127 | **−19.2%** |
| `9af7d59` | 2026-07-26 | main HEAD at time of writing | 1.112 | −1.3% |

Dates are **commit** dates, not author dates: `f5383d8` was authored 2026-05-17 and
committed 2026-05-31, so author dates do not order this table. Rows 1–2 and rows
3–4 are consecutive-commit pairs; rows 2→3 span 44 commits and are not an
attribution, only a bracket.

Two honest readings of this table:

1. **The allocation fix is entirely `f5383d8`.** Its parent counts 10 554 933
   allocations / 3 813 289 reallocations — identical to the 0.2.3 release;
   `f5383d8` counts 4 464 454 / 4 — identical to main HEAD. Nothing after it moves
   the allocation numbers at all.
2. **The allocation fix is the minority of the speed-up.** It buys ~8%; the
   per-token move elimination in `924d0b1` buys ~19%. The premise this bead
   inherited from row 3 — that the per-prefixed-name `String` is the thing worth
   chasing in oxttl — is directionally right but was **over-weighted**. Recorded
   here so the next reader of row 3 does not repeat the estimate.

## 6. Limitations

- Synthetic corpus, not the real sq-wrn61 slice. It reproduces the prefix set and
  the prefixed-name-dominant object column, which is what exercises the function
  under test; it does not reproduce the real slice's literal/language distribution.
  Re-run with `--corpus` on a box that has the slice before quoting anything.
- Non-canonical box, non-pinned toolchain (§4). The **ratios** are more robust than
  the absolute rates. The allocation counts are insensitive to box speed and load —
  they are not timings — but they are a property of the build configuration recorded
  in §4, not a universal constant: the repo's pinned 1.97.1 toolchain, a different
  target, or a different transitive resolution (the harness lockfiles are
  uncommitted) could move them. The **direction** of the delta — reallocation
  eliminated by an exact-size `concat` — is the robust part.
- The two legs resolve slightly different transitive graphs (notably `oxiri`
  0.2.11 vs 0.3.1). This is an honest "release vs current tree" comparison, not an
  isolated single-patch A/B — except in §5, where consecutive-commit pairs make the
  transitive graph nearly constant.
- No profiler was available in this environment. The residual candidate noted in §7
  (the per-prefixed-name `HashMap` lookup, still SipHash on `main`) is therefore an
  **unmeasured hypothesis** and is deliberately *not* claimed as a bottleneck.

## 7. What follows from this

1. **Do not file the upstream PR.** It exists and is merged (`f5383d8`).
2. **The real lever is the pin.** The sparq workspace declares
   `oxttl = { version = "0.2", features = ["rdf-12"] }`, resolving to 0.2.3 in
   `Cargo.lock` (plus a deliberate second `oxttl` 0.1.8 under the `oxttl01` alias in
   `sparq-canon`, which is a separate compatibility concern and out of scope here).
   Upstream's tree measured ~1.38× faster than that pin here. The axis it sits on —
   Turtle single-thread, ~2× behind serd — is an open P1 dominance gap (`sq-wrn61`,
   D14c), so a dependency bump is a live lever on it. Whether to take that via a
   temporary git pin or by waiting for the `oxttl` 0.3 release is a maintainer
   decision with real supply-chain cost (a git dependency is not covered by
   `cargo-vet`/`cargo-deny` the way a registry release is), and it must be
   re-measured on the canonical instance first. Captured as follow-up work.
3. **Residual upstream candidate, unproven.** `resolve_local_name` still does a
   `HashMap<OxString, Iri<OxString>>` lookup under the default SipHash hasher, once
   per prefixed name, on a 2–4 byte key. A one-entry memo would remove it. Order of
   magnitude is low single-digit percent at best, and the profile has just shifted
   under `924d0b1` — so this needs a profile before it needs a patch, and it is
   recorded as a hypothesis, not a finding.
