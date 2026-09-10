# Issue 6480: shared Next.js security patch

Actual author: GPT-6 Astra. Exact head `50c09eb2b7563924cccde9305ee51ff62b27af38`, parent `781f667c19a8ebb779cfccb24b05ea432360b025`. Three files, +44/-44. Application source, Next configuration and workflows are unchanged.

This implements the completed narrow security intake. Root revalidated the publisher advisories immediately before implementation: alert 107 is AVIF / GHSA-2xp9-vwfh-vxw4; alert 106 is Windows / GHSA-p293-qw3h-jr36. Both identify 15.5.24 as the first patched release on the existing line. This patch makes no production-exposure or exploitation claim.

## Change and provenance

Both workspace minimums move from `^15.5.21` to `^15.5.24`. The shared lock updates only `next`, `@next/env`, eight optional SWC binaries, and the two workspace records. All 1,126 package entries remain; 1,114 are identical, with no additions or removals. All four Linux `libc` selectors remain.

Next 15.5.24's published `sharp` range expands to `^0.34.3 || ^0.35.3`; locked `sharp@0.34.5` is unchanged. React/DOM 19.2.7, eslint-config-next/plugin 15.5.19, and root PostCSS overrides remain unchanged. Both `next.config.ts` files are byte-identical to the base.

npm 11.17.0 was downloaded from its official registry version endpoint and verified against the published SHA-512 before bounded extraction of regular files and directories. It ran through bundled Node 24.19.0, which satisfies npm's `^20.17.0 || >=22.9.0` requirement. This avoids npm 10 lock churn documented by existing issue 6133 without mixing in npm/workflow pinning. No npm-latest or global installation was used.

```sh
node <verified-npm-11.17.0>/bin/npm-cli.js install next@15.5.24 --workspace=site --workspace=gui/app --save-prefix='^' --package-lock-only --ignore-scripts --no-audit --no-fund
# Also supplied: task-only cache/config paths, official registry, fetch-retries=0,
# and a bounded fetch timeout. The same command ran once with --offline afterward.
node <verified-npm-11.17.0>/bin/npm-cli.js ls next --workspace=site --workspace=gui/app --package-lock-only --json --offline
```

All three commands exited 0. The offline repeat left all three files byte-identical. The virtual lock tree reports Next 15.5.24 under both workspaces; this is not an installed-tree result. No node_modules, lifecycle execution, actual install, Next/Wasm/Playwright build, or benchmark occurred.

The ten updated packages' tarball URLs and integrity values match official version metadata retrieved by npm and then independently read from its integrity-checked response cache. Published Next engines and React/DOM peers accept the configured versions. This is semver evidence, not Node 22 runtime proof. The npm tarball matches published SRI and SHA-1; the registry signing-key signature was not independently verified.

Author preflight exited 1 solely because the existing privacy checker requires `mapfile`, absent from installed Bash 3. Other invoked mechanical checks and `git diff --check` passed. In-memory metadata controls confirm the exact expected-lock comparison rejects an old Next version, a missing SWC platform, and a missing Linux selector. These are metadata checks, not application tests.

Task output/cache stayed below the authorized 256 MiB allowance, with free disk above 2 GiB. Exact command receipts, source hashes, downloaded tool/archive and cache hashes are retained in the local manifest.

## Exact full diff

