# Wasmer derive diagnostics

[GPT-6] This native-only patch keeps `wasmer-derive` at registry version 6.1.0.
It replaces its `proc-macro-error2` adapter with existing `syn::Error` support.
No cryptographic algorithm, generated unsafe implementation, field-offset or
padding operation changes. The two original rejection messages are retained;
malformed attribute metadata now produces a spanned error instead of panicking.
The constructor stops at the first accepted representation as before.

The bounded maintained-upstream check found the diagnostic dependency still in
[upstream at ca719eb](https://github.com/wasmerio/wasmer/blob/ca719eb81fa450a06bcd6f0dd46e0b7cca15d7a9/lib/derive/Cargo.toml)
and [release 7.4.2](https://docs.rs/crate/wasmer-derive/7.4.2).
That major release is not a compatible removal of this edge. This finding is
limited to the inspected sources, not a claim about every fork or future release.

`UPSTREAM.json` pins the exact 7,501-byte registry archive, VCS commit, all ten
archive files and four changed files. `diagnostics.patch` reconstructs every
original byte and reapplies to every candidate byte. The archive did not contain
a license file: `LICENSE.upstream` is the MIT notice fetched separately from
the exact VCS commit, with its own URL and hash. The archive's historical lock
and compiletest sources are preserved as provenance, not current test evidence.

`lock-delta.json` records the source-only native lock edit: same-version local
derive selection and removal of its only diagnostic edges, `proc-macro-error2`
2.0.1 and `proc-macro-error-attr2` 2.0.0. No other package version changes.
Actual locked Cargo resolution remains a required remote gate.

The existing native provenance entry point checks both patches and the exact
resolved Wasmer vendor manifest. `supply-chain/config.toml` retains the registry
obligation with `audit-as-crates-io = true`; a local path must not silently count
as an upstream audit. Independent full small-package/patch source review and
upstream exact-version vet coverage are separate requirements. No new exemption,
advisory ignore, trust source or audit record is introduced. The historical 134
uncovered units are not claimed closed; any new count needs an actual gate.

## Static checks

These commands reconstruct source and run adversarial Python controls only:

```sh
python3 zk/native-composition/vendor-support/wasmer/verify.py
python3 -m unittest discover -s zk/native-composition/vendor-support/wasmer -p 'test_*.py'
```

They check full inventory, license/patch hashes, both reconstruction directions,
unchanged generator bodies, exact manifest/lock selection and preserved audit
policy. Corrupt source, unlisted files, symlinks, forged upstream hashes, wrong
paths, registry fallback, inactive selection and removed-edge reintroduction
are rejected. Synthetic metadata tests are not actual Cargo resolution.

## Required remote validation

No Rust control in this checkpoint has been compiled or executed locally.
On the authorized remote worker, first fetch the candidate's locked native graph.
Then use an explicitly owned target and new retained output directory:

```sh
cargo fetch --locked --manifest-path zk/native-composition/Cargo.toml
python3 zk/native-composition/vendor-support/verify.py
python3 zk/native-composition/vendor-support/wasmer/remote_checks.py \
  --target-dir /path/to/owned-target --output /path/to/new-wasmer-evidence
```

The remote controller first resolves the actual native graph offline/locked.
Its temporary control crate starts from the native lock and may select only its
exact registry packages plus the two explicitly pinned diagnostic packages used
by the original baseline helper. This test-only comparison does not put them
back into the native graph. Missing cache inputs fail; there is no network retry.

Four Rust test functions define named/tuple/nested/generic/transparent/unit
layout controls, six exact baseline-versus-candidate generated-token comparisons,
and diagnostic checks. Six real compile-fail fixtures cover missing repr,
enum/union, malformed repr, non-ValueType field and unaligned packed fields.
Two mutations in a retained temporary copy remove the repr guard and discard
compile-error output; both must be discriminated, then exact source restored.
The sole test-only unsafe read consumes bytes initialized before the derive call;
its concrete fixtures use only integer fields and the unchanged derived code.

Retain command/exit/source/executable evidence from remote validation, then run
the native tuple/RDF/binding suites, own-policy vet/deny and license/provenance
gates on the combined source. These controls measure no proof performance and
count no cryptographic proofs. Static success is not remote gate success.
