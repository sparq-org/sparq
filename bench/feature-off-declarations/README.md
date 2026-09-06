<!-- [OPUS-4.8] sq-v3nel-v2 (2026-07-07): per-PR feature-OFF change declarations. -->

# feature-OFF wasm-bundle change declarations (mechanism V2)

The `vectorized-feature-off` gate (leg 2,
`.github/workflows/vectorized-feature-off.yml`) builds the feature-OFF `sparq-wasm`
bundle from the **base** tree and the **head** tree in the same CI run and compares
them **byte-for-byte**. Identical bytes pass automatically. If the bytes differ, the
change must be **declared** — otherwise the gate fails (the whole point: an accidental
`vectorized`/default-path leak into the feature-OFF build is caught).

## How to declare

If your PR *intentionally* changes always-compiled engine/core code in a way that moves
the feature-OFF bundle bytes, add **one file** named after your PR number:

```
bench/feature-off-declarations/<PR-number>.json
```

with the shape:

```json
{
  "pr": 1234,
  "date": "2026-07-07",
  "reason": "one line on what always-compiled change moves the feature-OFF bytes"
}
```

The gate is satisfied when the head tree's declarations directory contains at least one
`<digits>.json` (or `.md`) file the base tree's does **not** — a set difference on the
directory listing. Only names matching `<digits>.json|md` count as declarations, so this
`README.md` (and any `.gitkeep`) is ignored.

## Why per-PR files (V2)

