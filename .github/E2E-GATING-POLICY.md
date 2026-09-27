<!-- [OPUS-4.8] sq-ymr2e.12 — SPARQ agent. Operational gating/probation policy for the
     deterministic site + GUI Playwright lanes. Design of record:
     research/web-gui-test-program.md §6.3 (advisory-first, promotion earned). -->

# Web + GUI E2E — gating, probation & flake-quarantine policy

> 🤖 SPARQ agent policy record. This is the **operational runbook** for promoting the
> deterministic site (`site/e2e/`) and GUI (`gui/e2e-playwright/`) Playwright lanes from
> **advisory** to **required**. The *design* rationale lives in
> [`research/web-gui-test-program.md` §6.3](../research/web-gui-test-program.md); this file
> is the checked-in policy the workflows link to, and the **probation-evidence ledger**.

## 1. The one rule everything follows

`ci-summary / gate` (`.github/workflows/ci-summary.yml`) is the **single** required
branch-protection context. It aggregates every other check-run on the head commit and
**excludes only the check names DECLARED in
[`.github/advisory-registry.json`](advisory-registry.json)**. A lane's gating status is
therefore decided by whether it is **declared**, not by what it is called:

- an entry in the registry → the aggregator ignores it → **non-gating** (in probation).
- no entry → the aggregator waits on it → **gating** (promoted, or never demoted).

> **Corrected 2026-07-25 (#3773).** Until then the aggregator inferred advisory status from
> the job **NAME** matching `\b(advisory|informational)\b`. That silently neutralised **four
> real gates** — including the determinism grep-gates that §1.1 of the design and §4 below
> both called "hard", and which in fact gated nothing — because any job whose name happened
> to contain those words was dropped from the gating set wholesale, with no waiver and no
> record. The name token is now **diagnostic only**: a check carrying it with no registry
> entry GATES, and the gate summary prints a loud note naming it. A **rename can no longer
> flip gating status** either: each declaration binds to the job's stable identity (workflow
> file + job id), and `scripts/check-advisory-registry.py` C4 REDs when a job's current name
> stops matching its declaration — while the renamed job GATES, fail-closed.

Branch protection is therefore **never edited directly** to promote a lane. Promotion is a
registry edit (§4). Adding a raw required context is forbidden.

## 2. Scope — the promotion-set lanes

| Lane (what it proves) | Workflow · job | Today |
|---|---|---|
| Site foundation smoke (determinism harness + hermetic network, real browser) | `site-e2e-foundation.yml` · `foundation smoke (advisory)` | **advisory** |
| Home hero-runner journeys (5 P1 journeys + stress) | `site-e2e-hero.yml` · `home hero-runner journeys (advisory)` | **advisory** |
| GUI mocked-IPC desktop journeys (headless Chromium, `retries=0`) | `gui.yml` · `gui-mock-ipc` | **gating** — see §6 |
| Visual key-layouts (container-pinned snapshots) | `site-visual.yml` · `visual key layouts (container-pinned, advisory)` | **advisory** (promotes **last**, §5) |

**Already gating — NOT in the promotion set** (#3773 restored these; they are hermetic or
ratcheted, and none of them is declared in the registry):

| Check | Workflow · job | Why it gates |
|---|---|---|
| Site determinism grep-gate (zero `page.waitForTimeout` under `site/e2e/`) | `site-e2e-foundation.yml` · `determinism-gate` | pure grep, ~15 s, no npm install and no browser |
| Axe WCAG 2.1 AA scan + `bench/a11y-baseline.json` ratchet | `site-e2e-foundation.yml` · `a11y-ratchet` | ratchet over axe's own rule output; non-vacuous by construction (the suite's last test injects an unlabelled icon-button and asserts axe catches it) |
| #1740 `browserName` regression guard + gui/e2e `no-sleep-gate` | `gui.yml` · `gui-mock-ipc` (first two steps) | find/grep over checked-in source; no webview, no driver, no toolchain |

> These two guards had a standalone `gui-hermetic-guards` job between #3773 and #5691; that job
> was checkout + two greps, so its wall-time was ~entirely runner claim, and #5691 folded it into
> the gating `gui-mock-ipc` lane — chosen because it gates, shares gui.yml's trigger and path
> filter, and already hosts the analogous hermetic grep for its own harness. Note the guards
> cover the *sibling* `gui/e2e` tauri-driver harness, which `gui-mock-ipc` does not itself run;
> they run before its toolchain/node setup, so a violation still reds within seconds. **If
> `gui-mock-ipc` is ever
> demoted per §4, hoist the two guards back out into a hermetic gating job — do not give them a
> `gate_script_waiver`,** or #3773's defect (a hard guard silently riding inside an advisory job)
> comes straight back. `check-advisory-registry.py` C3 classifies both scripts as gate-classified,
> so that demotion REDs until it is resolved one way or the other.

