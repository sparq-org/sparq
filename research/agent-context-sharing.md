# Context sharing between fleet agents — three channels, bounded by construction [OPUS-5]

**Status:** design record, for maintainer review. Requested 2026-07-26: *"design a strategy for agents to
be able to appropriately share context between related tasks; e.g. an agent that is working on a PR may
[benefit] from some of the context from the agent that created the issue. We should also be careful to make
sure that this doesn't result in continually increasing context being given across agents. Perhaps we
should be looking at having some kind of 'memory' that all agents maintain and contribute to — like we have
with claude code in the command line."*

## 0. Grounding rules (binding on every claim below)

1. **This project has already measured most plausible context tricks, and rejected two of three.**
   `pkg-query` halved Opus tokens (N=30) → adopted. `ast-grep`/outline-first cost **~21k tokens MORE**
   than a scoped read → rejected. `sparq-terse` saved **~0%** real tokens (N=30) → rejected as anything
   but a legibility convenience. **Prior:** a context idea that sounds good is more likely than not to be
   worthless or negative. Nothing here ships unmeasured.
2. No hard-coded performance numbers in prose (repo hygiene). Figures below are dated observations.
3. Where this record states a mechanism exists today, it was checked against the tree on 2026-07-26.

## 1. The problem, measured rather than assumed

Fleet agents are **ephemeral GitHub Actions jobs**. Each starts cold: a repo checkout, an issue body, a
routed model. Nothing an earlier agent learned survives, except what happens to be written into the issue
or PR.

Observed consequences on 2026-07-26:

- **Worker yield 70 success / 126 failure (36%)** over 196 runs, dominated by `EXIT_CLASS: no_change` —
  the model runs and produces no diff. Every `no_change` target sampled was a genuinely hard,
  well-specified task (lock-free saturation, morsel pull-pipeline, NAF-aware counting).