```diff
diff --git a/gui/app/package.json b/gui/app/package.json
index 5181c1b5a..448dff68e 100644
--- a/gui/app/package.json
+++ b/gui/app/package.json
@@ -25,7 +25,7 @@
     "cmdk": "^1.1.1",
     "fzstd": "^0.1.1",
     "lucide-react": "^0.460.0",
-    "next": "^15.5.21",
+    "next": "^15.5.24",
     "next-themes": "^0.4.6",
     "radix-ui": "^1.5.0",
     "react": "^19.0.0",
diff --git a/package-lock.json b/package-lock.json
index 136679098..67ced45a6 100644
--- a/package-lock.json
+++ b/package-lock.json
@@ -31,7 +31,7 @@
         "cmdk": "^1.1.1",
         "fzstd": "^0.1.1",
         "lucide-react": "^0.460.0",
-        "next": "^15.5.21",
+        "next": "^15.5.24",
         "next-themes": "^0.4.6",
         "radix-ui": "^1.5.0",
         "react": "^19.0.0",
@@ -1795,9 +1795,9 @@
       }
     },
     "node_modules/@next/env": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/env/-/env-15.5.21.tgz",
-      "integrity": "sha512-hjJI/GfrjWHgNguRIBzItjRRu0m3Nrz17GhxsjuHfjIvg9hyg3239REd2dpI+bpMTFuVrVprHzEQ19m++cDtbw==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/env/-/env-15.5.24.tgz",
+      "integrity": "sha512-mBDF7T0XKZjs9SpUAl0buizVO+O02ULjOvWX8o/AZo/5AGw/UAS1Zzcylmd4pqbftzmKQi+L/nB4jgBYKEAl5Q==",
       "license": "MIT"
     },
     "node_modules/@next/eslint-plugin-next": {
@@ -1811,9 +1811,9 @@
       }
     },
     "node_modules/@next/swc-darwin-arm64": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-darwin-arm64/-/swc-darwin-arm64-15.5.21.tgz",
-      "integrity": "sha512-ZfjqPEdi6TRC/fWx7UDbwb1fbVgyh2uD5tVTRKIDZDlYM+UNuE/LafDG2fwuAoZilADpABh46OY/F5qf9JjqLQ==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-darwin-arm64/-/swc-darwin-arm64-15.5.24.tgz",
+      "integrity": "sha512-AGdNLvxZNY6eR2iSnV+6wUa8CiHTMr4F7g3uHH7fT4ICIJBE00R9u4tzN/Vuwsw0cOi8MTD2HJcTCb6siMH88Q==",
       "cpu": [
         "arm64"
       ],
@@ -1827,9 +1827,9 @@
       }
     },
     "node_modules/@next/swc-darwin-x64": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-darwin-x64/-/swc-darwin-x64-15.5.21.tgz",
-      "integrity": "sha512-TlCf1NpxgQLzTrexuev75xwmNCJMd1/qkJpTVP1GRRcih93hlIBn1P72hkh8T0gnRFr6BmWksQtbyG3jT6jnww==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-darwin-x64/-/swc-darwin-x64-15.5.24.tgz",
+      "integrity": "sha512-9HrQajBMmGcrrrvDfRimiCrbAPh3E6uHJmwBovYr6Yrmi9p9PZqI876BrXX280wICh3o2XwUlp4blkB0NNBqFg==",
       "cpu": [
         "x64"
       ],
@@ -1843,9 +1843,9 @@
       }
     },
     "node_modules/@next/swc-linux-arm64-gnu": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-linux-arm64-gnu/-/swc-linux-arm64-gnu-15.5.21.tgz",
-      "integrity": "sha512-LXRsq1p+HvHSi7ygwNcSEEcK0zuo5jS75ZlqFHtOH+LF7qntXAJVJxah+1Pi/GyBm7EpkwU7m4EgbvIKrMqm9A==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-linux-arm64-gnu/-/swc-linux-arm64-gnu-15.5.24.tgz",
+      "integrity": "sha512-rl9LSfE75si0WT3cDgdUC1XYCKS+TgxC+/IjitmeycrAG18X/plIP1/vy8dd/HPycYcIvE688PD7FuvEAiEAew==",
       "cpu": [
         "arm64"
       ],
@@ -1862,9 +1862,9 @@
       }
     },
     "node_modules/@next/swc-linux-arm64-musl": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-linux-arm64-musl/-/swc-linux-arm64-musl-15.5.21.tgz",
-      "integrity": "sha512-hyGixhFxpDKjqoev6l4KlcRBlt9AXWrGhDZwmwg49sMJM5tnKQPSi+SEj9+e5n+l/bthRGZUdh59GKIs6lQPRw==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-linux-arm64-musl/-/swc-linux-arm64-musl-15.5.24.tgz",
+      "integrity": "sha512-TlNAnpsjxSF3aAUtqnfmtXXf8m9sIDBlmF3c7bTAlnshUYu2U0OxN2uf5d0gcFwqHVEdivJNBcCaqNOwPGNimw==",
       "cpu": [
         "arm64"
       ],
@@ -1881,9 +1881,9 @@
       }
     },
     "node_modules/@next/swc-linux-x64-gnu": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-linux-x64-gnu/-/swc-linux-x64-gnu-15.5.21.tgz",
-      "integrity": "sha512-qfE+YfOba6S2+13e8qn1/UozDVNZ2clBlrs8UtDoax4s8ediu6sq93z66OEHUYlb69Tffh5JTNkgtsAKiSuugg==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-linux-x64-gnu/-/swc-linux-x64-gnu-15.5.24.tgz",
+      "integrity": "sha512-7dwtlhr0SLndqTG1z9ncRkbJswDZiKWlxzFyXDvJ2RDZRDRHp8zyMJ4D9UH/FgnQeXxxB6gZy2pMcIUoNKQ4pA==",
       "cpu": [
         "x64"
       ],
@@ -1900,9 +1900,9 @@
       }
     },
     "node_modules/@next/swc-linux-x64-musl": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-linux-x64-musl/-/swc-linux-x64-musl-15.5.21.tgz",
-      "integrity": "sha512-BXLGG+EvIwp/Rrgl6HY8sqvD6BOUOIRz8/naDbeLNX7mlA5H2XRcL6MW/0IGnJISfj5BA9gNhFyJj5yOoiIDJQ==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-linux-x64-musl/-/swc-linux-x64-musl-15.5.24.tgz",
+      "integrity": "sha512-kGZxM+WhkYs0276lFrMkj7PRtXT3Btp6cwvfSO/cCVLxJJttB5Ccnl2niaCgUja8HgSbEVnMHpg3FJWoOJ9e/g==",
       "cpu": [
         "x64"
       ],
@@ -1919,9 +1919,9 @@
       }
     },
     "node_modules/@next/swc-win32-arm64-msvc": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-win32-arm64-msvc/-/swc-win32-arm64-msvc-15.5.21.tgz",
-      "integrity": "sha512-tNGNOlT0Wn7E4IMsSnufjXN/l2L2/AGdLLpa2vzS89SYCBuihgLn3ngLsIrvndAnWo9nAkus+4gZHTI/Ijx9HA==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-win32-arm64-msvc/-/swc-win32-arm64-msvc-15.5.24.tgz",
+      "integrity": "sha512-jBDDkZ/qKAqkWivWDMkJSXUzbzV0QKRBKJjEHUAvSB97Hzw7NLzJ6yV56Lts/wjir7s4P31GYgpbS6ZL+hasAA==",
       "cpu": [
         "arm64"
       ],
@@ -1935,9 +1935,9 @@
       }
     },
     "node_modules/@next/swc-win32-x64-msvc": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/@next/swc-win32-x64-msvc/-/swc-win32-x64-msvc-15.5.21.tgz",
-      "integrity": "sha512-DmIdWmC9p4rdNIiQqo8ap0+Cnj6kKtTZnuSCxoYydSc8sgpDgAg9wFhxplunak9imLV0pTvc5WVCOHwm5eHLtQ==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/@next/swc-win32-x64-msvc/-/swc-win32-x64-msvc-15.5.24.tgz",
+      "integrity": "sha512-JqtwjvvorjacQ0spgjmUJoxySoYgPwdT1sFdQ0/zmW4iMlP2hjYlCoJIyS7o6Epb4Fug8eco3HXFoBDUCDeH7Q==",
       "cpu": [
         "x64"
       ],
@@ -11485,12 +11485,12 @@
       }
     },
     "node_modules/next": {
-      "version": "15.5.21",
-      "resolved": "https://registry.npmjs.org/next/-/next-15.5.21.tgz",
-      "integrity": "sha512-/TsdBtkWLhkl+NVL3Uqws2UphNd6IPzOtzSk1fHaf+0P7GQKLZDUytyhns/Ykbzdy9+YRjwG7ONvrHaaTDdFqQ==",
+      "version": "15.5.24",
+      "resolved": "https://registry.npmjs.org/next/-/next-15.5.24.tgz",
+      "integrity": "sha512-Y+xn8EQCoC3ZbsFPyzE+tE8XOdrWeUdUF7NeXbmg9DsgAxl5UYxlsrvgVESHTyTGigoTa1bCUrxn70F5bqt0Gw==",
       "license": "MIT",
       "dependencies": {
-        "@next/env": "15.5.21",
+        "@next/env": "15.5.24",
         "@swc/helpers": "0.5.15",
         "caniuse-lite": "^1.0.30001579",
         "postcss": "8.4.31",
@@ -11503,15 +11503,15 @@
         "node": "^18.18.0 || ^19.8.0 || >= 20.0.0"
       },
       "optionalDependencies": {
-        "@next/swc-darwin-arm64": "15.5.21",
-        "@next/swc-darwin-x64": "15.5.21",
-        "@next/swc-linux-arm64-gnu": "15.5.21",
-        "@next/swc-linux-arm64-musl": "15.5.21",
-        "@next/swc-linux-x64-gnu": "15.5.21",
-        "@next/swc-linux-x64-musl": "15.5.21",
-        "@next/swc-win32-arm64-msvc": "15.5.21",
-        "@next/swc-win32-x64-msvc": "15.5.21",
-        "sharp": "^0.34.3"
+        "@next/swc-darwin-arm64": "15.5.24",
+        "@next/swc-darwin-x64": "15.5.24",
+        "@next/swc-linux-arm64-gnu": "15.5.24",
+        "@next/swc-linux-arm64-musl": "15.5.24",
+        "@next/swc-linux-x64-gnu": "15.5.24",
+        "@next/swc-linux-x64-musl": "15.5.24",
+        "@next/swc-win32-arm64-msvc": "15.5.24",
+        "@next/swc-win32-x64-msvc": "15.5.24",
+        "sharp": "^0.34.3 || ^0.35.3"
       },
       "peerDependencies": {
         "@opentelemetry/api": "^1.1.0",
@@ -15741,7 +15741,7 @@
         "clsx": "^2.1.1",
         "cmdk": "^1.1.1",
         "lucide-react": "^0.460.0",
-        "next": "^15.5.21",
+        "next": "^15.5.24",
         "next-themes": "^0.4.6",
         "radix-ui": "^1.5.0",
         "react": "^19.0.0",
diff --git a/site/package.json b/site/package.json
index 6a43d92e5..da7f76534 100644
--- a/site/package.json
+++ b/site/package.json
@@ -34,7 +34,7 @@
     "clsx": "^2.1.1",
     "cmdk": "^1.1.1",
     "lucide-react": "^0.460.0",
-    "next": "^15.5.21",
+    "next": "^15.5.24",
     "next-themes": "^0.4.6",
     "radix-ui": "^1.5.0",
     "react": "^19.0.0",
```

