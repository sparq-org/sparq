//! [OPUS-4.8] (sq-lfo84) **Explicit-SIMD distance kernels** for the ANN hot loop.
//!
//! The HNSW graph build (`instant-distance`) and search evaluate the squared-Euclidean
//! distance between unit vectors *millions* of times; that inner loop is the dominant cost of
//! both build and query (the sq-z2z18 sweep root-caused the QPS/build-time gap vs hnswlib to
//! `instant-distance` shipping only a **scalar** distance kernel). This module replaces the
//! scalar reduction on the HNSW path with a hand-written, runtime-detected SIMD kernel —
//! **NEON** on `aarch64`, **AVX2 + FMA** on `x86_64` — with a scalar fallback that is
//! **bit-identical to the previous 8-lane auto-vectorised loop** so hardware without the
//! target feature (and the deterministic-gated exact/Vamana/PQ paths, which do not use this
//! module) are numerically unchanged.
//!
//! **No new dependency.** The kernels are `core::arch` intrinsics behind `#[cfg]` — the
//! lean-core constraint holds (this module adds nothing to the dependency graph). It compiles
//! only under the opt-in `approx-ann` feature (the HNSW backend it serves), so the default
//! `sparq-vectors` build carries zero SIMD code.
//!
//! **Safety.** The intrinsic kernels are `unsafe` (raw pointer loads, `#[target_feature]`).
//! Each is entered **only** after a runtime `is_*_feature_detected!` check confirms the ISA
//! extension is present, and reads exactly `a.len()` lanes with a scalar tail for the
//! remainder — no out-of-bounds access. The public [`l2_sq_dist`] wrapper is safe.
//!
//! **Non-vacuity.** Every numeric guard in this module is arch-GENERIC — it calls the
//! dispatcher and checks it against an f64 reference, which the *scalar fallback* satisfies
//! just as well as an intrinsic kernel does. So a host missing the ISA extension runs the whole
//! suite green with the kernel never executed. `active_kernel` states the dispatch decision
//! once so a test can assert WHICH kernel ran, and `simd::tests` fails closed on an x86_64 host
//! that would leave `l2_sq_avx2` unexecuted (`SPARQ_VECTORS_REQUIRE_SIMD` overrides the arming).
//! [SONNET-4.6] #5065. The NEON half is #5028, closed out-of-tree instead: the
//! `.github/workflows/vectors-aarch64.yml` lane supplies a real aarch64 host and fails closed on
//! a missing `asimd`. There is no in-tree NEON equivalent of the x86_64 guard below.

/// Which distance kernel `l2_sq_dist` dispatches to on this host.
///
/// Only the variants reachable on the target arch exist, so there is no unconstructible variant
/// to `allow(dead_code)` away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_arch = "aarch64", target_arch = "x86_64")), allow(dead_code))]
pub(crate) enum Kernel {
    /// The NEON kernel, `l2_sq_neon`.
    #[cfg(target_arch = "aarch64")]
    Neon,
    /// The AVX2 + FMA kernel, `l2_sq_avx2`.
    #[cfg(target_arch = "x86_64")]
    Avx2,
    /// The portable fallback, `l2_sq_scalar`.
    Scalar,
}

/// The kernel `l2_sq_dist` resolves to on this host, from runtime CPU feature detection.
///
/// **Single source of truth for the dispatch decision:** `l2_sq_dist` branches on this
/// function's answer, so a test asserting `active_kernel() == Kernel::Avx2` is asserting the
/// very condition that selects the kernel — it cannot drift into re-stating a *different*
/// predicate that happens to agree today (#5065).
///
/// Cost is unchanged: the `is_*_feature_detected!` macros memoise their answer in a
/// process-global after the first call, which is what the previous inline `if` already did.
/// On a target with no intrinsic kernel (e.g. `wasm32`) this always answers `Scalar` and
/// nothing branches on it — hence the `dead_code` allow, scoped to exactly those targets.
#[inline]
#[cfg_attr(not(any(target_arch = "aarch64", target_arch = "x86_64")), allow(dead_code))]
pub(crate) fn active_kernel() -> Kernel {
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            return Kernel::Neon;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
        {
            return Kernel::Avx2;
        }
    }
    Kernel::Scalar
}

