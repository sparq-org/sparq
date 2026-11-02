<!-- [OPUS-4.8] sq-toze — memsafety gap register. Honest open gaps + remediation beads.
     Re-review when Fable returns. -->

# Memory-safety attestation — gap register

Open gaps for the memsafety framework, with severity, remediation, and the `bd` bead that
tracks the fix. The headline GX-5 gap (no unsafe register + cargo-geiger informational-only)
from `research/production-certification-plan.md` is **CLOSED** by this framework's work
(register + gating ratchet — see below). The remaining gaps are the honest residue: a
documentation overclaim, one deferred deeper-coverage lane (MS-G4), a formal-verification
ceiling. **MS-G3 (standalone ASan over the corruption corpus) is
now CLOSED** by `.github/workflows/asan.yml` (sq-hybl). Former cross-doc count drift
(MS-G5) is also closed: the threat model and register both record 52 core sites.

> `bd` is not available in this isolated worktree, so beads are listed here for the
> orchestrator to create (`bd create … --epic sq-toze`); they are NOT hand-edited into
> `.beads/`. Each remediation names the exact change.

## CLOSED by this framework (was GX-5 — P0)

| ID | What | Evidence it is closed |
|---|---|---|
| GX-5a | No per-site unsafe justification register or mechanical site correspondence | `compliance/memsafety/unsafe-register.md` records the safety arguments. `python3 scripts/unsafe-gate.py --verify-register` requires exactly one row per live site from `--list` and rejects every stale File:line cell, missing row and duplicate row. Descriptions and safety arguments remain reviewed. `--self-test` exercises those failures and requires the real register to pass. |
| GX-5b | cargo-geiger informational only (no failing ratchet) | `scripts/unsafe-gate.py --check` + `bench/unsafe-snapshot.json` bound per-crate counts; `--verify-register` checks site correspondence even when counts are unchanged. Both commands and `--self-test` run without `continue-on-error` in the `unsafe-register (count ratchet)` job in `ci.yml` (nightly + dispatch, not a required PR merge check). cargo-geiger stays a separate visibility lane; the cited commands determine live compliance. |
| GX-5c | B5 mmap sites "not attested in one place" | The coverage matrix in `controls.md` + the oracle (`mmap_corruption_oracle.rs`) + fuzz (`graph_open.rs`) citations attest each B5 site. |
| **MS-G2** | First-party `clippy::undocumented_unsafe_blocks` lint missing (the `// SAFETY:` token was enforced by review + register, not mechanically) | **CLOSED (sq-8wbn, [OPUS-4.8]).** `#![warn(clippy::undocumented_unsafe_blocks)]` is a crate-root attribute on unsafe-bearing libraries, including `sparq-engine`; unsafe-bearing test/example crates set it locally. The 6 sites the lint originally flagged were normalised to a literal `// SAFETY:` comment immediately preceding each `unsafe` block/impl (the two `SlotPtr` `Send`/`Sync` pairs in `dict.rs`/`dictspill.rs`, plus `lib.rs` `from_raw_parts_mut` over `MmapMut` and the test `remove_var`). The existing `clippy --all-targets -D warnings` gate mechanically rejects any new undocumented `unsafe`. |

## OPEN gaps

| ID | Gap | Sev | Remediation | Bead (to create) |
|---|---|---|---|---|
| ~~MS-G3~~ | ~~No standalone AddressSanitizer lane outside cargo-fuzz.~~ | ~~Low~~ | **CLOSED (sq-hybl, [OPUS-4.8]).** Added `.github/workflows/asan.yml` — a standalone ASan lane that runs the deterministic mmap corruption corpus (`crates/sparq-core/tests/mmap_corruption_oracle.rs` under `--features mmap,dict-spill`, plus the `sparq-vectors` `store`/`diskann` open-validation corpus) under `RUSTFLAGS="-Zsanitizer=address" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu`. So the B5 reads now execute under ASan against the *deterministic* corpus, not only as deep as the fuzz corpus reaches. **Nightly schedule + workflow_dispatch only** (no PR/merge_group trigger — `-Zbuild-std` rebuilds std under ASan, many minutes), so it is a non-blocking UB safety net (cf. miri.yml / the nightly fuzz tier) and the `ci-summary / gate` aggregator never discovers/waits on it; the job name also carries the `informational` token belt-and-braces. | — (done) |
| **MS-G4** | **No formal verification / model checking of the unsafe core.** Assurance is Miri + oracle + fuzz + per-site argument (strong *testing/justification*) but not a *proof* of the mmap validators (`MappedDict::validate`, `VectorStore::open_validated`, DiskANN `open`). | **Low** (assurance ceiling, not a defect) | **PARTLY ADDRESSED (sq-hkud, [OPUS-4.8]) — see the feasibility verdict below.** Kani is a *tractable + worthwhile* fit for the `.spqv` validator and a bounded proof now exists for it; the dict mmap validator needs a refactor first (scoped as follow-up). | sq-hkud (this work) + a follow-up bead for the dict-validator seam |
| ~~MS-G5~~ | ~~Cross-doc unsafe-count drift.~~ | ~~Low~~ | **CLOSED.** `research/threat-model.md` (scope table and B5 section), the register and the regenerated snapshot all record **52** `sparq-core` sites, including Windows `publish_rename` / `MoveFileExW`. The per-crate ratchet confirms the live count matches; the register remains the authoritative per-site inventory. | — (done) |

