# Independent GPT-6 dependency source review

Verdict: **APPROVE** for the two exact source units below against the `safe-to-deploy` criterion. No required findings. This is an actual cached-source review, not an upstream audit attribution or a runtime test result.

## num-iter 0.1.45 → 0.1.46

I independently verified both official cached registry archive checksums against their cached index entries, enumerated every archive member, and inspected the complete six-path delta. The accepted baseline chain remains Google 0.1.43 plus ISRG 0.1.43→0.1.44→0.1.45; the exact retained records are in `review.json`. I did not re-audit the unchanged prior implementation.

The sole Rust source file is byte-identical (SHA-256 `220089a391d352be4884b36de457466a8a7c7b079c35a0d46f0ae14aeb77e92d`). Runtime dependency requirements and default/std/i128 feature definitions are unchanged. The old package already contains no build script; removing its unused `autocfg` build edge does not remove required generated code or configuration. The new normalized manifest explicitly disables discovery and names the same `src/lib.rs`; archive enumeration confirms there is no omitted binary, example, integration target, or build hook. `#![no_std]` remains, with the same feature forwarding to its dependencies.

The added packaged lockfile names normal registry packages/checksums and does not replace the consuming workspace's resolution; its transitive `autocfg` via num-traits is distinct from the removed direct build dependency. Remaining changes are version/VCS, release notes, and a README CI link. Both complete license files are byte-identical. All declared features and targets are covered by this source delta review. Dependencies retain separate audit obligations.

## cranelift-codegen-shared 0.110.3

I read all eight exact archive files, including the three Rust files (1,560 bytes), both manifests, VCS metadata, README, and Apache-2.0-with-LLVM-exception license. The official cached archive/index checksum is `efcff860573cf3db9ae98fbd949240d78b319df686cc306872e7fab60e9c84d7`; VCS is `72bedc12e00084fdf49f7d4f5d40b979c184b0a5`, subdirectory `cranelift/codegen/shared`.

There are no dependencies, feature declarations, build scripts, proc macros, external includes, unsafe blocks, I/O, mutable globals, or alternate target sources. The complete public surface is four fixed `u16` type-layout constants, a Cargo-provided version string, and `simple_hash(&str) -> usize`. The hash walks valid Unicode scalar values, uses explicit wrapping addition and defined rotation, and casts the final `u32` to usize. It allocates nothing and performs work linear in the caller-provided string. The cast is defined even on narrower targets; this review does not infer cross-target hash equality or compatibility of the surrounding compiler on unsupported platforms. The hash is intentionally non-cryptographic and may collide; callers must resolve collisions and cannot treat it as authentication. The constants are within u16 and perform no indexing or pointer arithmetic themselves. The existing two deterministic test vectors are source-inspected, not executed.

No package-level safety or malicious-code issue was found. This full package review does not attest the other Cranelift crates, generated code, Wasmer, or any cryptographic backend.

## Verification and limits

`review.json` contains all 27 member hashes across the three archives, exact registry metadata and baseline records; extracted review inputs and the exact num-iter delta are retained alongside it. No compiler, Cargo, native test, network download, proof, policy edit, or audit-store insertion was performed. This report supports a later explicitly attributed scoped audit proposal; it does not itself claim a passing dependency gate or deployment validation.