/// Squared-Euclidean distance `Σ (aᵢ − bᵢ)²` — the metric the HNSW graph ranks with (unit
/// vectors, so it is rank-equivalent to cosine; see `ann`'s module docs). Dispatches to the
/// best available SIMD kernel at runtime, falling back to [`l2_sq_scalar`] when no supported
/// vector ISA is detected.
///
/// # Panics
/// Debug-asserts equal lengths; a length mismatch is a programming error (the store's vectors
/// and the query always share `dim`).
#[inline]
pub(crate) fn l2_sq_dist(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "l2_sq_dist over mismatched dims");
    #[cfg(target_arch = "aarch64")]
    {
        if active_kernel() == Kernel::Neon {
            // SAFETY: `active_kernel` answers `Neon` ONLY inside a successful runtime
            // `is_aarch64_feature_detected!("neon")`, so the ISA extension is present; the kernel
            // reads exactly `a.len()`/`b.len()` (equal by the debug_assert) f32 lanes via
            // `vld1q_f32` in 4-wide steps plus a scalar tail, so every load is in-bounds.
            return unsafe { l2_sq_neon(a, b) };
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if active_kernel() == Kernel::Avx2 {
            // SAFETY: `active_kernel` answers `Avx2` ONLY inside a successful runtime
            // `is_x86_feature_detected!("avx2") && …("fma")`, so both extensions are present; the
            // kernel reads exactly `a.len()` f32 lanes via `_mm256_loadu_ps` (unaligned load) in
            // 8-wide steps plus a scalar tail, so every load is in-bounds.
            return unsafe { l2_sq_avx2(a, b) };
        }
    }
    l2_sq_scalar(a, b)
}

/// The portable scalar reduction — **bit-identical** to the 8-lane accumulator loop the ANN
/// distance functions used before this module (so a non-SIMD target, or a future exact-path
/// adopter, gets exactly the previous numeric result). Eight independent accumulator lanes so
/// the compiler still auto-vectorises it where it can.
#[inline]
pub(crate) fn l2_sq_scalar(a: &[f32], b: &[f32]) -> f32 {
    const LANES: usize = 8;
    let mut acc = [0f32; LANES];
    let chunks = a.len() / LANES;
    for c in 0..chunks {
        for l in 0..LANES {
            let d = a[c * LANES + l] - b[c * LANES + l];
            acc[l] += d * d;
        }
    }
    for i in chunks * LANES..a.len() {
        let d = a[i] - b[i];
        acc[0] += d * d;
    }
    acc.iter().sum()
}

/// NEON squared-Euclidean distance. Four 128-bit accumulators (16 f32 lanes/iteration) with
/// fused multiply-add (`vfmaq_f32`), a 4-wide drain, and a scalar tail.
///
/// # Safety
/// The caller must have confirmed the `neon` feature is available at runtime, and `a` and `b`
/// must have equal length. Reads exactly `a.len()` lanes (no over-read).
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn l2_sq_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;
    let n = a.len();
    let (pa, pb) = (a.as_ptr(), b.as_ptr());
    let mut acc0 = vdupq_n_f32(0.0);
    let mut acc1 = vdupq_n_f32(0.0);
    let mut acc2 = vdupq_n_f32(0.0);
    let mut acc3 = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 16 <= n {
        let d0 = vsubq_f32(vld1q_f32(pa.add(i)), vld1q_f32(pb.add(i)));
        let d1 = vsubq_f32(vld1q_f32(pa.add(i + 4)), vld1q_f32(pb.add(i + 4)));
        let d2 = vsubq_f32(vld1q_f32(pa.add(i + 8)), vld1q_f32(pb.add(i + 8)));
        let d3 = vsubq_f32(vld1q_f32(pa.add(i + 12)), vld1q_f32(pb.add(i + 12)));
        acc0 = vfmaq_f32(acc0, d0, d0);
        acc1 = vfmaq_f32(acc1, d1, d1);
        acc2 = vfmaq_f32(acc2, d2, d2);
        acc3 = vfmaq_f32(acc3, d3, d3);
        i += 16;
    }
    while i + 4 <= n {
        let d = vsubq_f32(vld1q_f32(pa.add(i)), vld1q_f32(pb.add(i)));
        acc0 = vfmaq_f32(acc0, d, d);
        i += 4;
    }
    let mut sum = vaddvq_f32(vaddq_f32(vaddq_f32(acc0, acc1), vaddq_f32(acc2, acc3)));
    while i < n {
        // SAFETY: `i < n == a.len() == b.len()`, so both reads are in-bounds.
        let d = *a.get_unchecked(i) - *b.get_unchecked(i);
        sum += d * d;
        i += 1;
    }
    sum
}

