<!-- [OPUS-5] #3767 — advisory-disposition record for the npm graph (the tracked
     repo-root `package-lock.json`). Authored by Opus 5. NON-CANONICAL timing; no
     measured performance numbers baked here. The fenced block under
     "Machine-readable record" is parsed by scripts/check-npm-advisory-record.py —
     edit the block and the prose together. -->

# npm graph — advisory disposition (repo-root `package-lock.json`)

> 🤖 SPARQ agent. This records the disposition of the Dependabot **npm** advisories that
> repeatedly conclude `security_update_not_possible` on this repo, and — unlike a
> `dependabot.yml` `ignore:` entry — it suppresses **nothing**. The alerts stay open, the
> GitHub-managed `Dependabot` check keeps reporting, and the check below REDs the moment
> the lock moves off the state recorded here. Tracking issue: **#3767**.

## Why this file exists (and why not the VEX)

`supply-chain/vex.cdx.json` is held in **set equality** with `deny.toml [advisories].ignore`
by the GATING `scripts/check-vex-deny-drift.py`. That pair is structurally a **cargo/RustSec**
artifact — every id in it is a crate advisory. Putting an npm GHSA there would red that gate
as unjustified drift. So the npm graph needs its own record, and this file is it, following
the markdown precedent set by [`supply-chain/gui-tauri-advisories.md`](./gui-tauri-advisories.md)
for the excluded Tauri workspace.

**Honest scope caveat.** There is no in-repo npm vulnerability *gate*. `cargo deny check
advisories` (inside the `supply-chain gates (deny + vet + SBOM + VEX + OpenSSF + js-sbom)`
job) covers the cargo graph only, and the `js-sbom` step *generates* CycloneDX SBOMs without
failing on advisories. Dependabot alerts remain the sole npm surveillance surface. This file
does not change that; it makes the *disposition* checkable, not the *advisory feed*.

## The findings

### `brace-expansion` — Dependabot alerts #25, #26, #27, #46

The lock carries **four independent instances**, each pinned by a different `minimatch`
generation. Every row below is asserted against the live lock by the check:

| lock path | version | pinned by | range |
|---|---|---|---|
| `node_modules/brace-expansion` | 1.1.15 | `node_modules/minimatch` 3.1.5 | `^1.1.7` |
| `node_modules/glob/node_modules/brace-expansion` | 2.1.1 | `node_modules/glob/node_modules/minimatch` 9.0.9 | `^2.0.2` |
| `node_modules/readdir-glob/node_modules/brace-expansion` | 2.1.1 | `node_modules/readdir-glob/node_modules/minimatch` 5.1.9 | `^2.0.1` |
| `node_modules/@typescript-eslint/typescript-estree/node_modules/brace-expansion` | 5.0.6 | `node_modules/@typescript-eslint/typescript-estree/node_modules/minimatch` 10.2.5 | `^5.0.5` |

Per the Dependabot job output quoted in #3767 (run `30136987253`, 2026-07-25 — **not**
re-verified offline here), the advisory set includes an entry whose affected range is
`<= 5.0.7`, so the only unaffected release is `5.0.8`. From `^1.1.7` / `^2.0.x` a resolver
reaches at most the 1.x / 2.x tips, which are still in range — hence
`security_update_not_possible`. Dependabot will not move the `minimatch` *dependents*, so it
can never clear these four alerts on its own.

### `postcss` — Dependabot alert #45

One hoisted instance, and the pin is **ours, not a dependent's**:

| lock path | version | pinned by |
|---|---|---|
| `node_modules/postcss` | 8.5.15 | root `package.json` `overrides.postcss` = `8.5.15` |

This corrects the reading in #3767, which attributed the block to `next` / `@tailwindcss/postcss`
pins. Those pins exist (`next@15.5.21` requires `postcss` exactly `8.4.31`;
`@tailwindcss/postcss@4.3.1` requires exactly `8.5.15`) — but the lock resolves a **single**
hoisted `postcss@8.5.15` with **no nested copy under `next`**, which is only possible because
the root `overrides` already force it past `next`'s exact pin. That override was added for
exactly this reason (`86dc0e94`, "bump next/postcss 8.4.31->8.5.15"); the sibling precedent
for the technique is `b978324f`, "remediate Dependabot npm devDep advisories via root
overrides".

So the effective blocker for `postcss` is a **one-line override bump**, not an upstream wall.
It is not taken in this change because the bump requires regenerating `package-lock.json`
(new `resolved` URL + `integrity` hash) and re-verifying the `site` / `gui/app` Next.js
builds — neither is possible in an environment without `npm`, and a hand-edited lock would be
unverifiable. `site/package.json` also declares a looser `postcss` override (`>=8.5.10`); a
future bump should reconcile both declarations in one PR.

