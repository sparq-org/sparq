<!-- [OPUS-5] Design record for GitHub issue #3713 (maintainability program).
Read-only investigation: no crate source was changed by the work that produced
this file. Upstream state could NOT be checked from the executing environment
(no network) — every upstream claim below is labelled as such. -->

# `sparq-canon`: deleting the oxrdf-0.3 ↔ 0.2 N-Quads bridge

> 🤖 SPARQ agent [OPUS-5] — design-for-review record for issue **#3713**
> ("Port zkp-ld rdf-canon to oxrdf 0.3 upstream and delete sparq-canon's double-parse
> N-Quads bridge + legacy oxrdf 0.2/oxttl 0.1 pins"). Part of the maintainability
> program. **Nothing is implemented by this record** and nothing was filed upstream.
> Related records: `research/gap-canon-2026-07.md` (canon bench panel),
> `research/gap-canon-native-2026-07.md` (in-process bridge-overhead column),
> `research/rdf-canon-rdf12-triple-terms-upstream.md` (the four already-drafted,
> still-unfiled upstream items).

## 1. Why this record exists rather than a patch

The issue's primary action is an **upstream** contribution to `zkp-ld/rdf-canon`, and its
stated fallback is a **large in-house substitution** (promote `crates/sparq-canon/src/rdf12.rs`
to the standard canonicalization path). Neither was executable by the read-only pass that
wrote this file: the environment has no network and no usable Rust toolchain (§6), so the
upstream crate could be neither fetched nor compiled. The fallback additionally turns out to
have concrete, code-visible blockers, and it trades away a property of the current design
that the issue does not price in.

That is an **environment limit on this pass, not a decision to wait for the maintainer.**
Per `AGENTS.md` §*STANDING RULE — proceed without waiting for the maintainer's greenlight*,
the choice between the options is made here on best judgment (§7) and every phase in §8 that
an agent with a network and a toolchain can execute proceeds **without** a greenlight. The
one genuinely owner-only step is the literal act of *opening* a PR on the third-party repo
(§8 phase 3) — an out-of-repo publication, which `AGENTS.md`'s maintenance loop step 7 lists
as owner-filed. Nothing waits on it: option B ships the identical diff locally.

## 2. Premise check against the actual tree

Verified by reading `crates/sparq-canon/{Cargo.toml,src/lib.rs,src/rdf12.rs}`,
`crates/sparq-difftest/{Cargo.toml,src/iso.rs}`, `Cargo.lock` and `deny.toml`.

### 2.1 Confirmed

| Claim | Verdict | Grounding |
|---|---|---|
| Every canonicalization crosses a serialize → re-parse → canonicalize → re-parse bridge | ✅ | `src/lib.rs:424-501` (`bridge_to_02`, `bridge_triples_to_02`, `parse_02`, `serialize_quads*`), plus `parse_canonical` at `src/lib.rs:521` |
| `Cargo.toml` pins a parallel legacy stack | ✅ | `oxrdf02 = 0.2.4`, `oxttl01 = 0.1.8`, `digest = 0.10`, `sha2 = 0.10` (dep + dev-dep) |
| `src/rdf12.rs` is a native oxrdf-0.3 RDFC-1.0 re-implementation that upstream lacks | ✅ | full §4.2/§4.4/§4.6/§4.7/§4.8 machinery, `Digest`-generic, HNDQ call guard; 1690 LOC (issue says 1679 — the file has moved since) |

### 2.2 Corrections to the issue's premise

**(a) This does not clear the `oxrdf ×2` / `oxttl ×2` rows of `sq-98w7z.7`.** `oxrdf 0.2.4`
has **three** holders in the lockfile, not one:

- `sparq-canon` (via `rdf-canon` and the `oxrdf02` alias) — the one this issue removes;
- **`sparq-difftest`** — a *direct* `rdf-canon 0.15.3` + `oxrdf 0.2.4` + `sha2 0.10`
  dependency (`crates/sparq-difftest/Cargo.toml`), deliberately so: `src/iso.rs` needs a
  canonical blank-node labelling that is **not** sparq's own, because a bug in `sparq-canon`
  would otherwise cancel on both sides of a value-level differential test. That crate's
  manifest states the constraint explicitly. It is not removable by this work — removing it
  would defeat the crate's reason to exist;
- the git-pinned `solid-oidc-verifier` (via `crates/sparq-lws-core`), which pulls
  `oxttl 0.1.8 → oxrdf 0.2.4`, and is the only remaining holder of `oxttl 0.1.8`.

So the honest effect on the duplicate-version registry is: **one of three** `oxrdf 0.2`
holders and **zero of one** `oxttl 0.1` holders removed. Both rows stay. `deny.toml`'s
own blocker registry (lines 180-185) names only two of the three holders — it omits
`sparq-difftest`; that is doc drift worth its own fix.

**(b) It does not lift the workspace `digest`/`sha2` 0.10 pin.** `sparq-difftest` pins
`sha2 = "0.10"` for the same `rdf_canon::Digest` bound. What it *does* lift is
`sparq-canon`'s own pin — including its **public** `pub use digest::Digest`
(`src/lib.rs:165`), which is the API-visible half and the one Dependabot #17 keeps
tripping over. That is a real win, and it is simultaneously a **breaking change** for
`sparq-zk`, `sparq-vc`, `sparq-wasm` and the crate's tests, all of which name
`sha2::Sha256`/`Sha384` against that re-exported trait.

**(c) The upstream-activity claims could not be verified here.** "last release 2025-06-25,
no open PRs" is taken from the issue and is **unchecked** — the executing environment has
no network access and the contract forbids GitHub API calls. The only upstream fact this
record can assert is the lockfile pin: `rdf-canon 0.15.3`, `oxrdf 0.2.4`, `itertools 0.14`,
`digest 0.10.7`, `sha2 0.10.9`, `thiserror 2.0.18`. Phase 1 below re-establishes upstream
state before anything is committed to.

## 3. The motivation the issue under-states: the bridge is a capability ceiling

The issue frames the bridge as cost (double parse) and hygiene (duplicate pins). There is a
third, sharper reason, and it is a correctness gap rather than a cost:

**Everything crossing the bridge must be expressible in RDF-1.1 N-Quads as `oxttl 0.1`
parses it.** sparq's term model is RDF-1.2. Directional-language literals
(`oxrdf::BaseDirection`) are supported end-to-end elsewhere in the workspace — SPARQL
Results JSON `its:dir` decoding in `sparq-fedclient`, the `sparq-py` term surface, the
core dictionary. Hand `"hello"@en--ltr` to `sparq_canon::canonicalize` today and the
serializer emits the RDF-1.2 token, `oxttl 0.1.8` cannot parse it, and the caller gets a
generic `CanonError::Bridge("oxrdf bridge error: …")`.

That failure is **accidental and undocumented**, unlike the triple-term rejection, which is
deliberate, typed (`CanonError::TripleTerm`), documented and tested. There is no test and
no doc sentence covering it anywhere in the crate. Deleting the bridge fixes it as a side
effect — and that reframes the work as closing a capability gap, not just as tidying.

## 4. What the current design buys, and what option C would spend

The single most valuable property of `sparq-canon` today is easy to lose sight of:

> The canonical form sparq signs, commits to, and content-addresses is produced by a
> **maintained third-party implementation of RDFC-1.0**, and validated against the W3C
> suite through sparq's own public API.

Two downstream claims rest on that and are non-circular *only* because of it:

1. `crates/sparq-canon/README.md`'s "W3C-conformant" claim is a claim about an
   implementation sparq did not write;
2. `sparq-difftest`'s oracle-independence argument (§2.2a) — which is about *another*
   dependency edge, but it is the same reasoning, and it is the workspace's own stated
   standard for when a canonicaliser may be trusted as a cross-check.

The issue's fallback (option C below) spends that property to remove a compile-time
duplicate-version *warning* that will not clear anyway (§2.2a). That trade should be made
consciously, if at all.

## 5. Options

### A — Bump `zkp-ld/rdf-canon` to the oxrdf 0.3 line upstream (issue's primary)

Deletes the bridge outright and keeps the algorithm third-party. Notes:

- oxrdf types appear in `rdf-canon`'s **public signatures** (`canonicalize_quads(&[Quad])`),
  so an oxrdf-line bump is a semver-breaking release for that crate (a `0.16`), not a patch.
  A maintainer may reasonably decline or defer.
- The issue's alternative phrasing — "or make it generic over the term model" — is a much
  larger design change (a term-model trait, or a `Cow`-ish abstraction over four term
  enums) and should not be offered as an equal-effort option in the same PR.
- Latency is upstream's. This option alone leaves sparq blocked for an unbounded time.
- **Not executable by the pass that wrote this record** (no network, no Rust toolchain) —
  but the port, the patch and its validation are executable by any agent that has both, and
  §8 phase 3 schedules them autonomously. Only *publishing* the PR on `zkp-ld/rdf-canon` is
  owner-filed, and that never blocks sparq because B lands the same diff locally.

### B — Fork + `[patch.crates-io]` now, upstream the same diff (recommended pairing with A)

This is what `AGENTS.md` §*upstream* actually prescribes for a blocked-upstream item:
vendor or fork locally, ship it so sparq is unblocked, and open the identical change
upstream. Applied here: fork `rdf-canon` at 0.15.3, port it to `oxrdf 0.3`, `[patch.crates-io]`
it, delete sparq's bridge, and open the port as the draft upstream PR.

- Unblocks sparq immediately; the algorithm stays third-party (property in §4 preserved
  in substance — the code is still not sparq's design, and the W3C suite still validates it).
- Costs a carried fork plus its `SPARQ-PATCHES.md`, and a `deny.toml` `[sources]` allowance
  for the git source (precedent exists: `solid-oidc-verifier` already has one).
- **The issue's "roll-your-own + upstream-PR rule" points at this option, not at option C.**
  A fork of the audited algorithm is a strictly smaller, strictly safer delta than
  substituting a home-grown canonicaliser.

### C — Promote `src/rdf12.rs` to the standard path (issue's stated fallback)

Blockers found by reading the code (§6). Beyond those, it converts the W3C suite from a
*validation* of an independent implementation into the *sole* oracle for sparq's own, on a
hash that VC proofs, ZK commitments and `urn:concept:` identifiers are computed over.
Recommended only if A and B are both refused upstream **and** §6.1/§6.2 are closed first.

### D — Status quo plus documentation

Fix the `deny.toml` registry (§2.2a), document the directional-literal ceiling (§3), and
stop. Cheap and honest; leaves the ceiling in place.

## 6. Blockers found for option C

These are code-read findings. None was executed — the environment has no usable Rust
toolchain (`/usr/local/rustup` is read-only, the pinned `1.97.1` channel is absent). Each
carries the exact command that would confirm it.

### 6.1 Eager permutation materialization defeats the poison-graph guard (live bug)

`permutations()` (`src/rdf12.rs:985`) returns a **fully materialized** `Vec<Vec<String>>`,
and `hash_n_degree_quads` calls it once per related-hash group (`src/rdf12.rs:641`). The
guard, `HndqCallCounter`, counts HNDQ **calls** and is checked only at function entry
(`src/rdf12.rs:582`) — it cannot fire *inside* a call. So a group of `k` co-hashed related
blank nodes allocates `k!` vectors of `k` `String`s before any guard check is reachable.

The suite's own negative vector, `tests/rdf-canon-testdata/rdfc10/test074-in.nq`
(a 10-node clique, `k = 9`), is already six-figure permutations per call; a 13-node clique
— trivially attacker-supplied — is nine-figure and aborts the process on allocation
failure instead of returning `CanonError::Canonicalization`. Upstream `rdf-canon` depends
on `itertools` and iterates permutations lazily, so its guard is reachable.

This contradicts the profile's documented "fails closed" property and is a **live defect in
the shipped opt-in profile**, independent of this issue. Note also that the existing
agreement test cannot catch it: `v2_agrees_with_standard_on_suite_inputs`
(`tests/rdf12_triple_term_canon.rs:50`) `continue`s on any input the standard path
rejects — *before* calling the v2 path — so the negative vector is precisely the one suite
entry the native profile has never been run on.

Confirm with: `cargo test -p sparq-canon --features rdf12-triple-terms` after adding a
case that feeds `test074-in.nq` to `canonicalize_rdf12` under a memory cap (expect: no
`Err`, resource exhaustion instead).

### 6.2 The native path's suite validation is transitive and partial

`v2_agrees_with_standard_on_suite_inputs` compares the native path against the *standard
path's output* over the `-in.nq` files; composed with `tests/rdf_canon_suite.rs` that does
give byte-equality against the W3C expected canonical N-Quads — **for eval tests under
SHA-256 only**. Not covered for the native path: the 21 `RDFC10MapTest` issued-identifier
expectations, the one `RDFC10NegativeEvalTest`, and the SHA-384 `test075` vector against
its own expected output. Promotion requires the manifest-driven runner (all 86 entries)
pointed at the native path, not the current directory walk.

### 6.3 Public-API blast radius

`pub use digest::Digest` is `sparq-canon`'s public trait re-export. Moving it to
`digest 0.11` changes trait identity for every downstream that names a hasher. Separately,
`sha2` is currently `optional = true` and gated behind `rdf12-triple-terms`/`concept`
precisely so the default build stays byte-identical; promoting `rdf12` makes it
unconditional unless the code is restructured, dissolving a property the manifest
currently guarantees in writing.

### 6.4 Surface gaps and tripwires

- No `issue_triples` sibling exists in the `rdf12` surface (`canonicalize_triples_rdf12`
  exists; the issuer-map-for-a-single-graph entry point does not).
- `tests/rdf12_nquads_token_tracking.rs` pins `PINNED_OXRDF_BRIDGE = "0.2.4"` and
  `PINNED_OXTTL_BRIDGE = "0.1.8"`. Any bridge deletion must remove those two constants and
  their assertions — and that file is the natural home for the re-divergence tripwire the
  issue's acceptance criteria ask for.

## 7. Recommendation

**Pursue B (fork + `[patch.crates-io]`) with A (the identical change offered upstream) as
the same diff. Do not take C.** Rationale in one line: C spends the "our canonical form is
computed by a maintained third-party implementation" property — the thing that makes both
the W3C-conformance claim and the workspace's own oracle-independence doctrine
non-circular — to remove a duplicate-version warning that §2.2a shows will not clear.

Two honesty caveats on that recommendation:

1. It assumes the oxrdf 0.2 → 0.3 port of `rdf-canon` is mechanical (term-model types at
   the boundary, not a deep coupling). **That is unverified** — the crate source is neither
   vendored here nor reachable. Phase 1 exists to settle it, and a large port would move
   the balance back toward C or D.
2. §6.1 must be fixed regardless of which option wins. It is a defect in code that ships
   today, not a hypothetical blocker for a future one.

## 8. Phased plan

**Tracking.** This section is a plan, not a tracker. Each phase is queued as a bead with
this record's landing, and the two items that are **defects in shipped code, independent of
this issue** — §6.1 (eager permutation materialization defeats the HNDQ guard) and §2.2a
(`deny.toml`'s duplicate-version blocker registry omits `sparq-difftest`) — are queued
additionally on the `self-improvement` issue lane, per the shared-contract rule that an
out-of-scope discovery becomes a git-native issue rather than an inline fix. Backfill the
concrete bead ids / issue URLs into the phase lines below once minted.

1. **Measure the port.** Fetch `rdf-canon 0.15.3`, port to `oxrdf 0.3`, record the diff
   size and whether any public signature changes beyond term types. Deliverable: a patch
   and a go/no-go for phases 2-4. *Blocks everything else; needs network.*
2. **Fix `permutations` to be lazy** in `src/rdf12.rs`, and run `test074-in.nq` through
   `canonicalize_rdf12` asserting `CanonError::Canonicalization`. Independent of phases
   1/3/4 — this is §6.1, a live bug. *Should land first regardless of the option chosen.*
3. **Prepare the upstream contribution — autonomously — and stop only at publication.**
   Everything up to the filing is ordinary agent work and must not wait on anyone: push the
   phase-1 port to a fork branch, validate it against the W3C suite there, and check the PR
   body into the fork's `SPARQ-PATCHES.md` in `zkp-ld/rdf-canon` house style per
   `AGENTS.md` §upstream — Why-first, @jeswr-tagged, carrying the "not yet ready for
   maintainer review" note. The **only** blocked step is the literal act of opening the PR
   on the third-party repo, an out-of-repo publication `AGENTS.md`'s maintenance loop step 7
   reserves to the owner; raise that one action, and nothing else, on the `needs:user` queue.
   **Default while it is unfiled:** carry the fork + `[patch.crates-io]` (option B)
   indefinitely — sparq is unblocked either way. Record the URL in this file and in
   `crates/sparq-canon/Cargo.toml` once filed.
4. **Land the port locally** via a fork + `[patch.crates-io]` + a `deny.toml` `[sources]`
   allowance; delete `bridge_to_02`, `bridge_triples_to_02`, `parse_02`, `serialize_quads`,
   `serialize_quads_default`, the `bridge-lowcopy` feature, the `oxrdf02`/`oxttl01`
   aliases, and the two `PINNED_*_BRIDGE` constants. `parse_canonical` stays (it already
   uses oxttl 0.2). **Acceptance:** all 86 suite entries byte-identical in both feature
   states, plus a new test asserting a directional-language literal canonicalizes rather
   than erroring (§3).
5. **Lift `digest`/`sha2` to 0.11** in `sparq-canon` once the fork does, sweeping
   `sparq-zk`/`sparq-vc`/`sparq-wasm`. Separate bead: it is a public-API change.
6. **Extend the native-vs-standard agreement test to the full manifest** (map + negative +
   SHA-384) — §6.2. Doubles as the re-divergence tripwire and as the evidence base if C
   ever has to be reconsidered.
7. **Correct `deny.toml`'s duplicate-version blocker registry** to name all three holders
   of `oxrdf 0.2` (§2.2a).
8. **Offer the RDF-1.2 triple-term extension upstream** as a follow-up feature — already
   drafted in `research/rdf-canon-rdf12-triple-terms-upstream.md` and still awaiting
   @jeswr's review. Do not re-draft it.

Phases 2, 6 and 7 are independent of the upstream decision and can start immediately.

## 9. Open questions for the maintainer

1. **Fork-and-patch, or wait?** Phase 4 carries a forked crate until upstream releases. The
   repo has precedent (`spargebra`, `solid-oidc-verifier`) but each fork is standing cost.
2. **If upstream prefers "generic over the term model"**, does sparq fund that larger
   design, or keep the fork and let upstream take its own path?
3. **Is "the standard path is a third-party implementation" a property to preserve?** This
   record argues yes and recommends accordingly (§4, §7); it is ultimately a call about how
   much independent validation sparq's signing/commitment hashes need.
4. **`digest` 0.11 timing** — phase 5 breaks `sparq-canon`'s public API. Bundle it with
   phase 4 (one break) or sequence it separately (two smaller reviews)?
