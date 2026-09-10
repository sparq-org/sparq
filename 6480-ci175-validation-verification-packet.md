# PR6481 exact-head CI verification

{
  "task": "PR6481 exact17594 CI/native review verification",
  "model": "GPT-6 Astra xhigh",
  "created_utc": "2026-09-10T11:04:47.156760+00:00",
  "head": "17594d4a6534c142cae764772fc42049e898eca3",
  "checkout_merge_commit": "26409755cc774590ef3ada1231ca72dcfd252caa",
  "checkout_base": "781f667c19a8ebb779cfccb24b05ea432360b025",
  "native_install": {
    "run": 34467004187,
    "job": 102837808254,
    "node": "22.23.2",
    "npm": "10.9.8",
    "next_installed": {
      "gui/app": "15.5.24",
      "site": "15.5.24"
    },
    "sharp_loaded": "0.35.4",
    "vips_loaded": "8.18.6",
    "png_bytes": 91,
    "png_properties": "Exact script asserts png format,1x1 dimensions,length>8 and PNG signature before success output.",
    "lock_match": "Loaded sharp.versions.sharp equalled committed packages[node_modules/sharp].version; assertion reached and passed.",
    "no_drift": "Final git diff --exit-code -- package-lock.json gui/app/package.json site/package.json ran after awaited PNG under bash -e; no diff output, step conclusion success and next build step started.",
    "log_lines": {
      "tool_versions": [
        448,
        449
      ],
      "workspace_resolution": [
        522,
        526
      ],
      "sharp_vips_png": [
        528,
        529
      ],
      "no_drift_command": 493,
      "bash_errexit": 494,
      "next_step": 530
    }
  },
  "copilot_disposition": {
    "versions_claim": {
      "thread": "PRRT_kwDOSz3qKM6hCljp",
      "assessment": "False for exact published sharp0.35.4 and actual Linux run. Public source explicitly assigns and exports versions.sharp; real log prints0.35.4 and assert passes. Suggested external sharp/package.json subpath is not exported by0.35.4.",
      "recommended_response": "Cite utility.cjs:80/:297 plus job log528-529,retain current code."
    },
    "consumer_path_assertion": {
      "assessment": "Accurate future regression-test gap, not a current missing trigger. Both paths are present. Prior actualOpus0c review explicitly classified this medium-low/nonblocking; original authoring bundle ran one-off path-removal controls, not a committed CI assertion.",
      "limit": "Current PR also changes workflow/lock, so this run is not an isolated manifest-only event test.",
      "disposition": "Record as optional retained regression coverage; no source change in this read-only phase."
    },
    "next_floor_assertion": {
      "assessment": "npm ls validates installed dependency resolution and reports version; it is not a permanent anti-downgrade floor. Current manifests ^15.5.24 and installed exact15.5.24 satisfy this patch. A coordinated future manifest+lock downgrade could pass that listing check.",
      "prior_review_precision": "No explicit Next-floor finding in the two inspected prior Opus results; do not attribute a nonexistent individual approval. Prior review approved this concrete patched graph for CI.",
      "disposition": "Potential future policy/test strengthening; no current patch defect demonstrated and no new immutable security-floor policy introduced."
    }
  },
  "caveats": [
    "npm ci still reported18 vulnerabilities (2 moderate,16 high); this inspection did not retrieve audit identities and does not claim all workspace advisories are resolved.",
    "Current smoke proves this Ubuntu native installation, not every optional platform package or security/exploit coverage.",
    "vips8.18.6 is observed; the script does not itself enforce a permanent vips floor or selected @img wrapper identity.",
    "Hard GUI guard/shared-client jobs verified through exact completed step metadata; raw logs were read only for site,GUI frontend andPlaywright jobs.",
    "All live/remote state remains unchanged; no review resolution, rerun, queue action, local install/build or model call."
  ],
  "request_budget": {
    "initial_core_remaining": 4986,
    "github_api_logical_requests": 19,
    "public_source_downloads": 1,
    "total_logical_read_requests": 20,
    "retries": 0,
    "note": "Job-log signed redirects are followed internally by gh;20 counts requested endpoint/download operations, not low-level HTTP hops. No final extra budget request."
  }
}