### `sharp` — Dependabot alert #32

Same failure class per #3767 (three `security_update_not_possible` runs on 2026-07-22). One
instance, reached as an **optional** dependency:

| lock path | version | pinned by |
|---|---|---|
| `node_modules/sharp` | 0.34.5 | `node_modules/next` 15.5.21, `optionalDependencies.sharp` = `^0.34.3` |

**Weaker claim than the two above.** Alert #32's current state and patched version were not
re-verified when this record was written (no network), and `^0.34.3` is a *minor* range, so
unlike `brace-expansion` a patched `0.34.x` may well be reachable — this may already be
resolved. It is recorded so the tripwire fires when the instance moves; do not read its
presence here as a claim that it is still blocked.

## Do NOT add `dependabot.yml` `ignore:` entries

An `ignore:` entry for these packages would suppress the **future patch notification** too.
The alerts are deliberately left open and noisy-but-honest. The GitHub-managed `Dependabot`
check-run is non-gating via the exact, fail-closed allow-list in `scripts/ci_summary_gate.py`
(`PLATFORM_MANAGED_ADVISORY_NAMES`) — that is a *gate-classification* decision about a check
this repo cannot fix, and it suppresses no alert and no scanner.

## The monitor

`scripts/check-npm-advisory-record.py` (GATING, wired into `supply-chain.yml`) asserts the
machine-readable record below against the live `package-lock.json`:

- the set of lock paths for each recorded package is **exactly** the recorded set — a new
  fifth `brace-expansion` copy, or a removed one, REDs;
- each instance resolves at the recorded version;
- each recorded pin still holds — for a `package` pin, the pinning entry exists at the
  recorded version and still declares the recorded range **in the recorded manifest field**
  (`dependencies` by default, `optionalDependencies` for `sharp`); for a `root_override`
  pin, the root `overrides` entry still carries the recorded value.

It reads only checked-in files (`package-lock.json`, `package.json`, this record) and has no
network and no advisory feed, so it can tell you the picture is **stale** — never that a
package is **vulnerable**.

**If this check REDs, that is usually the good news.** It means the graph moved — most likely
a patch became reachable and Dependabot opened its PR. The response is to re-check
`gh api repos/sparq-org/sparq/dependabot/alerts?state=open`, update or delete the entry, and
close #3767 — *not* to relax the check.

To close the finding sooner rather than waiting on Dependabot: move the dependents. For
`brace-expansion` that means bumping/deduping the packages pinning `minimatch@3` / `@5` so
`>= 5.0.8` becomes resolvable — note a root `overrides` force from `^1.1.7` to `5.0.8` is a
**cross-major** jump that would need the lint lanes actually re-run, not just a green lock.
For `postcss` it is the override bump described above. Verify either with a clean `npm ci`
plus `npm ls <package>`.

## Machine-readable record

<!-- npm-advisory-record:begin -->
```json
{
  "lock": "package-lock.json",
  "tracking_issue": 3767,
  "packages": [
    {
      "name": "brace-expansion",
      "dependabot_alerts": [25, 26, 27, 46],
      "instances": [
        {
          "path": "node_modules/brace-expansion",
          "version": "1.1.15",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/minimatch",
            "version": "3.1.5",
            "range": "^1.1.7"
          }
        },
        {
          "path": "node_modules/glob/node_modules/brace-expansion",
          "version": "2.1.1",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/glob/node_modules/minimatch",
            "version": "9.0.9",
            "range": "^2.0.2"
          }
        },
        {
          "path": "node_modules/readdir-glob/node_modules/brace-expansion",
          "version": "2.1.1",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/readdir-glob/node_modules/minimatch",
            "version": "5.1.9",
            "range": "^2.0.1"
          }
        },
        {
          "path": "node_modules/@typescript-eslint/typescript-estree/node_modules/brace-expansion",
          "version": "5.0.6",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/@typescript-eslint/typescript-estree/node_modules/minimatch",
            "version": "10.2.5",
            "range": "^5.0.5"
          }
        }
      ]
    },
    {
      "name": "postcss",
      "dependabot_alerts": [45],
      "instances": [
        {
          "path": "node_modules/postcss",
          "version": "8.5.15",
          "pinned_by": {
            "kind": "root_override",
            "key": "postcss",
            "value": "8.5.15"
          }
        }
      ]
    },
    {
      "name": "sharp",
      "dependabot_alerts": [32],
      "instances": [
        {
          "path": "node_modules/sharp",
          "version": "0.34.5",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/next",
            "version": "15.5.21",
            "field": "optionalDependencies",
            "range": "^0.34.3"
          }
        }
      ]
    }
  ]
}
```
<!-- npm-advisory-record:end -->
