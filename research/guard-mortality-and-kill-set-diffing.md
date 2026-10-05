# Guard mortality, and detecting it by diffing the mutation kill set (sq-1lc4i)

**Status:** design record — a test-quality failure class and the detection method for it.
Authored under the proceed-and-document rule.
**Author:** Claude Opus 5, 2026-07-28. [OPUS-5]
**Scope:** the Python orchestration/CI scripts and their YAML seams, where the evidence was
gathered. The mechanism is not Rust-specific, but every measurement below is from that
surface. Companion: `research/test-quality-program-plan.md` (the per-crate cargo-mutants
ratchet — a different surface and a different mechanism; this record does not restate it).

---

## 1. The finding

> **A guard can pass while protecting nothing: the unit under test still succeeds, the
> production path is unprotected, and line coverage cannot see the difference — because
> the guard genuinely executes.**

That is the claim this record is built on, and it is the one every instance below earns.
Coverage is blind by construction (the assertion runs), and review is blind because the
defect is an *absence* — a line that is not there, a call that is not made, a name that is
not re-exported.

A guard reaches that state whenever its effectiveness is *coupled* to the thing it guards.
Four couplings are attested here:

| coupling | how the guard dies |
|---|---|
| **Textual nesting** — assertion lives inside the feature's region | removed with the region |
| **Call-site indirection** — every assertion invokes the helper *directly*, never through the production path | the one line wiring the helper into production is deletable, and nothing objects |
| **Wrong-module binding** — the guard reads a symbol from the module that *defines* it, not the module the consumer *loads* | the guard passes while the consumer sees nothing |
| **Fixture-injected contract** — the test fixture *adds* the contract to its own copy of the subject before asserting on it | the fixture satisfies a contract the real target does not |

Only the first is *mortality* in the literal sense — a guard that existed, worked, and was
killed by an unrelated edit. The other three never protected the production path at all.
The record is titled for the mortality case because that is the one with a clean detection
method (§2.1); **the honest generalisation is the weaker, broader claim above**, and the
strongest evidence for the class being real is that it needs **three different detectors**
(§2.1 deletion-diff, §2.4 symmetric rename, §2.5 field capability) — §2.4 exists precisely
because §2.1 is blind to §3.3.

The last two couplings are the nastiest, because the guard is not merely absent — it is
*actively reporting success* about a contract nothing upstream honours.

## 2. The detection method

### 2.1 Diff the kill set across a deletion

Run the mutation battery **before** and **after** a removal, and compare per-mutant status.

> **A mutant that moves `KILLED` → `SURVIVED` when a feature is *removed* is the
> signature of guard mortality. Nothing else produces that transition.**

A mutant flipping to `SURVIVED` means the assertion that used to catch it is gone. If the
deletion was supposed to remove only the feature, then the deletion also took a guard with
it — one that was protecting something else, otherwise the mutant would have been removed
from the spec along with its target.

Distinguish the two legitimate outcomes so the signature stays sharp:

- mutant's **target** was deleted → the anchor no longer exists → the mutant leaves the
  spec entirely. This is expected and is *not* the signature.
- mutant's target **still exists** but the mutant now survives → **guard mortality.**

### 2.2 The corollary that makes it necessary

> **A green suite after a deletion proves nothing about what the deletion took with it.**

This is why review and CI both miss the class. The suite no longer *contains* the assertion
that would have complained, so its greenness is vacuous with respect to the removal. Asking
"did the tests pass after I deleted this?" is the wrong question; the right one is "which
assertions did I stop running?" — and only a kill-set diff answers it.

### 2.3 Method trap: the results artefact must be append-only

A results file named after the first run is silently **overwritten** by a re-run against the
same path, so the pre-fix state is destroyed exactly when it becomes the evidence. During
this work `results-reg-1031.json` was re-used for a post-fix run; the earlier `SURVIVED`
verdict survived only in the append-only per-run log, and the overwrite was provable only
from file mtimes. **Write each run to its own path, and cite the per-run log, not a mutable
results path.**

### 2.4 The symmetric rename, for contracts crossing a module or repo boundary

Kill-set diffing catches guards that *disappear*. It does **not** catch a guard that is
bound to the wrong side of a contract — for that the mutant must be a **symmetric rename**:

> **Rename the symbol in the production module *and* in its own tests, together. If nothing
> reds, no assertion is checking the name the CONSUMER resolves — only that the definition
> and the test agree with each other.**

