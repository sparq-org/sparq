# Evaluator SDK dependency patches

[GPT-6] These are narrowly patched crates.io archives used only by the detached
exact-evaluator workspaces. RISC Zero stays at the pinned SDK release. The main
workspace does not select these patches.

`UPSTREAM.json` records each immutable archive URL, its checksum from the original
evaluator lockfile, upstream VCS identity, complete original file hashes, changed
file hashes and the accompanying unified patch hash. Original license files and
`Cargo.toml.orig` are retained. The normalized `Cargo.toml` is the effective manifest.
The RISC Zero archives omitted standalone license text; their additional `LICENSE`
files come from the exact recorded upstream VCS revision, with URL/hash recorded.

| Package | Local change |
| --- | --- |
| `risc0-build` | Remove `dirs`, which no source in the archive uses. |
| `rzup` | Make RSA optional behind additive `signatures`, enabled by both existing `install` and `publish` features. Their default selection remains unchanged. Discovery omits key construction/storage; fetching a signed manifest without signature support returns an error. |
| `ark-relations` | Backport the tracing-subscriber dependency to the compatible maintained API, retaining disabled default features. Rename the empty layer callback to `on_new_span`; constraint arithmetic is unchanged. |

The rzup key-taking custom constructor is available with `signatures`; ordinary
`Rzup::new` and local toolchain discovery retain their API. The real upstream
signature implementation, official public key, signature verification and signing
bodies are unchanged. There is no replacement or accepting stub for signatures.
The SHA hashing helpers remain because distribution code also uses them for file
integrity. Install/publication still require their original verification paths.

Run `python3 vendor/zk-sdk/verify.py` with Python supporting `tomllib` to check all inventories/hashes, feature
defaults and both evaluator lockfiles. With an existing compatible Cargo cache,
`CARGO_TARGET_DIR=/absolute/cache python3 vendor/zk-sdk/verify.py --smoke` also runs
synthetic local discovery, constraint tracing/satisfaction, and unchanged upstream
signature round-trip/tampering checks in discovery and additive key-API modes.
The temporary smoke harness uses RSA only
to test the preserved signature code; it is absent from both evaluator graphs.
Add `--offline` when all test dependencies are cached; otherwise Cargo can fetch
public dependencies. The temporary smoke lock is derived from the evaluator lock
and adds explicitly pinned test dependencies; it is not a proof artifact lock.
The harness sets a synthetic GitHub token and a private temporary RISC0 directory.
It neither installs artifacts nor accesses hosted proving or publication services.

These checks do not replace a guest rebuild, receipt tests, the dependency audit,
or hosted CI. The full upstream installation/publication integration suite is not
claimed to have run here. Retire the patches when an upstream release provides
equivalent dependency boundaries and passes the same checks; do not change SDK
versions or cryptographic arithmetic merely to remove an advisory.

The literal unified patches contain blank context lines whose single leading
space is patch syntax. One untouched upstream rustdoc line also has a Markdown
hard break. Those import-only whitespace diagnostics are preserved with the exact
upstream bytes; authored source still receives the normal formatting checks.