## Relevant package and configuration source

`package.json`

```json
{
  "name": "sparq-monorepo",
  "version": "0.0.0",
  "private": true,
  "description": "Repo-root npm workspaces for the sparq JS surfaces (the published @sparq-org/sparq WASM client, the shared @sparq/client, the GitHub-Pages showcase site, the DISTINCT operational GUI frontend, and the Tauri GUI e2e harness). NOT a published package — it exists only to give CI a single root install + lockfile across the workspace members. Deliberately has NO \"type\" field so repo-root CommonJS scripts (scripts/*.js, bench/dashboard/*.js) keep resolving as CJS.",
  "workspaces": [
    "packages/*",
    "js",
    "site",
    "gui/app",
    "gui/e2e",
    "gui/e2e-playwright"
  ],
  "devDependencies": {
    "@solid/acl-check": "^0.4.5",
    "@solidlab/policy-engine": "^0.0.2",
    "n3": "^2.1.1"
  },
  "overrides": {
    "parse-url": "8.1.0",
    "gry": "6.0.0",
    "tmp": "0.2.7",
    "got": "11.8.5",
    "git-up": "7.0.0",
    "next": {
      "postcss": "8.5.15"
    },
    "postcss": "8.5.15"
  }
}
```

`site/package.json`

```json
{
  "name": "sparq-site",
  "version": "0.1.0",
  "private": true,
  "description": "sparq feature-showcase website — a live-interactive demonstration of the sparq RDF + SPARQL engine, statically exported for GitHub Pages.",
  "type": "module",
  "scripts": {
    "dev": "node scripts/sync-wasm.mjs && node scripts/bundle-wasm-esm.mjs && node scripts/sync-benchmarks.mjs && node scripts/sync-zk-catalog.mjs && node scripts/build-papers.mjs && node scripts/build-specs.mjs && next dev",
    "sync-wasm": "node scripts/sync-wasm.mjs",
    "bundle-wasm-esm": "node scripts/bundle-wasm-esm.mjs",
    "sync-benchmarks": "node scripts/sync-benchmarks.mjs",
    "sync-zk-catalog": "node scripts/sync-zk-catalog.mjs",
    "build-papers": "node scripts/build-papers.mjs",
    "build-specs": "node scripts/build-specs.mjs",
    "prebuild": "node scripts/sync-wasm.mjs && node scripts/bundle-wasm-esm.mjs && node scripts/sync-benchmarks.mjs && node scripts/sync-zk-catalog.mjs && node scripts/build-papers.mjs && node scripts/build-specs.mjs",
    "build": "next build",
    "postbuild": "node scripts/check-bundle.mjs",
    "check:bundle": "node scripts/check-bundle.mjs",
    "build:tauri": "cross-env NEXT_PUBLIC_BASE_PATH= npm run build",
    "start": "npx --yes serve out",
    "lint": "next lint",
    "test:unit": "node --import ./test-support/register-ts.mjs --test test/*.test.mjs",
    "test:e2e": "playwright test",
    "test:e2e:stress": "playwright test --repeat-each=5 --retries=0",
    "test:e2e:grep-gate": "bash e2e/support/no-timeout-gate.sh",
    "vr": "bash scripts/vr.sh",
    "vr:update": "bash scripts/vr.sh --update-snapshots=all",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "@aztec/bb.js": "5.0.0-nightly.20260324",
    "@noir-lang/noir_js": "1.0.0-beta.21",
    "class-variance-authority": "^0.7.1",
    "clsx": "^2.1.1",
    "cmdk": "^1.1.1",
    "lucide-react": "^0.460.0",
    "next": "^15.5.24",
    "next-themes": "^0.4.6",
    "radix-ui": "^1.5.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "sonner": "^2.0.7",
    "tailwind-merge": "^3.6.0"
  },
  "overrides": {
    "postcss": ">=8.5.10"
  },
  "devDependencies": {
    "@axe-core/playwright": "^4.12.1",
    "@eslint/eslintrc": "^3.2.0",
    "@playwright/test": "^1.61.0",
    "@tailwindcss/postcss": "^4.1.0",
    "@types/node": "^26.1.2",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "buffer": "^6.0.3",
    "cross-env": "^10.1.0",
    "esbuild": "^0.25.12",
    "eslint": "^9.18.0",
    "eslint-config-next": "^15.5.19",
    "fzstd": "^0.1.1",
    "seek-bzip": "^2.0.0",
    "tailwindcss": "^4.1.0",
    "tw-animate-css": "^1.4.0",
    "typescript": "^5.7.0"
  }
}
```

