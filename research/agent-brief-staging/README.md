# Agent-brief staging — maintainer-applied edits to `.claude/agents/*.md`

> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## Why this directory exists

`.claude/agents/*.md` (the role-agent configs) is a **PROTECTED** surface: an agent may not
rewrite its own or a sibling's operating rules (AGENTS.md rule 11 — self-modification is
blocked by design). So when a durable change genuinely needs a role config edited, the agent
**stages the delta here** and the **maintainer applies it** into `.claude/agents/`. This mirrors
the prior convention used for the Fable collaboration tier (`research/fable-collab-infra/agents/`,
since removed once applied in #1319). This is a reusable staging area — future staged deltas may
be appended, applied, and pruned the same way.

Apply flow: copy the insert block below into the named agent config, then either delete the
applied entry from this file or leave it as an applied-record. No agent edits `.claude/agents/`
directly.

## Pending: inject the "Review lessons" checklist pointer into the reviewer + verifier briefs (2026-07-06)

**Source of truth** for the nine distilled rules is `AGENTS.md` § *Review lessons — checkable
rules distilled from caught defects* (added in this same PR). The briefs must NOT duplicate all
nine rules (single-source-of-truth hygiene) — they point to that section and emphasise the subset
most load-bearing for the role. The rules were distilled from real verdict-gate catches on
2026-07-06: #1647, #1650, #1651, #1652, #1653, #1672, #1676, #1679.

### 1. `.claude/agents/sparq-reviewer.md` — append to the "honesty & soundness rules you enforce" list

The deep verdict-giver owns the soundness-judgment rules. Append these items (numbered to
continue the existing list) after rule 5 ("Scope honest."):

```markdown
6. **Effect-evidence, not config-shape (AGENTS.md § *Review lessons*).** Never accept "green + configured looks right." A test that compiled under `--all-features` but never ran, ran in a non-required job, or exercised nothing the engine consumed is VACUOUS — open the live check-run log and confirm the assertions executed and the enclosing JOB feeds `ci-summary` (#1672 / #1650 / #1679).
7. **Oracle strength + fail-closed branches.** A differential/conformance oracle must compare term structure and full answer SETS, never row COUNTS (a count-equal oracle passes a shared-blank-node cartesian product #1653). Every multi-branch operator (UNION/OPTIONAL/FILTER) must fail CLOSED, with an oracle case where exactly one branch is permissive (#1647).
8. **Graduation evidence + docs-honesty.** A ratchet floor moves only with per-CASE oracle evidence — never a force-pinned floor (#1653). A doc that asserts a soundness/correctness property IS a soundness surface: "Feature X supports Y" must trace to a passing test on the REAL path, else it is a false claim, not a nit (#1651).
9. **Reachability-seeding.** A reachability-pruned / orphan-dropping validator must not seed its reach set from anything the validated data controls (declaration-typing lets an orphan re-enter and bypass the check #1652); confirm the seed set is closed over TRUSTED roots only.
```

### 2. `.claude/agents/sparq-verify-mechanical.md` — extend the objective checklist

The cheap mechanical gate owns the objective/evidence rules. Add these checklist items (7–9,
continuing the existing six) to "The objective checklist":

```markdown
7. **Effect-evidence (not config-shape).** For any NEW test/leg in the diff, confirm from the LIVE check-run log (`gh run view <id> --log`) that it actually EXECUTED and the enclosing JOB is in the required set — a leg that compiles but never runs, or runs in an advisory job, is vacuous. A benchmark/"coverage" step whose output the engine never consumes is also vacuous → **FAIL** (#1672 / #1650 / #1679).
8. **Feature-leg pairing.** A new cargo feature must ship all three: (a) its own CI feature-matrix leg, (b) an `LC_ALL=C`-sorted golden-fixture line, (c) an assemble/self-test that goes RED when the leg or line is missing. Compiling under `--all-features` is NOT execution. Missing any → **FAIL** (#1672).
9. **Advisory-vs-gating + lane isolation.** "Does it gate?" keys on the JOB name, not the step name (`ci-summary` excludes `\b(advisory|informational)\b` by job) — a load-bearing step in an advisory-named job does NOT gate → **FAIL/ESCALATE** (#1679). A per-PR lane and a nightly/heavy lane must be proven disjoint by running the selector (`--list`/`testMatch`) in BOTH env-flag states and diffing (unfiltered `testMatch` leaks nightly specs into the per-PR lane #1676).
```

## Maintainer note

After applying, the pointers keep AGENTS.md § *Review lessons* as the single source of truth; the
briefs only carry the role-scoped emphasis above. If the section is later reworded, the briefs need
no re-edit (they reference it by name). Capture any follow-up as a bead, not a TODO here.
