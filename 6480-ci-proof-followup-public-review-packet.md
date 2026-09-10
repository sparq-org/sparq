# Next installed-tree CI proof supplement

Author: GPT-6 Astra xhigh.

Reviewed parent: `50c09eb2b7563924cccde9305ee51ff62b27af38`; candidate: `26d139520f07f1ceacafbacbeb9991de371e2b53`. Prior approve_for_ci review applies to the unchanged three package files; this supplement changes only `.github/workflows/js.yml`. No other source edits.

## Exact delta

```diff
diff --git a/.github/workflows/js.yml b/.github/workflows/js.yml
index 230cd6620..16911e0d3 100644
--- a/.github/workflows/js.yml
+++ b/.github/workflows/js.yml
@@ -132,7 +132,10 @@ jobs:
       # published-client SBOM stays clean (runtime tree = {fzstd} only).
       - name: Install npm dependencies (full workspace, from repo root)
         working-directory: .
-        run: npm ci
+        run: |
+          npm ci
+          npm ls next --workspace=site --workspace=gui/app
+          git diff --exit-code -- package-lock.json gui/app/package.json site/package.json
 
       - name: Build (wasm-pack + tsc)
         run: npm run build
```

## Relevant production caller

The existing `js` job uses `ubuntu-latest`, Node22, root npm ci with ordinary lifecycle scripts after wasm-pack installation. Default job working directory is js, but this existing step explicitly overrides it to `.`. No workflow trigger/permission/concurrency/toolchain/lifecycle field changes. Root package-lock.json and this workflow both match existing PR path triggers.

```yaml
      # NOT the root workspace devDeps, so those imports were ERR_MODULE_NOT_FOUND -> red `js`
      # gate. A root install hoists them into the workspace node_modules where the test (and
      # the `js`-member build) both resolve them. js/package.json is untouched, so the
      # published-client SBOM stays clean (runtime tree = {fzstd} only).
      - name: Install npm dependencies (full workspace, from repo root)
        working-directory: .
        run: |
          npm ci
          npm ls next --workspace=site --workspace=gui/app
          git diff --exit-code -- package-lock.json gui/app/package.json site/package.json

      - name: Build (wasm-pack + tsc)
        run: npm run build

      - name: Test (node --test)
        run: npm test
```

## Validation and limitations

{
  "head": "26d139520f07f1ceacafbacbeb9991de371e2b53",
  "parent": "50c09eb2b7563924cccde9305ee51ff62b27af38",
  "model": "GPT-6 Astra xhigh",
  "changed_file": ".github/workflows/js.yml",
  "diff": "+4/-1",
  "outcome": "Review-ready CI-only supplement; clean local commit.",
  "same_step_run": [
    "npm ci",
    "npm ls next --workspace=site --workspace=gui/app",
    "git diff --exit-code -- package-lock.json gui/app/package.json site/package.json"
  ],
  "working_directory": ".",
  "all_other_yaml_fields_unchanged": true,
  "original_three_files_unchanged": [
    {
      "path": "package-lock.json",
      "sha256": "8687c74f94a16e48688e23ed0e012470cd91eb0e3a34d4095dc14f01c6d1fcc3",
      "unchanged": true
    },
    {
      "path": "gui/app/package.json",
      "sha256": "b0e1e4d94d6fe2285e52e2524b2f30e20fe576275f8765f6d062732e40985df4",
      "unchanged": true
    },
    {
      "path": "site/package.json",
      "sha256": "60bbea3bc8c3de693274b92882b97173e514eb0f41b7ee8c3870c72235aa887d",
      "unchanged": true
    }
  ],
  "NB8": {
    "tracked_package_json_count": 12,
    "dependency_ranges": {
      "gui/app/package.json": "^15.5.24",
      "site/package.json": "^15.5.24"
    },
    "other_next_key": "Root overrides.next.postcss=8.5.15 is a transitive override, not an additional Next version declaration.",
    "residual_old_next_dependency_range_found": false
  },
  "checks": {
    "exact_text_delta": "pass",
    "parsed_YAML_before_after_except_run": "pass; installed Ruby Psych3.1.0",
    "git_diff_check": "pass",
    "preflight": "exit1, known Bash3 mapfile limitation only",
    "actionlint_shellcheck": "unavailable on PATH; no install"
  },
  "limits": [
    "No local npm install, lifecycle, Next/wasm build or installed-tree result.",
    "Linux CI must execute all three commands; the GitHub default Linux run shell provides fail-fast behavior.",
    "No Node/npm pin or trigger/permission/lifecycle change; original metadata-only evidence remains immutable.",
    "Initial lightweight validator assumed every next key was a dependency; corrected inventory includes existing root override and all tracked manifests."
  ],
  "commands_pending": false,
  "remote_mutations": false,
  "completed_at": "2026-09-10T09:19:51.947887+00:00"
}