`gui/app/package.json`

```json
{
  "name": "sparq-gui-app",
  "version": "0.1.0",
  "private": true,
  "description": "The sparq operational GUI frontend — a DISTINCT Next.js workbench app (NOT the marketing site) consumed by both the Tauri 2 desktop shell and a static-exported hosted 'Try the GUI live' web target. Reuses the @sparq/client wasm engine client + shared logic; renders its own operational shell (left rail · top bar · IDE tab strip · status bar), never the site's hero/Showcase/Benchmarks/About chrome.",
  "type": "module",
  "scripts": {
    "presync-wasm": "node scripts/check-wasm-built.mjs",
    "sync-wasm": "node scripts/sync-wasm.mjs",
    "predev": "npm run sync-wasm",
    "dev": "next dev -p 3001",
    "prebuild": "npm run sync-wasm",
    "build": "next build",
    "build:tauri": "cross-env NEXT_PUBLIC_BASE_PATH= NEXT_PUBLIC_GUI_TARGET=tauri npm run build && node scripts/strip-legacy-polyfills.mjs",
    "build:web": "cross-env NEXT_PUBLIC_GUI_TARGET=web npm run build",
    "start": "npx --yes serve out",
    "lint": "next lint",
    "typecheck": "tsc --noEmit",
    "test:unit": "node --import ./test-support/register-ts.mjs --test src/lib/editor-keys.test.ts src/lib/federation.test.ts src/lib/odrl.test.ts src/lib/tauri-fs.test.ts src/lib/import-batch.test.ts src/lib/workspace-actions-visibility.test.ts src/lib/rsp-feed.test.ts src/lib/plan-view.test.ts src/lib/query-monitor.test.ts src/lib/proof-view.test.ts src/lib/inferred-facts.test.ts src/lib/form-render-model.test.ts src/lib/forms-bridge.test.ts src/lib/file-decompress.test.ts src/lib/select-result-graph.test.ts scripts/strip-legacy-polyfills.test.mjs"
  },
  "dependencies": {
    "buffer": "^6.0.3",
    "class-variance-authority": "^0.7.1",
    "clsx": "^2.1.1",
    "cmdk": "^1.1.1",
    "fzstd": "^0.1.1",
    "lucide-react": "^0.460.0",
    "next": "^15.5.24",
    "next-themes": "^0.4.6",
    "radix-ui": "^1.5.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "seek-bzip": "^2.0.0",
    "tailwind-merge": "^3.6.0"
  },
  "devDependencies": {
    "@eslint/eslintrc": "^3.2.0",
    "@tailwindcss/postcss": "^4.1.0",
    "@types/node": "^26.1.2",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "cross-env": "^10.1.0",
    "eslint": "^9.18.0",
    "eslint-config-next": "^15.5.19",
    "tailwindcss": "^4.1.0",
    "tw-animate-css": "^1.4.0",
    "typescript": "^5.7.0"
  }
}
```

