<!-- [OPUS-4.8] sq-toze.6 / cert gap GX-5 (epic sq-toze). Authored while Fable
     unavailable — re-review when Fable returns. -->

# `unsafe` Rust justification register

**Certification gap GX-5** (production-hardening / memsafety framework + ASVS).
Modelled on the `unsafe-rust-attestation` pattern and the prod-solid-server
compliance discipline: every `unsafe` site in the workspace's first-party crates is
enumerated here with the invariant it relies on, why that invariant holds, and how it
is tested or bounded. `scripts/unsafe-gate.py` (the unsafe-count **ratchet**) mechanically
bounds the per-crate **count** so no PR can add an `unsafe` site without a reviewed re-seed —
see [§ Ratchet](#ratchet--how-this-stays-honest). Matching each counted site to a **row** here
(and keeping the file:line current) is still a **review-time** obligation, not a mechanical one:
sq-vopxw found a stale duplicate `sparq-vectors` row that the count ratchet could not see. [OPUS-5]

> This register covers **first-party `unsafe` only** (the `crates/` tree). Third-party
> `unsafe` (e.g. inside `memmap2`, `libc`, `rayon`, the `hdt` dependency) is out of
> scope here and is governed by the supply-chain lane (`cargo-deny`, SBOM/VEX, PR #210)
> and the upstream crates' own audits.

## Scope & threat model

The load-bearing boundary is **B5** in [`research/threat-model.md`](../../research/threat-model.md):
a *hostile on-disk index file → mmap loader → `unsafe` pointer reinterpret*. The
register distinguishes two trust classes of `unsafe`:

- **Trusted-input `unsafe`** — the pointer/slice/byte reinterprets whose backing was
  produced by this process (a `Vec`/slice we just built, or a file we wrote and own for
  the call's lifetime). Soundness rests on type/POD + alignment + length invariants
  that hold by construction.
- **Untrusted-input `unsafe`** (the B5 surface) — the mmap loaders that map a `.spq` /
  `dict-*.bin` / `.spqv` / DiskANN file that may be **hostile or corrupt**. These are
  **validated at open** before any unchecked access: `Dict::open_mmap → MappedDict::validate`
  (sq-znld), `VectorStore::open_validated`, and the DiskANN `open` validator bound every
  offset/length, and the hot read path uses bounds-/UTF-8-**checked** accessors
  (`rd_str_checked`) — the `from_utf8_unchecked` fast paths are reachable **only** for
  records this process built or already validated.

### How each site is bounded (test / lint coverage)

| Mechanism | Covers |
|---|---|
| `miri` lane (`.github/workflows/miri.yml`, sq-fo28) | pure-Rust unsafe in `sparq-core` reachable without `mmap`/`dict-spill`: the parallel scatter writes, POD↔bytes reinterprets, `from_utf8_unchecked` over in-memory buffers, `MaybeUninit`+`set_len`. Runs nightly under Tree-Borrows for aliasing/provenance/data-race UB. |
| `mmap_corruption_oracle` test (`crates/sparq-core/tests/`, run under `--features mmap,dict-spill`) + `fuzz` lane's mmap-loader target | the **B5** mmap sites Miri structurally cannot run (file-backed mappings): hostile/corrupt index → loader must reject or stay in-bounds, never UB. |
| `#![forbid(unsafe_code)]` across the safe-only crates (sq-emay) | proves the unsafe surface is confined; a new `unsafe` in those crates fails to compile. |
| `scripts/unsafe-gate.py --check` (this PR, GX-5) | the count **ratchet** — a PR cannot add `unsafe` without updating this register + re-seeding the snapshot. |
| `vectors-aarch64` lane (`.github/workflows/vectors-aarch64.yml`, #5028) [SONNET-4.6] | the **aarch64/NEON** `#[target_feature]` sites in `sparq-vectors`' `simd.rs`. Their guard (`simd::tests`) calls the *dispatcher*, so it is arch-generic and verifies whichever kernel the HOST selects — and every other lane that runs these tests is `ubuntu-latest` (x86_64). Before this lane the NEON kernels were compile-checked but never EXECUTED in CI, so their numeric-agreement argument rested on a test that had not run on the arch it was about. The lane runs the crate's suite on `ubuntu-24.04-arm` and fails closed if the host is not aarch64+`asimd` or if `mod simd` was not compiled into the run. |
| `// SAFETY:` argument on every site, **enforced by `clippy::undocumented_unsafe_blocks`** | the per-site argument lives next to the code. The lint is set at crate root on unsafe-bearing library crates (including `sparq-engine`) and locally on unsafe-bearing test/example crates, so the existing `clippy --all-targets -D warnings` gate **mechanically** rejects any `unsafe` block/impl that lacks a literal `// SAFETY:` comment. Combined with the **register + review + count ratchet** (a new `unsafe` cannot land without a register row, the `// SAFETY:` comment, and a re-seed). Closes gap MS-G2 (sq-8wbn) in [`gap-register.md`](./gap-register.md). [OPUS-4.8] |

## Register

**92 `unsafe` sites** across 9 crates (the other crates contain no first-party `unsafe`).
Counts and the file:line list are produced by `scripts/unsafe-gate.py --list` and
must equal `bench/unsafe-snapshot.json`. Two crates are special allocator cases:
**`sparq-lws-core`** (sq-gg0qq.2) ships a `forbid(unsafe_code)` lib + bin
with its 8 sites confined to two **example benchmark harnesses'** counting allocators, which
a custom `#[global_allocator]` unavoidably requires; **`sparq-lws-wasm`** (sq-wubkf) is the one
crate whose 4 allocator sites are **SHIPPING** — a `#[global_allocator]` is the only way to bound
wasm32 linear memory, so that crate's root is `deny(unsafe_code)` with a single
`#[allow(unsafe_code)]` module rather than `forbid` (see each crate's subsection below).

Recurring invariant shorthands used below:
- **POD-bytes** — `[u32;3]` / `u32` / `u64` / `f64` are plain-old-data with no invalid
  bit patterns; reinterpreting `&[T] ↔ &[u8]` only reads/writes valid bytes; `size_of_val`
  bounds the length exactly.
- **page-align** — an mmap base is page-aligned (≥ any of the 4/8-byte scalar alignments),
  and the file is a whole number of fixed-width records, so `from_raw_parts(base.cast::<T>(), len/size_of::<T>())` is aligned and in-bounds.
- **own-for-lifetime** — the mapped/opened file is owned by the structure for the
  borrow's duration and is not mutated by us; external concurrent mutation is explicitly
  out of contract (documented stance, same as the rest of the mmap surface).

### `sparq-core` — 51 sites (the unsafe core: mmap loaders, zero-copy dict, parallel build, N-Triples span decode)

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/lib.rs:285` | slice reinterpret (read) | page-align; `numerics.bin` is whole f64 | mmap base ≥ 8-byte f64 align; `n = len/8`. Mapped-file open validates size == `dict.len()*8`. |
| `src/lib.rs:389` | ptr read | page-align; instant section is `n` f64 at offset 0 | `i < n` checked at the call; f64 at `base+i`. |
| `src/lib.rs:456` | slice reinterpret (read) | page-align; instants are `n` f64 at offset 0 | `n = mapped_len`; materialises the cells. |
| `src/lib.rs:577` | slice reinterpret (write) | POD-bytes | reinterpret the f64 column as bytes to write `temporals.bin`. |
| `src/lib.rs:1584` | `Mmap::map` | own-for-lifetime | `numerics.bin` opened only if `size == dict.len()*8` (length pre-validated). |
| `src/lib.rs:1592` | `Mmap::map` | own-for-lifetime | `temporals.bin` opened only if `size == dict.len()*9` (length pre-validated). |
| `src/lib.rs:1885` | slice reinterpret (read) | page-align; perm0 is whole `[u32;3]` rows | `n` from `map_perm`; map outlives the loop. Written by us above. |
| `src/lib.rs:2303` | slice reinterpret (read) | page-align; perm0 is whole `[u32;3]` rows | same as 1885 (external-build path). |
| `src/lib.rs:3728` | slice reinterpret (write) | POD-bytes | reinterpret the f64 numerics cache as bytes to write `numerics.bin`. |
| `src/lib.rs:3761` | slice reinterpret (write) | POD-bytes | (sq-7ph8) `stream_write_numerics` flush: reinterprets a reusable `Vec<f64>` BLOCK as bytes for `write_all`. (a) `buf` is a live `Vec<f64>` of `buf.len()` elems; `size_of_val(&buf[..]) = len*8` covers the initialised contiguous region exactly. (b) target `u8` has align 1; the f64 source is over-aligned — no misalignment. (c) bytes are only READ (passed to `write_all`), never written through the alias. (d) the `&[u8]` is consumed inside the closure before `buf.clear()`; no provenance/lifetime escape past the source borrow. (e) NATIVE-endian reinterpret, identical to `write_numerics` (3728) it replaces and symmetric with the native-endian READ at `NumData::as_slice` (285): write-native + read-native round-trips on the same arch (the established cache contract; the cache is rebuilt, never shipped cross-arch). Test `streamed_caches_byte_identical_to_dense` asserts byte-identity to the dense write. **GX-5**. [OPUS-4.8] |
| `src/lib.rs:3805` | slice reinterpret (write) | POD-bytes | (sq-7ph8) `stream_write_temporals` flush_f: reinterprets a reusable `Vec<f64>` instant BLOCK as bytes for `write_all`. Same invariants (a)–(e) as 3761: full-length `len*8` byte view of a live `Vec<f64>`, `u8` align 1, read-only, no escape, native-endian — symmetric with the native-endian temporal read (`temporals.bin` first `n` f64; rows 285/389/456) and byte-identical to `write_temporals` (577). The trailing flag-byte column is written from a `Vec<u8>` (no unsafe). **GX-5**. [OPUS-4.8] |
| `src/lib.rs:3934` | `_mm_prefetch` (x86_64) | hint-only | prefetch is defined for any address; the hint is dropped on a bad one — cannot fault. |
| `src/lib.rs:3939` | `prfm` asm (aarch64) | hint-only | `prfm pldl1keep` is a hint; `nostack, preserves_flags`; cannot fault or write memory/regs. |
| `src/lib.rs:3994` | ptr `add` (prefetch arg) | `id-1 < remap.len()` for every dict id | only computes an address for the hint-only `prefetch_read`; never dereferenced here. |
| `src/lib.rs:5018` | ptr `add` (prefetch arg) | `id-1 < remap.len()` | same as 3994 (other build path). |
| `src/lib.rs:5175` | `MmapMut::map_mut` | own-for-lifetime; freshly-written perm file | read-write map of a perm file of whole `[u32;3]` rows we just wrote. |
| `src/lib.rs:5179` | mut slice reinterpret | page-align; POD-bytes | `len/4` u32; exclusively owned (the `MmapMut`); rayon writes are disjoint by index. |
| `src/lib.rs:4172` | slice reinterpret (read) | page-align; whole `[u32;3]` rows; map outlives the slice | `compress_perm_file_in_place` (sq-vkz7). `extsort::map_perm` returns `(Mmap, n)` with `n = len/12`, so the `n` `[u32;3]` rows are fully in-bounds and the base is ≥ 4-byte u32-aligned (page-aligned mmap). The raw SPO perm was written by this build (whole rows, sorted+deduped) and is read-only here — the `Mmap` (`map`) is held for the whole row-streaming loop and is not mutated through any other alias for the slice's lifetime; it is `drop`ped only *after* the slice's last use, before the file is renamed. `n==0` (empty perm) returns early without forming the slice. [OPUS-4.8] |
| `src/lib.rs:7668` | `std::env::set_var` | single-threaded test; var restored before return | TEST-only (`#[test] external_quads_fd_*`): sets `SPARQ_QUADS_SPILL_MAX_OPEN` to exercise the bounded-writer-pool path; no other thread reads the env in the test. Edition-2024 made `set_var` `unsafe`. |
| `src/lib.rs:7672` | `std::env::remove_var` | same test; restores the env | TEST-only: removes the var set at 7668 before returning so the process env is left clean. Edition-2024 `unsafe`. |
| `src/store.rs:106` | slice reinterpret (read) | page-align; whole `[u32;3]` triples | `n = len/12`; mmap base ≥ 4-byte u32 align. |
| `src/store.rs:375` | slice reinterpret (write) | POD-bytes | reinterpret contiguous `[u32;3]` rows as bytes for `std::fs::write`. |
| `src/store.rs:459` | `Mmap::map` | own-for-lifetime | per-permutation file; empty (size 0) skipped; format auto-detected after. **B5**. |
| `src/dict.rs:483` | `from_utf8_unchecked` | bytes came from a `&str` we wrote / a validated dict | TRUSTED path only — the untrusted mmap path uses `rd_str_checked` and is validated at open (sq-znld). |
| `src/dict.rs:736` | slice reinterpret (read) | page-align; LE u64 array | `len/8`; mmap base ≥ 8. **B5** (validated at open). |
| `src/dict.rs:751` | slice reinterpret (read) | page-align; LE u32 array | `len/4`; mmap base ≥ 4. **B5** (validated at open). |
| `src/dict.rs:1831` | `Mmap::map` | own-for-lifetime | read-only map of a dict section file. **B5** (`MappedDict::validate` runs after). |
| `src/dict.rs:2141` | slice reinterpret (write) | POD-bytes (`T: Copy`, u32/u64) | `write_pod_slice` reinterprets the POD array as bytes for `fs::write`. |
| `src/dict.rs:2391` | `unsafe impl Send for SlotPtr` | `SlotPtr` (`*mut Id`) only ever touches its own disjoint slot | the parallel scatter routes each `(shard,id)` slot to exactly one shard — no aliasing, no reads until the scope ends. |
| `src/dict.rs:2394` | `unsafe impl Sync for SlotPtr` | same as 2391 | shared read of `Copy` raw-pointer handles; the writes they perform are disjoint. |
| `src/dict.rs:2412` | ptr `write` (scatter) | `i < len`; slot owned by this shard | bounded by `assert!`/`debug_assert!` on `lid<stride` & `i<len`; covered by `miri`. |
| `src/dict.rs:2611` | `Vec::set_len` | all `total` slots initialised exactly once | the shard slices partition `[0,total)` and each fills its slice fully; covered by `miri`. |
| `src/dictspill.rs:119` | `sysconf` FFI | pure query | `_SC_PHYS_PAGES`/`_SC_PAGE_SIZE`; negative ⇒ "unsupported", handled. |
| `src/dictspill.rs:139` | `statvfs` FFI | zeroed out-param + NUL-terminated path; checked return | `statvfs` is a plain C struct (sound to zero-init); path from a validated `CString`; return checked. |
| `src/dictspill.rs:223` | `from_utf8_unchecked` | bytes written from a `&str` in `serialize_termparts` | round-trips our own serialisation — never untrusted input. |
| `src/dictspill.rs:228` | `from_utf8_unchecked` | same as 223 (the `rest` tail) | our own serialisation. |
| `src/dictspill.rs:723` | `unsafe impl Send for SlotPtr` | `*mut u64` only touches its own hash-routed slot | disjoint writes, no reads until the parallel scope ends. |
| `src/dictspill.rs:726` | `unsafe impl Sync for SlotPtr` | same as 723 | `Copy` handle shared; writes disjoint. |
| `src/dictspill.rs:741` | ptr `write` (scatter) | `i < len`; slot hash-routed to this shard | `debug_assert!(i<len)`; disjoint by hash routing. (`dict-spill` feature ⇒ outside the `miri` lane; covered by the deterministic corruption oracle + fuzz.) |
| `src/nt.rs:525` | `from_utf8_unchecked` | **`CHUNK-UTF8`** — the chunk buffer was proved valid UTF-8 by `validate_chunk_utf8` at the entry point, and the span ends are ASCII delimiters, hence character boundaries | The `nt` module has exactly TWO entry points (`parse_chunk`, `parse_quads_chunk`) and both validate the whole chunk on their first line, before any byte is scanned; every span this module cuts is bounded by a byte the scanner matched against an ASCII constant (`<` `>` `"` `\` `_` `:` `.` `@`, whitespace, `is_ascii_alphanumeric()`), and UTF-8 is self-synchronising, so those indices are character boundaries. Bounded by: `debug_assert!` re-check of the full precondition on EVERY call in debug/test builds; the `miri` lane (this module is default-features, no mmap — squarely in scope); `parse_chunk_rejects_invalid_utf8` / `parse_quads_chunk_rejects_invalid_utf8` pin the entry-point validation, `multibyte_spans_at_every_delimiter_round_trip` / `escaped_and_multibyte_literal_span_round_trips` / `language_tag_terminated_by_multibyte_errors_cleanly` pin the boundary argument at each delimiter. NOT a **B5** site: the input is an in-memory chunk that is fully validated in-process before any unchecked access, so hostile input yields a clean `Err` with a byte offset, never UB. See the full `CHUNK-UTF8` argument in the source doc-comment. (sq-7d3dj.21) [OPUS-5] |
| `src/extsort.rs:15` | slice reinterpret (write) | POD-bytes | `as_bytes` over `[Id;3]` for writing a run. |
| `src/extsort.rs:38` | `unchecked_advise_range` | read-only map, range already consumed | `DontNeed`/`FreeReusable` only drops the resident copy of clean file pages; never read again. |
| `src/extsort.rs:85` | `Mmap::map` | own-for-lifetime | run files written by us, not mutated during the merge. |
| `src/extsort.rs:100` | slice reinterpret (read) | page-align; whole `[u32;3]` triples | `n = len/12` per run. |
| `src/extsort.rs:232` | `Mmap::map` | own-for-lifetime | `map_perm` read-only map of a perm file we own for the call. |
| `src/compress.rs:646` | `Mmap::map` | own-for-lifetime (TEST) | TEST-only (`#[test] stream_writer_byte_identical_to_encode_write_to`, sq-vkz7): read-only map of a perm file the test just created and owns; `File`/`Mmap` live for the map's whole scope and nothing else mutates the file during the test, so the subsequent `CompressedPerm::from_mmap` read stays in-bounds over a stable region. [OPUS-4.8] |
| `src/compress.rs:1800` | `Mmap::map` | own-for-lifetime (TEST) | TEST-only (`#[test] v2_reader_ships_with_mmap_roundtrips`, sq-7d3dj.32.2.7): read-only map of a temp perm file the test itself just created and owns; `File`/`Mmap` live for the map's whole scope and nothing else mutates it during the test, so the `from_mmap` read stays in-bounds over a stable region. [FABLE-5] |
| `src/compress.rs:1987` | `Mmap::map` | own-for-lifetime (TEST) | TEST-only (`#[test] write_to_v2_roundtrips_through_from_mmap`, sq-7d3dj.32.2.7): read-only map of a temp perm file the test itself just created and owns; `File`/`Mmap` live for the map's whole scope and nothing else mutates it during the test, so the `from_mmap` read stays in-bounds over a stable region. [FABLE-5] |
| `src/compress.rs:2017` | `Mmap::map` | own-for-lifetime (TEST) | TEST-only (`#[test] v1_file_decodes_byte_identically_forever`, sq-7d3dj.32.2.7): read-only map of a temp perm file the test itself just created and owns; `File`/`Mmap` live for the map's whole scope and nothing else mutates it during the test, so the `from_mmap` read stays in-bounds over a stable region. [FABLE-5] |
| `src/compress.rs:2104` | `Mmap::map` | own-for-lifetime (TEST) | TEST-only (`#[test] corrupt_magic_is_loud_error_never_misdecode`, sq-7d3dj.32.2.7): read-only map of a temp perm file the test itself just created and owns; `File`/`Mmap` live for the map's whole scope and nothing else mutates it during the test, so the loud-error `from_mmap` read stays in-bounds over a stable region. [FABLE-5] |
| `src/compress.rs:2120` | `Mmap::map` | own-for-lifetime (TEST) | TEST-only (`#[test] corrupt_magic_is_loud_error_never_misdecode`, sq-7d3dj.32.2.7): read-only map of a second temp perm file the same test created and owns; `File`/`Mmap` live for the map's whole scope and nothing else mutates it during the test, so the `from_mmap` read stays in-bounds over a stable region. [FABLE-5] |

### `sparq-vectors` — 12 sites (aligned vector blobs + the ONE shared mmap backing for `.spqv`/`.spqg` + the SIMD ANN kernel)

(sq-vopxw.) This section previously listed **13** rows against a live/snapshot count of **12**.
The extra row was a stale duplicate: `VectorStore::open` and `DiskAnnIndex::open` used to each
carry their own `Mmap::map`, and sq-98c unified both behind the single `store::open_backing`
helper + the `Bytes` backing enum, so the `.spqv` and `.spqg` loaders now share **one** map site
(`store.rs:285`). On `wasm32` (memmap2 target-gated out) `open_backing` takes the owned
`AlignedBytes` branch instead, which contains no `unsafe` of its own. Every file:line below was
re-derived from `scripts/unsafe-gate.py --list`. [OPUS-5]

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/store.rs:213` | mut slice reinterpret | `words` is u32-aligned, holds ≥ `len` bytes; region exclusively owned | `AlignedBytes::from_vec` over-allocates to a word boundary (`len.div_ceil(4)` u32s); `copy_from_slice` fills exactly `len` bytes of the freshly-allocated, unaliased buffer. |
| `src/store.rs:221` | slice reinterpret (read) | u32-aligned, ≥ `len` initialised bytes | `AlignedBytes::as_bytes` reads the bytes copied in above; base ≥ 4-byte aligned by construction (review 1874). |
| `src/store.rs:285` | `Mmap::map` (`open_backing`) | own-for-lifetime | the SINGLE read-backing map site, shared by `VectorStore::open` (`.spqv`) and `DiskAnnIndex::open` (`.spqg`); `open_validated` bounds every offset afterwards. Native-only — wasm32 takes the owned-bytes branch. **B5**. |
| `src/store.rs:773` | slice reinterpret (write) | f32 has no invalid bit patterns; `align(f32) ≥ align(u8)` | `finalize`: the f32 data section → LE bytes for `write_all`; big-endian hosts are rejected by the WRITER (`create`/`create_inner`). The READER accepts them: `open_validated` validates the complete little-endian structure first and only then byte-swaps the dense f32 words into owned aligned storage (sq-i7w), so no `unsafe` site gains a new precondition. |
| `src/store.rs:1059` | slice reinterpret (read) | `start` is a multiple of 4 ⇒ f32-aligned; range validated in `open` | `slot_vector`: the backing is u32-aligned for BOTH branches — page-aligned map, or `AlignedBytes` (review 1874 fixed a UB align bug here); `debug_assert_eq!` checks it; f32 accepts any bit pattern. **B5**. |
| `src/store.rs:1579` | slice reinterpret (write) | f32 no invalid patterns; align ok | the streaming builder's `put`: f32→LE bytes for `write_all` after `validate_vector`. |
| `src/diskann.rs:461` | slice reinterpret (read) | f32 no invalid patterns; align ok | build path: f32→LE bytes copied into the fixed-width record; LE target asserted; borrows `b.vectors`. |
| `src/diskann.rs:808` | slice reinterpret (read) | `start` a multiple of 4 ⇒ f32-aligned; range validated in `open_validated` | `node_vector`: `debug_assert_eq!` checks alignment; both backings are ≥ 4-byte aligned; f32 accepts any bit pattern; borrows the backing. **B5**. |
| `src/simd.rs:96` (`approx-ann`) | `#[target_feature(enable="neon")]` call | the `neon` ISA extension is present at runtime | entered ONLY when `active_kernel()` answers `Neon`, which it does ONLY inside `if is_aarch64_feature_detected!("neon")`; `l2_sq_neon` reads exactly `a.len()==b.len()` lanes. [OPUS-4.8] sq-lfo84 |
| `src/simd.rs:106` (`approx-ann`) | `#[target_feature(enable="avx2,fma")]` call | both `avx2` and `fma` are present at runtime | entered ONLY when `active_kernel()` answers `Avx2`, which it does ONLY inside `if is_x86_feature_detected!("avx2") && …("fma")`; `l2_sq_avx2` reads exactly `a.len()` lanes via unaligned loads. [OPUS-4.8] sq-lfo84 |
| `src/simd.rs:142` (`approx-ann`) | `unsafe fn l2_sq_neon` (NEON L2² kernel) | caller confirmed `neon`; `a.len()==b.len()` | 16-wide FMA body + 4-wide drain + scalar tail (`get_unchecked` only for `i<len`), so every `vld1q_f32` load is in-bounds. Verified vs an f64 reference for dim 0..=257 (`simd::tests`), **executed on a real aarch64 host** by the `vectors-aarch64` lane (#5028) — before that lane this kernel was compile-checked only, since every test lane was x86_64. [OPUS-4.8] sq-lfo84 [SONNET-4.6] #5028 |
| `src/simd.rs:185` (`approx-ann`) | `unsafe fn l2_sq_avx2` (AVX2+FMA L2² kernel) | caller confirmed `avx2`+`fma`; `a.len()==b.len()` | 16-wide FMA body + 8-wide drain + scalar tail (`get_unchecked` only for `i<len`); `_mm256_loadu_ps` is unaligned so no alignment precondition. Numeric output verified by `simd::tests` on the x86_64 CI runner — `ci.yml`'s nextest matrix is `ubuntu-latest` (the aarch64 work box sq-lfo84 was authored on cannot execute AVX2). Those guards are arch-GENERIC and the scalar fallback satisfies them, so that evidence was previously compatible with the kernel never having run; `simd::tests::avx2_kernel_is_the_one_the_dispatcher_actually_ran` now asserts `active_kernel() == Avx2` and **fails closed under `CI`**, so a runner without AVX2+FMA reds the lane instead of silently supplying scalar-path evidence. [OPUS-4.8] sq-lfo84 [SONNET-4.6] #5065 |

### `sparq-cli` — 2 sites (the `dump-perm` debug command)

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/main.rs:492` | `Mmap::map` | own-for-lifetime | read-only map of a perm file held open for the call. |
| `src/main.rs:495` | slice reinterpret (read) | page-align; whole `[u32;3]` rows | `n = len/12`; `n==0` handled. CLI utility over a file the operator named. |

### `sparq-zk-compose` — 2 sites (cross-process advisory file lock)

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/verifier.rs:967` | `libc::flock(LOCK_EX)` | `fd` is a valid open fd owned by `file` for the call | the `MutexGuard` keeps `file` (hence `fd`) alive; an error fails closed (`return false`). |
| `src/verifier.rs:975` | `libc::flock(LOCK_UN)` | same valid, locked fd | unlock helper run on every return path so the advisory lock is never leaked (a leak would deadlock the next caller). |

### `sparq-bench` — 1 site (peak-RSS measurement; non-shipping bench binary)

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/main.rs:375` | `getrusage` FFI | `rusage` is a plain C struct (sound to zero-init); `getrusage(RUSAGE_SELF, &mut ru)` writes only into the valid out-param | return checked before any field read. Bench-only binary, not shipped. |

### `sparq-py` — 4 sites (opt-in PyO3 binding; the Arrow C-Data-Interface stream-capsule bridge) [OPUS-4.8]

(sq-lt1ml, gh-910.) The whole crate is `#![forbid(unsafe_code)]` **except**
`src/arrow_export.rs`, which is `#![allow(unsafe_code)]` with module docs and a `// SAFETY:`
comment on every site. The 4 sites are the single, canonical **Arrow C Data Interface**
producer pattern — exporting one already-materialised `RecordBatch` as an
`FFI_ArrowArrayStream` behind a `PyCapsule` named `arrow_array_stream` so `pyarrow.table(obj)`
ingests it directly (no re-serialise). The pattern is line-for-line the upstream
`pyo3::types::PyCapsule::new_with_pointer_and_destructor` **documented example** (pyo3 0.29:
`Box::into_raw` → `NonNull` → `'static` CStr name → an `unsafe extern "C"` destructor that does
`PyCapsule_GetPointer` + null-guard + `Box::from_raw`). Not a B5 (untrusted-input) surface: the
pointer is produced *and* reclaimed by this process; nothing from disk or the consumer is
dereferenced unchecked. The leak-free/double-free-free claim was verified against
`arrow-array`'s `FFI_ArrowArrayStream::Drop`, which runs `self.release` only while it is `Some`
and nulls it after — so whether or not pyarrow consumed (and thus released) the stream, our
capsule destructor's drop is idempotent.

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/arrow_export.rs:107` | `PyCapsule::new_with_pointer_and_destructor` (pyo3) | `ptr` is a freshly `Box::into_raw`'d, non-null, aligned `*mut FFI_ArrowArrayStream`; name is `'static`; destructor reclaims it | satisfies pyo3's documented `# Safety`: pointer valid for its use; data cleaned up by the destructor; destructor thread-safe (it only does `PyCapsule_GetPointer` + `Box::from_raw`). `NonNull::new(Box::into_raw(..)).expect(..)` guards null; called with the GIL held (`py: Python<'_>`). This is the only way to get the capsule's pointer to be the *value* pointer the C Data Interface mandates (`new_with_value` stores an internal box wrapper, not the value). |
| `src/arrow_export.rs:119` | `unsafe extern "C" fn release_stream_capsule` | invoked only by CPython on the capsule it was registered on | the capsule's C destructor. Declared `unsafe extern "C"` per the `PyCapsule_Destructor` ABI; pyo3 aborts (does not unwind) if it panics. Its body's two inner sites (123, 125) are individually justified. Runs with the GIL held. |
| `src/arrow_export.rs:123` | `ffi::PyCapsule_GetPointer` (CPython FFI) | `capsule` is the live capsule CPython passes; name matches the `'static` CStr it was created under | returns the leaked `*mut FFI_ArrowArrayStream` (or null on a name mismatch — guarded by the `!ptr.is_null()` check below). Raw CPython call, GIL held inside the destructor. |
| `src/arrow_export.rs:125` | `Box::from_raw` (reclaim) | `ptr` is exactly the pointer leaked at :107 via `Box::into_raw`, of the same type, same allocator | reconstitutes and drops the `Box<FFI_ArrowArrayStream>` exactly once per capsule (null-guarded, so never on a name mismatch). `FFI_ArrowArrayStream::Drop` is idempotent on an already-released stream (`release` is set `None` after the first call), so no double free / no leak whether or not pyarrow consumed it. |

### `sparq-engine` — 8 sites (cancellation pointer + test-only byte-counting allocator)

(sq-kq9ia.) Four shipped sites form one narrow cooperative-cancellation boundary. The
thread-local/rayon `Limits` snapshot must remain `Copy`, so it carries `CancelPtr`, a
`NonNull<AtomicBool>` borrowed from the `Arc<AtomicBool>` in `QueryBudget`. The returned
`Guard<'a>` carries `PhantomData<&'a QueryBudget>`, preventing the owning budget or its
last `Arc` from being dropped while the pointer is installed; `Guard::drop` clears
`ACTIVE` to `OFF` before the borrow can end. A second `PhantomData<Rc<()>>` makes the guard
non-`Send`, so it can only clear the thread-local state on the installing thread. Rayon
parallel iterators are scoped and join before that guard drops. The pointer is dereferenced
only for `AtomicBool::load(Relaxed)`:
the flag gates control flow and publishes no data, so no synchronisation edge is needed.
The direct 0-vs-1 test exercises both poll paths and the sticky `"cancelled"` reason; the
cross-thread test exercises visibility, and the same-thread second-query test witnesses
pointer cleanup. [GPT-5.6]

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/exec.rs:87` | `unsafe impl Send for CancelPtr` | `AtomicBool: Sync`; allocation remains live | moving the shared pointer to a worker is sound because `Guard<'a>` borrows the owning `QueryBudget` through every scoped rayon join and the pointer is only atomically loaded. |
| `src/exec.rs:90` | `unsafe impl Sync for CancelPtr` | `AtomicBool: Sync`; all shared access is atomic | no mutation or non-atomic access occurs through the pointer; the lifetime-bound guard keeps the allocation live. |
| `src/exec.rs:167` | `NonNull::as_ref` in `Limits::hit` | live, aligned `AtomicBool` from `Arc::as_ref` | `NonNull::from` preserves the reference's alignment/provenance; the snapshot cannot outlive the guard's scoped rayon work. Direct false/true assertions cover this poll. |
| `src/exec.rs:415` | `NonNull::as_ref` in `budget::exhausted` | live, aligned `AtomicBool` from `Arc::as_ref` | the installing thread holds `Guard<'a>` until the query returns, then clears the pointer before the borrowed budget may drop. Direct and cross-thread cancellation tests cover this poll. |

(sq-my8wd.4.) The other 4 sites live entirely in one integration test,
`tests/service_stream_bounded.rs`, which pins the DoS-relevant invariant of streaming
SERVICE consumption: a large/duplicate-heavy remote SPARQL result must be consumed in
memory **O(body)**, not O(parsed DOM + a term-level relation copy) — the old
collect-everything path amplified a remote result into a multiple of its wire size. The
test proves this with a **thread-local byte-counting allocator**, and a `#[global_allocator]`
unavoidably requires `unsafe` (the `GlobalAlloc` trait is `unsafe` by definition; there is
no safe substitute for a deterministic per-thread allocation counter). **Not** a B5
(untrusted-input) surface: every method is a
verbatim forward to the process `System` allocator with the identical arguments, so
`System` discharges all of `GlobalAlloc`'s obligations; the wrapper only reads
`Layout::size()`/`new_size` and mutates a thread-local `Cell<isize>` — no pointer `System`
returns is ever dereferenced, retained, or aliased. Bounded by **review + the trivial
forward-to-`System` argument** and enforced by the file-local
`#![warn(clippy::undocumented_unsafe_blocks)]` (every site carries a `// SAFETY:` comment);
Miri does not cover it (Miri supplies its own allocator and does not model a
`#[global_allocator]` that calls `System`), but the test itself runs on the standard
`cargo test -p sparq-engine --features service` lane.

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `tests/service_stream_bounded.rs:83` | `unsafe impl GlobalAlloc for Counting` | forward-to-`System` | every method delegates verbatim to `System` with the same args, so `System` upholds the trait contract; the wrapper adds only size reads + a thread-local counter. TEST-only. |
| `tests/service_stream_bounded.rs:87` | `unsafe fn alloc` | caller's `layout` contract forwarded | `System.alloc(layout)` unchanged; only the returned pointer's nullness is inspected before recording `layout.size()`. |
| `tests/service_stream_bounded.rs:98` | `unsafe fn dealloc` | `ptr` came from this allocator with this `layout` | `System.dealloc(ptr, layout)` unchanged; holds because every `alloc`/`realloc` also forwarded to `System`. |
| `tests/service_stream_bounded.rs:106` | `unsafe fn realloc` | caller's `ptr`/`layout`/`new_size` contract forwarded | `System.realloc(ptr, layout, new_size)` unchanged; only the returned pointer's nullness is inspected before recording the delta. |

### `sparq-lws-core` — 8 sites (example-only counting global allocators) [FABLE-5]

(sq-gg0qq.2 — crate imported whole from jeswr/solid-server-rs.) The **library and the
server binary** are `#![forbid(unsafe_code)]` and ship **zero** `unsafe`. These 8 sites
live entirely in two **example** benchmark harnesses (never the shipped server): the
deterministic allocation-count microbench `examples/read_response_alloc_microbench.rs`
(counts `GlobalAlloc::alloc`/`realloc` ops on the GET/HEAD read-response header path) and
the shared harness module `examples/support/mod.rs` (`#[path]`-included by the
`bench_harness` + `adversarial_bench` examples; counts allocation ops + bytes). A
`#[global_allocator]` unavoidably requires `unsafe` (the `GlobalAlloc` trait is `unsafe`
by definition; there is no safe substitute for a deterministic allocation counter) — the
same class as `sparq-engine`'s test allocator above. **Not** a B5 (untrusted-input)
surface: every method is a verbatim forward to the process `System` allocator with the
identical arguments, so `System` discharges all of `GlobalAlloc`'s obligations; the
wrappers only read `Layout::size()`/`new_size`/an armed flag and bump `Relaxed` atomics —
no pointer `System` returns is ever dereferenced, retained, or aliased. Bounded by
**review + the trivial forward-to-`System` argument**, enforced by the file-local
`#![warn(clippy::undocumented_unsafe_blocks)]` in both files (each `unsafe impl` carries a
`// SAFETY:` comment). Miri does not model a `#[global_allocator]` that calls `System`;
the examples run on the standard toolchain.

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `examples/read_response_alloc_microbench.rs:41` | `unsafe impl GlobalAlloc for CountingAlloc` | forward-to-`System` | every method delegates verbatim to `System` with the same args; the wrapper adds only an ARMED-flag read + a `Relaxed` counter bump. EXAMPLE-only; lib + bin stay `forbid(unsafe_code)`. |
| `examples/read_response_alloc_microbench.rs:42` | `unsafe fn alloc` | caller's `layout` contract forwarded | `System.alloc(layout)` unchanged; only the armed counter is touched before the forward. |
| `examples/read_response_alloc_microbench.rs:48` | `unsafe fn dealloc` | `ptr` came from this allocator with this `layout` | `System.dealloc(ptr, layout)` unchanged; holds because every `alloc`/`realloc` also forwarded to `System`. |
| `examples/read_response_alloc_microbench.rs:51` | `unsafe fn realloc` | caller's `ptr`/`layout`/`new_size` contract forwarded | `System.realloc(ptr, layout, new_size)` unchanged; only the armed counter is touched before the forward. |
| `examples/support/mod.rs:77` | `unsafe impl GlobalAlloc for CountingAllocator` | forward-to-`System` | every method delegates verbatim to `System` with the same args; the wrapper adds only `Relaxed` op/byte counters. EXAMPLE-only harness module; lib + bin stay `forbid(unsafe_code)`. |
| `examples/support/mod.rs:78` | `unsafe fn alloc` | caller's `layout` contract forwarded | `System.alloc(layout)` unchanged; `layout.size()` is only read into the byte counter. |
| `examples/support/mod.rs:83` | `unsafe fn dealloc` | `ptr` came from this allocator with this `layout` | `System.dealloc(ptr, layout)` unchanged; holds because every `alloc`/`realloc` also forwarded to `System`. |
| `examples/support/mod.rs:86` | `unsafe fn realloc` | caller's `ptr`/`layout`/`new_size` contract forwarded | `System.realloc(ptr, layout, new_size)` unchanged; `new_size` is only read into the byte counter. |

### `sparq-lws-wasm` — 4 sites (the SHIPPING bounded `#[global_allocator]`) [SONNET-4.6]

(sq-wubkf.) The only **shipping** `#[global_allocator]` sites in the register. On `wasm32` the
whole Solid pod lives in one linear memory; when growth fails, the allocation-error handler aborts,
which under the release profile's `panic=abort` lowers to an `unreachable` trap that poisons the
instance and answers the request with nothing. `memory::BoundedAlloc` keeps a running live-byte
total so `handleRequest` can refuse a request whose projected peak crosses a configured ceiling
with a clean HTTP 507 *before* the router runs. A `#[global_allocator]` unavoidably requires
`unsafe` (the `GlobalAlloc` trait is `unsafe` by definition, and there is no safe substitute for a
process-wide live-byte counter), so this crate alone relaxes its root from `forbid(unsafe_code)` to
`deny(unsafe_code)` plus a single `#[allow(unsafe_code)] pub mod memory;` — every other module in
the crate still fails to compile on `unsafe`.

**Not** a B5 (untrusted-input) surface, and the same forward-to-`System` class as the
`sparq-engine` / `sparq-lws-core` counting allocators above: every method delegates verbatim to
`System` with the identical arguments, so `System` discharges all of `GlobalAlloc`'s obligations.
The wrapper only reads `Layout::size()`/`new_size` and bumps `Relaxed` atomics; no pointer `System`
returns is ever dereferenced, retained, or aliased, and it **never refuses an allocation** (a null
return from `GlobalAlloc::alloc` routes into `handle_alloc_error` → abort, i.e. exactly the trap
this module removes — the bound is enforced at the request boundary instead, the only layer where a
refusal can be an HTTP status). `alloc_zeroed` is deliberately left as the trait default, which
routes through `BoundedAlloc::alloc` and is therefore accounted without a fourth forwarding method.
Bounded by **review + the trivial forward-to-`System` argument**, the crate-root
`clippy::undocumented_unsafe_blocks` (the `unsafe impl` carries a `// SAFETY:` comment), and the
host-side unit tests in `src/memory.rs` that cover the accounting and ceiling arithmetic on the
standard toolchain. Miri does not model a `#[global_allocator]` that calls `System`.

| File:line | Kind | Invariant relied on | Why sound / how bounded |
|---|---|---|---|
| `src/memory.rs:144` | `unsafe impl GlobalAlloc for BoundedAlloc` | forward-to-`System` | every method delegates verbatim to `System` with the same args; the wrapper adds only `Relaxed` live/peak counter updates and never alters the returned pointer or the requested layout. |
| `src/memory.rs:145` | `unsafe fn alloc` | caller's `layout` contract forwarded | `System.alloc(layout)` unchanged; only the returned pointer's nullness is inspected before recording `layout.size()`, so a failed allocation is not counted as live. |
| `src/memory.rs:153` | `unsafe fn dealloc` | `ptr` came from this allocator with this `layout` | `System.dealloc(ptr, layout)` unchanged; holds because every `alloc`/`realloc` also forwarded to `System`. The counter decrement is balanced by construction — the allocator is installed before the first allocation, so no pointer predates it. |
| `src/memory.rs:158` | `unsafe fn realloc` | caller's `ptr`/`layout`/`new_size` contract forwarded | `System.realloc(ptr, layout, new_size)` unchanged; only the returned pointer's nullness is inspected before recording the `layout.size()` → `new_size` delta, so a failed realloc leaves the old size counted (which it still is). |

## NEEDS-REVIEW

**None.** Every one of the 92 sites now carries a literal `// SAFETY:` comment
immediately preceding the `unsafe` block/impl, mechanically enforced by
`clippy::undocumented_unsafe_blocks` (MS-G2 closed, sq-8wbn, [OPUS-4.8]) — set
at crate root on unsafe-bearing libraries (including `sparq-lws-wasm`, sq-wubkf)
and file-local on the `sparq-engine`
integration test and the `sparq-lws-core` example files that carry counting
allocators. The 6 sites that
previously relied on an adjacent block comment — the two `unsafe impl Send`/`Sync for
SlotPtr` pairs (`dict.rs` + `dictspill.rs`), the `MmapMut` `from_raw_parts_mut` view, and
the test `remove_var` — were reworded so the `// SAFETY:` token sits on the line directly
above each `unsafe`; the `from_utf8_unchecked` TRUSTED fast path already led with the
token. Should a future site lack any argument, mark it `NEEDS-REVIEW` here and open a bead
(`bd create`) rather than fabricating a justification — do **not** re-seed the snapshot
over an unjustified `unsafe` (and clippy will now also reject it at compile time).

## Ratchet — how this stays honest

`scripts/unsafe-gate.py` counts the first-party `unsafe` sites per crate (the same
comment-/string-stripped keyword scan used to build this table) and compares against the
committed snapshot `bench/unsafe-snapshot.json`:

```sh
scripts/unsafe-gate.py --check        # FAIL if any crate rose above its snapshot
scripts/unsafe-gate.py --seed         # re-seed AFTER a reviewed register update
scripts/unsafe-gate.py --list         # file:line:text of every counted site
```

The **`unsafe-register (count ratchet)`** CI lane (`.github/workflows/ci.yml`) runs
`--check` on every PR and merge-queue ref. Because it does **not** contain the word
"advisory"/"informational", the `ci-summary / gate` aggregator treats it as a
**required** (gating) check — distinct from the pre-existing non-gating
`unsafe report (cargo-geiger, informational)` lane, which stays as a visibility-only
report. A PR that adds an `unsafe` site therefore fails CI until the author adds a
register row here, a `// SAFETY:` comment in source, and re-seeds the snapshot — all
three changes land in the same reviewable diff.