**Never promotable** (documented so nobody wires them by accident):

- `gui.yml` · `tauri-e2e` (`GUI tauri-driver e2e (Linux, advisory)`) — WebKitWebDriver +
  `xvfb` + a real native webview. Environment-coupled by nature → **advisory forever**
  (design §5.3). Its stabilisation/flake-probe is tracked by `sq-ymr2e.6`, not here.
- `gui.yml` · `tauri-e2e-probe` — a `workflow_dispatch` flake probe; never a PR check.
- The nightly full-sweep lane (`sq-ymr2e.11`: cross-browser, full-surface axe/visual,
  the per-platform tauri matrix) — **nightlies never gate** `ci-summary`.

> The per-platform Tauri **build + clippy** matrix (`gui.yml` · `tauri-build`) is a separate
> question governed by its own bead (`sq-var9`): it is a compile lane, not an e2e lane, and
> may earn gating once the macOS/Windows rows are proven — out of scope for *this* policy.

## 3. The probation bar (identical for every promotable lane)

A lane may be promoted only once it has, **on `main`**, accumulated:

> **50 consecutive green runs spanning ≥ 10 distinct PRs, OR two weeks — whichever is
> LONGER**, with **zero** quarantine events (§7) inside that window.

"Whichever is LONGER" is deliberate: 50 greens in three days is not enough soak, and two
green weeks with only two PRs is not enough surface coverage. Both floors must clear. The
evidence (a link to the run history / the ledger row) goes in the **promotion PR body**.

## 4. Promotion = delete the declaration (the runbook)

To promote a lane, **delete its entry from `.github/advisory-registry.json`**. Concretely,
per lane:

1. Delete the lane's entry from the registry (that is the load-bearing change — it takes the
   job out of the aggregator's exclusion set). Optionally drop the now-meaningless
   `(advisory)` token from the job `name:` in the same PR; if you do, the entry must go in
   the same commit or C4/C2 in `scripts/check-advisory-registry.py` REDs.
2. Remove the `continue-on-error: true` on the browser/exec step(s) so a real failure turns
   the job red.
3. Record the evidence in the §8 ledger and cite it in the PR body.

> **#3773 correction.** The previous wording of step 2 said "*the determinism grep-gates are
> already hard; leave them*". That was **false**: those grep-gates lived inside
> advisory-named jobs, so the aggregator excluded them and they gated nothing. They now run
> in their own undeclared (gating) jobs — see the second table in §2 — and step 2 no longer
> claims anything about them.

Nothing else changes — no branch-protection edit, no new required context. Deleting the
declaration alone moves the lane from "reported but ignored" to "waited on by
`ci-summary / gate`". To **demote** (if a promoted lane starts flaking), do the reverse: add
an entry back with an `owner_bead` + `promotion_criteria`, so an unstable gate is never left
blocking the train while it is fixed — and, unlike the old name flip, the demotion is a
reviewable diff in one file that carries its own justification.

## 5. Promotion sequence

- **Group A — functional E2E + a11y + the GUI mocked-IPC lane** promote **first**, together,
  each on its own §3 evidence. These assert *behavioural contracts* (states, values, hrefs,
  ARIA, invoke shapes) via roles/testids that survive restyling — the non-brittle half.
- **Visual key-layouts promote SEPARATELY and LAST.** Pixel snapshots are the most
  brittleness-prone lane (font rasterising, antialiasing, masked dynamic regions), so the
  visual subset must earn its own §3 window *after* the functional lanes are stable, never
  bundled with them.

## 6. `gui-mock-ipc` early promotion — RATIFIED 2026-07-06 (#1656)

The `gui-mock-ipc` job carries **no registry declaration and no `continue-on-error`**, so it
**gates** `ci-summary`. It was promoted at creation (`sq-ymr2e.5`, PR #1431) on the rationale
that it is a fully deterministic headless-Chromium lane (`retries=0`, mocked IPC). That
promotion **pre-dated this governance** and did not sit on recorded §3 probation evidence,
which the architect's plan of record (design §6.3 "everything lands advisory"; the
`sq-ymr2e.12` note classifying `gui-mock-ipc` as *in the promotion set*) says it should.