`site/next.config.ts`

```typescript
import path from "node:path";
import type { NextConfig } from "next";

// [OPUS-4.8] sq-8thu / sq-uj38w — static-export config for GitHub Pages.
// Pages serves this project site at the ROOT of the custom domain https://sparq.jeswr.org/
// (org-migration cutover, sq-uj38w), so the deployed build is ROOT-RELATIVE (basePath '',
// no asset prefix). `output: "export"` writes a fully static `out/` tree (no Node server)
// that the Pages deploy workflow uploads.
//
// [OPUS-4.8] sq-9vw5 — env-switch the base path so the SAME export tree serves multiple hosts:
//
//   * GitHub Pages @ the custom-domain root (production): the Pages workflow builds with
//     `NEXT_PUBLIC_BASE_PATH=''` (see .github/workflows/pages.yml "Build static site"), so
//     basePath/assetPrefix are unset and every asset/route is root-relative (`/_next/...`).
//   * The Tauri 2 desktop webview: serves the frontend from the `tauri://` root (a local
//     `frontendDist`), so EVERY asset must ALSO be ROOT-relative — a `/prefix` 404s there.
//     The GUI's `gui/src-tauri/tauri.conf.json` `beforeBuildCommand` builds the site with
//     `NEXT_PUBLIC_BASE_PATH=''` too, honoured by this same config.
//
// `NEXT_PUBLIC_BASE_PATH` is the single switch — and the `@sparq/client` wasm loader already
// keys its RUNTIME asset URLs off the SAME env var, so the build-time route prefix and the
// runtime wasm-fetch prefix stay in lockstep. Build modes (also in site/README.md):
//   * Pages / Tauri (root): `NEXT_PUBLIC_BASE_PATH='' npm run build` -> basePath '' (root-relative)
//   * Legacy sub-path      : `npm run build` (no env)                -> basePath '/sparq' (fallback)
//
// An UNSET var keeps the historical `/sparq` sub-path as a LEGACY fallback (pre-cutover behaviour,
// no test/caller change); production (Pages + Tauri) always sets an explicit empty string for the
// root-relative export. Next requires basePath to be empty or start with `/` (no trailing slash),
// so we honour only `''` or a `/`-leading value and otherwise fall back to the legacy default.
const rawBasePath = process.env.NEXT_PUBLIC_BASE_PATH;
const basePath =
  rawBasePath === undefined
    ? "/sparq" // unset -> legacy /sparq sub-path fallback (production sets '' for the custom-domain root)
    : rawBasePath === "" || rawBasePath.startsWith("/")
      ? rawBasePath // '' (root-relative: Pages custom domain + Tauri) or an explicit '/prefix'
      : "/sparq"; // a malformed value falls back to the legacy default

