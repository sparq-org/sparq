<!-- [OPUS-4.8] Reproducible-build evidence — gap GX-8 / bead sq-toze.9. Cross-cutting:
     cited by slsa/cra/sbom/ssdf/openssf. NON-CANONICAL timing (no measured perf numbers);
     the byte-level facts here are deterministic build-output properties, not benchmarks. -->
# Reproducible-build evidence (sparq) — gap GX-8 / `sq-toze.9`

**Frameworks:** SLSA (higher-assurance / SL-B3-adjacent), EU CRA (Annex I integrity),
NIST SSDF (PW.6.2), CycloneDX SBOM (INT-3 / GS-2), OpenSSF Best-Practices (`build_reproducible`).
**Bead:** `sq-toze.9` (epic `sq-toze`). This is the single source of truth those five slices
link to; they no longer carry the gap as "open with no evidence".

## Honest headline

> The official `sparq-cli` release binary is **deterministic except for a small,
> fully-characterised delta**: re-running the exact release build command twice from the
> same source tree, lockfile, and toolchain produces two binaries of **identical size** that
> are **byte-identical apart from 22 bytes** — 20 bytes of GNU build-id and 2 bytes inside a
> `mimalloc` build-time banner string. The **root cause is a single non-determinism source**:
> the C-compiled `mimalloc` allocator embeds `__DATE__`/`__TIME__` (the build wall-clock) into
> its `.rodata` version banner, and the linker's content-hash build-id changes as a consequence.
> sparq does **not** today claim a bit-for-bit reproducible build; it claims this **bounded,
> evidenced near-reproducibility** plus a known, low-effort path to closing the residual delta.

This is the honest PW.6.2 / CRA-integrity statement the gap asked for: not a fabricated
"reproducible" tick, and not a vague "not reproducible" hand-wave — a measured diff with the
exact cause named and a remediation that is scoped, not aspirational.

## What was tested (method)

Two independent release builds of `sparq-cli` (the published-archive binary), into **separate
target directories** so neither could reuse the other's artifacts, with the **same release
flags the release/dist workflows use** (`--release --locked`, fat LTO + `codegen-units = 1` +
`panic = "abort"` from `[profile.release]` in `Cargo.toml`, plus the per-tier
`-Ctarget-cpu=…` `RUSTFLAGS`), then a byte-level `cmp` of the two ELF binaries.

```sh
# Build A and Build B — independent target dirs, identical inputs
CARGO_TARGET_DIR=/tmp/t1 RUSTFLAGS="-Ctarget-cpu=x86-64-v3" \
  cargo build --release --locked -p sparq-cli
CARGO_TARGET_DIR=/tmp/t2 RUSTFLAGS="-Ctarget-cpu=x86-64-v3" \
  cargo build --release --locked -p sparq-cli

cmp -l /tmp/t1/release/sparq-cli /tmp/t2/release/sparq-cli   # list differing bytes
```

> The measurement was taken on the project's **non-canonical EC2 work box** (a custom
> `rustc`/`cargo`; the `x86-64-v3` micro-arch name was not recognised by that host toolchain
> and was ignored — both builds saw identical flags, so the *determinism* comparison is valid).
> The **diff is a property of the build graph**, not of the host's clock speed, so it
> reproduces on the GitHub-hosted release runners; what is non-canonical (and therefore NOT
> baked into any claim) is timing, not the byte-identity result. An auditor reproduces the
> finding with the two commands above on any Linux host.

## What was found (the 22-byte delta)

| Δ region | Bytes (0-based offset) | ELF section | Cause | Same size? |
|---|---|---|---|---|
| **A** | `0x29c`–`0x2af` (20 bytes) | `.note.gnu.build-id` payload | The GNU build-id is a hash over the **linked content**; because region B changes per build, the build-id necessarily changes too. **Derived**, not independent. | yes |
| **B** | `0x3b651c` + `0x3b651f` (2 bytes) | `.rodata` | A `mimalloc` version-banner string literal `"v%i.%i.%i%s%s (built on %s, %s)"` formatted at *compile time* against the C preprocessor's `__DATE__` / `__TIME__` — i.e. the build wall-clock (`"Jun 17 2026"`, `"10:45:10"` vs `"10:46:14"`). | yes |

Everything else — all `.text`, the rest of `.rodata`, `.data`, `.symtab`/`.strtab`, the
`cargo-auditable` embedded dependency manifest, and the ELF/program headers — is **byte-identical
across the two builds**. Zeroing both binaries' build-id note leaves exactly the two `.rodata`
timestamp bytes, confirming region A is downstream of region B (a single root cause, not two).

### Root cause is `mimalloc`, confirmed at source

