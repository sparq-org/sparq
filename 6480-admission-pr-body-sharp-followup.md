> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

Patches Next.js to `15.5.24` in both frontend manifests and the root lockfile, and resolves `sharp@0.35.4` under Next's supported optional-dependency range. This addresses the Next.js advisories tracked in #6480 and selects a version outside the affected range reported by sharp advisory [GHSA-f88m-g3jw-g9cj](https://github.com/lovell/sharp/security/advisories/GHSA-f88m-g3jw-g9cj).

The lock update preserves all eight SWC variants, Linux libc selectors, existing overrides and unrelated dependency floors. Sharp's native packages and the two transitives whose minimum versions it raises are updated with it. The advisory record matches the resolved graph while distinguishing the candidate patch from the still-open default-branch alert; alerts are not dismissed.

The existing root `npm ci` step checks installed Next resolution for both workspaces and rejects lockfile or manifest drift. Either consumer manifest now triggers that workflow. Its inspection test recognizes inline and literal-block commands while enforcing the root working directory and wasm-pack ordering.

Validation at `0c6a780593a9b3381fb158e426519a2a6d8d17f9`:

- Targeted metadata-only resolution with npm 11.17.0; offline regeneration is byte-identical. Changed lock entries match exact published metadata and required dependency constraints.
- 13 install-posture tests, 25 advisory-record tests, the unchanged record checker and shared command-parser self-test pass. Executed negative controls detect missing/incorrect installs, wrong directory and ordering; removing either manifest trigger loses coverage.
- Independent Claude Opus 5 review at extra-high reasoning completed before publication; disposition is recorded in the coordinator's frozen review evidence.
- Fresh Linux CI remains required for this revision's actual install, native dependencies, builds and tests. The earlier revision's Node 22.23.2/npm 10.9.8 installation and frontend checks passed, but do not validate this new sharp graph. Local preflight still encounters the known macOS Bash 3 `mapfile` limitation; supported CI must pass it.

This is a version-based security update. It does not establish deployment patching, application exploitability, or the health of a separately installed system libvips. Keep #6480 open until protected merge and automatic alert re-evaluation establish the default-branch result.