## Complete tracked package manifest inventory

{
  "tracked_package_json_count": 12,
  "manifests": [
    {
      "path": "bench/wasm-compare/browser/package.json",
      "sha256": "906a29a4fdf23cdef19bfbaaff34d77035a8122c8ed596df8d02ba0d610ae8e4",
      "next_keys": []
    },
    {
      "path": "crates/sparq-shacl/tests/diff_fuzz/package.json",
      "sha256": "08eb4db6fd0695e5db6d6eac4832a2483e5f9bb645cc5d7eba3739b1e0002696",
      "next_keys": []
    },
    {
      "path": "gui/app/package.json",
      "sha256": "b0e1e4d94d6fe2285e52e2524b2f30e20fe576275f8765f6d062732e40985df4",
      "next_keys": [
        {
          "json_path": [
            "dependencies",
            "next"
          ],
          "value": "^15.5.24"
        }
      ]
    },
    {
      "path": "gui/e2e-playwright/package.json",
      "sha256": "7b5d148a9d1ad3588b938672bfedd6e97bac13f7e1ac691372591f91b2108c74",
      "next_keys": []
    },
    {
      "path": "gui/e2e/package.json",
      "sha256": "03fd55deb03bec9466e8c4c29162bb66a3775d2b0900c0d44aab146591e5bf18",
      "next_keys": []
    },
    {
      "path": "js/package.json",
      "sha256": "d3724ed4ca15b407a4f86d2bb375e2cfa01ade235aaae6101455d38fc8220a4b",
      "next_keys": []
    },
    {
      "path": "package.json",
      "sha256": "3e89c829ab61bb7633b8d6cd3030fe44d8e73c45f30d50a57a21937d9d7dd0ef",
      "next_keys": [
        {
          "json_path": [
            "overrides",
            "next"
          ],
          "value": {
            "postcss": "8.5.15"
          }
        }
      ]
    },
    {
      "path": "packages/eyereasoner-compat/package.json",
      "sha256": "d1b3ed9aa31fe790d6872f80dbd15e85cdb263cca3270134374a69472013cd91",
      "next_keys": []
    },
    {
      "path": "packages/rdfjs-conformance/package.json",
      "sha256": "b0437b14b152959ca8d0641ce075e0c04a2dd5c69023ad1499b4adb78d453ec8",
      "next_keys": []
    },
    {
      "path": "packages/solid-server/package.json",
      "sha256": "dd6bbad69119e65d748264a5f4079c7d4650f141a9b0c63bcf1da2663c7eec55",
      "next_keys": []
    },
    {
      "path": "packages/sparq-client/package.json",
      "sha256": "3cdfd6c0f64d75e9cb3b21e9b7f6817f54cbc1f65765f9f03ab68a75b8d13599",
      "next_keys": []
    },
    {
      "path": "site/package.json",
      "sha256": "60bbea3bc8c3de693274b92882b97173e514eb0f41b7ee8c3870c72235aa887d",
      "next_keys": [
        {
          "json_path": [
            "dependencies",
            "next"
          ],
          "value": "^15.5.24"
        }
      ]
    }
  ]
}

The original implementation packet remains frozen; its full dependency/registry evidence is referenced rather than duplicated. No installed-tree or CI execution is claimed by this supplement.