**Decision (issue #1656, ratified 2026-07-06):** the early promotion is **ratified** — the
lane stays gating. Demoting a green, deterministic (`retries=0`, mocked IPC) gate on an
actively-developed surface would reduce real enforcement, so the early promotion is blessed
rather than reset under §3. The alternative (**reset** to advisory + `continue-on-error` and
re-earn gating under §3) was considered and declined for that reason. Its §8 ledger row is
backfilled to *gating — ratified 2026-07-06 (#1656)*. The decision is also recorded in the
repo decision ledger, [`docs/decisions/README.md`](../docs/decisions/README.md).

**Rollback (if the lane later flakes):** apply §4 in reverse — add a
`.github/advisory-registry.json` entry for `gui.yml` · `gui-mock-ipc` (with `owner_bead` +
`promotion_criteria` + `job_id`) **and** `continue-on-error: true` on the `Run Playwright
mocked-IPC tests` step in `gui.yml`; the lane then re-earns gating uniformly under §3. No
branch-protection edit is involved either way (§1). Since #3773 a name edit alone does
nothing — the declaration is what demotes.

## 7. Flake-quarantine policy (codified beside the suites)

A gate that is tolerated when flaky trains contributors to "re-run until green" and erodes
every other gate's authority. So:

- **A test that passes-on-retry twice within a 7-day window is QUARANTINED the same day**
  (`test.fixme(...)` / `test.skip(...)` with a comment linking the bead) and a **P2 fix bead
  is filed same-day**. A quarantined test cannot gate (it does not run in the gating set) and
  must be fixed or deleted, never left skipped indefinitely.
- **CI retry regime is diagnostic, not a safety net:**
  - Site lanes (`site/playwright.config.ts`): `retries: 1` in CI with `trace: on-first-retry`.
    A pass-on-retry is a **defect to fix**, not a success — the trace is the evidence.
  - GUI mocked-IPC lane (`gui/e2e-playwright/playwright.config.ts`): `retries: 0` with
    `trace: retain-on-failure` — **stricter**: a flake is an immediate hard failure, so the
    quarantine trigger there is "fails then passes on a re-push", handled the same way.
  Do not raise `retries` to hide a flake; that inverts the policy.
- **Anti-flake acceptance bar (pre-merge, before a spec is even advisory):** every new spec
  passes `--repeat-each=5 --retries=0` locally (`npm run test:e2e:stress` for the site suite).

## 8. Probation-evidence ledger

Evidence accumulates on `main`. Update the row on each green run once collection begins; a
lane is promotable only when its row clears **both** §3 floors with zero §7 quarantine events.

| Lane | Window opened | Consecutive green (on main) | Distinct PRs | Quarantine events | Promotable? | Evidence |
|---|---|---|---|---|---|---|
| site foundation smoke (`site-e2e-foundation` · `foundation-smoke`) | not yet opened | 0 — accumulating | 0 | 0 | **No** | — |
| home hero-runner (`site-e2e-hero`) | not yet opened | 0 — accumulating | 0 | 0 | **No** | — |
| GUI mocked-IPC (`gui-mock-ipc`) | n/a (gating from creation, §6) | not tracked pre-governance | — | 0 known | **Gating — ratified 2026-07-06** | #1656 (ratified, §6); rollback = §4 reverse flip |
| visual key-layouts (`site-visual`) | not yet opened | 0 — accumulating | 0 | 0 | **No** (promotes last, §5) | — |

> The window is "opened" the first green run after the lane's spec set is considered stable;
> record the date + first run URL then. Counts reset to 0 on any red run or quarantine event
> inside the window (the "consecutive" and "zero quarantine" requirements are strict).

## 9. See also

- Design of record: [`research/web-gui-test-program.md`](../research/web-gui-test-program.md) §6.3.
- Determinism doctrine + the shared harness: [`site/e2e/support/README.md`](../site/e2e/support/README.md).
- The GUI mocked-IPC suite: [`gui/e2e-playwright/README.md`](../gui/e2e-playwright/README.md).
- The aggregator semantics: [`.github/workflows/ci-summary.yml`](workflows/ci-summary.yml)
  (header) + `scripts/ci_summary_gate.py`.
- The declared-advisory registry + its integrity checks (C2/C3/C4):
  [`.github/advisory-registry.json`](advisory-registry.json) +
  `scripts/check-advisory-registry.py`.