- **The same issue is retried up to 3×** (#3241 attempt 2/3, #2575 attempt 3/3) **with the same model and
  no record of why the previous attempt produced nothing.** Attempt 2 begins exactly as ignorant as
  attempt 1. This is the single clearest context failure in the system: the information needed
  ("I tried X, the blocker was Y") existed and was discarded.
- **Review deadlocks**: 3 rounds of `request_changes`, or a fixer twice declaring the findings spurious,
  then a park. The disagreement itself — the most decision-relevant artifact produced — is preserved only
  as prose scattered across comments.
- **Defect classes recur across unrelated PRs**: vacuous tests, the workflow `if:`/step seam,
  exit-zero-swallows-failure (8 instances), tests passing for the wrong reason, the *headline* guard being
  the untested one (2 instances in one day). Each PR rediscovers these from scratch, and each is caught
  only by an expensive adversarial reviewer.

The last point is the economic argument: we are paying frontier-tier review tokens, repeatedly, to
re-derive lessons the fleet has already learned.

## 2. The core distinction: three channels, never conflated

Most "shared agent memory" designs fail by putting everything in one growing store. The decisive move is
to separate three kinds of information by **lifetime and derivability**, and give each a different
mechanism, budget, and eviction rule.

| Channel | Contains | Lifetime | Growth | Retrieval |
|---|---|---|---|---|
| **A. Lineage handoff** | what the previous stage of *this task* learned | one task | **bounded by schema** | pushed, always |
| **B. Method memory** | transferable lessons about *this codebase* | long | **asymptotic** | pulled, on demand |
| **C. Live state** | heads, labels, gate status, counts | seconds | **never stored** | queried, never cached |

**Channel C is a rule, not a store: never memorise what can be queried.** A head SHA, a label set, a park
count, a mergeability status — all change under you. This session produced two concrete harms from
violating it: a `VERDICT: pass` bound to a superseded head nearly armed a PR whose fix had been retracted,
and a park-cause finding recorded hours earlier ("all 33 carry `LOOPSTOP`") was false by the time it was
reused — the current parks carried an entirely different cause. **Anything derivable in one API call must
be derived, not remembered.** This single rule removes the largest source of both staleness and growth.

## 3. Channel A — lineage handoff (the maintainer's example case)

*"An agent working on a PR may benefit from context from the agent that created the issue."*

### 3.1 Shape

A single structured block, the **task ledger**, carried with the task and consumed by the next stage.
Every stage **owns exactly one slot** and **replaces** it — never appends.

```text
issue.created   → { intent, acceptance, known_constraints, decomposition_notes }
worker.attempt  → { approach_tried, blocker, files_touched, why_no_diff?, confidence }
review.round    → { findings[], severity, what_would_change_my_mind }
fix.attempt     → { addressed[], disputed[] + reason, residual }
adjudication    → { decision, rationale }
```

**Replace-not-append is the whole bounding story for this channel.** If round 3 carried rounds 1–2
verbatim, context would grow linearly in rounds — precisely the failure mode the maintainer is worried
about. Instead round N's slot holds round N's outcome plus a **distilled residue** of what remains
unresolved. The ledger is therefore O(stages), not O(attempts), and stages are fixed.

### 3.2 Hard limits

- A per-slot byte cap, enforced at write time; over-cap writes are **rejected**, not truncated (silent
  truncation would drop the tail, which is where residuals live).
- The whole ledger has a total cap. If the cap binds, the *oldest non-open* slot is dropped first, and the
  drop is recorded — a silently shrinking ledger is worse than a visibly bounded one.
- Slots are **structured fields, not prose**. Prose is what grows; fields are what compose.

### 3.3 Where it lives

The substrate already exists: the fleet writes structured HTML-comment markers on issues and PRs today
(e.g. `<!-- sparq-fix-nochange:v1 round=1 run=… -->`). The ledger is the same idea, promoted to a schema
with a version tag and a single writer per slot. No new infrastructure; it inherits GitHub's durability,
auditability, and the maintainer's ability to read it.

### 3.4 The highest-value single entry

`worker.attempt.why_no_diff`. At a 36% yield with 3 identical retries, the fleet's largest waste is
re-asking a question it has already answered. Requiring a departing worker to state *why it produced
nothing* — underspecified? blocked on a missing decision? too large for one session? judged the task
already done? — converts a silent failure into a routing signal, and directly feeds the
escalate-vs-decompose decision (registry #701) and the deadlock adjudication (registry #703).

## 4. Channel B — method memory (the "like Claude Code" part)

### 4.1 What qualifies — the admission test

A candidate memory must pass **both**:

1. **Non-derivable.** It cannot be recovered by reading the repo or making one API call. *"`ready-issues.py`
   requires a positive `status:ready`"* fails this test — read the file. *"GitHub Actions prints each
   `run:` block's source into the log prefixed with ANSI `[36;1m`, so grepping matches branches that never
   executed"* passes: it is invisible in the tree and caused three wrong diagnoses.
2. **Decision-changing.** It would have altered a concrete action. If nothing would have been done
   differently, it is a note, not a memory.

Everything else — architecture, conventions, how a crate works — belongs in `AGENTS.md`, a `SKILL.md`, or
a crate README, which is repo hygiene policy already.

### 4.2 Why this asymptotes rather than grows

This is the direct answer to *"we should be careful this doesn't result in continually increasing
context."*

**Method lessons about a codebase are a bounded set; state facts are unbounded.** There are only so many
ways this repo's CI can mislead you, only so many recurring defect classes, only so many traps in the
merge machinery. Restrict admission to non-derivable method knowledge and the corpus grows fast at first
and then flattens — new tasks mostly *re-encounter* known lessons rather than generating new ones.

This is a **falsifiable prediction, and it is the primary health metric**: plot corpus size against
cumulative tasks. Linear growth means the admission test is too loose and is admitting state. If that
happens, tighten admission — do not add storage.

Observed support, weak but real: this session generated ~7 new method memories across many hours of dense
work, while the same defect classes (vacuous tests, YAML seam, exit-zero, quantifier direction) recurred
repeatedly against an existing corpus of ~90.

### 4.3 Retrieval — pull, never push

**Do not inject the corpus into every agent.** That is the failure the maintainer named, and it is also
what the measurements predict: bulk context injection is exactly the shape that made outline-first cost
21k tokens *more* than a scoped read.

Instead, mirroring what works for the CLI orchestrator today:

- A **one-line index** per memory — cheap to scan in full.
- **Lazy load**: the agent pulls the two or three whose one-liners match the task at hand.
- A **hard per-agent retrieval budget**. When it binds, the agent gets the highest-ranked entries and is
  *told the budget bound* (so it knows its view is partial rather than complete).
- Retrieval via the **cheap-model NL tool**, measured here at ~30× cheaper than doing the round-trip on
  the expensive tier, with `pkg-query` measured to halve Opus tokens.

### 4.4 Staleness and retraction — the part usually omitted

Memories are point-in-time. Two mechanisms, both learned the hard way:

- **Every entry carries a last-confirmed date, and any entry naming a file, symbol, or flag must be
  re-verified against the tree before it is asserted.** An unverified entry may inform a hypothesis; it
  may not ground a claim.
- **Retraction must be as cheap as writing.** On 2026-07-26 a wrong normative claim (XPath numeric
  promotion) was relayed as authoritative, written into a `SKILL.md` and an issue, and flagged
  "do-not-fix". A false entry in a durable surface **outlives the PR that introduced it** and is trusted
  later by someone with less context. So: entries are individually addressable, deletable without
  ceremony, and carry provenance (who, when, from what evidence) so a wrong one can be traced to its
  source and swept.

**Confidence must be recorded, and disagreement preserved rather than flattened.** A deadlocked review is
information; collapsing it to a single "the answer is X" is how a wrong claim becomes durable.

### 4.5 Multi-writer discipline

The maintainer's phrasing — *"a memory that all agents maintain and contribute to"* — raises the hard
part: with many writers and no curator, quality decays toward noise.

- **Write on exit, not during.** An agent proposes memories in its final report; it does not mutate the
  shared store mid-task. This keeps writes reviewable and prevents a confused agent from poisoning peers
  mid-flight.
- **Proposals are candidates, not commits.** Admission runs as a periodic curation pass (cheap tier)
  applying §4.1, deduplicating against existing entries, and *merging into* a related entry rather than
  minting a near-duplicate. Near-duplicates are the main growth pathology in multi-writer stores.
- **A memory contradicting an existing one is an event, not an overwrite** — it surfaces for adjudication,
  because a contradiction usually means one of them is wrong or the codebase changed.

## 5. Fit with existing infrastructure — reuse, do not rebuild

- The **PKG** (`crates/sparq-kb`, `research/dogfooding-sparq-knowledge-graph.md`) already provides a
  queryable, provenance-and-confidence-carrying store with a measured token win, and dogfoods sparq
  itself. Channel B should be **PKG-backed** rather than a parallel store; the `sparq-pkg-nl` cheap-model
  NL tool is the retrieval path.
- **Foundational-ontology work already measured schema.org-as-top as the best agent-KM accuracy**, driven
  by LLM fluency with the vocabulary. Reuse that result rather than minting a bespoke schema.
- **Channel A needs no new storage** — it is a schema over the marker comments the fleet already writes.
- The neurosymbolic-KB direction (#1111) is the strategic parent; this record is the fleet-facing,
  bounded slice of it.

## 6. Rollout — measurement-gated, cheapest-first

Each step ships only if it moves a **fleet metric**, not if it feels better. Primary metric: **worker
yield** (currently 36%). Secondary: review rounds to converge; parks created per day; tokens per merged PR.

1. **`why_no_diff` on worker exit** (Channel A, one field). Smallest possible change, targets the largest
   measured waste, and is independently useful to #701/#703 even if nothing else ships.
2. **Retry consumes it**: attempt N+1 receives attempt N's slot; escalate-or-decompose instead of
   re-running identically. A/B against yield.
3. **Review/fix slots** — carry the disagreement into adjudication (#703).
4. **Channel B read-path** over the existing PKG, with a hard retrieval budget, seeded from the
   orchestrator's existing curated corpus. A/B on review rounds and on recurrence of known defect classes.
5. **Channel B write-path** — agent-proposed candidates plus a curation pass, only once the read-path has
   demonstrated value. Writing before reading pays off is how stores fill with noise.

**Stop conditions, stated in advance:** if step 2 does not improve yield, the hypothesis "workers fail for
want of prior context" is wrong and the effort should move to decomposition instead. If the corpus grows
linearly with task count, admission is too loose. Both are cheap to check and neither should be argued
away after the fact.

## 7. What this deliberately does not do

- **No shared mutable scratchpad between concurrently-running agents.** Cross-talk between agents working
  different tasks creates non-reproducibility and a poisoning path; the ledger is per-task and
  append-controlled.
- **No automatic summarisation of one agent's transcript into another's prompt.** Transcripts are the
  wrong unit (huge, unreliable, and this project's policy is not to read agent logs). Structured fields
  written deliberately at exit beat summarised logs.
- **No unbounded "just give the next agent everything."** That is the failure mode named in the request,
  and the measured evidence says bulk context loses.