A one-sided rename is useless here: it reds trivially because the test still names the old
symbol. The symmetric version is the one that exposes the coupling, and it is exactly the
mutant that survived in §3.3. Where the consumer is in another repository, or loads the
module dynamically, this is the only cheap way to test the contract at all.

The corresponding practice: **assert the symbol through the consumer's own resolution path**
(the same `getattr` / import the consumer performs), and never let a fixture supply the
contract it is testing for (§3.3).

### 2.5 Before tuning a threshold, ask whether the field can express the quantity

A guard can also be vacuous because the *variable* is incapable of carrying the signal, not
because the assertion is weak. This one is worth a named check because the failure looks
exactly like "we have not seen a positive yet", which invites an indefinite search for a
better threshold instead of a look at the data source.

The tell is a distribution that is **too clean to be a measurement**:

- an interval that is **exactly** zero across a very large N — not near-zero, identically
  zero — is what two fields set *together* look like, not two events being timed;
- an interval that is **negative**, at all, for a quantity that cannot be negative.

MEASURED while cutting the queue-wait detector (§3.1's PR): `run_started_at − created_at`
was **exactly 0.0 on all 44,123** attempt-1 runs across two repositories, and
`run_started_at − attempt.created_at` was **negative on all 67** re-run attempts
(`{−1: 48, −2: 17, −4: 1, −5: 1}`). A queue wait cannot be negative, so those fields are
stamped together at start rather than spanning enqueue-to-start. **No threshold on them
could ever have worked**, and the all-zero corpus was never evidence that no queueing
happened — only that the fields do not report queueing.

The practical consequence is for the *exit condition* you write down. "Ship it when we
observe a positive" is wrong guidance if the field cannot produce one; it sends the next
reader to re-derive a detector that cannot exist. Say instead what *different data source*
would be required. Getting this backwards is itself a guard that reports success about
something it cannot see.

### 2.6 Cheap approximations when a full battery is too slow

- After any deletion, ask of each removed assertion: *what was this protecting, and does
  that thing still exist?*
- Grep the removed diff hunk for assertion keywords (`assert`, `chk(`, `check(`, `expect`)
  before committing a removal.
- Structurally, prefer that a guard live **outside** the region it protects (§4).
- Eyeball the distribution before tuning a threshold on it (§2.5).

## 3. Instances

Three instances, same class, one working session, all in Python orchestration scripts with
hermetic self-tests plus a separate unittest suite.

⚠️ **§3.1 and §3.2 share a subject** — both concern the same registry watchdog, and §3.1's
lost assertion is the `fetch_lanes` → `created_at` guard while §3.2's is the adjacent
`fetch_live_runs` → enrichment call site. They are distinct *couplings*, not distinct
subsystems. Treat them as two failure modes observed on one component, which is weaker
evidence of generality than three independent components would be.

**Evidence for §3.1 and §3.2 is the registry merge commit `93dac564`**, which records both
transitions verbatim and is permanent and in-tree. Earlier drafts of this record cited
per-run battery logs from an agent scratchpad and a task transcript; those paths exist in
neither repository, and agents are instructed never to read task transcripts. A record
whose evidence cannot be opened is not evidence-bound — the citation is the evidence.

### 3.1 Textual nesting — the sharpest case

**Where:** `scripts/ci-latency-alert.py` (jeswr/agent-account-registry#1031).

A `fetch_lanes` assertion — that each lane carries the `created_at` the new-lane guard
reads — had been appended *inside* the queue-wait mode's self-test region. When that whole
mode was cut, the assertion went with it.

| tree state | `fetch_lanes drops created_at` |
|---|---|
| after the queue-wait mode was cut | **SURVIVED** |
| after the guard was restored standalone | **KILLED** |

Recorded in `93dac564`: *"`fetch_lanes drops created_at` went from KILLED back to
SURVIVED, which would have left the new-lane guard permanently unreachable in production"*.

Consequence had it shipped: the new-lane guard would have been **permanently unreachable in
production** — the field it reads would never be populated — while the self-test stayed
green, because the self-test exercised the guard against a hand-built fixture rather than
against the fetch path.

**Fix:** restore the assertion outside any mode's region, so no single feature removal can
take it.

### 3.2 Call-site indirection — inside the fix for the same class

**Where:** `scripts/ci-latency-alert.py` / `scripts/ci_execution_latency_alarm.py`.

An enrichment pass was added to correct a detector that read a stale timestamp. Every
assertion called the enrichment helper **directly**. The single line wiring it into the
production fetch path was therefore deletable with the suite green.

| tree state | enrichment call site deleted |
|---|---|
| before a call-site test existed | **SURVIVED** |
| after a test driving the real fetch path was added | **KILLED** |

Recorded in `93dac564`: *"`fetch_live_runs` -> `resolve_attempt_created`. Deleting it
survived everything, and would have shipped M2 still reading the frozen attempt-1
timestamp — the exact defect this PR exists to fix."*

Notable because this occurred **inside the code written to fix call-site vacuity**. Knowing
about the class is not protection against it; only the kill-set diff caught it.

**Fix:** a test that drives the production entry point against a stubbed transport and
asserts both that the enrichment happened *and* that the downstream detector changed
behaviour because of it.

### 3.3 Wrong-module binding, plus a fixture-injected contract

**Where:** sparq-org/sparq#4823 with jeswr/agent-account-registry#1032 — the dispatch
inertness-proof pair.

⚠️ **Every state claim below is anchored to a SHA, because the first version of this
section was not, and was stale on arrival — see §3.4.** The defect described here existed
**at `8058b0e6f`** (and identically at `ca84a9cf1`); it was **fixed at `5dda060fd`**. This
section describes the pre-fix tree, in the past tense, deliberately.

The consumer is the registry's `dispatch.yml`, which loads **sparq's**
`scripts/dispatch-plan.py` as the target planner and probes it with `getattr` for an
inertness contract, falling back to *"planner declares no inertness contract"*. Note the
filename is ambiguous across the two repos: the registry has its own
`scripts/dispatch-plan.py`, which is only its self-test harness — the module that matters
is sparq's.

`INERT_FIELD` and `MACHINE_PARK_PR_LABEL` were both added to sparq's
`scripts/ready-issues.py`. **At `8058b0e6f`, neither appeared in
`scripts/dispatch-plan.py`** — the module the consumer loads — so the probe failed on the
first `getattr` and the pair was inert as shipped. At `5dda060fd` both were re-exported
there (`INERT_FIELD = _ready.INERT_FIELD` and the same for the label), which is the fix.

*(An earlier draft also stated how many other names that re-export block forwarded. Three
parties computed that count three different ways — the block forwards names from two
different source modules, and the answer differs by which you include. The number was
decorative: the load-bearing fact is that the two PROBED symbols were absent. It has been
removed rather than adjudicated, which is the right treatment for any figure whose
precision is not doing work.)*

Two independent green signals failed to notice, one per repo, by different mechanisms:

- **sparq's vacuity battery reported all of its mutants dead**, but a *symmetric rename*
  (§2.4) of `INERT_FIELD` in production and self-test together **survives** it — every
  assertion references the symbol, so none can observe whether the consumer resolves it.
- **the registry's battery was green because its fixture planners inject the contract into
  the fixture's own source** before asserting on it. The fixture therefore satisfies a
  contract the real target does not — the fixture-injected row in §1.

**Fix shape:** re-export the names from the module the consumer loads, and assert them
through the consumer's own resolution path rather than the definer's.

**On the two frontier numbers**, which look contradictory and are not: the authors' `1 → 9`
measures the *lever* — the effect if the attestation is carried — while an independent
reviewer's replay of the merged state measured no movement, and movement only after adding
the missing re-export. Both are real; they quantify different things. The defect is that
the PR title asserts the lever figure as the delivered effect. *(The numeric replays are
the reviewer's, not this record's — see §6.)*

### 3.4 This record itself — the fourth instance

The first version of this record was reviewed and **failed**, because §3.3 described a tree
that had already been fixed. The timeline, all 2026-07-28:

| time (UTC) | event |
|---|---|
| 15:20 | the wrong-module binding is reported on #4823 |
| 15:32 | `5dda060fd` pushed — re-exports both symbols on the module the registry loads |
| 15:42 | that head live on GitHub |
| **15:44** | **this record committed, asserting the pair was "open and uncorrected"** |

Four present-tense claims were false against the live head twelve minutes before the commit.
The fix at `5dda060fd` was authored under **this record's own author identity**.

This is the same class. §3.3 was a guard — an assertion about the state of a system — whose
correctness was coupled to a tree that moved underneath it, and nothing in the record's own
process re-checked it. The specific defect is precisely what §2.3 warns about in the small
(cite the artefact, not a mutable path) generalised to the large: **a state claim with no
SHA is a claim about whichever tree the reader happens to have.**

Two things follow, and they are the most transferable content in this record:

1. **Anchor every state claim to a SHA.** Not "as of writing" — the SHA. A record about
   readings taken from the wrong tree must say which tree, or it cannot be checked at all.
   This is now done throughout §3.3.
2. **Verification has a shelf life measured against a moving branch.** §6 originally claimed
   verification "at the two PR head SHAs" *without naming them*, which made the staleness
   invisible to a future reader — the check looked rigorous and was unfalsifiable.

Recorded rather than quietly corrected, because a record that documented this class while
concealing its own instance of it would be the strongest possible evidence against its own
method.

## 4. Practice

1. **Place a guard outside the region it protects.** If an assertion protects behaviour X,
   it must not live inside the block that implements X. A separate, plainly-named test
   class or self-test section survives a feature removal; a nested assertion does not.
2. **Diff the kill set across every deletion** that removes a region containing assertions.
   Report the transitions, not just the final totals — `20/20 killed` on the post-deletion
   tree says nothing about what left the spec.
3. **Use a symmetric rename for any contract crossing a module or repo boundary** (§2.4),
   and never let a fixture inject the contract it is asserting on.
4. **A mutant must target the production entry point**, not only the helper. If every
   assertion calls the helper directly, add one that goes through the real call site.
5. **Bind guards to the consumer's view.** Assert a symbol through the module that loads it.
6. **Write each battery run to its own path** and cite per-run logs (§2.3).
7. **Check the field can express the quantity before tuning a threshold on it** (§2.5), and
   write the exit condition in terms of the data source, not a future observation.
8. **A crash is not a kill.** A mutant counts as killed only on a *named* assertion failure;
   a traceback certifies nothing, because it reads the same whether the guarantee broke or a
   test stub broke. Anything else should be reported separately as crash-only.

## 5. Limits — what this record does not claim

- **Not a claim about frequency.** Three instances in one session on one surface is enough
  to name the class, not to estimate a rate. No sampling was done.
- **Not Rust-validated.** Every measurement is from Python orchestration scripts with
  hand-written mutation batteries. Whether the per-crate cargo-mutants ratchet exhibits the
  same class is untested; the ratchet reports a surviving *count*, which by construction
  cannot show a per-mutant transition, so detecting this class there would need per-mutant
  status diffing that the current gate does not emit.
- **Not a proposal to gate on it.** No CI enforcement is proposed here. Kill-set diffing is
  a manual practice for deletions; automating it needs a stable per-mutant identity across
  runs, which the current tooling does not provide.
- **⚠️ The one existing tool that looks like it covers this is structurally blind to it,
  and reports green while being so.** `scripts/mutants-diff.sh` runs
  `cargo mutants --in-diff "$DIFF_FILE"`, i.e. it only generates mutants for code the PR
  **changed**. The signature in §2.1 is a mutant in **unchanged** code flipping to
  `SURVIVED` because a deletion elsewhere removed its guard — by construction that mutant
  is outside `--in-diff`'s scope, so the advisory lane cannot enumerate it and emits a
  clean summary regardless. This is a sharper argument than "the ratchet counts survivors":
  the deletion case is exactly the one a changed-code-only mutation lane cannot see, and
  its greenness on such a PR carries no information about the class.
- **The detection method has a blind spot**: it only fires for guards whose target survives
  the deletion. A guard protecting something that is *also* being removed is correctly
  silent — and a guard removed while its target is quietly kept elsewhere would need the
  target's own mutant to still be in the spec to show up.

## 6. Verification status

Evidence grades differ per instance and are stated rather than blurred.

- **§3.1 and §3.2 — measured here.** Both `KILLED` → `SURVIVED` → `KILLED` transitions are
  reproducible from the named per-run logs.
- **§3.3 — structurally verified, at named SHAs, and initially recorded STALE.** The
  mechanism was checked directly against file contents: absent at **`8058b0e6f`** and
  **`ca84a9cf1`**, present at **`5dda060fd`** (`scripts/dispatch-plan.py` lines 63–64), with
  the consumer's probe read from the registry's `dispatch.yml`. The *frontier numbers* are
  an independent reviewer's replay, reproduced as attribution, not as this record's own
  measurement.

  The first version of this section asserted verification "at the two PR head SHAs"
  **without naming either**, and asserted a present-tense state that a fix twelve minutes
  earlier had already invalidated. Both defects are recorded as §3.4 rather than silently
  repaired.

**The general rule this record now follows: every state claim names the SHA it was measured
at.** A verification that does not is unfalsifiable — it looks rigorous and cannot be
checked, which is strictly worse than an unverified claim honestly labelled. The repeated
failure this record exists to prevent is a claim that outlived its evidence, and an unnamed
SHA is how a claim outlives its evidence without anyone noticing.