const nextConfig: NextConfig = {
  output: "export",
  // Only emit basePath/assetPrefix when there IS a prefix. An empty basePath must not be
  // set as `assetPrefix: ""` either — leaving both unset is exactly the root-relative
  // behaviour the Tauri webview needs.
  ...(basePath ? { basePath, assetPrefix: basePath } : {}),
  trailingSlash: true,
  // Static export cannot run the Next.js image optimiser.
  images: { unoptimized: true },
  // [FABLE-5] sq-qgkwy.1 — tree-shake the MONOLITHIC `radix-ui` barrel. The site imports a
  // handful of primitives (Slot, Dialog, Tooltip, …) via `import { X } from "radix-ui"`, but
  // webpack does not shake the package's namespace re-export barrel: the ENTIRE primitive set
  // (Select, Menu, NavigationMenu, ScrollArea, Toast, Slider, Form, Menubar, …) was bundled
  // into a ~170 KB raw commons chunk shipped on almost every route's first load (measured via
  // source-map attribution — see PR). `optimizePackageImports` rewrites the barrel imports to
  // direct per-primitive imports at compile time, so only primitives actually used are bundled.
  // Purely mechanical (same symbols, same behaviour); lucide-react is already on Next's
  // built-in default list, radix-ui (the monolith) is not.
  //
  // [OPUS-5] sq-w728o — this line is a WORKAROUND for a missing upstream default. Getting
  // `radix-ui` onto Next's built-in list is already proposed in vercel/next.js#76065 (open,
  // unreviewed); see research/nextjs-optimize-package-imports-radix-upstream.md for the
  // measured evidence prepared for that thread. DELETE this line once sparq's Next floor
  // ships the default entry — it is then redundant, not load-bearing.
  experimental: { optimizePackageImports: ["radix-ui"] },
  // [FABLE-5] sq-ymr2e.10 — the visual-regression suite (SPARQ_VR=1, set only by scripts/vr.sh
  // inside the pinned Playwright container) screenshots pages served by `next dev`, and the dev
  // indicator badge would otherwise appear in — and destabilise — every baseline. Scoped to the
  // VR run only; normal `next dev` keeps the indicators.
  ...(process.env.SPARQ_VR ? { devIndicators: false as const } : {}),
  // The @sparq-org/sparq wrapper ships ESM with `.js` import specifiers that resolve
  // to `.ts`/`.tsx` sources in dev; mirror solid-pod-manager's webpack alias so the
  // bundler follows them.
  webpack: (config) => {
    config.resolve.extensionAlias = {
      ".js": [".ts", ".tsx", ".js", ".jsx"],
    };
    // [OPUS-4.8] sq-2e93 — resolve the shared framework-agnostic client
    // (`packages/sparq-client`) to its TS source. The package is consumed via a path
    // alias (no repo-root workspaces yet — see research/gui-design.md §3), so the
    // bundler needs this alias to follow the import the same way tsconfig `paths` does.
    config.resolve.alias = {
      ...config.resolve.alias,
      "@sparq/client": path.resolve(
        __dirname,
        "../packages/sparq-client/src/index.ts",
      ),
    };
    return config;
  },
};

