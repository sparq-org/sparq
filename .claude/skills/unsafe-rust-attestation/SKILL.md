---
name: unsafe-rust-attestation
description: How sparq attests memory safety — the forbid(unsafe_code) posture across most crates, the concentrated sparq-core unsafe surface (mmap / dict-spill / SIMD, threat-model boundary B5), the nightly Miri lane, cargo-geiger, and the per-site unsafe-justification register (gap GX-5) — and how to document plus verify every unsafe block for the memory-safety attestation framework. Use when working the memsafety certification worktree, enumerating or justifying an unsafe site, wiring the cargo-geiger ratchet, reasoning about what Miri can and cannot reach, or attesting the Miri/fuzz/oracle coverage matrix. Grounded in sparq's real research/threat-model.md + crates/sparq-core + .github/workflows/miri.yml.
---

# Unsafe-Rust attestation (sparq)

sparq's headline safety claim is memory safety. This skill is how to *attest* it
honestly for the `memsafety` certification worktree — the `forbid(unsafe_code)`
posture, the concentrated `sparq-core` unsafe surface (threat-model boundary
**B5**), and the verification estate (Miri + fuzz + the deterministic oracle).

> NON-CANONICAL timing. No measured numbers belong in this file.

## The posture: forbid where you can, concentrate + justify where you can't

- **`#![forbid(unsafe_code)]`** is declared in the large majority of workspace
  crates (sparq-serve, sparq-reason, sparq-zk, sparq-nlq, sparq-shacl, and most
  others — `grep -rl "forbid(unsafe_code)" crates/`). `forbid` (not `deny`) means
  the lint can't even be locally `#[allow]`-ed away — a hard compile error if any
  `unsafe` is introduced. **This is the strongest attestable claim and it's free.**
- **`sparq-core` carries the *vast majority* of the `unsafe`, and all of the B5
  (untrusted-input) risk.** A few other crates have small, non-B5 unsafe surfaces
  (e.g. `sparq-vectors` mmap/raw-slice casts, `sparq-cli`'s loader mmap in
  `main.rs`, `sparq-zk-compose`'s `libc::flock` FFI in `verifier.rs`, one
  `sparq-bench` block) — these do **not** carry `forbid(unsafe_code)`, so the
  audit surface is "the crates without `forbid` " not "one crate". Verify the
  live set with `grep -rL "forbid(unsafe_code)" $(git ls-files 'crates/*/src/lib.rs')`.
  Attestation stays tractable because the *dangerous* (hostile-input → unsafe)
  risk is still concentrated in `sparq-core`'s B5 boundary, enumerated below.

When scoring, the honest statement is: *"N of M crates forbid unsafe; the residual
unsafe is concentrated in sparq-core's B5 boundary, enumerated and verified
below."* Verify the live count with grep before quoting it — don't hard-code it.

## The B5 unsafe surface (`research/threat-model.md` §B5)