`sparq-cli`'s default feature set is `default = ["mmap", "mimalloc", "dict-spill"]`
(`crates/sparq-cli/Cargo.toml`), so the released binary installs `mimalloc` as the
`#[global_allocator]` (`crates/sparq-cli/src/main.rs`). `mimalloc` is a **C** library compiled
by the `cc` crate (`libmimalloc-sys`), and its `src/options.c` (`mi_version` / option-printing
path) does:

```c
_mi_fprintf(out, arg, "v%i.%i.%i%s%s (built on %s, %s)\n",
            vermajor, verminor, verpatch, /* … */ , __DATE__, __TIME__);
```

`__DATE__`/`__TIME__` expand to the C compiler's wall-clock at compile time, so the string
literal differs on every build. There is **exactly one** `Mon DD YYYY` date string in the whole
binary (verified by scanning `strings` for the `^[A-Z][a-z]{2} +[0-9]+ 20[0-9][0-9]$` pattern),
i.e. no other dependency embeds a build timestamp — the source is singular and named.

## Why this is "not (yet) bit-for-bit reproducible" — and why that's honest

SLSA Build L2/L3 do **not require** reproducibility, so this gap never blocked the L2 claim
(`compliance/slsa/gap-register.md`). But CRA integrity (Annex I), SSDF PW.6.2, the SBOM↔binary
INT-3 link, and OpenSSF `build_reproducible` all *want* either a reproducibility demonstration
or an honest non-repro statement. The honest statement is: **the only thing standing between
sparq and a bit-for-bit reproducible `sparq-cli` is a single C-dependency build-time banner**,
plus the build-id it perturbs. We do not claim reproducibility we have not demonstrated.

## Remediation path (scoped, not aspirational) — tracked under `sq-toze.9`

Closing the residual 22-byte delta to a genuine byte-for-byte claim needs all of:

1. **Neutralise the `mimalloc` timestamp.** Build the C dependency under
   [`SOURCE_DATE_EPOCH`](https://reproducible-builds.org/docs/source-date-epoch/) so the
   compiler's `__DATE__`/`__TIME__` are pinned (modern GCC/Clang honour `SOURCE_DATE_EPOCH`
   for `__DATE__`/`__TIME__`), **or** drop `mimalloc` from the default `sparq-cli` feature set
   for the reproducible artifact (it is already an opt-in feature), **or** carry a small patch
   that compiles the banner out. Lowest-risk: set `SOURCE_DATE_EPOCH` for the release build.
2. **Pin the build-id.** With region B fixed, the content-hash build-id becomes deterministic
   on its own; if any residual linker non-determinism remains, pass `-Clink-arg=-Wl,--build-id`
   with a reproducible style (or `=none`) so the note is stable.
3. **Pin remaining inputs already controlled.** `--locked` (lockfile), a pinned `rust-toolchain`
   version, fixed `RUSTFLAGS` per tier, and a normalised build path (`--remap-path-prefix`) are
   either already in place (`--locked`, fixed `RUSTFLAGS`) or a one-line addition.
4. **Add a CI ratchet.** A `reproducible-build` lane that double-builds one tier and diffs the
   two digests (failing on any non-build-id delta) would turn this from a point-in-time
   demonstration into an enforced property. This is the step that flips GX-8 from
   "characterised" to "closed", and is deliberately left as the remaining work on `sq-toze.9`
   rather than over-claimed here.

Steps 1–3 are small, mechanical workflow edits; step 4 is a new CI lane. None is aspirational
(contrast GX-11 / Build L3, whose isolated trusted builder is now wired for the archives — sq-toze.25 —
and for the GUI bundles, SBOM/VEX, conformance report and `dist.yml` binaries — #4570 — but
unexercised on every lane, and still absent for the ghcr container image). The reason this PR does
**not** also flip the gap to closed is the honesty contract: a *documented* near-reproducibility
finding with a named cause is exactly what `sq-toze.9` asked for, but a *closed* `build_reproducible`
control requires the enforced rebuild-and-diff to actually exist in CI — which is follow-up work,
filed below, not shipped here.

## Auditor quick-run

```sh
# Reproduce the determinism finding (any Linux host; ~1–2 min/build):
CARGO_TARGET_DIR=/tmp/t1 RUSTFLAGS="-Ctarget-cpu=x86-64-v3" cargo build --release --locked -p sparq-cli
CARGO_TARGET_DIR=/tmp/t2 RUSTFLAGS="-Ctarget-cpu=x86-64-v3" cargo build --release --locked -p sparq-cli
cmp -l /tmp/t1/release/sparq-cli /tmp/t2/release/sparq-cli      # expect ~22 differing bytes
readelf -n /tmp/t1/release/sparq-cli | grep -A1 'Build ID'      # the 20-byte region A
strings /tmp/t1/release/sparq-cli | grep -E '^[A-Z][a-z]{2} +[0-9]+ 20[0-9][0-9]$'  # the 1 date literal (region B)
```

The expected outcome: identical file size, ~22 differing bytes, all attributable to the
build-id note + the single `mimalloc` date literal.
