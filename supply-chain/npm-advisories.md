<!-- #3767 — advisory-disposition record for the npm graph (the tracked
     repo-root `package-lock.json`). No measured performance numbers baked here. The fenced block under
     "Machine-readable record" is parsed by scripts/check-npm-advisory-record.py —
     edit the block and the prose together. -->

# npm graph — advisory disposition (repo-root `package-lock.json`)

> 🤖 SPARQ agent. This records the disposition of the **npm** advisories that have no
> non-breaking fix reachable from this repo's lock, and — unlike a
> `dependabot.yml` `ignore:` entry — it suppresses **nothing**. The alerts stay open, the
> GitHub-managed `Dependabot` check keeps reporting, and the check below REDs the moment
> the lock moves off the state recorded here. Historical tracking: **#3767** (closed).

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

### Resolved (2026-10-08)

The earlier entries are closed. `brace-expansion` (alerts #25/#26/#27/#46) moved to
patched 1.1.21 / 2.1.7 / 5.0.12 with #6676. `postcss` (#45) moved to 8.5.29 through the
root override, and `sharp` (#32) to 0.35.5 under next 15.5.27, both in #6692.

### Open, dev-only: no non-breaking fix upstream

Each remaining instance is reached only from lint or end-to-end test tooling, never from a
published package or a built site/GUI bundle. Every row is asserted against the live lock:

| package | lock path | version | pinned by | advisories |
|---|---|---|---|---|
| `braces` | `node_modules/braces` | 3.0.3 | `node_modules/micromatch` 4.0.8 (`^3.0.3`), via `fast-glob` ← `@next/eslint-plugin-next` ← `eslint-config-next` | GHSA-vfj7-8cjw-p6xm |
| `extract-zip` | `node_modules/extract-zip` | 2.0.1 | `node_modules/@puppeteer/browsers` 2.13.2 (`^2.0.1`), via `webdriverio` 9 (GUI e2e) | GHSA-jmr9-qjv8-65gv, GHSA-7pqw-9j4j-h8q3 |
| `basic-ftp` | `node_modules/basic-ftp` | 5.3.1 | `node_modules/get-uri` 6.0.5 (`^5.0.2`), via `proxy-agent` ← `@puppeteer/browsers` | GHSA-c475-qrg2-pj4r |

`braces` 3.0.3 and `extract-zip` 2.0.1 are the latest releases, so no resolver move can fix
them. `basic-ftp` is fixed only in 6.x, outside `get-uri`'s `^5` range; the upstream route
is webdriverio 10 (a major bump of the GUI e2e harness).

## Do NOT add `dependabot.yml` `ignore:` entries

An `ignore:` entry for these packages would suppress the **future patch notification** too.
The alerts are deliberately left open and noisy-but-honest. The GitHub-managed `Dependabot`
check-run is not a required check (the only one is `ci-fast`), so it blocks nothing — and
leaving it red suppresses no alert and no scanner.

## The monitor

`scripts/check-npm-advisory-record.py` (GATING, wired into `supply-chain.yml`) asserts the
machine-readable record below against the live `package-lock.json`:

- the set of lock paths for each recorded package is **exactly** the recorded set — a new
  second copy of a recorded package, or a removed one, REDs;
- each instance resolves at the recorded version;
- each recorded pin still holds — for a `package` pin, the pinning entry exists at the
  recorded version and still declares the recorded range **in the recorded manifest field**
  (`dependencies` by default); for a `root_override`
  pin, the root `overrides` entry still carries the recorded value.

It reads only checked-in files (`package-lock.json`, `package.json`, this record) and has no
network and no advisory feed, so it can tell you the picture is **stale** — never that a
package is **vulnerable**.

**If this check REDs, that is usually the good news.** It means the graph moved — most likely
a patch became reachable and Dependabot opened its PR. The response is to re-check
`gh api repos/sparq-org/sparq/dependabot/alerts?state=open`, update or delete the entry, and
close the finding — *not* to relax the check.

## Machine-readable record

<!-- npm-advisory-record:begin -->
```json
{
  "lock": "package-lock.json",
  "tracking_issue": 3767,
  "packages": [
    {
      "name": "braces",
      "advisories": [
        "GHSA-vfj7-8cjw-p6xm"
      ],
      "instances": [
        {
          "path": "node_modules/braces",
          "version": "3.0.3",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/micromatch",
            "version": "4.0.8",
            "range": "^3.0.3"
          }
        }
      ]
    },
    {
      "name": "extract-zip",
      "advisories": [
        "GHSA-jmr9-qjv8-65gv",
        "GHSA-7pqw-9j4j-h8q3"
      ],
      "instances": [
        {
          "path": "node_modules/extract-zip",
          "version": "2.0.1",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/@puppeteer/browsers",
            "version": "2.13.2",
            "range": "^2.0.1"
          }
        }
      ]
    },
    {
      "name": "basic-ftp",
      "advisories": [
        "GHSA-c475-qrg2-pj4r"
      ],
      "instances": [
        {
          "path": "node_modules/basic-ftp",
          "version": "5.3.1",
          "pinned_by": {
            "kind": "package",
            "path": "node_modules/get-uri",
            "version": "6.0.5",
            "range": "^5.0.2"
          }
        }
      ]
    }
  ]
}
```
<!-- npm-advisory-record:end -->
