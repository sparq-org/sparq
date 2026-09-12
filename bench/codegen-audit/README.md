<!-- 🤖 SPARQ agent — codegen audit artifact (sq-98w7z.5 / issue #3079). -->
# Codegen audit of hot decode/join/dict loops (sq-98w7z.5)

**What this is.** The hot loops named below were *assumed* well-compiled but never
verified against actual assembly. This directory replaces that assumption with
evidence: post-LTO release-profile asm for each function, a per-function verdict,
and minimized testcases for the one claim that did not hold. Read-only with
respect to crate code — no `crates/` change is part of this audit.

## Method (reproducible)

- **Probe:** [`asm-probe/`](./asm-probe/) is a standalone bin crate (opted out of
  the workspace) with `#[no_mangle] #[inline(never)]` wrappers around each audited
  function. Its `[profile.release]` mirrors the root workspace exactly
  (`opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`), and it
  mirrors the root `spargebra` vendor patch, so the emitted asm is the *shipped*
  codegen shape, not a lookalike. The audited chunk kernels only exist under
  sparq-engine's **non-default `vectorized` feature**, which the probe enables.
- **Emit:** `cargo rustc --release --manifest-path bench/codegen-audit/asm-probe/Cargo.toml -- --emit asm`
  → post-LTO `.s` (fat LTO + one codegen unit ⇒ the emitted file IS the final
  optimized module). Per-function bodies are extracted under [`asm/`](./asm/).
- **Tiers:** baseline `x86-64` (what `cargo build --release` produces, and the
  baseline dist tier) and `x86-64-v3` (AVX2 — the dist tier built by
  `.github/workflows/build-matrix.yml` via `RUSTFLAGS=-Ctarget-cpu=x86-64-v3`).
- **Toolchain:** rustc 1.88.0 (6b00bc388), LLVM 20.1.5, x86_64-unknown-linux-gnu.
- Wasm `+simd128` claims are **not** audited here (no wasm target installed on the
  audit box); the wasm column below is marked unverified.

> Caveat discovered while producing the v3 evidence: passing `-Ctarget-cpu` only
> to the **final** crate (e.g. via `cargo rustc -- -Ctarget-cpu=x86-64-v3`) does
> NOT widen codegen for dependency-crate functions even under fat LTO — every
> function keeps the `target-cpu` attribute of its *defining* crate's compilation
> (verified: zero AVX instructions in the whole leaf-flag "v3" binary —
> `asm/x86-64-v3-leaf-flag-only/` preserves that build's still-SSE2 decode loop
> as evidence). The dist
> workflow already does this correctly (`RUSTFLAGS` applies to every crate); any
> future per-kernel experiment must do the same.

## Verdict table

| # | Function (file) | Claim under audit | Verdict | Evidence |
|---|---|---|---|---|
| 1 | `DataChunk::decode_numeric_column` — all-inline fast path (`crates/sparq-engine/src/chunk.rs`) | "straight-line loop the compiler is free to vectorise (NEON/AVX, `+simd128`)" | **(i) CONFIRMED vectorized** on both tiers (SSE2 baseline / AVX2 v3); the `all(is_inline)` prescan is scalar-with-early-exit (expected — multi-exit loops don't vectorize) | [below](#1-decode_numeric_column-inline-fast-path), `asm/*/audit_decode_numeric_column.s` |
| 2 | `DataChunk::decode_numeric_column` — general gather path | (documented as non-vectorizable gather — no vectorization claim) | **(i) as designed**, with one surprise: `NumData::lookup` stays an **outlined call per element** despite `#[inline]` → (ii) follow-up filed | [below](#2-decode_numeric_column-gather-path) |
| 3 | `DataChunk::select_decoded` (+ `select_numeric`) | "branchless straight-line loop … compiler is free to auto-vectorise (NEON/AVX)" | **(ii) NOT vectorized — claim does not hold today.** The loop-invariant `VecCmp` match is dispatched through a **jump table every iteration**, and the conditional `sel.push(r)` (compress-store) blocks vectorization even when manually unswitched. Fixable locally by a kernel shape change; follow-up filed, doc overstatement flagged | [below](#3-select_decoded--the-headline-finding), `asm/x86-64-baseline/audit_select_decoded.s`, `testcases/select_unswitch.rs` |
| 4 | `probe_emit` / `probe_gather_indices` raw-hash probe (`crates/sparq-substrate/src/join.rs`) | bounds-check elision on the probe hot path | **mostly (i) CONFIRMED**: `key_hash` + hashbrown SIMD group-probe fully inlined, `% JOIN_PARTS` strength-reduced to `and $63`, posting-list `extend` lowered to a vectorized copy. Two non-elided spots: the `tables[h % 64]` partition index keeps a bounds check + panic path, and `Key` equality is an **outlined indirect `bcmp` call** even for single-id (4-byte) keys → (ii) follow-up filed | [below](#4-hash-join-probe), `asm/x86-64-baseline/audit_probe_gather_indices.s` |
| 5 | `Dict::find_iri` memcmp path (`crates/sparq-core/src/dict.rs`, default features — no `mmap`) | bounds-check elision / memcmp lowering in the candidate-compare | **(i) CONFIRMED sane**: prefix and suffix comparisons both lower to `bcmp` (the memcmp path is real); hashbrown group-probe inlined. Remaining checks (`prefixes[pid]` bound, one-byte UTF-8 boundary test for `iri[p.len()..]`) guard **data-dependent** indices decoded from stored bytes — not elidable without changing the storage invariant, each a predictable cmp+branch | [below](#5-dictfind_iri), `asm/x86-64-baseline/Dict_find_iri.s` |

**Bottom line:** 4 of 5 audited surfaces are confirmed fine (the bead's stated
expectation). The one real gap is the **`select_decoded`/`select_numeric`
auto-vectorization claim**, which is *not* a rustc/LLVM missed-optimization worth
upstreaming (class iii) but a kernel-shape issue (class ii) — see the testcase
analysis below for why, with the control experiment that pins the blame.

---

## 1. `decode_numeric_column` inline fast path

Baseline x86-64 (`asm/x86-64-baseline/audit_decode_numeric_column.s`, `.LBB17_29`):
the `id - INLINE_BASE → f64` map is vectorized with the classic SSE2
magic-constant u32→f64 sequence, 4 elements/iteration, behind a runtime
alias check LLVM inserts between the borrowed id column and the fresh output:

```asm
.LBB17_30:
        movsd   (%rbp,%rcx,4), %xmm3
        movsd   8(%rbp,%rcx,4), %xmm4
        xorpd   %xmm0, %xmm3          # flip sign bit == subtract INLINE_BASE (1<<31) bias
        xorpd   %xmm0, %xmm4
        unpcklps %xmm1, %xmm3
        orpd    %xmm2, %xmm3          # magic-constant u32 -> f64
        subpd   %xmm2, %xmm3
        ...
        movupd  %xmm3, (%r14,%rcx,8)
        movupd  %xmm4, 16(%r14,%rcx,8)
```

x86-64-v3 (`asm/x86-64-v3/audit_decode_numeric_column.s`, `.LBB17_32`): AVX2
form, 16 elements/iteration (`vpxor` bias-flip + `vpmovzxdq`/`vpor`/`vsubpd`
u32→f64 into four ymm stores) — the "AVX2 genuinely unlocks the autovectorized
loops" premise of the dist microarch tiers
(`research/hardware/optimization-findings.md`) is real for THIS kernel. The `col.iter().all(|&id| is_inline(id))`
prescan (`.LBB17_2`) is a scalar 4-byte-load + compare + early-exit loop — LLVM
does not vectorize multi-exit loops; this is expected and branch-predictable, as
the source comment says.

## 2. `decode_numeric_column` gather path

The general path is, as documented, a scalar per-element gather. The surprise is
that `Graph::numeric_value`'s inline-integer test is inlined but the cache probe
is **an outlined call per element** — despite `NumData::lookup` being `#[inline]`
(LTO cost model declined; the enum-match over Owned/Sparse/Mapped/Forked bodies
is big):

```asm
.LBB17_11:
        movl    (%rbp,%r12,4), %esi
        ...                            # inline-range test (inlined)
        jne     .LBB17_9
        movq    %r15, %rdi
        callq   _ZN10sparq_core7NumData6lookup17h832debf515382f5bE.702   # per element
```

Same shape inside `select_numeric` (`asm/x86-64-baseline/audit_select_numeric.s`
`.LBB18_5`). Call overhead per non-inline element on top of the intended cache
probe → follow-up bead to measure forcing the `Owned` fast case inline.

## 3. `select_decoded` — the headline finding

Baseline asm (`asm/x86-64-baseline/audit_select_decoded.s`): ONE scalar loop
whose body dispatches the loop-invariant `VecCmp` through an indirect jump
**every iteration** (perfectly predicted, but it defeats the vectorizer), plus a
capacity-check + `grow_one` branch per push despite the up-front
`with_capacity`:

```asm
.LBB16_7:
        movsd   (%r14,%r13,8), %xmm1
        jmpq    *%r12                  # per-iteration dispatch of the invariant match
.LBB16_15:
        ucomisd %xmm0, %xmm1           # one scalar compare per element
        jbe     .LBB16_16
        ...
.LBB16_12:
        cmpq    8(%rsp), %rbp          # Vec capacity check per push
        jne     .LBB16_14
        ...
        callq   ..RawVec..grow_one..
```

**Why this is class (ii), not a class (iii) upstream report** — minimized in
[`testcases/select_unswitch.rs`](./testcases/select_unswitch.rs) (plain rustc
1.88, `-Copt-level=3 -Ccodegen-units=1`, no LTO — reproduces exactly):

- `select_a` (verbatim shape): jump table stays inside the loop → scalar.
- `select_b` (match manually unswitched into 5 dedicated loops): **still
  scalar** on both baseline and v3 — the conditional `sel.push(r)` is a
  compress-store, which LLVM will not auto-vectorize without AVX-512
  `vpcompress`-class hardware, and the potentially-allocating push keeps a call
  in the loop body.
- `count_a` (same invariant match, conditional push removed —
  `filter(..).count()`): LLVM unswitches the match AND vectorizes every arm
  (`cmpltpd/cmpeqpd + psubq` on SSE2; ymm forms on v3).

So LLVM's behavior is internally consistent: it unswitches when the resulting
loop is worth it and declines when the body contains the allocating early-exit
push — a cost-model call, not a missed optimization with a crisp upstream case.
The *kernel shape* is what forecloses vectorization. Local options for a
follow-up (measure-first, in cost order): manual unswitch (removes the
per-iteration indirect jump; modest), a branchless write-index kernel
(`sel[k] = r; k += test as usize` over a pre-sized buffer — removes the per-push
capacity branch; `forbid(unsafe)`-compatible via truncate), or a two-phase
bitmask kernel (vectorized compare into a mask column — the shape `count_a`
proves vectorizes — then a scalar mask→indices expansion). Filed as a follow-up
rather than implemented here; the chunk.rs doc comments overstating "the
compiler is free to auto-vectorise" this kernel are flagged in the same
follow-up.

`select_numeric` (also commented "free to auto-vectorise the value test") is
scalar for the same reasons plus the per-element outlined cache call (§2); its
module-level docs already concede the gather path is not vectorizable, so this
is primarily a comment-accuracy fix.

## 4. Hash-join probe

`asm/x86-64-baseline/audit_probe_gather_indices.s` (identical structure in
`audit_probe_emit.s`):

- `key_hash` (FxHasher) fully inlined — the single-hash design is real: one hash
  feeds both the partition select and `raw_entry().from_hash`.
- `% JOIN_PARTS` strength-reduced: `andl $63, %edi`.
- hashbrown's SSE2 group probe inlined (`movdqu` + `pcmpeqb` + `pmovmskb`).
- `build_indices.extend(matches)` lowers to a reserve + 2×16-byte/iter copy loop.

Two non-elided spots, both per-probe-row scale (not per-match):

```asm
        andl    $63, %edi
        cmpq    %r12, %rdi
        jae     .LBB20_63              # tables[h % 64] bounds check
...
.LBB20_63:
        callq   _ZN4core9panicking18panic_bounds_check...
```

The slice length isn't provably 64, so the check stays — one predictable
cmp+branch per probe row; cheap, but it *is* an unelided bounds check on the
raw-hash path the bead asked about. And key equality (`*k == key` on
`SmallVec<[Id; 2]>`) compiles to a length compare + **outlined indirect `bcmp`
call** per candidate — for the dominant single-column join key that is an
indirect call to compare 4 bytes:

```asm
        movq    bcmp@GOTPCREL(%rip), %r12
...
        callq   *%r12                  # bcmp for a (typically) 4-byte key
```

Follow-up filed to measure a specialized single-id key compare (the same
single-column axis as the earlier `#1810` key-projection finding, which
specialized construction but not comparison). `JoinKeys::right_key` is also
outlined per row (SmallVec collect machinery); same follow-up covers it.

## 5. `Dict::find_iri`

`asm/x86-64-baseline/Dict_find_iri.s` (default features — the `mmap` branch is
compiled out; `mapped_find` is not audited here):

- hashbrown group probe inlined; the frozen-base fallback is a self-recursive
  call (one level by invariant).
- The candidate compare (`stored_is_iri` / `tabled_is_iri` through the blob
  base): prefix compare lowers to `bcmp`, the `iri[p.len()..]` slice needs only
  a **single-byte UTF-8 boundary test** (`cmpb $-64, (%r10,%r14)`), and the
  suffix compare is a second `bcmp`. That is exactly the intended memcmp path.
- Remaining checks — `prefixes[prefix_id]` bounds, blob-header byte-ladder
  guards, boundary test — all index by values decoded from stored bytes, which
  LLVM cannot prove in range; eliding them would mean trusting stored data
  (an `unsafe` invariant change, not a shape fix). Each is a predictable
  cmp+branch dominated by the adjacent `bcmp` work. **No action recommended.**

## Follow-ups filed

Filed as deduplicated follow-up issues by the worker (class-(ii) shape
opportunities + doc honesty), not implemented here:

1. Vectorizable selection-kernel shape for `select_decoded`/`select_numeric` +
   chunk.rs comment accuracy (§3).
2. `NumData::lookup` outlined per-element call in the decode/select gather (§2).
3. Single-column join-key comparison specialization (outlined `bcmp` + unelided
   partition-index bounds check) (§4).

## Repro quickstart

```sh
# post-LTO asm, baseline tier
cargo rustc --release --manifest-path bench/codegen-audit/asm-probe/Cargo.toml -- --emit asm
# post-LTO asm, v3 tier (RUSTFLAGS, NOT a leaf-only flag — see the caveat above)
RUSTFLAGS='-Ctarget-cpu=x86-64-v3' cargo rustc --release \
  --manifest-path bench/codegen-audit/asm-probe/Cargo.toml -- --emit asm
# minimized select-kernel testcase
rustc -Copt-level=3 -Ccodegen-units=1 --emit asm bench/codegen-audit/testcases/select_unswitch.rs
```