## MS-G4 — Kani feasibility verdict (sq-hkud, [OPUS-4.8])

MS-G4 was an *evaluate-then-deliver* task: is a Kani bounded-proof of the mmap on-disk-format
validators feasible and worthwhile against the existing Miri + oracle + fuzz + ASan coverage?

**Verdict: tractable and worthwhile for the `.spqv` (vectors) validator — a bounded proof
now exists for it; deferred-with-a-prerequisite for the dict mmap validator.**

What Kani adds over the existing coverage. The Miri (UB), oracle (deterministic corruption
sweep), fuzz (libFuzzer corpus) and ASan (sanitised corpus) lanes are all *execution-based*:
they run the validator on **specific** inputs. None **proves** the validator is panic-/OOB-
free for **every** hostile input. Kani model-checks the validator over **every** byte buffer
up to a small bound, closing the residual "did the corpus miss a hostile header/index?" gap.
It is a **complement, not a replacement** — Kani's bound is small (model checking is
exponential), so fuzz/oracle/ASan still carry the unbounded + UB coverage.

Why the `.spqv` validator is a good Kani target.
`VectorStore::open_from_bytes` (`crates/sparq-vectors/src/store.rs`) is the **pure,
mmap-free, FFI-free, syscall-free** in-memory twin of the mmap'd `open` — both call the same
private `open_validated` header/length/bounds logic. Kani cannot model `mmap` / file I/O /
FFI, but it does not need to: the in-memory entry point runs the identical validator. The
state space is **boundable** — the checked size arithmetic ties `count`/`dim` to the buffer
length, so a small symbolic buffer bounds every loop and the `vec![false; count]`
allocation. Delivered: two `#[cfg(kani)]` harnesses (`open_from_bytes_never_panics`,
`open_validated_v2_tail_never_panics`) + the nightly, non-blocking `kani` CI lane
(`.github/workflows/kani.yml`, same posture as `miri.yml`/`asan.yml`). **UNTESTED-HERE:**
Kani was not installed in the authoring environment; the harness is written to Kani's
documented API and is validated on the lane's first run.

Why the dict mmap validator (`MappedDict::validate`, `dict.rs`) is deferred — not because
Kani is the wrong tool, but because it has **no mmap-free public entry point**: it is
reachable only via `Dict::open_mmap`, which needs a live `memmap2::Mmap` (Kani cannot model
file-backed mappings — the same structural reason Miri cannot, see `miri.yml`), and it pulls
in the term-record parser + the prefix/datatype tables + a `Vec` of collected triples. A
Kani proof of it first needs a refactor to lift `validate` onto a plain `&[u8]` (an
`open_from_bytes`-style seam, mirroring what made the vectors validator tractable). That is
**follow-up work**, recorded as a bead for the orchestrator to create.

> **Bead to create** (`bd create … --epic sq-toze`): *"memsafety: lift `MappedDict::validate`
> onto a plain `&[u8]` seam so a Kani bounded-proof can cover the dict mmap validator
> (follow-up to sq-hkud / MS-G4)"* — P2.

The certificate-grade *external* formal-methods review + accredited third-party B5 audit
remain external by definition; the AUDIT-READY label stands.

## NOT gaps (decisions, recorded so the auditor does not re-flag them)

- **Miri does not gate per-PR.** Intentional — Miri is ~100× native; it is a nightly UB
  safety net (like fuzz-nightly / zk-toolchain). `clippy -D warnings` (MS-10) runs per-PR;
  the ratchet (MS-3) runs nightly + dispatch. Not a gap.
- **Miri does not run mmap-backed or dict-spill sites.** Structural (Miri rejects
  file-backed mappings) — covered by the oracle + fuzz+ASan instead. Documented, not a gap.
- **Third-party `unsafe` (memmap2/libc/rayon/hdt) is not in this register.** Scoped to the
  supply-chain lane (cargo-deny, SBOM/VEX). Correct separation of concerns, not a gap.

## Honest overall posture

The memsafety framework confines the unsafe surface, records per-site justifications,
and checks counts and live site correspondence in the nightly/dispatch ratchet job.
The verifier determines current register coverage; equal counts alone do not establish it.
The Miri+oracle+fuzz+ASan matrix handles the B5 boundary explicitly. The
open gaps are **not** missing safety — they are (a) [CLOSED] the first-party
`undocumented_unsafe_blocks` lint, now enforced on unsafe-bearing libraries and locally on
unsafe-bearing test/example crates (MS-G2, sq-8wbn),
(b) one remaining *deeper-assurance* lane that is deferred-but-tracked (MS-G4 formal proof —
MS-G3 ASan-standalone is now CLOSED, sq-hybl), and (c) [CLOSED] count drift in a neighbouring
doc (MS-G5). None of these
(b') [CLOSED] the standalone ASan lane over the corruption corpus (MS-G3, sq-hybl —
`asan.yml`),
contradicts the safety claim; the certificate-grade ceiling (formal proof + external audit) is
external by definition and labelled AUDIT-READY.