export default nextConfig;
```

`gui/app/next.config.ts`

```typescript
import path from "node:path";
import type { NextConfig } from "next";

// [OPUS-4.8] sq-ixc3.8 / sq-ixc3.9 — static-export config for the DISTINCT operational GUI
// frontend (NOT the marketing site). This app builds to a backend-free `out/` tree consumed by
// BOTH targets, selected by NEXT_PUBLIC_BASE_PATH (the same single switch the site + the
// @sparq/client wasm loader already key off, kept in lockstep):
//
//   * Tauri 2 desktop webview ("build:tauri"): serves the frontend from the `tauri://` root, so
//     EVERY asset must be ROOT-relative (a `/prefix` 404s there). `tauri.conf.json`'s
//     `beforeBuildCommand` runs `build:tauri`, which sets NEXT_PUBLIC_BASE_PATH='' → basePath ''.
//   * Hosted "Try the GUI live" web target ("build:web"): served under a sub-path on the same
//     GitHub-Pages-style host. The maintainer picked the URL slot (bead sq-vnd0i, Option B):
//     the live GUI is hosted at the `/app` sub-path (the site's "App" nav destination;
//     "/try" stays the lightweight REPL). This app defaults the web build to `/app`,
//     overridable via the env var.
//
// An UNSET var keeps the web default (`/app`); an explicit empty string selects the
// root-relative (Tauri) export. Next requires basePath to be empty or start with `/` (no
// trailing slash), so we honour only `''` or a `/`-leading value and otherwise fall back.
const rawBasePath = process.env.NEXT_PUBLIC_BASE_PATH;
const basePath =
  rawBasePath === undefined
    ? "/app" // unset → hosted-web default (the live-GUI "/app" sub-path)
    : rawBasePath === "" || rawBasePath.startsWith("/")
      ? rawBasePath // '' (Tauri root-relative) or an explicit '/prefix'
      : "/app";