## Required gate and actual suite results

{
  "docs_inspection": {
    "run": 34467004155,
    "job": 102837808255,
    "tests": 13,
    "result": "OK",
    "lines": [
      3396,
      3433
    ]
  },
  "supply_advisory": {
    "run": 34467004107,
    "job": 102837807815,
    "tests": 25,
    "result": "OK",
    "standalone_drift": "matches lock including1 sharp instance",
    "lines": [
      285,
      335
    ]
  },
  "frontend_raw_log_observations": {
    "site": "Lint/typecheck/static export succeeded; Next15.5.24,72 static pages.",
    "gui": "Lint/typecheck,123 unit tests with0 failures; web andTauri exports succeeded withNext15.5.24.",
    "playwright_mocked_ipc": "92 passed."
  },
  "aggregate": {
    "run": 34467004259,
    "check_run": 102837808339,
    "app_id": 15368,
    "app_slug": "github-actions",
    "result": "success",
    "completed_at": "2026-09-10T10:55:21Z",
    "gating_total": 65,
    "gating_ran": 30,
    "gating_skipped": 35,
    "advisory_excluded": 15,
    "interpretation": "Policy-scoped aggregate success, not65 executed jobs or full-workspace proof."
  }
}

## Public sharp API source

```text
# utility.cjs:11-18
11: const is = require('./is.cjs');
12: const libvips = require('./libvips.cjs');
13: const sharp = require('./sharp.cjs');
14: const pkg = require("../package.json");
15: 
16: 
17: const runtimePlatform = libvips.runtimePlatformArch();
18: const libvipsVersion = sharp.libvipsVersion();
# utility.cjs:46-80
46:   locallyBoundedBicubic: 'lbb',
47:   /** [Nohalo interpolation](http://eprints.soton.ac.uk/268086/). Prevents acutance but typically reduces performance by a factor of 3. */
48:   nohalo: 'nohalo',
49:   /** [VSQBS interpolation](https://github.com/libvips/libvips/blob/master/libvips/resample/vsqbs.cpp#L48). Prevents "staircasing" when enlarging. */
50:   vertexSplitQuadraticBasisSpline: 'vsqbs'
51: };
52: 
53: /**
54:  * An Object containing the version numbers of sharp, libvips
55:  * and (when using prebuilt binaries) its dependencies.
56:  *
57:  * @member
58:  * @example
59:  * console.log(sharp.versions);
60:  */
61: let versions = {
62:   vips: libvipsVersion.semver
63: };
64: /* node:coverage ignore next 15 */
65: if (!libvipsVersion.isGlobal) {
66:   if (!libvipsVersion.isWasm) {
67:     try {
68:       versions = require(`@img/sharp-${runtimePlatform}/versions`);
69:     } catch (_) {
70:       try {
71:         versions = require(`@img/sharp-libvips-${runtimePlatform}/versions`);
72:       } catch (_) {}
73:     }
74:   } else {
75:     try {
76:       versions = require('@img/sharp-wasm32/versions');
77:     } catch (_) {}
78:   }
79: }
80: versions.sharp = pkg.version;
# utility.cjs:289-301
289:  */
290: module.exports = (Sharp) => {
291:   Sharp.cache = cache;
292:   Sharp.concurrency = concurrency;
293:   Sharp.counters = counters;
294:   Sharp.simd = simd;
295:   Sharp.format = format;
296:   Sharp.interpolators = interpolators;
297:   Sharp.versions = versions;
298:   Sharp.queue = queue;
299:   Sharp.block = block;
300:   Sharp.unblock = unblock;
301: };
```
