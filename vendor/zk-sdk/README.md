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
| `risc0-build` | Remove unused `dirs`; select the kernel's embedded-ELF library without its binary-only dependencies. |
| `rzup` | Make RSA optional behind additive `signatures`, enabled by both existing `install` and `publish` features. Their default selection remains unchanged. Discovery omits key construction/storage; fetching a signed manifest without signature support returns an error. |
| `ark-relations` | Backport the tracing-subscriber dependency to the compatible maintained API, retaining disabled default features. Rename the empty layer callback to `on_new_span`; constraint arithmetic is unchanged. |
| `ark-crypto-primitives` | Make `derivative` optional and select it from the existing `crh`, `encryption` and `signature` feature families. `commitment` and `merkle_tree` inherit `crh`. Gate the macro import the same way; algorithms and derives are unchanged. |
| `risc0-zkvm` | Select `rrs-lib` from the existing `prove` feature, whose server profiler is its only consumer. Select the embedded kernel library without binary-only dependencies. The existing `client,bonsai` defaults are unchanged. |
| `risc0-zkos-v1compat` | Add a default-on `kernel` feature for the existing binary's `no_std_strings` and `include_bytes_aligned` dependencies. The binary requires this feature; ELF-only library consumers disable defaults. All Rust source, assembly, blobs and the embedded ELF are unchanged. |

The rzup key-taking custom constructor is available with `signatures`; ordinary
`Rzup::new` and local toolchain discovery retain their API. The real upstream
signature implementation, official public key, signature verification and signing
bodies are unchanged. There is no replacement or accepting stub for signatures.
The SHA hashing helpers remain because distribution code also uses them for file
integrity. Install/publication still require their original verification paths.

Run `python3 vendor/zk-sdk/verify.py` with Python supporting `tomllib` and Git to
check inventories/hashes, feature defaults and explicit local patch selection in
both evaluator lockfiles and the exact vendor directories in both workspace
patch tables. It reverses each patch in a private temporary copy,
checks every reconstructed file against its upstream hash, then replays the patch
and compares the patched bytes. All patch paths use `a/` and `b/` relative to the
package directory and one strip level. Additional upstream license files have
their own recorded provenance and are excluded from patch reconstruction.
With an existing compatible Cargo cache,
`CARGO_TARGET_DIR=/absolute/cache python3 vendor/zk-sdk/verify.py --smoke` also runs
synthetic local discovery, constraint tracing/satisfaction, and unchanged upstream
signature round-trip/tampering checks in discovery and additive key-API modes.
The smoke harness targets macOS and Linux. The additive key-taking constructor
is compiled; it is not called by these tests. The upstream rzup unit-test module
requires its default features; the synthetic harness does not claim that suite.
`--feature-matrix` also compiles the empty/default-equivalent and active
SNARK/sponge feature selections, each derive-consuming family, their constraint
gadget combinations and the combined selection. It checks that only selections
using those families resolve `derivative`. Gadget combinations include `std` and
`prf`: untouched upstream `commitment,r1cs` without those prerequisites fails to
compile, independently reproduced against the registry archive. This patch does
not repair that existing upstream feature-combination limitation.
The temporary smoke harness uses RSA only
to test the preserved signature code; it is absent from both evaluator graphs.
The feature harness similarly resolves `derivative` only for its consumer checks;
both evaluator graphs omit it with their current SNARK/sponge selection.
Add `--offline` when all test dependencies are cached; otherwise Cargo can fetch
public dependencies. The temporary smoke lock is derived from the evaluator lock
and adds explicitly pinned test dependencies; it is not a proof artifact lock.
The harness sets a synthetic GitHub token and a private temporary RISC0 directory.
It neither installs artifacts nor accesses hosted proving or publication services.

The earlier scoped native Clippy record selects `risc0-build`, `rzup`,
`ark-relations` and `ark-crypto-primitives` from the locked host workspace, with
`--lib -- -D warnings`. Its resolved features are `risc0-build/unstable`, no rzup
features, `ark-relations/std,tracing-subscriber`, and
`ark-crypto-primitives/merlin,snark,sponge,std`. The later
[feature-edge record](feature-edge-evidence.json) selects `risc0-zkvm`,
`risc0-zkos-v1compat` and `risc0-build`, with the same library-only Clippy flags.
That selection resolves `risc0-zkvm/client,disable-dev-mode,std`, no kernel library
features, and `risc0-build/unstable`. Together these recorded scopes include every
patched package; neither covers every feature combination or the guest target.
The feature compile matrix uses `cargo check`, not code generation or linking;
the separate embedded-ELF API test builds and executes a native test binary.

These checks do not replace a guest rebuild, receipt tests, the dependency audit,
or hosted CI. The full upstream installation/publication integration suite is not
claimed to have run here. Retire the patches when an upstream release provides
equivalent dependency boundaries and passes the same checks; do not change SDK
versions or cryptographic arithmetic merely to remove an advisory.

`python3 vendor/zk-sdk/tests/test_provenance.py` exercises corruption controls for
every required local patch in both lockfiles, the embedded ELF, upstream hashes,
and incorrect patch path nesting even when the recorded hash matches the changed
patch bytes. It also rejects unrelated local manifest paths and registry
substitutions in resolved Cargo metadata.
`CARGO_TARGET_DIR=/absolute/cache python3 vendor/zk-sdk/edge_matrix.py` compiles
the SDK's empty, client, default and prover selections and the kernel library with
its default feature. It verifies the resolved dependency edges and exact vendor manifest paths, then compares the
public embedded-ELF slice with its recorded source file. It does not run a prover,
profile a program, or contact the hosted proving service. `--offline` is available
when these additional feature dependencies are cached and `RECURSION_SRC_PATH`
points to the upstream checksum-pinned recursion archive. Cargo's offline option
alone does not prohibit a dependency build script from downloading that archive;
the edge harness checks its bytes before the offline compile and rejects
`DOCS_RS` and `RISC0_SKIP_BUILD_KERNELS` build stubs. Feature-harness lockfiles
are temporary and separate from the two pinned evaluator lockfiles.

Direct Cargo kernel builds retain default feature activation. Code embedding the
kernel package through `risc0-build` must explicitly request the `kernel` feature
in its guest options: that upstream helper filters binary targets using the
requested feature list and does not infer default-enabled `required-features`.
No evaluator consumer embeds this kernel package through that helper.

The actual RISC-V kernel binary build has an upstream limitation with the pinned
platform package: the unchanged `Syscall` match omits `ProveZkr` and Rust reports
`E0004`. The exact registry baseline and the candidate fail at the same match with
the same compiler and dependency versions. This is not a successful kernel binary
build. Its default dependency activation is preserved, the feature-off binary
request is rejected, and the ELF-only library compiles for the real guest target.
The [feature-edge evidence](feature-edge-evidence.json) distinguishes those
controls from native compilation and the unchanged embedded-ELF identity. No
kernel instruction or cryptographic behavior was changed to repair that separate
upstream source incompatibility.

These are Cargo dependency boundaries, not a removal of code from existing
binaries. The preserved kernel ELF was compiled from an implementation using
the formatting and alignment helpers; an external `r0vm` can contain its own
server/profiler dependencies. Their binary provenance remains separate. Omitting
unused source packages from the evaluator graphs is not a claim that an embedded
or external binary has received a security fix or a source audit.

The literal unified patches contain blank context lines whose single leading
space is patch syntax. One untouched upstream rustdoc line also has a Markdown
hard break. Those import-only whitespace diagnostics are preserved with the exact
upstream bytes; authored source still receives the normal formatting checks.