/// AVX2 + FMA squared-Euclidean distance. Two 256-bit accumulators (16 f32 lanes/iteration)
/// with `_mm256_fmadd_ps`, an 8-wide drain, and a scalar tail.
///
/// # Safety
/// The caller must have confirmed both `avx2` and `fma` are available at runtime, and `a` and
/// `b` must have equal length. Uses unaligned loads and reads exactly `a.len()` lanes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn l2_sq_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    let n = a.len();
    let (pa, pb) = (a.as_ptr(), b.as_ptr());
    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 16 <= n {
        let d0 = _mm256_sub_ps(_mm256_loadu_ps(pa.add(i)), _mm256_loadu_ps(pb.add(i)));
        let d1 = _mm256_sub_ps(_mm256_loadu_ps(pa.add(i + 8)), _mm256_loadu_ps(pb.add(i + 8)));
        acc0 = _mm256_fmadd_ps(d0, d0, acc0);
        acc1 = _mm256_fmadd_ps(d1, d1, acc1);
        i += 16;
    }
    while i + 8 <= n {
        let d = _mm256_sub_ps(_mm256_loadu_ps(pa.add(i)), _mm256_loadu_ps(pb.add(i)));
        acc0 = _mm256_fmadd_ps(d, d, acc0);
        i += 8;
    }
    // Horizontal sum of the two 256-bit accumulators.
    let acc = _mm256_add_ps(acc0, acc1);
    let lo = _mm256_castps256_ps128(acc);
    let hi = _mm256_extractf128_ps(acc, 1);
    let mut s128 = _mm_add_ps(lo, hi);
    s128 = _mm_hadd_ps(s128, s128);
    s128 = _mm_hadd_ps(s128, s128);
    let mut sum = _mm_cvtss_f32(s128);
    while i < n {
        // SAFETY: `i < n == a.len() == b.len()`, so both reads are in-bounds.
        let d = *a.get_unchecked(i) - *b.get_unchecked(i);
        sum += d * d;
        i += 1;
    }
    sum
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "x86_64")]
    use super::{active_kernel, Kernel};
    use super::{l2_sq_dist, l2_sq_scalar};

    /// The reference squared distance — the mathematical definition, computed left-to-right in
    /// f64 so it is the highest-precision baseline the SIMD/scalar kernels are checked against.
    fn reference_l2_sq(a: &[f32], b: &[f32]) -> f64 {
        a.iter().zip(b).map(|(&x, &y)| ((x - y) as f64).powi(2)).sum()
    }

    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn randf(state: &mut u64) -> f32 {
        ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32) * 2.0 - 1.0
    }

    #[test]
    fn simd_matches_reference_across_all_dims_and_tails() {
        // The dispatched kernel (SIMD on this box, scalar elsewhere) must agree with the exact
        // f64 reference to within f32 rounding for EVERY length 0..=257 — this exercises the main
        // 16-lane body, the 4/8-wide drain, and the scalar tail on every remainder class.
        let mut st = 0xC0FF_EE00_u64;
        for dim in 0..=257usize {
            let a: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            let b: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            let got = l2_sq_dist(&a, &b);
            let refv = reference_l2_sq(&a, &b) as f32;
            // Relative tolerance scaled by dim (accumulation error grows with the term count).
            let tol = 1e-4 * (dim as f32).max(1.0);
            assert!(
                (got - refv).abs() <= tol,
                "dim {}: simd {} vs ref {} (tol {})",
                dim,
                got,
                refv,
                tol
            );
        }
    }

    #[test]
    fn scalar_fallback_is_nonnegative_and_zero_for_equal() {
        // Squared distance is >= 0 and exactly 0 for identical inputs (no rounding drift on the
        // zero difference), for both the dispatched kernel and the pinned scalar reduction.
        let mut st = 7u64;
        for dim in [1usize, 3, 8, 15, 16, 100, 128, 129] {
            let a: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            assert_eq!(l2_sq_dist(&a, &a), 0.0, "dim {}: self-distance is exactly 0", dim);
            assert_eq!(l2_sq_scalar(&a, &a), 0.0, "dim {}: scalar self-distance is exactly 0", dim);
            let b: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            assert!(l2_sq_dist(&a, &b) >= 0.0, "dim {}: distance is non-negative", dim);
        }
    }

    #[test]
    fn simd_and_scalar_kernels_agree_closely() {
        // On a SIMD box the dispatched kernel and the scalar fallback take DIFFERENT reduction
        // orders, so they need not be bit-identical — but they must agree to within f32 rounding.
        // (On a non-SIMD box they are the same code and this is trivially exact.)
        let mut st = 0xABCD_1234_u64;
        for dim in [16usize, 32, 96, 128, 200, 256] {
            let a: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            let b: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            let d = l2_sq_dist(&a, &b);
            let s = l2_sq_scalar(&a, &b);
            assert!((d - s).abs() <= 1e-4 * dim as f32, "dim {}: simd {} vs scalar {}", dim, d, s);
        }
    }

    /// Whether a host that leaves the arch's intrinsic kernel UNEXECUTED must fail this run
    /// rather than skip it.
    ///
    /// Armed by `CI` — GitHub Actions sets it on every lane, so the x86_64 test lanes fail
    /// closed with no edit to `ci.yml`'s shared nextest matrix (which is out of this crate's
    /// scope). `SPARQ_VECTORS_REQUIRE_SIMD` forces it either way: `1` to demand kernel
    /// execution on a dev box, `0` to accept scalar-only coverage on a runner that genuinely
    /// lacks the ISA extension.
    ///
    /// Never armed under Miri: its `is_x86_feature_detected!` answers from an interpreter shim,
    /// not the host CPU, so a `false` there says nothing about the real runner.
    #[cfg(target_arch = "x86_64")]
    fn simd_execution_is_required() -> bool {
        if cfg!(miri) {
            return false;
        }
        match std::env::var("SPARQ_VECTORS_REQUIRE_SIMD") {
            Ok(v) => !matches!(v.trim(), "" | "0" | "false"),
            Err(_) => std::env::var_os("CI").is_some(),
        }
    }

    /// [SONNET-4.6] #5065 — NON-VACUITY GUARD for the x86_64 AVX2+FMA kernel.
    ///
    /// The guards above are arch-generic: they check the dispatcher against an f64 reference,
    /// and `l2_sq_scalar` satisfies them exactly as well as `l2_sq_avx2` does. So on a host
    /// without AVX2+FMA the module goes green with the intrinsic kernel never executed — and
    /// the `l2_sq_avx2` row of `compliance/memsafety/unsafe-register.md`, whose evidence IS
    /// this module's numeric agreement, would be resting on a run that never touched it.
    /// #5028 closed that hole for NEON by giving the kernel a real aarch64 host plus a
    /// fail-closed `asimd` preflight; this is the x86_64 half, in-tree rather than in the
    /// shared CI matrix.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_kernel_is_the_one_the_dispatcher_actually_ran() {
        let kernel = active_kernel();
        if kernel != Kernel::Avx2 {
            assert!(
                !simd_execution_is_required(),
                "this x86_64 host does not advertise avx2+fma, so l2_sq_dist took the {:?} \
                 fallback and l2_sq_avx2 was NEVER EXECUTED — every simd::tests guard on this \
                 run, and the unsafe-register row that cites them, is vacuous for that kernel. \
                 Move the lane onto an AVX2-capable runner, or set SPARQ_VECTORS_REQUIRE_SIMD=0 \
                 to knowingly accept scalar-only coverage here.",
                kernel
            );
            eprintln!("skipping: this host has no avx2+fma, l2_sq_dist dispatches to {:?}", kernel);
            return;
        }
        // AVX2 is what the dispatcher selects, so these calls DO execute `l2_sq_avx2` — as did
        // every `l2_sq_dist` call in the tests above. Re-check its numerics on the remainder
        // classes that split across the 16-lane body, the 8-wide drain and the scalar tail.
        let mut st = 0x5065_u64;
        for dim in [0usize, 1, 7, 8, 9, 16, 17, 31, 32, 33, 128, 257] {
            let a: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            let b: Vec<f32> = (0..dim).map(|_| randf(&mut st)).collect();
            let got = l2_sq_dist(&a, &b);
            let refv = reference_l2_sq(&a, &b) as f32;
            let tol = 1e-4 * (dim as f32).max(1.0);
            assert!(
                (got - refv).abs() <= tol,
                "dim {}: avx2 {} vs ref {} (tol {})",
                dim,
                got,
                refv,
                tol
            );
        }
    }
}