B5 = **hostile on-disk index file → mmap loader → unsafe code** — an
*untrusted-input → unsafe-code* boundary, the most dangerous class in the system.
`grep -rn "unsafe" crates/sparq-core/src` enumerates the sites — concentrated
across five files (run it for the live count; don't hard-code one here):

| File | Nature of the unsafe |
|---|---|
| `lib.rs` | `Graph::open` loader: mmap of permutation + dict + numerics/temporals caches; POD↔bytes reinterprets |
| `dict.rs` | zero-copy dictionary: `MappedDict::stored` (attacker-controlled `u64` offset), `rd_str` → `from_utf8_unchecked` over the mmap'd blob |
| `store.rs` | `TripleStore::open` raw-perm reinterpret (`from_raw_parts` over mmap bytes) |
| `dictspill.rs` | `#[cfg(feature = "dict-spill")]` ingest spill: libc `sysconf`/`statvfs` FFI, `from_utf8_unchecked` on own records, parallel scatter `ptr.add().write` |
| `extsort.rs` | external-sort buffer reinterprets |

**The sharpest edges (track these — they have beads):**
- **T-MMAP-UB (sq-znld):** `rd_str` calls `from_utf8_unchecked` on the mmap'd blob
  with **no UTF-8 check** → immediate UB on a hostile/corrupt store. Fix: checked
  `from_utf8` + bounds-check every offset (`dict-offs.bin` length `== len*8`,
  every offset `< dict-terms.bin.len()`) at open time.
- **T-MMAP-DoS (sq-ed2i):** `CompressedPerm::from_mmap` header arithmetic has no
  overflow guard + unchecked per-block offsets/varints → panic / OOB-read. Fix:
  `checked_mul`/`checked_add` + bounds-check directory offsets and every varint.
- **T-MMAP-FUZZ (sq-ky2a):** the loader isn't fuzzed against corrupt/truncated
  files. Fix: a fuzz target asserting *error-not-UB* under `--features dict-spill`.

The dict-spill unsafe is **NOT** the B5 attack surface (it operates on
internally-produced spill files, not hostile index files) — say so explicitly so
the attestation doesn't overstate the threat there.

## The verification estate — what reaches each site

| Verifier | Wired in | Reaches | Does NOT reach |
|---|---|---|---|
| **Miri** (UB) | `.github/workflows/miri.yml`, nightly | pure-Rust unsafe with default features (`parallel`): the `par_iter_mut` scatter writes in `dict.rs`, POD↔bytes reinterprets, `from_utf8_unchecked` over in-memory buffers, `MaybeUninit`+`set_len` remap | the **mmap-backed** sites — Miri rejects file-backed mappings ("Miri does not support file-backed memory mappings"), so `mmap`/`dict-spill` features are **off** in this lane |
| **mmap_corruption_oracle** (deterministic) | `ci.yml` under `--features mmap,dict-spill` | the mmap B5 sites Miri can't reach | non-determinism (it's a fixed oracle, not a fuzzer) |
| **fuzz** lane (cargo-fuzz) | `fuzz.yml`, PR smoke + nightly | hostile-input panics/OOM on the loader | UB detection (panics ≠ UB) |
| **cargo-geiger** | `ci.yml` `geiger` job, **informational** | counts unsafe sites (sparq-core, via `--manifest-path`; can't run the virtual root) | nothing gates on it yet (GX-5) |

**Miri flags (load-bearing — `miri.yml`):** `-Zmiri-tree-borrows` (rayon's
`crossbeam-epoch` violates Stacked Borrows; Tree Borrows is the correct model and
still catches real UB in sparq-core), `-Zmiri-ignore-leaks` (rayon daemon threads
outlive `main`), `-Zmiri-disable-isolation` (incidental clock/temp access). It's
**nightly-only** (Miri ships only on nightly; pinned by date) with **no PR
trigger**, so it creates no PR check-run —
the same "nightly safety net, not a per-PR tax" posture as fuzz/zk-toolchain.

The intended future addition is an **ASan lane** (`-Zsanitizer=address`, non-musl
target) to reach the mmap sites dynamically — deferred (noted in the sq-fo28 PR).

## The two attestation deliverables for memsafety (GX-5)

GX-5 is the open gap: *the unsafe surface has no per-site justification register,
and cargo-geiger is informational only (no gating ratchet).* Two things close it:

### 1. The unsafe-justification register
One document enumerating every `sparq-core` unsafe site with, per site:
- **Location** (`file.rs:line`) and **operation** (e.g. `from_utf8_unchecked`).
- **Safety invariant** — the precondition the caller must uphold for soundness.
- **Why it holds** — or, for the B5 untrusted-input sites, the validation that
  *must* run first (and the bead if it doesn't yet — sq-znld, sq-ed2i).
- **Which verifier covers it** — the Miri / oracle / fuzz column from the matrix.
This is the SSDF/memsafety evidence: every `unsafe` block has a written, reviewed
justification. Pair each with a `// SAFETY:` comment in source.

### 2. The cargo-geiger ratchet
Promote the informational geiger job to a **gating unsafe-count ratchet**:
check a checked-in expected count, fail if the count *increases* without an
accompanying register update (mirror the coverage/conformance ratchet idiom). A
PR that adds `unsafe` then can't merge without justifying it.

## How to use this

1. **Cite, don't re-derive.** The posture + verification estate above is live —
   reference `miri.yml`, the threat-model §B5, the `geiger` job in `ci.yml`.
2. **Gap-fixes land test-first** (`test-driven-development` skill): a register
   entry pairs with a `// SAFETY:` comment; the geiger ratchet lands with the
   checked-in count + a failing-on-increase test.
3. **Honesty contract.** Don't claim Miri covers the mmap sites (it structurally
   can't) — attest the *split* coverage (Miri for pure-Rust UB, oracle+fuzz for
   mmap) honestly. Don't claim "no unsafe" — claim "unsafe concentrated, enumerated,
   justified, verified". The known UB gaps (sq-znld) are real until fixed — never
   paper over them in a control table.

## Local commands
```
grep -rln "forbid(unsafe_code)" crates/          # the forbid-unsafe crate set
grep -rn "unsafe" crates/sparq-core/src          # enumerate the B5 surface
cargo +nightly miri test -p sparq-core           # the UB lane (no mmap features)
cargo test -p sparq-core --features mmap,dict-spill mmap_corruption_oracle
cargo geiger --manifest-path crates/sparq-core/Cargo.toml   # unsafe report
```

<!-- Authored for bead sq-toze.1 (epic sq-toze, cert framework). Grounded in
research/threat-model.md §B5, crates/sparq-core/src, .github/workflows/miri.yml + ci.yml geiger.
 -->