The previous mechanism stored one scalar `change_token` in
`bench/feature-off-declaration.json`. Every declaring PR edited the **same line**, so the
first declared PR to merge made every other declared PR textually **CONFLICTING** in git
(`#1720` and `#1718` both went `DIRTY` after `#1726`'s declaration merged). The gate check
was order-independent but the file was not. Per-PR files remove the shared line: different
PRs add different files, so git never conflicts regardless of merge order.

The legacy scalar file `bench/feature-off-declaration.json` is **retired** (frozen, kept
parseable). During the transition window the gate still accepts a scalar-token inequality
so in-flight pre-V2 branches keep working; new PRs must use a per-PR file here instead.

## Scope

This declaration governs **intent** (did you mean to change the feature-OFF bytes at all?).
The **size** of an accepted change is governed separately, unchanged, by the
`metrics.wasm_bundle_bytes` floor ratchet (±2% band) in `bench/perf-baseline.json` /
`bench.yml`. A change moving the bundle > ±2% must also raise that floor.

## Why the gate fires on comment-only edits (measured)

<!-- [OPUS-5] sq-v3nel-v3 (2026-07-28): the dominant benign class, and the tool that derives it. -->

Leg 2 compares the bundles **byte-for-byte**, and a bundle byte can move without a single
instruction changing. The workspace `release` profile sets `panic = "abort"` but not
`panic_immediate_abort`, so every panicking call site still carries a `core::panic::Location`
record — and that record embeds the call site's **line number**. Insert a comment block, or an
off-by-default `#[cfg(feature = ...)]` item, anywhere **above** always-compiled code in the same
file, and every line below it moves; the line numbers baked into those records move with it.

This was measured directly on this repo (`cargo build --profile release-wasm -p sparq-wasm
--target wasm32-unknown-unknown`, the leg's own build), each row a single change against the
same tree:

| change                                                              | bundle verdict | size delta | differing bytes |
| ------------------------------------------------------------------- | -------------- | ---------- | --------------- |
| rebuild, no source change                                            | identical      | 0          | 0               |
| 34 pure comment lines inserted mid-file (`sparq-core/src/compress.rs`) | **differs**    | **0**      | **3**           |
| off-by-default `#[cfg]` module added *below* all compiled code       | identical      | 0          | 0               |
| comment block + off-by-default `#[cfg]` statement inserted mid-file  | **differs**    | **0**      | **2**           |
| four **ungated** code lines added to `Graph::build`                  | **differs**    | **+228**   | **373027**      |

The separation is not subtle: a change that adds no compiled code moves single-digit bytes and
leaves the size **exactly** unchanged, while real code entering the default build moves
hundreds of thousands of bytes and shifts the size.

**This is not a reason to loosen the gate.** The last row is precisely what the leg exists to
catch, and byte-for-byte is what catches it. It *is* a reason not to make a human hand-write a
prose declaration for the rows above it.

## Deriving a declaration instead of asserting one

`scripts/feature_off_autodeclare.py` decides that case from the compiler:

```
python3 scripts/feature_off_autodeclare.py --repo . \
    --base-sha <pr base sha> --head-sha <pr head sha> --pr <number> --write
```

It rebuilds the head tree with every **added** line replaced by an **empty** line — line count
preserved, so every line position is unchanged — and requires the result to be **byte-identical**
to the head bundle. If blanking the additions changes nothing the compiler emits, the additions
emitted nothing. When the diff also removes lines it runs the same proof on the base side. Only
then is a declaration written, carrying the measured byte counts.

Everything else **refuses**, with a named reason, and the leg stays red:

| refusal                          | meaning                                                       |
| -------------------------------- | ------------------------------------------------------------- |
| `added-lines-are-semantic`       | blanking the additions changed the bundle — real code was added |
| `deleted-lines-are-semantic`     | blanking the deletions in the base changed it — real code was removed |
| `neutral-build-failed`           | the additions were load-bearing (the tree stopped compiling)   |
| `proof-would-be-vacuous`         | nothing was blanked and nothing was deleted, so the comparison would be `head == head` |
| `unsupported-file-change`        | a symlink or gitlink, which blanking cannot speak for          |

### Why every changed path is blanked, not just the ones in the build closure

An earlier version scoped blanking to a closure derived from `cargo tree` ∩ `cargo metadata
--no-deps`, and that shipped a **live false pass**. `--no-deps` returns **workspace members
only**, while sparq's root manifest carries `exclude = ["vendor/spargebra", …]` *together
with* `[patch.crates-io] spargebra = { path = "vendor/spargebra" }` — so spargebra is
compiled into the feature-OFF bundle while not being a member. A change under `vendor/`
classified inert, was never blanked, and `neutral == head` therefore held **by
construction**: three genuinely non-benign changes there all came back `declared`.

The emitted declaration even confessed it — *"rebuilding the head tree with all **0** added
non-blank line(s) blanked produced a BYTE-IDENTICAL bundle"* — recording that nothing had
been proved, and declaring anyway.

Two fixes, and the second matters more than the first:

1. **Nothing is skipped.** Every changed non-manifest path is blanked, wherever it lives.
   Blanking a file the build never reads is free; blanking one it *does* read moves the
   bundle and the derivation refuses. The closure is now **reporting only** — no soundness
   claim rests on it — and its query no longer uses `--no-deps`.
2. **A proof that proves nothing must refuse.** If the neutral tree comes out identical to
   the head tree *and* the diff deletes no non-blank line, the comparison is `head == head`
   and holds regardless. That guard catches the whole class without anyone needing to know
   which path type was missed.

An **intentional always-compiled change** — a hot-path optimisation, a refactor of code the
default build ships — is refused on purpose. Its author is asserting *intent*, and intent cannot
be derived; write the declaration by hand, as `3679.json` and `3755.json` do.

Three deliberate limits:

* The tool **does not make the leg pass**. It writes a file that still has to be committed, so
  the escape hatch stays inside the reviewed diff. An auto-passing gate whose derivation had a
  hole would be exactly the rubber stamp this leg exists to prevent.
* Every obligation builds into its **own** target directory. Sharing one warm directory
  across the materialised trees was tried for speed and reverted: on #4350 it reported a
  299-line `crates/sparq-engine/src/exec.rs` rewrite as *byte-identical* to its base, because
  `git archive` stamps each tree's files with its commit time and cargo's freshness check is
  mtime-based. A false "identical" is the one outcome that would auto-declare a real code
  change, so cold builds are the price.
* It attributes against the **merge base**, not `pull_request.base.sha`. Those differ whenever a
  branch is behind its base, and leg 2 itself compares `base.sha` — so on such a PR the leg's
  reported difference also carries, in reverse, base-branch commits the PR does not have. The
  tool says so in its output rather than attributing them to the PR.

A census of the live class (how many open PRs are red on this leg, how many already carry a
declaration) is available without running any build:

```
python3 scripts/feature_off_autodeclare.py --census sparq-org/sparq
```