const nextConfig: NextConfig = {
  output: "export",
  // Only emit basePath/assetPrefix when there IS a prefix. An empty basePath must not be set as
  // `assetPrefix: ""` either — leaving both unset is exactly the root-relative behaviour the
  // Tauri webview needs.
  ...(basePath ? { basePath, assetPrefix: basePath } : {}),
  trailingSlash: true,
  // Static export cannot run the Next.js image optimiser.
  images: { unoptimized: true },
  // The @sparq-org/sparq wrapper ships ESM with `.js` import specifiers that resolve to `.ts`/`.tsx`
  // sources; mirror the site's webpack alias so the bundler follows them.
  webpack: (config) => {
    config.resolve.extensionAlias = {
      ".js": [".ts", ".tsx", ".js", ".jsx"],
    };
    // Resolve the shared framework-agnostic client (`packages/sparq-client`) to its TS source —
    // the SAME single-source-of-truth the site consumes, so the GUI is a zero-new-copy consumer
    // of the engine TS surface (research/gui-design.md §4).
    config.resolve.alias = {
      ...config.resolve.alias,
      "@sparq/client": path.resolve(
        __dirname,
        "../../packages/sparq-client/src/index.ts",
      ),
    };
    return config;
  },
};

export default nextConfig;
```

## Remaining validation and limits

Actual Linux Node 22 `npm ci` and installed-tree resolution remain unexecuted, as do normal GUI/site lint, typecheck, unit, export, development-server and browser checks. The GUI workflow supplies PR static web and Tauri exports. Pages is a deployment route, not a premerge probe. Visual and other advisory outcomes require inspection; screenshots must not be refreshed blindly.

Root owns independent Opus review, normal PR/CI, protected merge queue, and the postmerge check that both dependency alerts close automatically. No remote mutation, alert dismissal or admission claim was made.

This packet omits raw host paths/environment logs, bulk npm cache and third-party maintainer metadata. Exact local evidence is enumerated in the manifest; the earlier frozen readiness report supplies detailed CI routing.
